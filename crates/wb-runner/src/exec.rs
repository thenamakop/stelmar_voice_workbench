//! Running one work unit on an engine process: builds the batch request from the job and the unit, runs it
//! through the supervisor (suspended start, a job of its own, a memory limit, watchdog, restarts from the staged
//! file's last chunk marker) and reports how it ended. The workers use the [`UnitExecutor`] trait, so their
//! logic is tested with a fake one.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use wb_protocol::{
    BatchRequest, Device as ProtoDevice, LangLogEntry, ModelRef, Settings, Source, Unit,
};
use wb_queue::store::{Device, Job};
use wb_supervisor::{Cancel, Committer, Observer, Policy, RunError, RunReport, run_batch_with};

use crate::gate::Grant;
use crate::session_queue::Claim;

/// How a unit ended.
#[derive(Debug)]
pub enum UnitEnd {
    /// Finished: every chunk of the unit is in its staged file.
    Done(RunReport),
    /// Stopped on request (a pause rule, the operator, shutdown): the staged file resumes it.
    Cancelled,
    /// The supervisor gave up (the engine kept failing, or reported an error a restart cannot fix).
    Failed(String),
}

/// What a job decodes with on the processor when the graphics engine of a graphics-only profile gave up on it
/// (auditor item 6 of 20 Nov 2026).
#[derive(Debug, Clone, PartialEq)]
pub enum CpuFallback {
    /// Not a case for a fallback (the profile has no graphics-only setup, or the executor knows nothing of one).
    NotApplicable,
    /// The best measured processor setup for the job: its `models_by_lang`.
    Models(serde_json::Value),
    /// The profile has no measured processor cell for the job's languages; the text says which. The units wait.
    NoCell(String),
}

/// What the app tells the executor about the session and the machine for one unit.
#[derive(Debug, Clone)]
pub struct UnitContext {
    pub session_dir: PathBuf,
    pub models_by_lang: std::collections::BTreeMap<String, ModelRef>,
    pub vad_model: ModelRef,
    pub languages: Vec<String>,
    /// Every language lock change of the session; the unit gets the entries in its range and the last one before.
    pub lang_log: Vec<LangLogEntry>,
    pub settings: Settings,
    pub threads: u32,
    /// The graphics adapter index in the engine's own device list (a `vulkan` or `cuda` engine), for a GPU slot.
    pub gpu_index: Option<u32>,
}

pub trait UnitExecutor: Send + Sync {
    /// The measured peak commit and integrated-GPU local memory of the engine this claim would start (the start
    /// gate's inputs). An unmeasured model reports the largest peak the profile holds.
    fn expected_memory(&self, claim: &Claim) -> (u64, u64);

    /// The profile's real-time factor for this unit on its device: each language's measured factor for the model
    /// the job uses for it, weighted by `mix_ms` (milliseconds of speech per language the unit produced). The
    /// manager compares it with the time the unit really took, so estimates move to this computer's measured
    /// speed. `None` when the profile has nothing for the unit (the unit then says nothing about speed).
    fn expected_rtf(&self, _claim: &Claim, _mix_ms: &BTreeMap<String, u64>) -> Option<f64> {
        None
    }

    /// Where a graphics-only profile goes when its graphics engine keeps failing (a driver fault): the models the best
    /// measured processor setup uses for this job, or why there is none. The manager asks it when a unit is given up
    /// on the graphics device with no other device to move to, and again for every unit it then runs on the processor.
    fn cpu_fallback(&self, _job: &Job) -> CpuFallback {
        CpuFallback::NotApplicable
    }

    /// Runs the unit to the end (or until `cancel`). The grant carries the per-process memory limit.
    fn run(
        &self,
        claim: &Claim,
        grant: Grant,
        cancel: &Cancel,
        observer: &mut dyn Observer,
    ) -> UnitEnd;
}

