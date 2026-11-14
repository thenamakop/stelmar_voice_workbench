//! The manager against a fake executor and a fake gate: an engine slot does not claim or start a unit while the
//! scheduler's rules hold, runs it when they clear and commits it in order; a rule that begins to hold stops the
//! running engine and leaves its unit pending; and the planner does not start `wb-engine chunks` while the gate
//! refuses, and says why (owner, 14 Nov 2026: the per-part pause finder is processing like any other engine).

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use wb_protocol::SegmentEvent;
use wb_queue::store::{Device, JobState, Segment, UnitState};
use wb_runner::chunks::PartPauses;
use wb_runner::exec::{UnitEnd, UnitExecutor};
use wb_runner::gate::{Grant, StartGate, StartRequest, Wait};
use wb_runner::manager::{Config, Events, LogLevel, Manager, SessionHandle, Slot};
use wb_runner::session_queue::{Claim, SessionQueue};
use wb_supervisor::{Cancel, Committer, Observer, RunReport};

struct Gate {
    paused: Mutex<Option<String>>,
    refuse_starts: Mutex<Option<String>>,
    requests: Mutex<Vec<StartRequest>>,
}

impl Gate {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            paused: Mutex::new(None),
            refuse_starts: Mutex::new(None),
            requests: Mutex::new(Vec::new()),
        })
    }
    fn set_paused(&self, why: Option<&str>) {
        *self.paused.lock().unwrap() = why.map(str::to_owned);
    }
}

