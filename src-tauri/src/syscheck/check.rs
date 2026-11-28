//! The system check: probe, try every engine build, benchmark the ranked models on each device
//! configuration the way they will be used, run the recorder safety test, and hand the measurements to
//! the rules in `wb-capability` (PLAN section 3).
//!
//! Order, so the quick check stays inside its 8 minutes: engines and hardware, a throw-away Vulkan
//! warm-up per graphics adapter, rolling speed per language (ranked models top-down, stopping at the
//! first that meets the bar), the hybrid test running at the same time as a 2-minute recorder safety
//! run, near-live on the CPU, the live stand-in on a discrete GPU, then the night model. A step that
//! would start after the bar has passed is left for the full check and the profile says `partial`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use wb_capability::{
    CheckInfo, CheckKind, DeviceConfig, GpuKind, HybridMeasurement, Lang, Measurement, Mode,
    Options, Profile, Rankings, RecorderSafety, SCHEMA_VERSION, Skipped, decide,
    estimated_vram_bytes, model_fits, model_fits_card,
};
use wb_machine::Machine;
use wb_models::{Cache, Manifest};
use wb_protocol::{BatchRequest, BenchMode, BenchRequest, Device, ModelRef, Settings, Source};

use super::bench::{self, BenchError, Cell};
use super::excerpt::{self, ExcerptError};
use super::probe::{self, Roots};
use super::safety::{SafetyError, SafetyRun};
use crate::engines::{self, EngineRow, EngineState};

/// The quick check must finish within this on the low-baseline laptop.
pub const QUICK_BAR_S: f64 = 480.0;

/// The quick check aims to end by this on the low-baseline laptop. Optional steps that would start too late
/// for it are left for the full check; `QUICK_BAR_S` stays the hard limit.
pub const QUICK_TARGET_S: f64 = 360.0;

/// The least time the hybrid and recorder safety step needs (the safety run lasts 120 s).
const HYBRID_SAFETY_MIN_S: f64 = 150.0;
const SAFETY_SECONDS: f64 = 120.0;
/// Seconds of audio the CPU engine decodes in each round of the hybrid test.
const HYBRID_CPU_CLIP_S: f64 = 10.0;
/// A bench run decodes its clip twice (a warm-up, then one timed run), so a round that did not end in its
/// time limit decoded less than half of what a single decode would have.
const HYBRID_DECODES_PER_RUN: f64 = 2.0;
/// A recorder safety run shorter than this proves nothing about load (the test lasts `SAFETY_SECONDS`).
const SAFETY_MIN_COUNTED_S: f64 = 110.0;
/// The most the full check waits for one CPU round of the hybrid test.
const HYBRID_CPU_LIMIT_FULL_S: u64 = 300;
/// A cell whose warm-up runs slower than this gets one timed run only ("impractical: single run").
const SINGLE_RUN_ABOVE_RTF: f64 = 2.0;
const VAD_ID: &str = "silero-v6.2.0";
/// The one Hinglish file the check measures, on every configuration (owner, 27 Nov 2026, option A after step 3c): q8_0, with
/// the small detector in front of it as the accepted pair on a discrete card. It finishes the night wherever f16 does and
/// is about twice as fast on a processor; f16 is not measured, not chosen and not in the pack. A file that was not measured
/// is never chosen.
const HINGLISH_CANDIDATE: &str = "hinglish-q8_0";

/// The Hinglish files the check measures on a configuration: q8_0 everywhere (and then its pair on a discrete card).
fn hinglish_candidates(_discrete: bool) -> &'static [&'static str] {
    &[HINGLISH_CANDIDATE]
}
/// The quick check measures the pair (a Hinglish model with a small detector in front of it) only when this many seconds
/// of its target are left.
const PAIR_QUICK_MIN_S: f64 = 90.0;
/// The quick check measures the Hinglish model only when this many seconds of its target are left.
const HINGLISH_QUICK_MIN_S: f64 = 120.0;
/// How many times slower than the rolling model the Hinglish model is at the least, from the Hindi cells of the Phase 2
/// speed tables (a large-v3-sized model against turbo q5_0 on the same device: 1.78 to 2.09). The quick check skips the
/// measurement when even this ratio puts the model over the bar; it never skips one that would have fitted.
const HINGLISH_MIN_SLOWDOWN: f64 = 1.5;

/// The optional steps that follow the recorder safety test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TailStep {
    Hinglish,
    NearLive,
    LiveStandin,
    Night,
}

impl TailStep {
    /// The name of the step in `Profile::skipped` (the Hinglish one is read by the verdict).
    fn step_name(self) -> &'static str {
        match self {
            TailStep::Hinglish => wb_capability::SKIPPED_HINGLISH,
            TailStep::NearLive => "near-live speed on the processor",
            TailStep::LiveStandin => "live stand-in on the graphics card",
            TailStep::Night => "night model speed",
        }
    }
}

/// The reason recorded for a step that has no result: the quick check's time cut it (the Full Check measures it), or it
/// began and did not complete on `config` (the Full Check would not change that). The verdict and the night mark read the
/// first word of it (`wb_capability::HinglishGap`).
fn not_measured_reason(cut_by_time: bool, config: &str) -> String {
    if cut_by_time {
        format!(
            "{}: the quick check's time ran out while this step was measured on {config}",
            wb_capability::LEFT_FOR_FULL_CHECK
        )
    } else {
        format!("{}{config}", wb_capability::SKIPPED_NOT_COMPLETED)
    }
}

/// The least time limit of a Hinglish cell in the quick check, however little of the time target is left.
const HINGLISH_CELL_MIN_S: f64 = 30.0;

/// The time limit of a Hinglish cell started `elapsed_s` seconds into a quick check.
fn hinglish_cell_cap_s(elapsed_s: f64) -> f64 {
    (QUICK_TARGET_S - elapsed_s).max(HINGLISH_CELL_MIN_S)
}

/// The order of the optional steps. The Hinglish model decides which model every English + Hindi session is decoded with
/// (a wrong choice writes Hindi speech as English text), so it comes before the steps that only add information. In the
/// pack proof runs of 28 Nov 2026 the mandatory steps took 187 s and near-live and the night model 67 s more; the Hinglish
/// step then had 106 s left of its 360 s target and was skipped, so a machine with a discrete card chose
/// large-v3-turbo-q5_0 for English + Hindi without the pair ever being measured.
const TAIL_ORDER: [TailStep; 4] = [
    TailStep::Hinglish,
    TailStep::NearLive,
    TailStep::LiveStandin,
    TailStep::Night,
];

#[derive(Debug, thiserror::Error)]
pub enum CheckError {
    #[error("The system check was cancelled.")]
    Cancelled,
    #[error("No speech engine was found next to the app.")]
    NoEngines,
    #[error("A bench clip is missing: {0}")]
    Clip(String),
    #[error("{0}")]
    Excerpt(#[from] ExcerptError),
}

/// One cell of the speed table to measure.
struct Spec<'a> {
    model: &'a str,
    lang: Lang,
    mode: Mode,
    config: DeviceConfig,
    clip: &'a str,
    excerpt_s: f64,
    standin: bool,
}

/// Where the check finds things and what it knows about this computer.
pub struct CheckEnv {
    pub engines_dir: PathBuf,
    pub models_dir: PathBuf,
    pub bench_dir: PathBuf,
    pub work_dir: PathBuf,
    pub model_cache: PathBuf,
    pub recordings_root: PathBuf,
    pub backup_root: Option<PathBuf>,
    pub app_version: String,
    pub machine: Machine,
    pub manifest: Manifest,
    pub manifest_hash: String,
    pub rankings: Rankings,
    pub options: Options,
    pub started_at: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Progress {
    pub phase: String,
    pub detail: String,
    pub elapsed_s: f64,
    pub cells_done: u32,
}

pub struct Outcome {
    pub profile: Profile,
    /// The check was cut short: `profile` holds the cells measured before that and is partial.
    pub cancelled: bool,
}

struct Run<'a> {
    env: &'a CheckEnv,
    kind: CheckKind,
    cancel: &'a AtomicBool,
    progress: &'a dyn Fn(Progress),
    started: Instant,
    cells: u32,
    rows: Vec<EngineRow>,
    cpu_variant: Option<(String, PathBuf)>,
    gpu_variant: Option<(String, PathBuf)>,
    profile: Profile,
    cache: Cache,
    excerpts: HashMap<String, PathBuf>,
    partial: bool,
    /// True while the Hinglish step runs: its cells then stop at the quick check's time target instead of running on to the
    /// 480 s bar, so that near-live, the live stand-in and the night model, which come after it, are not starved.
    hinglish_running: bool,
    /// Set when a measurement of the step in hand was cut by the quick check's time (its limit, or less than 15 s left); the
    /// step then records "left for the full check", not "did not complete". Each optional step clears it before it starts.
    cut_by_time: bool,
}

/// Stops a safety run that is cut short and deletes its test recording; its result is not used.
fn discard_safety(run: Result<SafetyRun, SafetyError>) {
    if let Ok(r) = run {
        let _ = r.finish();
    }
}

fn clip_file(lang: &str) -> String {
    format!("bench-{lang}.wav")
}

