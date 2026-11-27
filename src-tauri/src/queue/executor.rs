//! Runs a claimed unit on a real engine process: finds the engine build for the unit's device, resolves the
//! job's model ids to verified model files, builds the batch request and hands it to the supervisor
//! (`wb_runner::exec::run_unit`: suspended start, a Job Object of its own, a memory limit, watchdog, restarts
//! from the staged file's last chunk marker). The process runs at below-normal priority.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use wb_models::{Cache, Manifest};
use wb_protocol::{ModelRef, Settings};
use wb_queue::store::Device;
use wb_runner::exec::{CpuFallback, UnitContext, UnitEnd, UnitExecutor, run_unit};
use wb_runner::gate::Grant;
use wb_runner::session_queue::Claim;
use wb_supervisor::{Cancel, Observer, Policy, Priority};

use super::profile::ProfileView;

/// The id of the Silero VAD model in `models/manifest.json`.
pub const VAD_ID: &str = "silero-v6.2.0";

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What the app knows about the engines and models of this installation, and how the profile in use sets the
/// engines up.
pub struct Environment {
    pub models_dir: PathBuf,
    pub manifest: Manifest,
    /// Engine executable per variant (`avx2`, `avx512`, `vulkan`, `cuda`), from `engines::discover`.
    pub engines: Vec<(String, PathBuf)>,
    /// Only the development and gate builds pass `WB_*` variables on (emulation needs them).
    pub scrub_env: bool,
    pub profile: Arc<dyn Fn() -> Option<ProfileView> + Send + Sync>,
    /// Session folders by session name, for the glossary, the language list and the lock log.
    pub contexts: SessionLookup,
}

/// Finds an open session's folder, glossary and languages by name.
pub type SessionLookup = Arc<dyn Fn(&str) -> Option<SessionInfoForUnits> + Send + Sync>;

/// What a unit needs to know about its session beyond the job.
#[derive(Debug, Clone)]
pub struct SessionInfoForUnits {
    pub dir: PathBuf,
    pub glossary: String,
    pub languages: Vec<String>,
}

/// Four times the time the measured speed needs for the longest chunk (15 s), never under 60 s.
pub fn watchdog_for(rtf: f64) -> Duration {
    let wanted = if rtf.is_finite() && rtf > 0.0 {
        4.0 * 15.0 * rtf
    } else {
        0.0
    };
    Duration::from_secs_f64(wanted.max(60.0))
}

/// How the app starts every batch engine: below-normal priority, a watchdog from the measured speed, and the
/// environment scrubbed of `WB_*` and the device-selection variables where `scrub_env` says so (row 102).
pub fn engine_policy(scrub_env: bool, rtf: f64) -> Policy {
    Policy {
        watchdog: watchdog_for(rtf),
        priority: Priority::BelowNormal,
        scrub_env,
        ..Policy::default()
    }
}

pub struct AppExecutor {
    env: Environment,
    cache: Mutex<Cache>,
    /// Batch threads currently allowed (the scheduler lowers them after a second overrun).
    threads_fewer: std::sync::atomic::AtomicU32,
}

impl AppExecutor {
    pub fn new(env: Environment) -> Self {
        Self {
            env,
            cache: Mutex::new(Cache::default()),
            threads_fewer: std::sync::atomic::AtomicU32::new(0),
        }
    }

    /// The scheduler's "fewer threads" for the batch engines (a second overrun within ten minutes).
    pub fn set_threads_fewer(&self, n: u32) {
        self.threads_fewer
            .store(n, std::sync::atomic::Ordering::SeqCst);
    }

    fn engine_exe(&self, device: Device, profile: &ProfileView) -> Option<(String, PathBuf)> {
        let want: &[&str] = match device {
            Device::Cpu => &["avx512", "avx2"],
            Device::Gpu => &["vulkan", "cuda"],
        };
        // The build the profile measured for this kind of device comes first.
        let measured = profile.engine_build(device == Device::Gpu);
        let (name, path, fallback) = pick_engine(measured, want, &self.env.engines)?;
        if fallback {
            // Auditor item 17 of 20 Nov 2026: the engine's own CPU check keeps another build safe, but the speed
            // the profile measured belongs to the build it measured, so say which one runs instead.
            tracing::warn!(
                device = ?device,
                measured = measured.unwrap_or("none"),
                running = name,
                "the engine build the profile measured is not installed; another build runs the unit"
            );
        }
        Some((name.to_owned(), path.clone()))
    }

