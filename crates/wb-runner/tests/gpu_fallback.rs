//! Auditor item 6 of 20 Nov 2026 (owner's brief of 27 Nov 2026): a graphics-only profile whose graphics engine keeps
//! failing (a driver fault) goes on with the best measured processor setup for the rest of the run, says so in the
//! scheduler log and as a notice, and loses no unit; with no measured processor cell the job waits, parked with a clear
//! message, and nothing is lost. Both against a fake graphics engine that always fails.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use wb_protocol::SegmentEvent;
use wb_queue::store::{Device, Job, JobState, Segment, UnitState};
use wb_runner::chunks::PartPauses;
use wb_runner::exec::{CpuFallback, UnitEnd, UnitExecutor};
use wb_runner::gate::{Grant, StartGate, StartRequest, Wait};
use wb_runner::manager::{Config, Events, LogLevel, Manager, SessionHandle, Slot};
use wb_runner::session_queue::{Claim, SessionQueue};
use wb_supervisor::{Cancel, Committer, Observer, RunReport};

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
struct Log {
    lines: Mutex<Vec<String>>,
    problems: Mutex<Vec<String>>,
    committed: Mutex<Vec<Segment>>,
}

impl Events for Log {
    fn committed(&self, _session: &str, segments: &[Segment]) {
        self.committed.lock().unwrap().extend_from_slice(segments);
    }
    fn changed(&self) {}
    fn log(&self, _level: LogLevel, text: &str) {
        self.lines.lock().unwrap().push(text.to_owned());
    }
    fn problem(&self, text: &str) {
        self.problems.lock().unwrap().push(text.to_owned());
    }
}

/// A graphics engine that always dies (a driver fault) and a processor engine that decodes. The processor setup the
/// profile would fall back to is `fallback`.
struct FailingGpu {
    gpu_runs: AtomicU32,
    cpu_runs: AtomicU32,
    /// The `models_by_lang` of every unit that ran on the processor.
    cpu_models: Mutex<Vec<serde_json::Value>>,
    fallback: CpuFallback,
}

fn ev(unit: u32, start: u64, end: u64) -> SegmentEvent {
    SegmentEvent {
        unit,
        chunk: 0,
        start_ms: start,
        end_ms: end,
        text: "x".into(),
        lang: "hi".into(),
        lang_prob: 0.9,
        mixed: false,
        dir: "ltr".into(),
        avg_logprob: -0.3,
        no_speech_prob: 0.01,
        compression_ratio: 1.0,
        model: "m".into(),
        decoded_both_ways: false,
        detection: vec![],
        flags: vec![],
        truncated: false,
        hint_text: None,
    }
}

impl UnitExecutor for FailingGpu {
    fn expected_memory(&self, _claim: &Claim) -> (u64, u64) {
        (1 << 30, 0)
    }

    fn cpu_fallback(&self, _job: &Job) -> CpuFallback {
        self.fallback.clone()
    }

    fn run(
        &self,
        claim: &Claim,
        _grant: Grant,
        _cancel: &Cancel,
        _observer: &mut dyn Observer,
    ) -> UnitEnd {
        if claim.device == Device::Gpu {
            self.gpu_runs.fetch_add(1, Ordering::SeqCst);
            return UnitEnd::Failed("the graphics driver stopped responding".into());
        }
        self.cpu_runs.fetch_add(1, Ordering::SeqCst);
        self.cpu_models
            .lock()
            .unwrap()
            .push(claim.job.models_by_lang.clone());
        let mut c = Committer::open_staged(&claim.staged).unwrap();
        c.commit_chunk(
            &[ev(
                claim.unit.idx,
                claim.unit.start_ms + 100,
                claim.unit.end_ms - 100,
            )],
            claim.unit.end_ms,
        )
        .unwrap();
        UnitEnd::Done(RunReport {
            restarts: 0,
            segments_committed: 1,
            committed_until_ms: claim.unit.end_ms,
            done: None,
            peak_memory_bytes: Some(900 << 20),
            memory_limit_deaths: 0,
        })
    }
}

struct Rig {
    manager: Arc<Manager>,
    log: Arc<Log>,
    exec: Arc<FailingGpu>,
    handle: Arc<SessionHandle>,
    job: i64,
}