pub fn run(
    env: &CheckEnv,
    kind: CheckKind,
    cancel: &AtomicBool,
    progress: &dyn Fn(Progress),
) -> Result<Outcome, CheckError> {
    let started = Instant::now();
    std::fs::create_dir_all(&env.work_dir).map_err(|e| CheckError::Clip(e.to_string()))?;
    let discovered = engines::discover(&env.engines_dir);
    if discovered.is_empty() {
        return Err(CheckError::NoEngines);
    }
    let mut run = Run {
        env,
        kind,
        cancel,
        progress,
        started,
        cells: 0,
        rows: Vec::new(),
        cpu_variant: None,
        gpu_variant: None,
        profile: empty_profile(env, kind),
        cache: Cache::load(&env.model_cache),
        excerpts: HashMap::new(),
        partial: false,
        hinglish_running: false,
        cut_by_time: false,
    };

    // 1. Engines and hardware.
    for (variant, exe) in &discovered {
        run.tick("Checking the speech engines", &format!("{variant} build"))?;
        run.rows
            .push(engines::probe_engine(variant, exe, &env.work_dir));
    }
    let usable = |v: &str| {
        discovered
            .iter()
            .find(|(n, _)| n == v)
            .filter(|(n, _)| {
                run.rows
                    .iter()
                    .any(|r| &r.variant == n && matches!(r.state, EngineState::Ok))
            })
            .cloned()
    };
    run.cpu_variant = usable("avx2").or_else(|| usable("avx512"));
    run.gpu_variant = usable("vulkan");
    run.tick("Reading the hardware", "")?;
    let roots = Roots {
        recordings: &env.recordings_root,
        backup: env.backup_root.as_deref(),
    };
    let (hardware, engine_status) = probe::hardware(&env.machine, &run.rows, &roots);
    run.profile.hardware = hardware;
    run.profile.engines = engine_status;
    run.profile.injectors = probe::running_injectors();
    run.profile.fingerprint = probe::fingerprint(
        &run.profile.hardware,
        &env.app_version,
        &env.manifest_hash,
        env.machine.emulated.as_deref(),
    );
    if run.gpu_variant.is_none() {
        for g in &mut run.profile.hardware.gpus {
            g.vulkan_usable = false;
        }
    }

    // A cancelled check (the operator pressed Start, or Cancel) keeps the cells measured so far as a partial
    // profile; the caller saves it only when it holds a measurement.
    let mut cancelled = false;
    if run.cpu_variant.is_some() || run.gpu_variant.is_some() {
        match run.measure_all() {
            Err(CheckError::Cancelled) => {
                cancelled = true;
                run.partial = true;
            }
            other => other?,
        }
    }

    run.profile.check.duration_s = started.elapsed().as_secs_f64();
    run.profile.check.partial = run.partial;
    Ok(Outcome {
        profile: run.profile,
        cancelled,
    })
}

fn empty_profile(env: &CheckEnv, kind: CheckKind) -> Profile {
    use wb_capability::{Fingerprint, Hardware};
    Profile {
        schema_version: SCHEMA_VERSION,
        app_version: env.app_version.clone(),
        created_at: env.started_at.clone(),
        emulated: env.machine.emulated.clone(),
        fingerprint: Fingerprint {
            os: String::new(),
            cpu_brand: String::new(),
            logical_processors: 0,
            physical_cores: 0,
            total_ram_gib: 0,
            gpus: vec![],
            app_version: env.app_version.clone(),
            manifest_hash: env.manifest_hash.clone(),
            emulated: env.machine.emulated.clone(),
        },
        check: CheckInfo {
            kind,
            partial: false,
            started_at: env.started_at.clone(),
            duration_s: 0.0,
            bar_s: QUICK_BAR_S,
            excerpt_seconds: excerpt_seconds(kind),
        },
        hardware: Hardware {
            os: String::new(),
            cpu_brand: String::new(),
            logical_processors: 0,
            physical_cores: 0,
            cpu_features: vec![],
            avx2: false,
            avx512: false,
            total_ram_bytes: 0,
            available_ram_bytes: 0,
            gpus: vec![],
            disks: vec![],
            on_mains_power: None,
            audio_inputs: 0,
        },
        engines: vec![],
        measurements: vec![],
        hybrid: None,
        recorder_safety: vec![],
        injectors: vec![],
        skipped: vec![],
    }
}

/// Seconds of each bundled clip that are timed: 30 in the quick check, 60 (the whole clip) in the full one.
/// The quick check times its deciding cells on the whole clip, see `cell_excerpt_seconds`.
pub fn excerpt_seconds(kind: CheckKind) -> f64 {
    match kind {
        CheckKind::Quick => 30.0,
        CheckKind::Full => 60.0,
    }
}

/// Seconds of the whole bundled clip (the clips are 60 s long).
const WHOLE_CLIP_S: f64 = 60.0;

/// The excerpt of one rolling cell. The quick check times the deciding cells, the first-choice rolling model of
/// Hindi and Urdu on a graphics adapter, on the whole 60 s clip (owner's decision of 14 Nov 2026: the first 30 s
/// of those clips overstate the speed by 14 to 38% and decide the verdict); every other quick-check cell keeps
/// its 30 s excerpt, and the full check times everything on the whole clip.
fn cell_excerpt_seconds(kind: CheckKind, lang: Lang, config: DeviceConfig, rank: usize) -> f64 {
    let deciding = kind == CheckKind::Quick
        && rank == 0
        && matches!(lang, Lang::Hi | Lang::Ur)
        && matches!(config, DeviceConfig::Gpu { .. });
    if deciding {
        WHOLE_CLIP_S
    } else {
        excerpt_seconds(kind)
    }
}

