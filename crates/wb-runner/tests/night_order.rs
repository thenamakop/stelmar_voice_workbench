//! Owner, 27 Nov 2026 (priority order of the night) and the auditor's finding of the same day: the night plan's order
//! must be the order the night runs in across sessions. Every session's own queue numbers its jobs from 1, so before a
//! night job was ranked the manager took the sessions' night jobs in registration (recording) order, whatever the plan
//! said. Two sessions whose plan order is the reverse of their registration order.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use wb_queue::store::{Device, JobState, Segment};
use wb_runner::chunks::PartPauses;
use wb_runner::exec::{UnitEnd, UnitExecutor};
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

struct Quiet;

impl Events for Quiet {
    fn committed(&self, _session: &str, _segments: &[Segment]) {}
    fn changed(&self) {}
    fn log(&self, _level: LogLevel, _text: &str) {}
    fn problem(&self, _text: &str) {}
}

/// Commits an empty chunk for every unit and notes which session's night unit ran, in order.
#[derive(Default)]
struct Recorder {
    night_runs: Mutex<Vec<String>>,
}

impl UnitExecutor for Recorder {
    fn expected_memory(&self, _claim: &Claim) -> (u64, u64) {
        (1 << 30, 0)
    }

    fn run(
        &self,
        claim: &Claim,
        _grant: Grant,
        _cancel: &Cancel,
        _observer: &mut dyn Observer,
    ) -> UnitEnd {
        if claim.job.revision == "post-2" {
            self.night_runs
                .lock()
                .unwrap()
                .push(claim.job.session.clone());
        }
        let mut c = Committer::open_staged(&claim.staged).unwrap();
        c.commit_chunk(&[], claim.unit.end_ms).unwrap();
        UnitEnd::Done(RunReport {
            restarts: 0,
            segments_committed: 0,
            committed_until_ms: claim.unit.end_ms,
            done: None,
            peak_memory_bytes: Some(900 << 20),
            memory_limit_deaths: 0,
        })
    }
}

fn session(name: &str, dir: &std::path::Path) -> Arc<SessionHandle> {
    let dir = dir.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    let mut q = SessionQueue::open(&dir).unwrap();
    let job = q
        .ensure_rolling_job(name, serde_json::json!({"hi": "m"}), vec!["hi".into()], 1)
        .unwrap();
    q.add_part(1, 0, 200_000, 48_000, &PartPauses::default())
        .unwrap();
    q.plan_units(job, true).unwrap();
    Arc::new(SessionHandle {
        name: name.into(),
        queue: Mutex::new(q),
        work_dir: dir.join("work"),
    })
}

fn manager(dir: &std::path::Path, exec: Arc<Recorder>) -> Arc<Manager> {
    Manager::new(
        Arc::new(OpenGate),
        exec,
        Arc::new(Quiet),
        Config {
            chunks_exe: PathBuf::from("this-engine-does-not-exist.exe"),
            models_dir: dir.to_path_buf(),
            scrub_env: false,
            chunks_timeout: Duration::from_secs(5),
            gate_poll: Duration::from_millis(50),
            clock_ms: Arc::new(|| 1_000),
            ist_text: Arc::new(|| "t".to_owned()),
        },
    )
}

fn wait_until(what: &str, mut ok: impl FnMut() -> bool) {
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(30) {
        if ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out waiting for: {what}");
}

fn all_done(handles: &[Arc<SessionHandle>], kind: wb_queue::store::JobKind) -> bool {
    handles.iter().all(|h| {
        h.queue
            .lock()
            .unwrap()
            .jobs()
            .unwrap()
            .iter()
            .any(|j| j.kind == kind && j.state == JobState::Done)
    })
}

#[test]
fn the_nights_of_two_sessions_run_in_the_plans_order_not_the_recording_order() {
    let dir = std::env::temp_dir().join(format!("wb-runner-night-order-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // "first" is registered (recorded) first; the night plan puts "second" first.
    let first = session("first", &dir);
    let second = session("second", &dir);
    let handles = [first.clone(), second.clone()];

    // The day: both rolling passes finish.
    let exec = Arc::new(Recorder::default());
    let day = manager(&dir, exec.clone());
    day.add_session(first.clone());
    day.add_session(second.clone());
    let _slot = day
        .start_slot(Slot {
            name: "cpu".into(),
            device: Device::Cpu,
        })
        .unwrap();
    wait_until("both rolling passes", || {
        all_done(&handles, wb_queue::store::JobKind::Rolling)
    });
    day.stop();

    // The night: planned for "second" then "first" (ranks 0 and 1), jobs created in the plan's order.
    for (rank, h) in [&second, &first].into_iter().enumerate() {
        let mut q = h.queue.lock().unwrap();
        q.create_night_job(
            &h.name,
            serde_json::json!({"hi": "big"}),
            vec!["hi".into()],
            2,
        )
        .unwrap();
        assert!(q.rank_night_job(rank).unwrap());
    }
    let night_exec = Arc::new(Recorder::default());
    let night = manager(&dir, night_exec.clone());
    night.add_session(first.clone());
    night.add_session(second.clone());
    let _slot = night
        .start_slot(Slot {
            name: "cpu".into(),
            device: Device::Cpu,
        })
        .unwrap();
    wait_until("both night passes", || {
        all_done(&handles, wb_queue::store::JobKind::Night)
    });
    night.stop();
    let order = night_exec.night_runs.lock().unwrap().clone();
    let first_runs = order.iter().take_while(|n| *n == "second").count();
    let second_runs = order.iter().filter(|n| *n == "second").count();
    assert!(second_runs > 0 && order.len() > second_runs, "{order:?}");
    assert_eq!(
        first_runs, second_runs,
        "every unit of the higher-ranked session runs before the other session starts: {order:?}"
    );
}