fn rig(name: &str, fallback: CpuFallback) -> Rig {
    let dir = std::env::temp_dir().join(format!("wb-runner-gpufb-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut q = SessionQueue::open(&dir).unwrap();
    let job = q
        .ensure_rolling_job(
            "hall-a",
            serde_json::json!({"hi": "gpu-model"}),
            vec!["hi".into()],
            1,
        )
        .unwrap();
    // About twelve minutes of audio: more than one unit.
    q.add_part(1, 0, 700_000, 48_000, &PartPauses::default())
        .unwrap();
    q.plan_units(job, true).unwrap();
    let handle = Arc::new(SessionHandle {
        name: "hall-a".into(),
        queue: Mutex::new(q),
        work_dir: dir.join("work"),
    });
    let log = Arc::new(Log::default());
    let exec = Arc::new(FailingGpu {
        gpu_runs: AtomicU32::new(0),
        cpu_runs: AtomicU32::new(0),
        cpu_models: Mutex::new(Vec::new()),
        fallback,
    });
    let manager = Manager::new(
        Arc::new(OpenGate),
        exec.clone(),
        log.clone(),
        Config {
            chunks_exe: PathBuf::from("this-engine-does-not-exist.exe"),
            models_dir: dir.clone(),
            scrub_env: false,
            chunks_timeout: Duration::from_secs(5),
            gate_poll: Duration::from_millis(50),
            clock_ms: Arc::new(|| 1_000),
            ist_text: Arc::new(|| "t".to_owned()),
        },
    );
    manager.add_session(handle.clone());
    // A graphics-only profile: one graphics slot, no processor slot, no other device to move to.
    manager.set_other_device_available(false);
    Rig {
        manager,
        log,
        exec,
        handle,
        job,
    }
}

fn wait_until(what: &str, secs: u64, mut ok: impl FnMut() -> bool) {
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(secs) {
        if ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out waiting for: {what}");
}

#[test]
fn a_failing_graphics_engine_moves_the_rest_of_the_run_to_the_measured_processor_model_and_loses_no_unit()
 {
    let r = rig(
        "falls-back",
        CpuFallback::Models {
            models: serde_json::json!({"hi": "cpu-model", "*": "cpu-model"}),
            without_hinglish: false,
        },
    );
    let units = r.handle.queue.lock().unwrap().units(r.job).unwrap().len();
    assert!(units >= 2, "the test needs more than one unit: {units}");
    let _slot = r
        .manager
        .start_slot(Slot {
            name: "gpu".into(),
            device: Device::Gpu,
        })
        .unwrap();
    wait_until("the job to finish on the processor", 30, || {
        r.handle.queue.lock().unwrap().jobs().unwrap()[0].state == JobState::Done
    });
    assert_eq!(
        r.exec.gpu_runs.load(Ordering::SeqCst),
        3,
        "the graphics engine is tried three times, then given up on"
    );
    assert_eq!(
        r.exec.cpu_runs.load(Ordering::SeqCst) as usize,
        units,
        "every unit, the one that failed included, ran on the processor"
    );
    let q = r.handle.queue.lock().unwrap();
    assert!(
        q.units(r.job)
            .unwrap()
            .iter()
            .all(|u| u.state == UnitState::Committed),
        "no unit is lost: {:?}",
        q.units(r.job).unwrap()
    );
    drop(q);
    assert!(
        r.exec
            .cpu_models
            .lock()
            .unwrap()
            .iter()
            .all(|m| m["hi"] == "cpu-model"),
        "the units used the processor's model: {:?}",
        r.exec.cpu_models.lock().unwrap()
    );
    assert_eq!(r.log.committed.lock().unwrap().len(), units);
    let said = |texts: &Vec<String>| {
        texts
            .iter()
            .any(|l| l.contains("continues on the processor") && l.contains("cpu-model"))
    };
    assert!(
        said(&r.log.lines.lock().unwrap()),
        "the Queue log says so: {:?}",
        r.log.lines.lock().unwrap()
    );
    assert!(
        said(&r.log.problems.lock().unwrap()),
        "the notice says so: {:?}",
        r.log.problems.lock().unwrap()
    );
    assert!(r.manager.gpu_abandoned());
    r.manager.stop();
}

#[test]
fn with_no_measured_processor_cell_the_job_waits_parked_with_a_clear_message_and_no_unit_is_lost() {
    let r = rig(
        "no-cell",
        CpuFallback::NoCell("the system check has no measured processor speed for hi".into()),
    );
    let _slot = r
        .manager
        .start_slot(Slot {
            name: "gpu".into(),
            device: Device::Gpu,
        })
        .unwrap();
    wait_until("the job to be parked", 20, || {
        r.handle.queue.lock().unwrap().jobs().unwrap()[0].state == JobState::Parked
    });
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(
        r.exec.gpu_runs.load(Ordering::SeqCst),
        3,
        "no endless retries on the graphics engine"
    );
    assert_eq!(
        r.exec.cpu_runs.load(Ordering::SeqCst),
        0,
        "nothing runs on the processor without a measured cell"
    );
    let job = r.handle.queue.lock().unwrap().jobs().unwrap()[0].clone();
    let why = job.error.clone().unwrap_or_default();
    assert!(
        why.contains("no measured processor speed") && why.contains("nothing is lost"),
        "the Queue row says why and that nothing is lost: {why}"
    );
    assert!(
        r.log
            .problems
            .lock()
            .unwrap()
            .iter()
            .any(|p| p.contains("no measured processor speed")),
        "the notice says why: {:?}",
        r.log.problems.lock().unwrap()
    );
    // Nothing is deleted: every unit is still in the job, none committed, and the operator can resume it.
    let q = r.handle.queue.lock().unwrap();
    let units = q.units(r.job).unwrap();
    assert!(!units.is_empty());
    assert!(units.iter().all(|u| u.state != UnitState::Committed));
    drop(q);
    assert!(!r.manager.gpu_abandoned());
    r.manager.stop();
}