impl Run<'_> {
    fn tick(&mut self, phase: &str, detail: &str) -> Result<(), CheckError> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(CheckError::Cancelled);
        }
        (self.progress)(Progress {
            phase: phase.to_string(),
            detail: detail.to_string(),
            elapsed_s: self.started.elapsed().as_secs_f64(),
            cells_done: self.cells,
        });
        Ok(())
    }

    fn over_budget(&mut self) -> bool {
        let over =
            self.kind == CheckKind::Quick && self.started.elapsed().as_secs_f64() >= QUICK_BAR_S;
        if over {
            self.partial = true;
        }
        over
    }

    /// Seconds of the quick-check budget left (None for the full check, which has no budget).
    fn remaining_s(&self) -> Option<f64> {
        (self.kind == CheckKind::Quick).then(|| QUICK_BAR_S - self.started.elapsed().as_secs_f64())
    }

    /// The time limit of a cell that runs beside the recorder safety run. In the quick check it ends with the
    /// safety run (plus a minute) and never runs past the budget: a CPU engine that cannot finish one short
    /// clip in that time adds nothing to a hybrid configuration, and the hybrid test then records nothing
    /// instead of holding the check for minutes. The full check keeps the long limit.
    fn beside_safety(&self, long_limit_s: u64) -> Duration {
        match self.remaining_s() {
            Some(left) => Duration::from_secs(
                long_limit_s
                    .min((SAFETY_SECONDS + 60.0) as u64)
                    .min(left.max(15.0) as u64),
            ),
            None => Duration::from_secs(long_limit_s),
        }
    }

    /// Notes a step the check left out (once per step).
    fn skip(&mut self, step: &str, reason: String) {
        if !self.profile.skipped.iter().any(|s| s.step == step) {
            self.profile.skipped.push(Skipped {
                step: step.to_string(),
                reason,
            });
        }
    }

    /// Whether an optional step of the quick check that needs `seconds` at the least can still start in time
    /// for the 360 s target. A step that cannot is left for the full check and the profile says `partial`.
    fn fits_target(&mut self, seconds: f64, step: &str) -> bool {
        if self.kind != CheckKind::Quick {
            return true;
        }
        let elapsed = self.started.elapsed().as_secs_f64();
        if target_allows(elapsed, seconds) {
            return true;
        }
        self.partial = true;
        self.skip(
            step,
            format!(
                "left for the full check: the quick check aims to end by {QUICK_TARGET_S:.0} s and {elapsed:.0} s had passed"
            ),
        );
        false
    }

    /// Records a step whose measurement did not produce a result: cut by the quick check's time (left for the full check,
    /// and the profile is partial), or begun and not completed.
    fn skip_not_measured(&mut self, step: &str, config: DeviceConfig) {
        if self.cut_by_time {
            self.partial = true;
        }
        let reason = not_measured_reason(self.cut_by_time, &config_name(config));
        self.skip(step, reason);
    }

    /// The time limit of a Hinglish cell in the quick check: what is left of the time target (at least `HINGLISH_CELL_MIN_S`),
    /// so a slow processor cannot carry the Hinglish step on to the 480 s bar and leave nothing for the steps after it.
    fn hinglish_cap_s(&self) -> Option<f64> {
        (self.kind == CheckKind::Quick && self.hinglish_running)
            .then(|| hinglish_cell_cap_s(self.started.elapsed().as_secs_f64()))
    }

    /// The measured speed of a rolling cell, if there is one.
    fn rolling_rtf(&self, model: &str, lang: Lang, config: DeviceConfig) -> Option<f64> {
        self.profile
            .measurements
            .iter()
            .find(|m| {
                m.model == model && m.lang == lang && m.mode == Mode::Rolling && m.config == config
            })
            .map(|m| m.rtf)
    }

    /// Whether the cells measured so far already show that a CPU engine cannot make the graphics adapter
    /// `hybrid_min_gain` faster: the CPU alone adds `gpu_rtf / cpu_rtf` of the adapter's speed at the most.
    fn predicted_hybrid_gain_too_small(&self, model: &str, lang: Lang, gpu: u32) -> Option<String> {
        let g = self.rolling_rtf(model, lang, DeviceConfig::Gpu { index: gpu })?;
        let c = self.rolling_rtf(model, lang, DeviceConfig::Cpu)?;
        let most = g / c;
        extra_too_small(most, self.env.options.hybrid_min_gain).then(|| {
            format!(
                "predicted from the measured cells: {model} {} runs at RTF {c:.2} on the processor and {g:.2} on the graphics adapter, so both together could be {:.0}% faster than the adapter alone at the most (the rule needs {:.0}%); the hybrid configuration is not chosen",
                lang.name(),
                most * 100.0,
                (self.env.options.hybrid_min_gain - 1.0) * 100.0
            )
        })
    }

    /// The time limit of the CPU half of the hybrid test: it ends with the recorder test.
    fn hybrid_cpu_limit(&self) -> Duration {
        match self.remaining_s() {
            Some(left) => Duration::from_secs(SAFETY_SECONDS.min(left.max(15.0)) as u64),
            None => Duration::from_secs(HYBRID_CPU_LIMIT_FULL_S),
        }
    }

    /// Whether a step that needs `seconds` at the least still fits the quick-check budget. A step that does
    /// not fit is left for the full check and the profile says `partial`.
    fn fits(&mut self, seconds: f64) -> bool {
        match self.remaining_s() {
            Some(left) if left < seconds => {
                self.partial = true;
                false
            }
            _ => true,
        }
    }

    fn full(&self) -> bool {
        self.kind == CheckKind::Full
    }

    /// A piece of a bench clip, cut once.
    fn clip(&mut self, name: &str, start_s: f64, len_s: f64) -> Result<PathBuf, CheckError> {
        let key = format!("{name}@{start_s}+{len_s}");
        if let Some(p) = self.excerpts.get(&key) {
            return Ok(p.clone());
        }
        let src = self.env.bench_dir.join(name);
        if !src.is_file() {
            return Err(CheckError::Clip(src.display().to_string()));
        }
        let dst = self.env.work_dir.join(format!(
            "excerpt-{}-{}-{}.wav",
            name.trim_end_matches(".wav"),
            start_s as u32,
            len_s as u32
        ));
        excerpt::write_excerpt(&src, &dst, start_s, len_s)?;
        self.excerpts.insert(key, dst.clone());
        Ok(dst)
    }

    fn exe_for(&self, config: DeviceConfig) -> Option<&(String, PathBuf)> {
        match config {
            DeviceConfig::Cpu => self.cpu_variant.as_ref(),
            DeviceConfig::Gpu { .. } | DeviceConfig::Hybrid { .. } => self.gpu_variant.as_ref(),
        }
    }

    fn model_ref(&mut self, id: &str) -> Option<(ModelRef, u64)> {
        let entry = self.env.manifest.find(id).ok()?;
        let path = wb_models::path_for_engine(
            &self.env.models_dir,
            &self.env.manifest,
            id,
            &mut self.cache,
        )
        .ok()?;
        Some((
            ModelRef {
                id: id.to_string(),
                path: path.to_string_lossy().into_owned(),
            },
            entry.bytes,
        ))
    }

    fn vad_ref(&mut self) -> Option<ModelRef> {
        self.model_ref(VAD_ID).map(|(m, _)| m)
    }

    fn request(
        &mut self,
        model: &ModelRef,
        lang: Lang,
        clip: &Path,
        mode: Mode,
        config: DeviceConfig,
        threads: u32,
    ) -> Option<BenchRequest> {
        let vad = self.vad_ref()?;
        let settings = Settings {
            role: match mode {
                Mode::Rolling => "rolling".into(),
                Mode::NearLive => "near-live".into(),
            },
            ..Settings::default()
        };
        Some(BenchRequest {
            model: model.clone(),
            vad_model: vad,
            lang: lang.code().to_string(),
            clip: clip.to_string_lossy().into_owned(),
            mode: match mode {
                Mode::Rolling => BenchMode::Rolling,
                Mode::NearLive => BenchMode::NearLive,
            },
            device: match config.gpu_index() {
                Some(index) if !matches!(config, DeviceConfig::Cpu) => Device::Gpu { index },
                _ => Device::Cpu,
            },
            threads,
            runs: if self.full() { 3 } else { 1 },
            single_run_above_rtf: Some(SINGLE_RUN_ABOVE_RTF),
            settings,
        })
    }

    fn threads_for(&self, config: DeviceConfig, mode: Mode) -> u32 {
        let batch = self.env.machine.batch_threads() as u32;
        match (config, mode) {
            (DeviceConfig::Cpu, Mode::Rolling) => batch,
            (DeviceConfig::Cpu, Mode::NearLive) => self.env.machine.live_threads() as u32,
            (_, _) => batch.min(2),
        }
    }

    /// Measures one cell. `Ok(None)` when the model is missing, does not fit memory, or the engine failed
    /// (the reason goes to the log); the check carries on with the next candidate.
    fn measure(&mut self, spec: Spec<'_>) -> Result<Option<Measurement>, CheckError> {
        let Spec {
            model: model_id,
            lang,
            mode,
            config,
            clip: clip_name,
            excerpt_s,
            standin: live_standin,
        } = spec;
        let Some((exe_variant, exe)) = self.exe_for(config).cloned() else {
            return Ok(None);
        };
        let Some((model, bytes)) = self.model_ref(model_id) else {
            tracing::info!(model = model_id, "system check: model not usable, skipped");
            return Ok(None);
        };
        if !model_fits(&self.profile.hardware, bytes, &self.env.options) {
            tracing::info!(
                model = model_id,
                "system check: model does not fit memory, skipped"
            );
            return Ok(None);
        }
        // On a discrete card the model is tried only if it can fit the card's memory with the headroom left (file size
        // x 1.15 + 0.5 GB before it is measured); the verdict then says it does not fit this card's memory.
        if !model_fits_card(&self.profile, config, bytes, &self.env.options) {
            let card = config
                .gpu_index()
                .and_then(|i| self.profile.hardware.gpus.iter().find(|g| g.index == i));
            let gb = |v: u64| v as f64 / (1u64 << 30) as f64;
            let reason = format!(
                "{model_id} does not fit this card's memory (about {:.1} GB needed with {:.1} GB kept free, the card has {:.1} GB).",
                gb(estimated_vram_bytes(bytes, &self.env.options)),
                gb(self.env.options.vram_headroom_bytes),
                gb(card.map_or(0, |g| g.dedicated_bytes))
            );
            tracing::info!(model = model_id, "system check: {reason}");
            self.skip(&format!("{model_id} on {}", config_name(config)), reason);
            return Ok(None);
        }
        self.tick(
            "Measuring speed",
            &format!("{model_id}, {}, {}", lang.name(), config_name(config)),
        )?;
        let clip = self.clip(clip_name, 0.0, excerpt_s)?;
        let threads = self.threads_for(config, mode);
        let Some(request) = self.request(&model, lang, &clip, mode, config, threads) else {
            return Ok(None);
        };
        let runs = request.runs;
        // A quick-check cell never runs past the budget: its time limit is what is left (at least 15 s), and a
        // cell that cannot even start is left for the full check.
        let mut limit_s = excerpt_s * 12.0 + 120.0;
        if let Some(left) = self.remaining_s() {
            if left < 15.0 {
                self.partial = true;
                self.cut_by_time = true;
                return Ok(None);
            }
            limit_s = limit_s.min(left);
        }
        if let Some(cap) = self.hinglish_cap_s() {
            limit_s = limit_s.min(cap);
        }
        let cell = Cell {
            exe: &exe,
            variant: &exe_variant,
            request,
            work_dir: &self.env.work_dir,
            timeout: Duration::from_secs(limit_s as u64),
        };
        let report = match bench::run_bench(&cell, self.cancel) {
            Ok(r) => r,
            Err(BenchError::Cancelled) => return Err(CheckError::Cancelled),
            Err(e) => {
                tracing::warn!(model = model_id, "system check: {e}");
                // In the quick check every limit is the time's: the cap, what is left of the bar, or the cell's own.
                if self.kind == CheckKind::Quick && matches!(e, BenchError::Timeout(_)) {
                    self.cut_by_time = true;
                }
                return Ok(None);
            }
        };
        self.cells += 1;
        let m = Measurement {
            model: model_id.to_string(),
            lang,
            mode,
            config,
            engine_build: report.variant.clone(),
            device_name: report.device_name.clone(),
            threads: report.threads,
            method: format!(
                "bench clip {clip_name}, first {excerpt_s:.0} s, {}, warm-up + {runs} run(s), single run above RTF {SINGLE_RUN_ABOVE_RTF:.0}, {} threads{}",
                match mode {
                    Mode::Rolling => "rolling settings",
                    Mode::NearLive => "near-live settings",
                },
                report.threads,
                if report.single_run {
                    format!(
                        "; impractical: single run (warm-up RTF {:.2}, one timed run)",
                        report.warmup_rtf
                    )
                } else {
                    String::new()
                }
            ),
            clip_seconds: report.clip_seconds,
            rtf: report.rtf_median,
            warmup_rtf: Some(report.warmup_rtf),
            load_s: report.load_s,
            peak_working_set_bytes: report.peak_working_set_bytes,
            peak_commit_bytes: report.peak_commit_bytes,
            gpu_local_bytes: report.gpu_local_bytes,
            truncated_chunks: report.truncated_chunks,
            live_standin,
            detector: None,
        };
        self.profile.measurements.push(m.clone());
        Ok(Some(m))
    }

    /// A throw-away run that fills the driver's shader cache, so the first measured cell is not charged
    /// for compiling. Its speed is never stored. A graphics adapter that fails it is marked unusable.
    fn warm_up(&mut self, gpu: u32) -> Result<(), CheckError> {
        let smallest = self
            .env
            .rankings
            .rolling
            .values()
            .flatten()
            .min_by_key(|r| r.bytes)
            .map(|r| r.model.clone());
        let Some(id) = smallest else { return Ok(()) };
        let Some((model, _)) = self.model_ref(&id) else {
            return Ok(());
        };
        let Some((variant, exe)) = self.gpu_variant.clone() else {
            return Ok(());
        };
        self.tick(
            "Warming up the graphics card",
            &config_name(DeviceConfig::Gpu { index: gpu }),
        )?;
        let clip = self.clip(&clip_file("en"), 0.0, 10.0)?;
        let Some(mut request) = self.request(
            &model,
            Lang::En,
            &clip,
            Mode::Rolling,
            DeviceConfig::Gpu { index: gpu },
            2,
        ) else {
            return Ok(());
        };
        request.runs = 1;
        let cell = Cell {
            exe: &exe,
            variant: &variant,
            request,
            work_dir: &self.env.work_dir,
            timeout: Duration::from_secs(180),
        };
        match bench::run_bench(&cell, self.cancel) {
            Ok(_) => {}
            Err(BenchError::Cancelled) => return Err(CheckError::Cancelled),
            Err(e) => {
                tracing::warn!(gpu, "system check: the Vulkan warm-up failed: {e}");
                if let Some(g) = self
                    .profile
                    .hardware
                    .gpus
                    .iter_mut()
                    .find(|g| g.index == gpu)
                {
                    g.vulkan_usable = false;
                }
            }
        }
        Ok(())
    }

    /// Rolling speed of each language on one configuration: ranked models top-down, stopping at the first
    /// that meets the bar (the full check measures them all). Returns the languages that met the bar.
    ///
    /// `first_only` (the quick check's CPU cells when a graphics adapter exists): only the top-ranked model is
    /// measured. Everything the quick check leaves out is measured by the full check.
    fn rolling_on(
        &mut self,
        config: DeviceConfig,
        langs: &[Lang],
        first_only: bool,
    ) -> Result<Vec<Lang>, CheckError> {
        let bar = wb_capability::bars(&self.profile).rolling_rtf;
        let mut met = Vec::new();
        for &lang in langs {
            let list = self
                .env
                .rankings
                .rolling
                .get(&lang)
                .cloned()
                .unwrap_or_default();
            for (i, rk) in list.into_iter().enumerate() {
                if (first_only && i > 0) || self.over_budget() {
                    break;
                }
                // With a graphics adapter the quick check's CPU cells are optional: they start only if they
                // can end by the target.
                if first_only
                    && config == DeviceConfig::Cpu
                    && !self.fits_target(90.0, "CPU speed of the ranked models")
                {
                    return Ok(met);
                }
                let Some(m) = self.measure(Spec {
                    model: &rk.model,
                    lang,
                    mode: Mode::Rolling,
                    config,
                    clip: &clip_file(lang.code()),
                    excerpt_s: cell_excerpt_seconds(self.kind, lang, config, i),
                    standin: false,
                })?
                else {
                    continue;
                };
                if m.rtf <= bar {
                    if !met.contains(&lang) {
                        met.push(lang);
                    }
                    if !self.full() {
                        break;
                    }
                }
            }
        }
        Ok(met)
    }

    fn measure_all(&mut self) -> Result<(), CheckError> {
        let langs = Lang::ALL;
        // Graphics adapters, discrete before integrated; the next one is measured only while the languages
        // are not all met (the full check measures every adapter).
        let mut gpus: Vec<(u32, GpuKind)> = self
            .profile
            .hardware
            .gpus
            .iter()
            .filter(|g| g.vulkan_usable)
            .map(|g| (g.index, g.kind))
            .collect();
        gpus.sort_by_key(|(i, k)| (matches!(k, GpuKind::Integrated), *i));
        let mut met_by_gpu: Vec<Lang> = Vec::new();
        for (index, _) in gpus {
            if !self.full() && met_by_gpu.len() == langs.len() {
                break;
            }
            self.warm_up(index)?;
            if !self
                .profile
                .hardware
                .gpus
                .iter()
                .any(|g| g.index == index && g.vulkan_usable)
            {
                continue;
            }
            let met = self.rolling_on(DeviceConfig::Gpu { index }, &langs, false)?;
            for l in met {
                if !met_by_gpu.contains(&l) {
                    met_by_gpu.push(l);
                }
            }
        }
        // The recorder safety test (and the hybrid test with it) comes right after the graphics cells: it is
        // mandatory, so the quick check does it before the optional CPU cells can use up the budget. With no
        // working adapter it runs after the CPU cells, because the CPU configuration is what it tests.
        let had_gpu = self
            .profile
            .measurements
            .iter()
            .any(|m| matches!(m.config, DeviceConfig::Gpu { .. }));
        let mut safety_done = false;
        // The full check measures the CPU cells first: the hybrid test is then predicted from them.
        if had_gpu && !self.full() && self.fits(HYBRID_SAFETY_MIN_S) {
            self.hybrid_and_safety(false)?;
            safety_done = true;
        }
        // The CPU: every language when no adapter works, otherwise only those no adapter met (the full
        // check measures all). With an adapter the quick check measures only the top-ranked model on the CPU.
        if self.cpu_variant.is_some() {
            let todo: Vec<Lang> = if self.full() {
                langs.to_vec()
            } else {
                langs
                    .iter()
                    .copied()
                    .filter(|l| !met_by_gpu.contains(l))
                    .collect()
            };
            if !todo.is_empty() {
                self.rolling_on(DeviceConfig::Cpu, &todo, had_gpu && !self.full())?;
            }
        }

        // Hybrid test and recorder safety run, together (here only when there was no adapter; with one it
        // ran above, and what is left is the safety run of the CPU configuration when that was chosen).
        if safety_done {
            if self.fits(HYBRID_SAFETY_MIN_S) {
                self.hybrid_and_safety(true)?;
            }
        } else if self.fits(HYBRID_SAFETY_MIN_S) {
            self.hybrid_and_safety(false)?;
        }
        // The optional steps that follow, in `TAIL_ORDER`.
        for (i, step) in TAIL_ORDER.into_iter().enumerate() {
            if self.over_budget() {
                // The quick check's bar passed: every step that did not run is left for the full check, and the profile says so.
                for left_out in &TAIL_ORDER[i..] {
                    self.skip(
                        left_out.step_name(),
                        "left for the full check: the quick check's time ran out before this step"
                            .into(),
                    );
                }
                break;
            }
            match step {
                // Rule (d2): the Hinglish model (and its pair), on the configuration the rules choose now.
                TailStep::Hinglish => {
                    if let Some(config) = self.best_config() {
                        self.hinglish_running = true;
                        let done = self.hinglish_step(config);
                        self.hinglish_running = false;
                        done?;
                    }
                }
                // Near-live on the CPU.
                TailStep::NearLive => self.near_live()?,
                // The live stand-in on a discrete GPU.
                TailStep::LiveStandin => self.live_standin()?,
                // The night model on the configuration the rules choose now.
                TailStep::Night => self.night()?,
            }
        }
        Ok(())
    }

    fn decided(&self) -> wb_capability::Decision {
        decide(&self.profile, &self.env.rankings, &self.env.options)
    }

    /// The configuration a language's rolling model ran on best, for later steps.
    fn best_config(&self) -> Option<DeviceConfig> {
        self.decided().config.map(|c| match c.config {
            DeviceConfig::Hybrid { gpu } => DeviceConfig::Gpu { index: gpu },
            other => other,
        })
    }

    /// `cpu_tail_only`: the hybrid and graphics safety runs were done earlier; only the safety run for the CPU
    /// configuration is left, when the rules chose it.
    fn hybrid_and_safety(&mut self, cpu_tail_only: bool) -> Result<(), CheckError> {
        let Some(best) = self.best_config() else {
            return Ok(());
        };
        let gpu_index = match best {
            DeviceConfig::Gpu { index } => Some(index),
            DeviceConfig::Cpu => self
                .profile
                .hardware
                .gpus
                .iter()
                .find(|g| g.vulkan_usable)
                .map(|g| g.index),
            DeviceConfig::Hybrid { .. } => None,
        };
        let batch = self.env.machine.batch_threads() as u32;
        let hybrid_cpu_threads = batch.saturating_sub(2).max(1);
        // The model and language of the slowest rolling pick are the hardest case for the hybrid test.
        let decision = self.decided();
        let Some((lang, pick)) = decision
            .languages
            .iter()
            .filter_map(|(l, d)| d.rolling.as_ref().map(|p| (*l, p.clone())))
            .max_by(|a, b| a.1.rtf.total_cmp(&b.1.rtf))
        else {
            return Ok(());
        };

        if cpu_tail_only {
            // Nothing before the tail.
        } else if let (Some(gpu), true) = (gpu_index, self.cpu_variant.is_some()) {
            let gpu_config = DeviceConfig::Gpu { index: gpu };
            if let Some(reason) = self.predicted_hybrid_gain_too_small(&pick.model, lang, gpu) {
                // The measured cells show the CPU cannot add enough: no hybrid test, but the recorder is
                // still tested under the graphics engine's load.
                self.skip("hybrid test", reason);
                self.tick(
                    "Testing the recorder under load",
                    "while a test recording runs",
                )?;
                let safety =
                    SafetyRun::start(&self.env.work_dir, &self.env.app_version, gpu_config);
                if let Err(CheckError::Cancelled) = self.load_until_done(
                    gpu_config,
                    &pick.model,
                    lang,
                    batch.min(2),
                    safety.as_ref().ok(),
                ) {
                    discard_safety(safety);
                    return Err(CheckError::Cancelled);
                }
                self.finish_safety(safety, &[gpu_config])?;
            } else {
                self.tick(
                    "Testing both processors at once",
                    "while a test recording runs",
                )?;
                let config = DeviceConfig::Hybrid { gpu };
                let safety = SafetyRun::start(&self.env.work_dir, &self.env.app_version, config);
                let gpu_alone = self
                    .rolling_rtf(&pick.model, lang, gpu_config)
                    .unwrap_or(pick.rtf);
                let hybrid = self.hybrid_pair(
                    &pick.model,
                    lang,
                    gpu,
                    gpu_alone,
                    hybrid_cpu_threads,
                    safety.as_ref().ok(),
                );
                let hybrid = match hybrid {
                    Err(CheckError::Cancelled) => {
                        discard_safety(safety);
                        return Err(CheckError::Cancelled);
                    }
                    other => other?,
                };
                match hybrid {
                    HybridOutcome::Measured(h) => self.profile.hybrid = Some(h),
                    HybridOutcome::Predicted(reason) | HybridOutcome::NotMeasured(reason) => {
                        self.skip("hybrid test", reason);
                    }
                }
                self.finish_safety(safety, &[config, gpu_config])?;
            }
        } else if gpu_index.is_none() {
            // No usable graphics adapter: the safety run is for the CPU configuration itself.
            self.tick(
                "Testing the recorder under load",
                "while a test recording runs",
            )?;
            let config = DeviceConfig::Cpu;
            let safety = SafetyRun::start(&self.env.work_dir, &self.env.app_version, config);
            if let Err(CheckError::Cancelled) = self.load_until_done(
                DeviceConfig::Cpu,
                &pick.model,
                lang,
                batch,
                safety.as_ref().ok(),
            ) {
                discard_safety(safety);
                return Err(CheckError::Cancelled);
            }
            self.finish_safety(safety, &[config])?;
        }

        // The chosen configuration is the CPU alone although a graphics adapter exists: it uses more CPU
        // threads than the hybrid run did, so it gets a run of its own.
        let chosen = self.decided().config.map(|c| c.config);
        if chosen == Some(DeviceConfig::Cpu)
            && gpu_index.is_some()
            && !self
                .profile
                .recorder_safety
                .iter()
                .any(|s| s.config == DeviceConfig::Cpu)
            && !self.over_budget()
        {
            self.tick("Testing the recorder under load", "CPU configuration")?;
            let safety =
                SafetyRun::start(&self.env.work_dir, &self.env.app_version, DeviceConfig::Cpu);
            if let Err(CheckError::Cancelled) = self.load_until_done(
                DeviceConfig::Cpu,
                &pick.model,
                lang,
                batch,
                safety.as_ref().ok(),
            ) {
                discard_safety(safety);
                return Err(CheckError::Cancelled);
            }
            self.finish_safety(safety, &[DeviceConfig::Cpu])?;
        }
        Ok(())
    }

    /// Stops a safety run and stores its result for each configuration it covers (a heavier load covers
    /// the lighter ones). A run that could not start stores nothing: the rules then cannot choose hybrid.
    fn finish_safety(
        &mut self,
        run: Result<SafetyRun, SafetyError>,
        covers: &[DeviceConfig],
    ) -> Result<(), CheckError> {
        match run {
            Ok(r) => {
                match r.finish() {
                    Ok(result) if !safety_counts(result.duration_s) => {
                        // Too short to say anything about load: nothing is recorded, the profile stays
                        // partial and processing waits for Stop.
                        tracing::warn!(
                            "system check: the recorder safety run lasted only {:.0} s and is not counted",
                            result.duration_s
                        );
                        self.partial = true;
                        self.skip(
                        "recorder safety test",
                        format!(
                            "not recorded: the test recording ran only {:.0} s under load (it needs {SAFETY_MIN_COUNTED_S:.0} s)",
                            result.duration_s
                        ),
                    );
                    }
                    Ok(result) => {
                        for c in covers {
                            self.profile.recorder_safety.push(RecorderSafety {
                                config: *c,
                                ..result.clone()
                            });
                        }
                    }
                    Err(e) => {
                        tracing::warn!("system check: {e}");
                        self.partial = true;
                        self.skip(
                        "recorder safety test",
                        format!("not recorded: the test recording could not be stopped cleanly ({e})"),
                    );
                    }
                }
            }
            Err(e) => {
                tracing::warn!("system check: the recorder safety run did not start: {e}");
                self.partial = true;
                self.skip(
                    "recorder safety test",
                    format!("not recorded: the test recording did not start ({e})"),
                );
            }
        }
        Ok(())
    }

    /// Runs the engine of `config` (the CPU engine, or the graphics adapter's) on the chosen model until the
    /// safety run has lasted its time.
    fn load_until_done(
        &mut self,
        config: DeviceConfig,
        model_id: &str,
        lang: Lang,
        threads: u32,
        safety: Option<&SafetyRun>,
    ) -> Result<(), CheckError> {
        let engine = match config {
            DeviceConfig::Gpu { .. } => self.gpu_variant.clone(),
            _ => self.cpu_variant.clone(),
        };
        let Some((variant, exe)) = engine else {
            return Ok(());
        };
        let Some((model, _)) = self.model_ref(model_id) else {
            return Ok(());
        };
        let clip = self.clip(&clip_file(lang.code()), 0.0, 15.0)?;
        let timeout = self.beside_safety(600);
        loop {
            let Some(request) = self.request(&model, lang, &clip, Mode::Rolling, config, threads)
            else {
                return Ok(());
            };
            let cell = Cell {
                exe: &exe,
                variant: &variant,
                request,
                work_dir: &self.env.work_dir,
                timeout,
            };
            match bench::run_bench(&cell, self.cancel) {
                Ok(_) => {}
                Err(BenchError::Cancelled) => return Err(CheckError::Cancelled),
                Err(e) => {
                    tracing::warn!("system check: load run failed: {e}");
                    return Ok(());
                }
            }
            if safety.is_none_or(|s| s.elapsed().as_secs_f64() >= SAFETY_SECONDS) {
                return Ok(());
            }
        }
    }

    /// A CPU engine and a GPU engine at the same time on different halves of the clip, repeated until
    /// the safety run has lasted its time. The GPU's speed is the median of its runs that finished while
    /// the CPU engine was still going.
    fn hybrid_pair(
        &mut self,
        model_id: &str,
        lang: Lang,
        gpu: u32,
        gpu_alone_rtf: f64,
        cpu_threads: u32,
        safety: Option<&SafetyRun>,
    ) -> Result<HybridOutcome, CheckError> {
        let (Some((cpu_variant, cpu_exe)), Some((gpu_variant, gpu_exe))) =
            (self.cpu_variant.clone(), self.gpu_variant.clone())
        else {
            return Ok(HybridOutcome::NotMeasured(
                "the hybrid test needs a processor engine and a graphics engine".into(),
            ));
        };
        let Some((model, _)) = self.model_ref(model_id) else {
            return Ok(HybridOutcome::NotMeasured(format!(
                "the model {model_id} is not usable"
            )));
        };
        let a = self.clip(&clip_file(lang.code()), 0.0, 30.0)?;
        let b = self.clip(&clip_file(lang.code()), 30.0, HYBRID_CPU_CLIP_S)?;
        let gpu_threads = (self.env.machine.batch_threads() as u32).min(2);
        let mut gpu_rtfs: Vec<f64> = Vec::new();
        let mut cpu_rtfs: Vec<f64> = Vec::new();
        let mut cpu_peak: u64 = 0;
        let mut gpu_peak: (u64, Option<u64>) = (0, None);
        let started = Instant::now();
        let cpu_timeout = self.hybrid_cpu_limit();
        let mut cpu_timed_out = false;
        // The graphics engine alone on the excerpt its half of the test decodes, so the gain is compared like with
        // like (the cell of this model can have been timed on a longer excerpt). It runs beside the recorder
        // test, which it counts towards.
        let gpu_alone_same = self.hybrid_gpu_alone(&model, lang, &a, gpu, gpu_threads)?;
        loop {
            let (Some(cpu_req), Some(gpu_req)) = (
                self.request(
                    &model,
                    lang,
                    &b,
                    Mode::Rolling,
                    DeviceConfig::Cpu,
                    cpu_threads,
                ),
                self.request(
                    &model,
                    lang,
                    &a,
                    Mode::Rolling,
                    DeviceConfig::Gpu { index: gpu },
                    gpu_threads,
                ),
            ) else {
                return Ok(HybridOutcome::NotMeasured(
                    "the hybrid test could not build its requests".into(),
                ));
            };
            let work = self.env.work_dir.clone();
            let cancel = self.cancel;
            let finished = AtomicBool::new(false);
            let (cpu_result, gpu_round, gpu_round_peak) = std::thread::scope(|s| {
                let cpu = s.spawn(|| {
                    let cell = Cell {
                        exe: &cpu_exe,
                        variant: &cpu_variant,
                        request: cpu_req,
                        work_dir: &work,
                        timeout: cpu_timeout,
                    };
                    let r = bench::run_bench(&cell, cancel);
                    finished.store(true, Ordering::Release);
                    r
                });
                // Let the CPU engine load its model before the GPU runs are counted.
                std::thread::sleep(Duration::from_secs(3));
                let mut round: Vec<f64> = Vec::new();
                let mut gpu_peak: (u64, Option<u64>) = (0, None);
                loop {
                    let cell = Cell {
                        exe: &gpu_exe,
                        variant: &gpu_variant,
                        request: gpu_req.clone(),
                        work_dir: &work,
                        timeout: Duration::from_secs(300),
                    };
                    let r = bench::run_bench(&cell, cancel);
                    let overlapped = !finished.load(Ordering::Acquire);
                    if let Ok(rep) = r
                        && (overlapped || round.is_empty())
                    {
                        round.push(rep.rtf_median);
                        if rep.peak_commit_bytes > gpu_peak.0 {
                            gpu_peak = (rep.peak_commit_bytes, rep.gpu_local_bytes);
                        }
                    }
                    if !overlapped || cancel.load(Ordering::Relaxed) {
                        break;
                    }
                }
                (cpu.join(), round, gpu_peak)
            });
            match cpu_result {
                Ok(Ok(rep)) => {
                    cpu_rtfs.push(rep.rtf_median);
                    cpu_peak = cpu_peak.max(rep.peak_commit_bytes);
                }
                Ok(Err(BenchError::Cancelled)) => return Err(CheckError::Cancelled),
                Ok(Err(BenchError::Timeout(_))) => cpu_timed_out = true,
                Ok(Err(e)) => tracing::warn!("system check: hybrid CPU run failed: {e}"),
                Err(_) => tracing::warn!("system check: hybrid CPU run panicked"),
            }
            gpu_rtfs.extend(gpu_round);
            if gpu_round_peak.0 > gpu_peak.0 {
                gpu_peak = gpu_round_peak;
            }
            self.cells += 2;
            self.tick(
                "Testing both processors at once",
                "while a test recording runs",
            )?;
            let long_enough = safety.is_none_or(|s| s.elapsed().as_secs_f64() >= SAFETY_SECONDS);
            if long_enough
                || cpu_timed_out
                || started.elapsed().as_secs_f64() > 4.0 * SAFETY_SECONDS
            {
                break;
            }
        }
        let median = |v: &mut Vec<f64>| {
            v.sort_by(f64::total_cmp);
            v.get(v.len() / 2).copied()
        };
        let (Some(g), Some(c)) = (median(&mut gpu_rtfs), median(&mut cpu_rtfs)) else {
            if cpu_timed_out {
                // The CPU engine did not finish HYBRID_CPU_CLIP_S seconds of audio in the time it had, so it
                // decodes less than that many seconds in that time: an upper bound on what it adds.
                let secs = cpu_timeout.as_secs_f64();
                let most = hybrid_bound(gpu_alone_rtf, HYBRID_CPU_CLIP_S, secs);
                let reason = format!(
                    "{model_id} {}: the processor engine ({cpu_threads} threads) did not finish a warm-up and one timed decode of {HYBRID_CPU_CLIP_S:.0} s of audio in {secs:.0} s beside the recorder test (RTF above about {:.1}); the graphics adapter runs at {gpu_alone_rtf:.2}, so the processor could add {:.0}% at the most (the rule needs {:.0}%)",
                    lang.name(),
                    secs / (HYBRID_DECODES_PER_RUN * HYBRID_CPU_CLIP_S),
                    most * 100.0,
                    (self.env.options.hybrid_min_gain - 1.0) * 100.0
                );
                return Ok(if extra_too_small(most, self.env.options.hybrid_min_gain) {
                    HybridOutcome::Predicted(format!(
                        "predicted, not measured: {reason}; the hybrid configuration is not chosen"
                    ))
                } else {
                    HybridOutcome::NotMeasured(format!("not measured: {reason}"))
                });
            }
            return Ok(HybridOutcome::NotMeasured(
                "the hybrid test produced no complete round".into(),
            ));
        };
        Ok(HybridOutcome::Measured(HybridMeasurement {
            model: model_id.to_string(),
            lang,
            gpu,
            cpu_threads,
            gpu_rtf_concurrent: g,
            cpu_rtf_concurrent: c,
            cpu_peak_commit_bytes: cpu_peak,
            gpu_peak_commit_bytes: gpu_peak.0,
            gpu_local_bytes: gpu_peak.1,
            gpu_alone_rtf: gpu_alone_same,
            method: format!(
                "CPU engine ({cpu_threads} threads, {HYBRID_CPU_CLIP_S:.0} s of the clip) and GPU engine (30 s) at the same time, rolling settings, with a test recording running; {} GPU run(s), {} CPU run(s); the gain is against the GPU engine alone on the same 30 s, measured just before",
                gpu_rtfs.len(),
                cpu_rtfs.len()
            ),
        }))
    }

    /// One bench of the graphics engine alone on `clip`; `None` if it did not finish (the rules then compare
    /// with the single-device cell).
    fn hybrid_gpu_alone(
        &mut self,
        model: &ModelRef,
        lang: Lang,
        clip: &Path,
        gpu: u32,
        threads: u32,
    ) -> Result<Option<f64>, CheckError> {
        let Some((gpu_variant, gpu_exe)) = self.gpu_variant.clone() else {
            return Ok(None);
        };
        let Some(request) = self.request(
            model,
            lang,
            clip,
            Mode::Rolling,
            DeviceConfig::Gpu { index: gpu },
            threads,
        ) else {
            return Ok(None);
        };
        let cell = Cell {
            exe: &gpu_exe,
            variant: &gpu_variant,
            request,
            work_dir: &self.env.work_dir,
            timeout: Duration::from_secs(300),
        };
        match bench::run_bench(&cell, self.cancel) {
            Ok(rep) => Ok(Some(rep.rtf_median)),
            Err(BenchError::Cancelled) => Err(CheckError::Cancelled),
            Err(e) => {
                tracing::warn!(
                    "system check: the graphics engine alone on the hybrid excerpt failed: {e}"
                );
                Ok(None)
            }
        }
    }

    fn near_live(&mut self) -> Result<(), CheckError> {
        if self.cpu_variant.is_none() {
            return Ok(());
        }
        let excerpt_s = if self.full() { 30.0 } else { 20.0 };
        for lang in [Lang::En, Lang::Hi] {
            let list = self
                .env
                .rankings
                .near_live
                .get(&lang)
                .cloned()
                .unwrap_or_default();
            // Near-live text is offered only from a validated model, so a model nobody has validated is not
            // timed: the minutes go to the steps that decide something.
            if !list.is_empty() && !list.iter().any(|rk| rk.validated) {
                self.skip(
                    &format!("near-live speed on the processor ({})", lang.name()),
                    "not measured: no validated near-live model".into(),
                );
                continue;
            }
            if let Some(rk) = list.iter().find(|rk| rk.validated) {
                self.cut_by_time = false;
                let done = self.measure(Spec {
                    model: &rk.model,
                    lang,
                    mode: Mode::NearLive,
                    config: DeviceConfig::Cpu,
                    clip: &clip_file(lang.code()),
                    excerpt_s,
                    standin: false,
                })?;
                if done.is_none() {
                    self.skip_not_measured(
                        &format!("near-live speed on the processor ({})", lang.name()),
                        DeviceConfig::Cpu,
                    );
                }
            }
        }
        Ok(())
    }

    fn live_standin(&mut self) -> Result<(), CheckError> {
        let Some(gpu) = self
            .profile
            .hardware
            .gpus
            .iter()
            .find(|g| g.kind == GpuKind::Discrete && g.vulkan_usable)
            .map(|g| g.index)
        else {
            return Ok(());
        };
        let bar = wb_capability::bars(&self.profile).live_standin_rtf;
        for lang in Lang::ALL {
            let list = self
                .env
                .rankings
                .live
                .get(&lang)
                .cloned()
                .unwrap_or_default();
            self.cut_by_time = false;
            let mut any = false;
            let had_list = !list.is_empty();
            for rk in list {
                let m = self.measure(Spec {
                    model: &rk.model,
                    lang,
                    mode: Mode::NearLive,
                    config: DeviceConfig::Gpu { index: gpu },
                    clip: &clip_file(lang.code()),
                    excerpt_s: excerpt_seconds(self.kind).min(30.0),
                    standin: true,
                })?;
                any |= m.is_some();
                if m.is_some_and(|m| m.rtf <= bar) {
                    break;
                }
            }
            if had_list && !any {
                self.skip_not_measured(
                    &format!("live stand-in on the graphics card ({})", lang.name()),
                    DeviceConfig::Gpu { index: gpu },
                );
            }
        }
        Ok(())
    }

    fn night(&mut self) -> Result<(), CheckError> {
        let Some(config) = self.best_config() else {
            return Ok(());
        };
        if !self.fits_target(90.0, "night model speed") {
            return Ok(());
        }
        let limit = self.env.options.night_window_hours / self.env.options.day_audio_hours;
        let hi_rtf = self
            .decided()
            .languages
            .get(&Lang::Hi)
            .and_then(|l| l.rolling.as_ref().map(|r| r.rtf));
        for lang in Lang::ALL {
            // The quick check measures the night model for Hindi only, and only where the rolling speed
            // predicts it fits (a large-v3-sized model is about 2.5 times slower than the rolling one).
            if !self.full() && (lang != Lang::Hi || hi_rtf.is_none_or(|r| r * 2.5 > limit)) {
                // A language with no night model listed has nothing to measure, now or in the full check.
                if self
                    .env
                    .rankings
                    .night
                    .get(&lang)
                    .is_some_and(|l| !l.is_empty())
                {
                    let why = if lang != Lang::Hi {
                        "left for the full check: the quick check times the night model for Hindi only, the language that decides it"
                    } else if hi_rtf.is_none() {
                        "not measured: there is no Hindi rolling speed to predict from"
                    } else {
                        "not measured: the rolling speed predicts a night model of this size does not fit overnight"
                    };
                    self.skip(&format!("night model speed ({})", lang.name()), why.into());
                }
                continue;
            }
            let list = self
                .env
                .rankings
                .night
                .get(&lang)
                .cloned()
                .unwrap_or_default();
            self.cut_by_time = false;
            let mut any = false;
            let had_list = !list.is_empty();
            for rk in list {
                let m = self.measure(Spec {
                    model: &rk.model,
                    lang,
                    mode: Mode::Rolling,
                    config,
                    clip: &clip_file(lang.code()),
                    excerpt_s: excerpt_seconds(self.kind),
                    standin: false,
                })?;
                any |= m.is_some();
                if !self.full() {
                    break;
                }
            }
            if had_list && !any {
                self.skip_not_measured(&format!("night model speed ({})", lang.name()), config);
            }
        }
        Ok(())
    }

    /// Rule (d2) rests on a measurement of the Hinglish model itself: always in the full check; in the quick check when
    /// the time target allows and the Hindi speed of the rolling model does not already rule the model out.
    fn hinglish_step(&mut self, config: DeviceConfig) -> Result<(), CheckError> {
        if self.full() {
            return self.hinglish(config);
        }
        let bar = wb_capability::bars(&self.profile).rolling_rtf;
        let hi_rtf = self
            .decided()
            .languages
            .get(&Lang::Hi)
            .and_then(|l| l.rolling.as_ref().map(|r| r.rtf));
        match hi_rtf {
            Some(r) if r * HINGLISH_MIN_SLOWDOWN > bar => {
                self.skip(
                    wb_capability::SKIPPED_HINGLISH,
                    format!(
                        "not measured: Hindi already runs at {r:.2}x real time with the rolling model, and the Hinglish model is at least {HINGLISH_MIN_SLOWDOWN} times slower, over the {bar:.2} bar"
                    ),
                );
                Ok(())
            }
            _ => {
                if self.fits_target(HINGLISH_QUICK_MIN_S, wb_capability::SKIPPED_HINGLISH) {
                    self.hinglish(config)
                } else {
                    Ok(())
                }
            }
        }
    }

    /// The Hinglish model on the alternating English/Hindi clip, so rule (d) rests on a measurement: the q8_0 file, and on a
    /// discrete card then the pair with the small detector.
    fn hinglish(&mut self, config: DeviceConfig) -> Result<(), CheckError> {
        let discrete = wb_capability::discrete_card(&self.profile, config).is_some();
        let candidates = hinglish_candidates(discrete);
        self.cut_by_time = false;
        let mut measured = None;
        for id in candidates {
            if self.env.manifest.find(id).is_err() {
                continue;
            }
            measured = self.measure(Spec {
                model: id,
                lang: Lang::Hi,
                mode: Mode::Rolling,
                config,
                clip: "bench-codeswitch.wav",
                excerpt_s: excerpt_seconds(self.kind),
                standin: false,
            })?;
            if measured.is_some() {
                break;
            }
        }
        if measured.is_none() {
            // The cell gave no result: cut by the quick check's time (the Full Check measures it), or the model is unusable
            // here, does not fit the memory or the card, or failed (a Full Check would not change that).
            self.skip_not_measured(wb_capability::SKIPPED_HINGLISH, config);
        }
        if let Some(h) = measured {
            if discrete {
                self.hinglish_pair(config, &h)?;
            } else {
                self.skip(
                    wb_capability::SKIPPED_HINGLISH_PAIR,
                    format!(
                        "not measured: the pair (a small model in front of the Hinglish model) is only offered on a discrete graphics card, and this computer transcribes on {}",
                        config_name(config)
                    ),
                );
            }
        }
        Ok(())
    }

    /// The Hinglish model with a small detector in front of it (`wb_capability::HINGLISH_PAIRS`), measured the way the
    /// post-processing runs it: one batch decode of the code-switched clip with the language detected for every chunk
    /// (the engine's `bench` has neither detection nor a second model). Its memory is the two models' own cells added up
    /// (a little above the true peak, never below it). The cell is stored with `detector` set; the rules choose it only
    /// if it meets the bar and fits the card beside the live model.
    fn hinglish_pair(&mut self, config: DeviceConfig, h: &Measurement) -> Result<(), CheckError> {
        self.cut_by_time = false;
        if !self.hinglish_pair_run(config, h)? {
            // Every way out without a stored cell leaves a record (the step is recorded once, the first reason stays).
            self.skip_not_measured(wb_capability::SKIPPED_HINGLISH_PAIR, config);
        }
        Ok(())
    }

    /// Runs the pair; `false` when no cell was stored.
    fn hinglish_pair_run(
        &mut self,
        config: DeviceConfig,
        h: &Measurement,
    ) -> Result<bool, CheckError> {
        let Some((_, detector)) = wb_capability::HINGLISH_PAIRS
            .iter()
            .find(|(m, _)| *m == h.model)
        else {
            // A model with no pair defined has nothing to record.
            return Ok(true);
        };
        if self.over_budget() {
            self.cut_by_time = true;
            return Ok(false);
        }
        if !self.fits_target(PAIR_QUICK_MIN_S, wb_capability::SKIPPED_HINGLISH_PAIR) {
            return Ok(false);
        }
        if self.env.manifest.find(detector).is_err() {
            return Ok(false);
        }
        // The detector's own cell (memory and load time); it is not stored: the speed table of the rankings is not changed.
        let Some(d) = self.measure(Spec {
            model: detector,
            lang: Lang::En,
            mode: Mode::Rolling,
            config,
            clip: &clip_file("en"),
            excerpt_s: excerpt_seconds(self.kind),
            standin: false,
        })?
        else {
            return Ok(false);
        };
        self.profile.measurements.pop();
        let (Some((hinglish_ref, _)), Some((detector_ref, _)), Some(vad)) = (
            self.model_ref(&h.model),
            self.model_ref(detector),
            self.vad_ref(),
        ) else {
            return Ok(false);
        };
        let Some((_, exe)) = self.exe_for(config).cloned() else {
            return Ok(false);
        };
        let excerpt_s = excerpt_seconds(self.kind);
        let clip = self.clip("bench-codeswitch.wav", 0.0, excerpt_s)?;
        self.tick(
            "Measuring speed",
            &format!(
                "{} with {detector} in front, {}",
                h.model,
                config_name(config)
            ),
        )?;
        let mut limit_s = excerpt_s * 12.0 + 120.0;
        if let Some(left) = self.remaining_s() {
            if left < 15.0 {
                self.partial = true;
                self.cut_by_time = true;
                return Ok(false);
            }
            limit_s = limit_s.min(left);
        }
        if let Some(cap) = self.hinglish_cap_s() {
            limit_s = limit_s.min(cap);
        }
        let request = BatchRequest {
            source: Source::File {
                path: clip.to_string_lossy().into_owned(),
            },
            units: Vec::new(),
            models_by_lang: std::collections::BTreeMap::from([
                ("en".to_string(), detector_ref),
                ("hi".to_string(), hinglish_ref),
            ]),
            vad_model: vad,
            languages: vec!["en".to_string(), "hi".to_string()],
            lang_log: Vec::new(),
            settings: Settings::default(),
            device: match config.gpu_index() {
                Some(index) if !matches!(config, DeviceConfig::Cpu) => Device::Gpu { index },
                _ => Device::Cpu,
            },
            threads: self.threads_for(config, Mode::Rolling),
            resume_from_ms: 0,
            detect_language: true,
        };
        let timing = match bench::run_batch(
            &exe,
            &h.engine_build,
            &request,
            &self.env.work_dir,
            Duration::from_secs(limit_s as u64),
            self.cancel,
        ) {
            Ok(t) => t,
            Err(BenchError::Cancelled) => return Err(CheckError::Cancelled),
            Err(e) => {
                tracing::warn!(model = %h.model, "system check: pair with {detector}: {e}");
                if self.kind == CheckKind::Quick && matches!(e, BenchError::Timeout(_)) {
                    self.cut_by_time = true;
                }
                return Ok(false);
            }
        };
        self.cells += 1;
        self.profile.measurements.push(Measurement {
            model: h.model.clone(),
            lang: Lang::Hi,
            mode: Mode::Rolling,
            config,
            engine_build: h.engine_build.clone(),
            device_name: h.device_name.clone(),
            threads: h.threads,
            method: format!(
                "batch of bench-codeswitch.wav, first {excerpt_s:.0} s, en={detector} and hi={}, language detected for every chunk ({} detections), rolling settings, one run, {} threads; memory: the two models' own bench cells added up",
                h.model, timing.detections, h.threads
            ),
            clip_seconds: timing.audio_s,
            rtf: timing.rtf(),
            warmup_rtf: None,
            load_s: h.load_s + d.load_s,
            peak_working_set_bytes: h
                .peak_working_set_bytes
                .saturating_add(d.peak_working_set_bytes)
                .max(timing.peak_working_set_bytes),
            peak_commit_bytes: h.peak_commit_bytes.saturating_add(d.peak_commit_bytes),
            gpu_local_bytes: match (h.gpu_local_bytes, d.gpu_local_bytes) {
                (Some(a), Some(b)) => Some(a.saturating_add(b)),
                _ => None,
            },
            truncated_chunks: timing.truncated_chunks,
            live_standin: false,
            detector: Some((*detector).to_string()),
        });
        Ok(true)
    }
}