/// The lock-log entries a unit needs: those inside its range, and the last one before it (the lock in force
/// when it starts), moved to the unit's start. An entry sits on the session's audio clock.
pub fn lang_log_for_unit(log: &[LangLogEntry], start_ms: u64, end_ms: u64) -> Vec<LangLogEntry> {
    let mut sorted: Vec<&LangLogEntry> = log.iter().collect();
    sorted.sort_by_key(|e| e.audio_ms);
    let mut out = Vec::new();
    if let Some(before) = sorted.iter().rev().find(|e| e.audio_ms <= start_ms) {
        out.push(LangLogEntry {
            audio_ms: start_ms,
            lang: before.lang.clone(),
        });
    }
    out.extend(
        sorted
            .iter()
            .filter(|e| e.audio_ms > start_ms && e.audio_ms < end_ms)
            .map(|e| (*e).clone()),
    );
    out
}

/// The batch request for one unit.
pub fn build_request(claim: &Claim, ctx: &UnitContext, resume_from_ms: u64) -> BatchRequest {
    let device = match (claim.device, ctx.gpu_index) {
        (Device::Gpu, Some(index)) => ProtoDevice::Gpu { index },
        _ => ProtoDevice::Cpu,
    };
    BatchRequest {
        source: Source::Session {
            dir: ctx.session_dir.to_string_lossy().into_owned(),
        },
        units: vec![Unit {
            index: claim.unit.idx,
            start_ms: claim.unit.start_ms,
            end_ms: claim.unit.end_ms,
        }],
        models_by_lang: ctx.models_by_lang.clone(),
        vad_model: ctx.vad_model.clone(),
        languages: ctx.languages.clone(),
        lang_log: lang_log_for_unit(&ctx.lang_log, claim.unit.start_ms, claim.unit.end_ms),
        settings: ctx.settings.clone(),
        device,
        threads: ctx.threads,
        resume_from_ms,
        // One language is decoded as that language; several are detected per chunk.
        detect_language: ctx.languages.len() > 1,
    }
}