impl StartGate for Gate {
    fn paused(&self) -> Option<String> {
        self.paused.lock().unwrap().clone()
    }
    fn check(&self, r: &StartRequest) -> Result<Grant, Wait> {
        self.requests.lock().unwrap().push(r.clone());
        if let Some(reason) = self.paused() {
            return Err(Wait { reason });
        }
        if let Some(reason) = self.refuse_starts.lock().unwrap().clone() {
            return Err(Wait { reason });
        }
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

/// Stages one segment for the unit (what an engine would have written) and reports it done; or, when `hold` is
/// set, waits until cancelled.
struct Fake {
    runs: AtomicU32,
    hold: AtomicBool,
    /// Set once a held engine has reported its peak so far.
    reported: AtomicBool,
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

impl UnitExecutor for Fake {
    fn expected_memory(&self, _claim: &Claim) -> (u64, u64) {
        (1 << 30, 0)
    }

    fn run(
        &self,
        claim: &Claim,
        _grant: Grant,
        cancel: &Cancel,
        observer: &mut dyn Observer,
    ) -> UnitEnd {
        self.runs.fetch_add(1, Ordering::SeqCst);
        if self.hold.load(Ordering::SeqCst) {
            // The engine has reached 700 MiB of its expected 1 GiB.
            observer.memory(700 << 20);
            self.reported.store(true, Ordering::SeqCst);
            while !cancel.is_cancelled() {
                std::thread::sleep(Duration::from_millis(10));
            }
            return UnitEnd::Cancelled;
        }
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

fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wb-runner-mgr-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

struct Rig {
    manager: Arc<Manager>,
    gate: Arc<Gate>,
    log: Arc<Log>,
    fake: Arc<Fake>,
    handle: Arc<SessionHandle>,
    job: i64,
}

fn rig(name: &str, hold: bool) -> Rig {
    let dir = temp(name);
    let mut q = SessionQueue::open(&dir).unwrap();
    let job = q
        .ensure_rolling_job(
            "hall-a",
            serde_json::json!({"hi": "m"}),
            vec!["en".into(), "hi".into()],
            1,
        )
        .unwrap();
    let handle = Arc::new(SessionHandle {
        name: "hall-a".into(),
        queue: Mutex::new(q),
        work_dir: dir.join("work"),
    });
    let gate = Gate::new();
    let log = Arc::new(Log::default());
    let fake = Arc::new(Fake {
        runs: AtomicU32::new(0),
        hold: AtomicBool::new(hold),
        reported: AtomicBool::new(false),
    });
    let manager = Manager::new(
        gate.clone(),
        fake.clone(),
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
    Rig {
        manager,
        gate,
        log,
        fake,
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

fn plan_one_unit(r: &Rig) {
    let mut q = r.handle.queue.lock().unwrap();
    q.add_part(1, 0, 20_000, 48_000, &PartPauses::default())
        .unwrap();
    q.plan_units(r.job, true).unwrap();
}

#[test]
fn a_slot_waits_while_the_rules_hold_then_runs_the_unit_and_commits_it() {
    let r = rig("slot", false);
    plan_one_unit(&r);
    r.gate.set_paused(Some(
        "recording is in progress and this computer does not process while recording",
    ));
    let _slot = r
        .manager
        .start_slot(Slot {
            name: "cpu".into(),
            device: Device::Cpu,
        })
        .unwrap();
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!(
        r.fake.runs.load(Ordering::SeqCst),
        0,
        "no engine starts while the rules hold"
    );
    {
        let q = r.handle.queue.lock().unwrap();
        assert_eq!(
            q.units(r.job).unwrap()[0].state,
            UnitState::Pending,
            "the unit is not shown as running"
        );
    }
    assert!(
        r.log
            .lines
            .lock()
            .unwrap()
            .iter()
            .any(|l| l.contains("waiting, not processing: recording is in progress")),
        "the wait and its reason are logged: {:?}",
        r.log.lines.lock().unwrap()
    );
    // Stop was pressed: the rules clear and the unit runs.
    r.gate.set_paused(None);
    r.manager.poke();
    wait_until("the job to finish", 10, || {
        r.handle.queue.lock().unwrap().jobs().unwrap()[0].state == JobState::Done
    });
    assert_eq!(r.fake.runs.load(Ordering::SeqCst), 1);
    assert_eq!(r.log.committed.lock().unwrap().len(), 1);
    assert!(r.log.problems.lock().unwrap().is_empty());
    assert!(
        r.log
            .lines
            .lock()
            .unwrap()
            .iter()
            .any(|l| l.contains("engine peak 900 MiB against limit 2048 MiB")),
        "every run logs its peak against its limit: {:?}",
        r.log.lines.lock().unwrap()
    );
    r.manager.stop();
}

#[test]
fn a_rule_that_begins_to_hold_stops_the_running_engine_and_leaves_its_unit_pending() {
    let r = rig("cancel", true);
    plan_one_unit(&r);
    let _slot = r
        .manager
        .start_slot(Slot {
            name: "cpu".into(),
            device: Device::Cpu,
        })
        .unwrap();
    wait_until("the engine to start", 5, || {
        r.fake.runs.load(Ordering::SeqCst) == 1
    });
    // The scheduler's rule begins to hold (free RAM under 2 GB): the monitor pauses the gate and cancels.
    r.gate.set_paused(Some("free memory is under 2 GB"));
    r.manager.cancel_running("free memory is under 2 GB");
    wait_until("the unit to be released", 5, || {
        r.handle.queue.lock().unwrap().units(r.job).unwrap()[0].state == UnitState::Pending
    });
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(
        r.fake.runs.load(Ordering::SeqCst),
        1,
        "the engine is not started again while the rule holds"
    );
    r.manager.stop();
}

#[test]
fn the_pause_finder_does_not_start_while_the_gate_refuses_and_the_reason_is_logged() {
    let r = rig("chunks", false);
    *r.gate.refuse_starts.lock().unwrap() = Some(
        "starting needs 3.0 GB free after the engine's peak (3 GB stay free), 2.1 GB are free now"
            .into(),
    );
    let _planner = r.manager.start_planner().unwrap();
    r.manager.part_closed(
        &r.handle,
        r.job,
        1,
        0,
        600_000,
        48_000,
        PathBuf::from("part-0001.wav"),
    );
    wait_until("the wait to be logged", 5, || {
        r.log
            .lines
            .lock()
            .unwrap()
            .iter()
            .any(|l| l.contains("chunks for part 1") && l.contains("waits: starting needs 3.0 GB"))
    });
    // It never started: the engine path does not exist, so a start would have parked the job with an error.
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        r.log.problems.lock().unwrap().is_empty(),
        "{:?}",
        r.log.problems.lock().unwrap()
    );
    {
        let q = r.handle.queue.lock().unwrap();
        assert!(q.known_parts().is_empty());
        assert_ne!(q.jobs().unwrap()[0].state, JobState::Parked);
    }
    // When the gate opens it starts (and, with no engine here, fails and parks the job with the reason: a cut is
    // never guessed).
    *r.gate.refuse_starts.lock().unwrap() = None;
    wait_until("the job to be parked after a failed start", 10, || {
        r.handle.queue.lock().unwrap().jobs().unwrap()[0].state == JobState::Parked
    });
    assert!(
        r.log
            .problems
            .lock()
            .unwrap()
            .iter()
            .any(|p| p.contains("could not find the pauses of part 1")),
        "{:?}",
        r.log.problems.lock().unwrap()
    );
    r.manager.stop();
}

/// Two engines at once (a hybrid): the second must leave room for what the first may still grow by, which is its
/// measured peak minus what it has reached (AGENTS.md memory rules).
#[test]
fn the_second_engine_of_a_hybrid_counts_what_the_first_may_still_grow_by() {
    let r = rig("hybrid", true);
    {
        // 11.7 minutes of audio with a pause at 5 minutes: two units.
        let mut q = r.handle.queue.lock().unwrap();
        let pauses = PartPauses {
            pauses: vec![wb_queue::units::Pause {
                start_ms: 298_000,
                end_ms: 302_000,
            }],
            fallback: vec![],
        };
        q.add_part(1, 0, 700_000, 48_000, &pauses).unwrap();
        q.plan_units(r.job, true).unwrap();
        assert!(
            q.units(r.job).unwrap().len() >= 2,
            "{:?}",
            q.units(r.job).unwrap()
        );
    }
    let _cpu = r
        .manager
        .start_slot(Slot {
            name: "cpu".into(),
            device: Device::Cpu,
        })
        .unwrap();
    wait_until("the first engine to report its memory", 5, || {
        r.fake.reported.load(Ordering::SeqCst)
    });
    let _gpu = r
        .manager
        .start_slot(Slot {
            name: "gpu".into(),
            device: Device::Gpu,
        })
        .unwrap();
    wait_until("the second engine to ask the gate", 5, || {
        r.gate
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|q| q.what.contains("on gpu"))
    });
    let reqs = r.gate.requests.lock().unwrap().clone();
    let first = reqs.iter().find(|q| q.what.contains("on cpu")).unwrap();
    let second = reqs.iter().find(|q| q.what.contains("on gpu")).unwrap();
    assert_eq!(first.other_headroom_bytes, 0, "nothing else was running");
    assert_eq!(
        second.other_headroom_bytes,
        (1 << 30) - (700 << 20),
        "the first engine may still grow by its expected peak minus the 700 MiB it has reached"
    );
    r.manager.stop();
}