/// Whether an optional step of the quick check that needs `seconds` at the least can still start in time for the
/// `QUICK_TARGET_S` target when `elapsed_s` seconds have passed.
fn target_allows(elapsed_s: f64, seconds: f64) -> bool {
    QUICK_TARGET_S - elapsed_s >= seconds
}

/// Whether a CPU that adds `extra` (a fraction of the graphics adapter's speed) at the most cannot reach the
/// rule's gain. `min_gain` is a ratio: `Options::hybrid_min_gain` of 1.15 means 15% faster than the best single
/// device.
fn extra_too_small(extra: f64, min_gain: f64) -> bool {
    1.0 + extra < min_gain
}

/// The most a CPU engine can add, as a fraction of the graphics adapter's speed, when it did not finish its
/// round of `clip_s` seconds in `secs` seconds (the round is `HYBRID_DECODES_PER_RUN` decodes of the clip).
fn hybrid_bound(gpu_rtf: f64, clip_s: f64, secs: f64) -> f64 {
    HYBRID_DECODES_PER_RUN * clip_s / secs * gpu_rtf
}

/// Whether a finished recorder safety run lasted long enough to count.
fn safety_counts(duration_s: f64) -> bool {
    duration_s >= SAFETY_MIN_COUNTED_S
}

/// What the hybrid test came to.
enum HybridOutcome {
    Measured(HybridMeasurement),
    /// The processor could not finish its share in the time given: it cannot add the gain the rule needs.
    Predicted(String),
    NotMeasured(String),
}