    fn model_ref(&self, id: &str) -> Result<ModelRef, String> {
        let mut cache = lock(&self.cache);
        let path =
            wb_models::path_for_engine(&self.env.models_dir, &self.env.manifest, id, &mut cache)?;
        Ok(ModelRef {
            id: id.to_owned(),
            path: path.to_string_lossy().into_owned(),
        })
    }

    fn context(
        &self,
        claim: &Claim,
        profile: &ProfileView,
        info: &SessionInfoForUnits,
    ) -> Result<UnitContext, String> {
        let mut models_by_lang: BTreeMap<String, ModelRef> = BTreeMap::new();
        if let Some(map) = claim.job.models_by_lang.as_object() {
            for (lang, id) in map {
                if let Some(id) = id.as_str() {
                    models_by_lang.insert(lang.clone(), self.model_ref(id)?);
                }
            }
        }
        if models_by_lang.is_empty() {
            return Err("the job has no model for any language".into());
        }
        let gpu = claim.device == Device::Gpu;
        let base = profile.cpu_threads.max(1);
        let fewer = self.threads_fewer.load(std::sync::atomic::Ordering::SeqCst);
        let mut threads = base.saturating_sub(fewer).max(1);
        if gpu {
            // A graphics engine needs few processor threads (its own decoder feed).
            threads = threads.min(2);
        }
        let role = match claim.job.kind {
            wb_queue::store::JobKind::Night => "night",
            _ => "rolling",
        };
        let languages = if info.languages.is_empty() {
            claim.job.languages.clone()
        } else {
            info.languages.clone()
        };
        Ok(UnitContext {
            session_dir: info.dir.clone(),
            models_by_lang,
            vad_model: self.model_ref(VAD_ID)?,
            lang_log: super::langlock::for_job(super::langlock::read(&info.dir), &languages),
            languages,
            settings: Settings {
                role: role.into(),
                glossary: info.glossary.clone(),
                ..Settings::default()
            },
            threads,
            gpu_index: if gpu {
                profile.gpu_index.or(Some(0))
            } else {
                None
            },
        })
    }
}

impl UnitExecutor for AppExecutor {
    fn expected_memory(&self, claim: &Claim) -> (u64, u64) {
        let Some(profile) = (self.env.profile)() else {
            return (u64::MAX / 4, 0);
        };
        let gpu = claim.device == Device::Gpu;
        // The engine loads every distinct model of the job (up to two: the night pass can have the rolling model for
        // English and a large model for Hindi), so their peaks add up (found by the second smoke: 73.6% of a limit
        // that was sized on the larger model alone).
        let (peak, local) = match claim.job.models_by_lang.as_object() {
            Some(map) => profile.memory_for_models(map.values().filter_map(|v| v.as_str()), gpu),
            None => (0, 0),
        };
        if peak == 0 {
            // Nothing measured at all: assume the largest the profile holds, else a figure that refuses to
            // start rather than guessing small (AGENTS.md: measured, not assumed).
            let (p, l) = profile.peak_for("", gpu);
            return (if p == 0 { 6 << 30 } else { p }, l);
        }
        (peak, local)
    }

    fn cpu_fallback(&self, job: &wb_queue::store::Job) -> CpuFallback {
        let Some(profile) = (self.env.profile)() else {
            return CpuFallback::NoCell("there is no capability profile".into());
        };
        // Only a graphics-only profile has nothing else to move to; any other setup keeps its own rules.
        if !matches!(
            profile.config,
            Some(wb_capability::DeviceConfig::Gpu { .. })
        ) {
            return CpuFallback::NotApplicable;
        }
        // A job whose models all have a measured processor cell (a night redo planned for the processor) runs as it is.
        if job.models_by_lang.as_object().is_some_and(|m| {
            // A night job only needs its models measured there (the night pass catches up); a day job must also keep up.
            profile.models_measured_on_processor(m, job.kind != wb_queue::store::JobKind::Night)
        }) {
            return CpuFallback::Models {
                models: job.models_by_lang.clone(),
                without_hinglish: false,
            };
        }
        let rankings = crate::syscheck::rankings::from_manifest(&self.env.manifest);
        let night = job.kind == wb_queue::store::JobKind::Night;
        let ranked = |lang: &str| -> Vec<String> {
            let Some(l) = wb_capability::Lang::parse(lang) else {
                return Vec::new();
            };
            let mut list: Vec<String> = Vec::new();
            let sources = if night {
                [&rankings.night, &rankings.rolling]
            } else {
                [&rankings.rolling, &rankings.rolling]
            };
            for source in sources {
                for r in source.get(&l).map(Vec::as_slice).unwrap_or_default() {
                    if !list.contains(&r.model) {
                        list.push(r.model.clone());
                    }
                }
            }
            list
        };
        match profile.cpu_fallback_models(&job.languages, &ranked) {
            Ok(m) => CpuFallback::Models {
                models: super::models_json(&m),
                without_hinglish: job
                    .models_by_lang
                    .as_object()
                    .is_some_and(|models| profile.job_decodes_with_hinglish(models)),
            },
            Err(why) => CpuFallback::NoCell(why),
        }
    }