/// Runs a unit through the supervisor.
pub fn run_unit(
    engine_exe: &Path,
    claim: &Claim,
    ctx: &UnitContext,
    grant: Grant,
    mut policy: Policy,
    cancel: &Cancel,
    observer: &mut dyn Observer,
) -> UnitEnd {
    policy.memory_limit_bytes = grant.memory_limit_bytes;
    let mut committer = match Committer::open_staged(&claim.staged) {
        Ok(c) => c,
        Err(e) => return UnitEnd::Failed(format!("staged file {}: {e}", claim.staged.display())),
    };
    let resume = committer.committed_until_ms().max(claim.unit.start_ms);
    let request = build_request(claim, ctx, resume);
    let work_dir = ctx
        .session_dir
        .join("work")
        .join(format!("{}-unit-{:04}", claim.job.revision, claim.unit.idx));
    match run_batch_with(
        engine_exe,
        request,
        &mut committer,
        &work_dir,
        &policy,
        observer,
        cancel,
    ) {
        Ok(report) => UnitEnd::Done(report),
        Err(RunError::Cancelled { .. }) => UnitEnd::Cancelled,
        Err(e) => UnitEnd::Failed(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wb_queue::store::{Job, JobKind, JobState, Unit as StoreUnit, UnitState};

    fn entry(ms: u64, lang: Option<&str>) -> LangLogEntry {
        LangLogEntry {
            audio_ms: ms,
            lang: lang.map(str::to_owned),
        }
    }

    #[test]
    fn a_unit_gets_the_lock_in_force_at_its_start_and_the_changes_inside_it() {
        let log = vec![
            entry(10_000, Some("hi")),
            entry(400_000, None),
            entry(450_000, Some("en")),
            entry(700_000, Some("hi")),
        ];
        // Unit 300 s to 600 s: the Hindi lock of 10 s is in force at its start, then detection returns at 400 s and
        // English is locked at 450 s. The change at 700 s is not its business.
        let got = lang_log_for_unit(&log, 300_000, 600_000);
        assert_eq!(
            got,
            vec![
                entry(300_000, Some("hi")),
                entry(400_000, None),
                entry(450_000, Some("en")),
            ]
        );
        // Before any lock nothing is passed.
        assert!(lang_log_for_unit(&log, 0, 5_000).is_empty());
        // A change exactly at the unit's start is the lock in force.
        assert_eq!(
            lang_log_for_unit(&log, 400_000, 500_000),
            vec![entry(400_000, None), entry(450_000, Some("en"))]
        );
    }

    #[test]
    fn a_lock_change_inside_a_unit_reaches_the_request_at_its_audio_time() {
        let claim = Claim {
            job: Job {
                id: 1,
                session: "s".into(),
                kind: JobKind::Rolling,
                models_by_lang: serde_json::json!({}),
                languages: vec!["en".into(), "hi".into()],
                revision: "post-1".into(),
                state: JobState::Running,
                cursor_ms: 0,
                position: 1,
                created_ms: 0,
                started_ms: None,
                finished_ms: None,
                error: None,
            },
            unit: StoreUnit {
                id: 1,
                job: 1,
                idx: 3,
                start_ms: 900_000,
                end_ms: 1_200_000,
                state: UnitState::Running,
                device: Some(Device::Cpu),
                cpu_attempts: 0,
                gpu_attempts: 0,
                last_error: None,
                seam: None,
            },
            device: Device::Cpu,
            staged: PathBuf::from("x"),
        };
        let m = |id: &str| ModelRef {
            id: id.into(),
            path: format!("{id}.bin"),
        };
        let ctx = UnitContext {
            session_dir: PathBuf::from("s"),
            models_by_lang: std::collections::BTreeMap::from([("*".into(), m("small"))]),
            vad_model: m("vad"),
            languages: vec!["en".into(), "hi".into()],
            lang_log: vec![entry(1_000_000, Some("hi")), entry(1_100_000, None)],
            settings: Settings::default(),
            threads: 3,
            gpu_index: None,
        };
        let r = build_request(&claim, &ctx, 900_000);
        assert_eq!(r.units.len(), 1);
        assert_eq!(
            (r.units[0].index, r.units[0].start_ms, r.units[0].end_ms),
            (3, 900_000, 1_200_000)
        );
        assert_eq!(
            r.lang_log,
            vec![entry(1_000_000, Some("hi")), entry(1_100_000, None)]
        );
        assert!(r.detect_language, "two languages are detected per chunk");
        assert_eq!(r.device, ProtoDevice::Cpu);
        assert_eq!(r.resume_from_ms, 900_000);
    }

    #[test]
    fn a_gpu_slot_asks_for_its_adapter_and_a_single_language_is_not_detected() {
        let claim = Claim {
            job: Job {
                id: 1,
                session: "s".into(),
                kind: JobKind::Rolling,
                models_by_lang: serde_json::json!({}),
                languages: vec!["hi".into()],
                revision: "post-1".into(),
                state: JobState::Running,
                cursor_ms: 0,
                position: 1,
                created_ms: 0,
                started_ms: None,
                finished_ms: None,
                error: None,
            },
            unit: StoreUnit {
                id: 1,
                job: 1,
                idx: 0,
                start_ms: 0,
                end_ms: 1_000,
                state: UnitState::Running,
                device: Some(Device::Gpu),
                cpu_attempts: 0,
                gpu_attempts: 0,
                last_error: None,
                seam: None,
            },
            device: Device::Gpu,
            staged: PathBuf::from("x"),
        };
        let m = |id: &str| ModelRef {
            id: id.into(),
            path: format!("{id}.bin"),
        };
        let ctx = UnitContext {
            session_dir: PathBuf::from("s"),
            models_by_lang: std::collections::BTreeMap::from([("*".into(), m("small"))]),
            vad_model: m("vad"),
            languages: vec!["hi".into()],
            lang_log: vec![],
            settings: Settings::default(),
            threads: 2,
            gpu_index: Some(1),
        };
        let r = build_request(&claim, &ctx, 0);
        assert_eq!(r.device, ProtoDevice::Gpu { index: 1 });
        assert!(!r.detect_language);
    }
}