fn config_name(config: DeviceConfig) -> String {
    match config {
        DeviceConfig::Cpu => "processor".into(),
        DeviceConfig::Gpu { index } => format!("graphics adapter {index}"),
        DeviceConfig::Hybrid { gpu } => format!("processor and graphics adapter {gpu}"),
    }
}

#[cfg(test)]
mod tests {
    use super::{extra_too_small, hinglish_candidates, hybrid_bound, safety_counts};

    /// Owner, 26 and 27 Nov 2026: every configuration is measured with the one accepted file (q8_0), never with f16: a
    /// discrete card measures q8_0 and its pair, a computer without one measures q8_0 alone and it is the night model there
    /// (step 3c: it finishes wherever f16 does and is about twice as fast on a processor).
    #[test]
    fn every_configuration_measures_hinglish_q8_0_and_never_f16() {
        assert_eq!(hinglish_candidates(true), ["hinglish-q8_0"]);
        assert_eq!(hinglish_candidates(false), ["hinglish-q8_0"]);
        assert!(
            wb_capability::HINGLISH_PAIRS
                .iter()
                .all(|(m, _)| hinglish_candidates(true).contains(m)),
            "the check must measure the file of every accepted pair"
        );
    }

    /// Owner, 27 Nov 2026: the rules accept only the Hinglish ids the manifest lists. The list in the rules, the manifest
    /// (the file the app ships and whose hash is in the fingerprint, so a change of it makes an old `capability.json`
    /// fall back to safe defaults: `crates/wb-capability/src/fingerprint.rs`, `inuse.rs::choose_in_use`) and the ids the
    /// check measures must be one set. An id that is in the manifest and not in the rules (or the other way round) fails here.
    #[test]
    fn every_hinglish_id_of_the_manifest_is_accepted_by_the_rules_and_only_those() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("models")
            .join("manifest.json");
        let manifest = wb_models::Manifest::load(&path).expect("models/manifest.json loads");
        let mut in_manifest: Vec<&str> = manifest
            .models
            .iter()
            .map(|m| m.id.as_str())
            .filter(|id| id.starts_with("hinglish"))
            .collect();
        in_manifest.sort_unstable();
        let mut accepted: Vec<&str> = wb_capability::HINGLISH_MODELS.to_vec();
        accepted.sort_unstable();
        assert_eq!(in_manifest, accepted, "the manifest and the rules disagree");
        // Owner, 27 Nov 2026: f16 and q5_0 are not shipped; this fails if either comes back into the manifest.
        for gone in ["hinglish-f16", "hinglish-q5_0"] {
            assert!(
                !in_manifest.contains(&gone),
                "{gone} must not be in the shipped manifest"
            );
        }
        assert!(
            hinglish_candidates(true)
                .iter()
                .all(|c| accepted.contains(c)),
            "the check measures only ids the rules accept"
        );
    }

    #[test]
    fn the_hybrid_gain_is_a_ratio_not_a_fraction() {
        // 1.15 means fifteen percent faster than the best single device.
        assert!(extra_too_small(0.07, 1.15), "7% extra cannot reach 15%");
        assert!(
            extra_too_small(0.12, 1.15),
            "12% extra is under the 15% rule"
        );
        assert!(!extra_too_small(0.15, 1.15), "exactly 15% can");
        assert!(!extra_too_small(0.45, 1.15), "a fast processor can add 45%");
        // emu-mid: the adapter at 0.82 and a processor at 2.4 add 34% in isolation: not ruled out.
        assert!(!extra_too_small(0.82 / 2.4, 1.15));
    }

    #[test]
    fn a_processor_round_that_timed_out_is_bounded_by_both_decodes() {
        // The 13 Nov emu-low run: the adapter at 0.83, a 10 s clip, 120 s. The bench decodes the clip twice,
        // so the processor could add 14% at the most: under the 15% rule, so the hybrid is ruled out by prediction.
        let most = hybrid_bound(0.83, 10.0, 120.0);
        assert!((most - 0.1383).abs() < 0.001, "{most}");
        assert!(extra_too_small(most, 1.15));
        // A slightly faster processor round would not be ruled out.
        assert!(!extra_too_small(hybrid_bound(0.83, 10.0, 100.0), 1.15));
        // A slower adapter makes the bound looser, a faster one tighter.
        assert!(hybrid_bound(0.4, 10.0, 120.0) < hybrid_bound(0.83, 10.0, 120.0));
        assert!(extra_too_small(hybrid_bound(0.4, 10.0, 120.0), 1.15));
    }

    #[test]
    fn the_quick_check_times_its_deciding_cells_on_the_whole_clip() {
        use super::{CheckKind, cell_excerpt_seconds};
        use wb_capability::{DeviceConfig, Lang};
        let gpu = DeviceConfig::Gpu { index: 0 };
        // Hindi and Urdu, first choice, on a graphics adapter: the whole 60 s clip.
        assert_eq!(
            cell_excerpt_seconds(CheckKind::Quick, Lang::Hi, gpu, 0),
            60.0
        );
        assert_eq!(
            cell_excerpt_seconds(CheckKind::Quick, Lang::Ur, gpu, 0),
            60.0
        );
        // Everything else in the quick check keeps 30 s: English, a second choice, the processor.
        assert_eq!(
            cell_excerpt_seconds(CheckKind::Quick, Lang::En, gpu, 0),
            30.0
        );
        assert_eq!(
            cell_excerpt_seconds(CheckKind::Quick, Lang::Hi, gpu, 1),
            30.0
        );
        assert_eq!(
            cell_excerpt_seconds(CheckKind::Quick, Lang::Hi, DeviceConfig::Cpu, 0),
            30.0
        );
        // The full check times every cell on the whole clip.
        assert_eq!(
            cell_excerpt_seconds(CheckKind::Full, Lang::En, gpu, 2),
            60.0
        );
    }

    #[test]
    fn a_hinglish_cell_stops_at_the_time_target_and_never_starves_the_steps_after_it() {
        use super::{HINGLISH_CELL_MIN_S, QUICK_BAR_S, QUICK_TARGET_S, hinglish_cell_cap_s};
        // Started where the proof run's mandatory steps ended: until the target.
        assert_eq!(hinglish_cell_cap_s(187.0), QUICK_TARGET_S - 187.0);
        // Started late: the least limit, well inside the bar.
        assert_eq!(hinglish_cell_cap_s(350.0), HINGLISH_CELL_MIN_S);
        assert!(350.0 + hinglish_cell_cap_s(350.0) < QUICK_BAR_S);
    }

    #[test]
    fn a_step_cut_by_the_time_is_left_for_the_full_check_and_one_that_failed_is_not() {
        use super::not_measured_reason;
        use wb_capability::HinglishGap;
        let cut = not_measured_reason(true, "NVIDIA GeForce (Vulkan)");
        assert_eq!(
            HinglishGap::of_reason(&cut, true),
            HinglishGap::TimeRanOut { pair: true },
            "{cut}"
        );
        let failed = not_measured_reason(false, "NVIDIA GeForce (Vulkan)");
        assert!(matches!(
            HinglishGap::of_reason(&failed, false),
            HinglishGap::DidNotComplete { pair: false, .. }
        ));
        // The reasons the quick check already wrote for the target and the bar are the time's too.
        assert_eq!(
            HinglishGap::of_reason(
                "left for the full check: the quick check aims to end by 360 s and 327 s had passed",
                false
            ),
            HinglishGap::TimeRanOut { pair: false }
        );
    }

    #[test]
    fn every_tail_step_has_its_own_skip_name() {
        use super::TAIL_ORDER;
        let names: std::collections::BTreeSet<&str> =
            TAIL_ORDER.iter().map(|s| s.step_name()).collect();
        assert_eq!(names.len(), TAIL_ORDER.len());
        assert!(names.contains(wb_capability::SKIPPED_HINGLISH));
    }

    #[test]
    fn the_hinglish_step_runs_before_the_steps_that_only_add_information() {
        use super::{TAIL_ORDER, TailStep};
        let at = |s: TailStep| TAIL_ORDER.iter().position(|t| *t == s);
        let hinglish = at(TailStep::Hinglish);
        assert!(hinglish.is_some());
        for later in [TailStep::NearLive, TailStep::LiveStandin, TailStep::Night] {
            assert!(
                hinglish < at(later),
                "{later:?} must come after the Hinglish step"
            );
        }
    }

    /// The first pack proof runs (28 Nov 2026, the Legion, quick check, graphics card): the mandatory steps ended at 187 s and
    /// near-live with the night model took 67 s more, so the Hinglish step started at 254 s with 106 s of the 360 s target
    /// left, needed 120 s, and was skipped: turbo was chosen for English + Hindi and the pair was never measured. In the
    /// new order the step starts at 187 s and the pair step (90 s) still fits after the model's own cell.
    #[test]
    fn the_hinglish_model_and_its_pair_fit_the_quick_target_where_the_proof_run_skipped_them() {
        use super::{HINGLISH_QUICK_MIN_S, PAIR_QUICK_MIN_S, target_allows};
        let (mandatory_end_s, near_live_and_night_s, own_cell_s) = (187.0, 67.0, 30.0);
        // The old order: the Hinglish step came last.
        assert!(!target_allows(
            mandatory_end_s + near_live_and_night_s,
            HINGLISH_QUICK_MIN_S
        ));
        // The new order: it comes first, and the pair step follows the model's own cell.
        assert!(target_allows(mandatory_end_s, HINGLISH_QUICK_MIN_S));
        assert!(target_allows(
            mandatory_end_s + own_cell_s,
            PAIR_QUICK_MIN_S
        ));
    }

    #[test]
    fn a_recorder_safety_run_counts_only_if_it_lasted() {
        assert!(
            !safety_counts(0.4),
            "a run that ended at once proves nothing"
        );
        assert!(
            !safety_counts(15.0),
            "a run cut short by the budget proves nothing"
        );
        assert!(safety_counts(120.0));
        assert!(safety_counts(159.0));
    }
}