    fn expected_rtf(&self, claim: &Claim, mix_ms: &BTreeMap<String, u64>) -> Option<f64> {
        let profile = (self.env.profile)()?;
        let models = claim.job.models_by_lang.as_object()?;
        profile.expected_rtf(models, mix_ms, claim.device == Device::Gpu)
    }

    fn run(
        &self,
        claim: &Claim,
        grant: Grant,
        cancel: &Cancel,
        observer: &mut dyn Observer,
    ) -> UnitEnd {
        let Some(profile) = (self.env.profile)() else {
            return UnitEnd::Failed(
                "no capability profile: nothing is known about this computer".into(),
            );
        };
        let Some((variant, exe)) = self.engine_exe(claim.device, &profile) else {
            return UnitEnd::Failed(format!(
                "no {} engine build is installed",
                if claim.device == Device::Gpu {
                    "graphics"
                } else {
                    "processor"
                }
            ));
        };
        let Some(info) = (self.env.contexts)(&claim.job.session) else {
            return UnitEnd::Failed(format!("session {} is not open", claim.job.session));
        };
        let ctx = match self.context(claim, &profile, &info) {
            Ok(c) => c,
            Err(e) => return UnitEnd::Failed(e),
        };
        // Watchdog: the engine reports every chunk; a chunk is at most 15 s of audio, so four times the time the
        // slowest measured speed of the job's models needs for one, never under 60 s (closes the row 105 (c) note).
        let gpu = claim.device == Device::Gpu;
        let rtf = claim
            .job
            .models_by_lang
            .as_object()
            .map(|m| {
                m.values()
                    .filter_map(|v| v.as_str())
                    .map(|id| profile.rtf_for(id, gpu))
                    .fold(0.0, f64::max)
            })
            .unwrap_or(0.0);
        let policy = engine_policy(self.env.scrub_env, rtf);
        tracing::info!(
            unit = claim.unit.idx,
            job = claim.job.id,
            engine = variant,
            threads = ctx.threads,
            limit_bytes = grant.memory_limit_bytes.unwrap_or(0),
            "engine starting for a unit"
        );
        run_unit(&exe, claim, &ctx, grant, policy, cancel, observer)
    }
}

/// The engine to start: the build the profile measured first, then the usual order for the device. The flag is true
/// when the build chosen is not the measured one (the measured build is not installed, or the profile has none).
fn pick_engine<'a>(
    measured: Option<&str>,
    want: &[&str],
    installed: &'a [(String, PathBuf)],
) -> Option<(&'a str, &'a PathBuf, bool)> {
    let (name, path) = measured
        .into_iter()
        .chain(want.iter().copied())
        .find_map(|v| installed.iter().find(|(name, _)| name == v))
        .map(|(n, p)| (n.as_str(), p))?;
    Some((name, path, measured.is_some_and(|m| m != name)))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn installed(names: &[&str]) -> Vec<(String, PathBuf)> {
        names
            .iter()
            .map(|n| ((*n).to_owned(), PathBuf::from(format!("wb-engine-{n}.exe"))))
            .collect()
    }

    /// Auditor item 17: the measured build wins when it is installed; otherwise the usual order runs and the
    /// choice is flagged so the executor logs it.
    #[test]
    fn the_measured_engine_build_is_used_and_a_different_one_is_flagged() {
        let all = installed(&["avx2", "avx512", "vulkan"]);
        let cpu = ["avx512", "avx2"];
        let (name, _, fallback) = pick_engine(Some("avx2"), &cpu, &all).unwrap();
        assert_eq!((name, fallback), ("avx2", false));
        let only_avx2 = installed(&["avx2"]);
        let (name, _, fallback) = pick_engine(Some("avx512"), &cpu, &only_avx2).unwrap();
        assert_eq!(
            (name, fallback),
            ("avx2", true),
            "measured avx512 is not installed"
        );
        let (name, _, fallback) = pick_engine(None, &cpu, &all).unwrap();
        assert_eq!(
            (name, fallback),
            ("avx512", false),
            "no measured build: the usual order, nothing to flag"
        );
        assert!(pick_engine(Some("avx2"), &cpu, &installed(&["vulkan"])).is_none());
    }

    /// Row 102: the app's engines get a scrubbed environment in an installed (release) build, and keep `WB_EMULATE`
    /// and the device variables in gate and debug builds, which pass them on purpose.
    #[test]
    fn the_engine_policy_scrubs_wb_variables_in_a_release_build_and_keeps_them_in_gate_and_debug_builds()
     {
        let policy = engine_policy(crate::engines::scrub_engine_env(), 1.0);
        assert_eq!(policy.scrub_env, crate::engines::scrub_engine_env());
        #[cfg(any(debug_assertions, feature = "gate-hooks"))]
        assert!(
            !policy.scrub_env,
            "a gate or debug build must keep WB_EMULATE"
        );
        #[cfg(not(any(debug_assertions, feature = "gate-hooks")))]
        assert!(policy.scrub_env, "an installed build must scrub WB_*");
        assert_eq!(policy.priority, Priority::BelowNormal);
    }

    /// The VAD model every unit asks for must be in the committed manifest (a wrong id would fail every unit).
    #[test]
    fn the_vad_model_id_is_in_the_committed_manifest() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("models")
            .join("manifest.json");
        let manifest = Manifest::load(&path).unwrap();
        assert!(
            manifest.find(VAD_ID).is_ok(),
            "{VAD_ID} is not in models/manifest.json"
        );
    }

    // ---- a Hinglish pair job whose card engine fails (owner, 27 Nov 2026) ----

    use std::sync::atomic::{AtomicU32, Ordering};
    use wb_runner::chunks::PartPauses;
    use wb_runner::gate::{StartGate, StartRequest, Wait};
    use wb_runner::manager::{Config, Events, LogLevel, Manager, SessionHandle, Slot};
    use wb_runner::session_queue::SessionQueue;

    struct OpenGate;
    impl StartGate for OpenGate {
        fn paused(&self) -> Option<String> {
            None
        }
        fn check(&self, _r: &StartRequest) -> Result<Grant, Wait> {
            Ok(Grant {
                memory_limit_bytes: Some(2 << 30),
            })
        }
    }

    #[derive(Default)]
    struct Heard {
        lines: Mutex<Vec<String>>,
        without_hinglish: Mutex<Vec<(String, i64, u32)>>,
    }
    impl Events for Heard {
        fn committed(&self, _session: &str, _segments: &[wb_queue::store::Segment]) {}
        fn changed(&self) {}
        fn log(&self, _level: LogLevel, text: &str) {
            lock(&self.lines).push(text.to_owned());
        }
        fn problem(&self, text: &str) {
            lock(&self.lines).push(text.to_owned());
        }
        fn without_hinglish(&self, session: &str, job: i64, unit: u32) {
            lock(&self.without_hinglish).push((session.to_owned(), job, unit));
        }
    }

    /// The app's own `cpu_fallback` (profile, manifest rankings), with a card engine that always fails and a processor
    /// engine that decodes.
    struct CardFails {
        app: AppExecutor,
        card_runs: AtomicU32,
        cpu_models: Mutex<Vec<serde_json::Value>>,
    }
    impl UnitExecutor for CardFails {
        fn expected_memory(&self, _claim: &Claim) -> (u64, u64) {
            (1 << 30, 0)
        }
        fn cpu_fallback(&self, job: &wb_queue::store::Job) -> CpuFallback {
            self.app.cpu_fallback(job)
        }
        fn run(
            &self,
            claim: &Claim,
            _grant: Grant,
            _cancel: &Cancel,
            _observer: &mut dyn Observer,
        ) -> UnitEnd {
            if claim.device == Device::Gpu {
                self.card_runs.fetch_add(1, Ordering::SeqCst);
                return UnitEnd::Failed("the graphics driver stopped responding".into());
            }
            lock(&self.cpu_models).push(claim.job.models_by_lang.clone());
            let mut c = wb_supervisor::Committer::open_staged(&claim.staged).unwrap();
            c.commit_chunk(&[], claim.unit.end_ms).unwrap();
            UnitEnd::Done(wb_supervisor::RunReport {
                restarts: 0,
                segments_committed: 0,
                committed_until_ms: claim.unit.end_ms,
                done: None,
                peak_memory_bytes: Some(900 << 20),
                memory_limit_deaths: 0,
            })
        }
    }

    /// The profile of a card-only machine whose check chose the Hinglish pair, with the per-language models of the
    /// manifest measured on the processor too (so the fallback has something to pick) and, when `hinglish_on_cpu`, the
    /// Hinglish model measured there as well.
    fn pair_profile(manifest: &Manifest, hinglish_on_cpu: bool) -> ProfileView {
        let mut cap = wb_capability::parse_capability(include_str!(
            "../../../crates/wb-capability/fixtures/tier-g.json"
        ))
        .unwrap();
        cap.decision.config = Some(wb_capability::ConfigChoice {
            config: wb_capability::DeviceConfig::Gpu { index: 0 },
            cpu_threads: 4,
            ..cap.decision.config.clone().unwrap()
        });
        cap.decision.hinglish = wb_capability::Hinglish {
            ok: true,
            rtf: Some(0.3),
            source: "measured".into(),
            model: Some("hinglish-q8_0".into()),
            detector: Some("small-q5_0".into()),
            night_ok: false,
            night_hours: None,
        };
        let mut v = ProfileView::from_capability(&cap, vec![]);
        let rankings = crate::syscheck::rankings::from_manifest(manifest);
        for (lang, code) in [
            (wb_capability::Lang::En, "en"),
            (wb_capability::Lang::Hi, "hi"),
        ] {
            let first = rankings.rolling[&lang][0].model.clone();
            v.set_cell_on_for_test(&first, code, 0.5, false);
        }
        v.set_cell_on_for_test("hinglish-q8_0", "hi", 0.3, true);
        v.set_cell_on_for_test("hinglish-q8_0", "en", 0.12, true);
        if hinglish_on_cpu {
            v.set_cell_on_for_test("hinglish-q8_0", "hi", 1.1, false);
            v.set_cell_on_for_test("hinglish-q8_0", "en", 0.45, false);
        }
        v
    }

    fn night_session(minutes: u64) -> super::super::night_route::NightSession {
        super::super::night_route::NightSession {
            name: "hall-a".into(),
            languages: vec!["en".into(), "hi".into()],
            audio: wb_queue::night::SessionAudio {
                id: "hall-a".into(),
                audio_ms_by_lang: BTreeMap::from([
                    ("en".to_owned(), minutes * 30_000),
                    ("hi".to_owned(), minutes * 30_000),
                ]),
            },
        }
    }

    /// 18:00 IST on 2027-02-08 in epoch ms.
    fn six_pm() -> u64 {
        let day = 86_400_000u64;
        let utc_midnight = 1_802_174_400_000u64 - 1_802_174_400_000 % day;
        utc_midnight - day - 19_800_000 + 18 * 3_600_000
    }

    #[test]
    fn a_hinglish_pair_job_whose_card_engine_fails_falls_back_flags_every_unit_and_is_planned_for_the_night_redo_or_marked()
     {
        let manifest = Manifest::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("models")
                .join("manifest.json"),
        )
        .unwrap();
        let run = |hinglish_on_cpu: bool| {
            let profile = pair_profile(&manifest, hinglish_on_cpu);
            let for_env = profile.clone();
            let app = AppExecutor::new(Environment {
                models_dir: PathBuf::new(),
                manifest: manifest.clone(),
                engines: vec![],
                scrub_env: false,
                profile: Arc::new(move || Some(for_env.clone())),
                contexts: Arc::new(|_| None),
            });
            let exec = Arc::new(CardFails {
                app,
                card_runs: AtomicU32::new(0),
                cpu_models: Mutex::new(Vec::new()),
            });
            let dir = std::env::temp_dir().join(format!(
                "wb-app-pairfb-{hinglish_on_cpu}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let mut q = SessionQueue::open(&dir).unwrap();
            let job = q
                .ensure_rolling_job(
                    "hall-a",
                    serde_json::json!({"en": "small-q5_0", "hi": "hinglish-q8_0", "*": "hinglish-q8_0"}),
                    vec!["en".into(), "hi".into()],
                    1,
                )
                .unwrap();
            q.add_part(1, 0, 700_000, 48_000, &PartPauses::default())
                .unwrap();
            q.plan_units(job, true).unwrap();
            let units = q.units(job).unwrap().len();
            let handle = Arc::new(SessionHandle {
                name: "hall-a".into(),
                queue: Mutex::new(q),
                work_dir: dir.join("work"),
            });
            let heard = Arc::new(Heard::default());
            let manager = Manager::new(
                Arc::new(OpenGate),
                exec.clone(),
                heard.clone(),
                Config {
                    chunks_exe: PathBuf::from("none.exe"),
                    models_dir: dir.clone(),
                    scrub_env: false,
                    chunks_timeout: Duration::from_secs(5),
                    gate_poll: Duration::from_millis(50),
                    clock_ms: Arc::new(|| 1_000),
                    ist_text: Arc::new(|| "t".to_owned()),
                },
            );
            manager.add_session(handle.clone());
            manager.set_other_device_available(false);
            let _slot = manager
                .start_slot(Slot {
                    name: "gpu".into(),
                    device: Device::Gpu,
                })
                .unwrap();
            let t = std::time::Instant::now();
            while handle.queue.lock().unwrap().jobs().unwrap()[0].state
                != wb_queue::store::JobState::Done
            {
                assert!(
                    t.elapsed() < Duration::from_secs(30),
                    "the job never finished"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
            let flagged = handle.queue.lock().unwrap().fallback_units();
            let heard_units = lock(&heard.without_hinglish).clone();
            let cpu_models = lock(&exec.cpu_models).clone();
            manager.stop();
            (
                profile,
                units,
                flagged,
                heard_units,
                cpu_models,
                exec.card_runs.load(Ordering::SeqCst),
            )
        };

        // The Hinglish model was never measured on the processor: the units fall back to the per-language models, every
        // one is flagged in the session folder and announced, and the night routing has no redo to plan: marked.
        let (profile, units, flagged, heard_units, cpu_models, card_runs) = run(false);
        assert_eq!(card_runs, 3, "the card is tried three times");
        assert_eq!(flagged.len(), units, "every unit is flagged: {flagged:?}");
        assert_eq!(heard_units.len(), units, "every unit is announced");
        assert!(
            cpu_models.iter().all(|m| m["hi"] != "hinglish-q8_0"),
            "{cpu_models:?}"
        );
        let redo = super::super::night_route::project_redo(
            &profile,
            true,
            six_pm(),
            &night_session(120),
            1.5,
        );
        assert!(!redo.redone, "{redo:?}");
        assert!(
            redo.text.contains("needs a faster machine"),
            "{}",
            redo.text
        );

        // Measured on the processor too (and the night long enough for it): the same units are flagged and the night
        // pass redoes the session with the Hinglish model.
        let (profile, units, flagged, _, _, _) = run(true);
        assert_eq!(flagged.len(), units);
        let redo = super::super::night_route::project_redo(
            &profile,
            true,
            six_pm(),
            &night_session(120),
            1.5,
        );
        assert!(redo.redone, "{redo:?}");
        assert!(
            redo.text.ends_with("will be redone at night"),
            "{}",
            redo.text
        );
        // ...and marked again when the audio is too long for the night.
        let redo = super::super::night_route::project_redo(
            &profile,
            true,
            six_pm(),
            &night_session(1200),
            1.5,
        );
        assert!(!redo.redone, "{redo:?}");
    }

    #[test]
    fn the_watchdog_follows_the_measured_speed_and_never_goes_under_a_minute() {
        assert_eq!(watchdog_for(0.0), Duration::from_secs(60));
        assert_eq!(watchdog_for(0.5), Duration::from_secs(60));
        assert_eq!(watchdog_for(2.0), Duration::from_secs(120));
        assert_eq!(watchdog_for(f64::NAN), Duration::from_secs(60));
    }
}
