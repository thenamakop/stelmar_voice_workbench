//! What the Phase 4 queue needs from the supervisor, against the mock engine only (the real-engine kill tests
//! stay in `kill_real.rs`): a batch that can be cancelled and resumed, and a staged committer whose chunk-end
//! markers keep a killed run exactly resumable.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use wb_protocol::{BatchRequest, Device, Event, ModelRef, SegmentEvent, Settings, Source};
use wb_supervisor::{
    Cancel, Committer, Notice, Observer, Policy, RunError, run_batch, run_batch_with,
};

fn mock() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_mock_engine"))
}

fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wb-supervisor-queue-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("temp dir");
    d
}

fn request() -> BatchRequest {
    let m = |id: &str| ModelRef {
        id: id.into(),
        path: format!("{id}.bin"),
    };
    BatchRequest {
        source: Source::File {
            path: "unused.wav".into(),
        },
        units: vec![],
        models_by_lang: std::collections::BTreeMap::from([("*".into(), m("mock"))]),
        vad_model: m("vad"),
        languages: vec!["hi".into()],
        lang_log: vec![],
        settings: Settings::default(),
        device: Device::Cpu,
        threads: 1,
        resume_from_ms: 0,
        detect_language: false,
    }
}

#[derive(Default)]
struct Watch {
    notices: Vec<Notice>,
}

impl Observer for Watch {
    fn event(&mut self, _e: &Event) {}
    fn notice(&mut self, n: &Notice) {
        self.notices.push(n.clone());
    }
}

fn policy(dir: &Path, plan: &str, chunks: u32, delay_ms: u32, max_restarts: u32) -> Policy {
    Policy {
        watchdog: Duration::from_secs(5),
        load_timeout: Duration::from_secs(10),
        max_restarts,
        backoff_s: vec![0],
        engine_env: vec![
            ("MOCK_PLAN".into(), plan.into()),
            (
                "MOCK_COUNTER".into(),
                dir.join("launches").to_string_lossy().into_owned(),
            ),
            ("MOCK_CHUNKS".into(), chunks.to_string()),
            ("MOCK_DELAY_MS".into(), delay_ms.to_string()),
        ],
        ..Policy::default()
    }
}

fn segments_in(path: &Path) -> Vec<SegmentEvent> {
    std::fs::read_to_string(path)
        .expect("read")
        .lines()
        .filter_map(|l| serde_json::from_str::<SegmentEvent>(l).ok())
        .collect()
}

fn assert_exactly_once_in_order(segs: &[SegmentEvent], chunks: u32) {
    assert_eq!(segs.len(), chunks as usize, "every chunk exactly once");
    for (i, s) in segs.iter().enumerate() {
        assert_eq!(s.chunk as usize, i, "audio order");
        assert_eq!(
            (s.start_ms, s.end_ms),
            (i as u64 * 1000, (i as u64 + 1) * 1000)
        );
    }
}

#[test]
fn a_cancelled_batch_stops_at_a_chunk_boundary_and_a_later_run_finishes_it_exactly_once() {
    let dir = temp("cancel");
    let path = dir.join("post-1.jsonl");
    let mut committer = Committer::open(&path).expect("open");
    let mut watch = Watch::default();
    let cancel = Cancel::new();
    let canceller = cancel.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(700));
        canceller.cancel();
    });
    let t = Instant::now();
    let err = run_batch_with(
        &mock(),
        request(),
        &mut committer,
        &dir,
        &policy(&dir, "ok", 200, 20, 2),
        &mut watch,
        &cancel,
    )
    .expect_err("cancelled");
    let RunError::Cancelled { committed_until_ms } = err else {
        panic!("expected Cancelled, got {err:?}");
    };
    assert!(
        t.elapsed() < Duration::from_secs(3),
        "stopped promptly: {:?}",
        t.elapsed()
    );
    assert!(
        committed_until_ms > 0 && committed_until_ms < 200_000,
        "{committed_until_ms}"
    );
    assert_eq!(committed_until_ms % 1000, 0, "whole chunks only");
    assert_eq!(committer.committed_until_ms(), committed_until_ms);
    // Resume with a fresh token (the scheduler resumes the job): finishes, nothing repeated, nothing lost.
    let report = run_batch(
        &mock(),
        request(),
        &mut committer,
        &dir,
        &policy(&dir, "ok", 200, 1, 2),
        &mut watch,
    )
    .expect("finished");
    assert!(report.done.is_some());
    assert_exactly_once_in_order(&segments_in(&path), 200);
}

#[test]
fn a_batch_cancelled_before_it_starts_never_starts_an_engine() {
    let dir = temp("cancel-early");
    let mut committer = Committer::open(&dir.join("post-1.jsonl")).expect("open");
    let cancel = Cancel::new();
    cancel.cancel();
    let err = run_batch_with(
        Path::new("this-engine-does-not-exist.exe"),
        request(),
        &mut committer,
        &dir,
        &Policy::default(),
        &mut Watch::default(),
        &cancel,
    )
    .expect_err("cancelled");
    assert!(matches!(
        err,
        RunError::Cancelled {
            committed_until_ms: 0
        }
    ));
}

#[test]
fn a_cancel_during_the_backoff_after_a_failed_start_returns_at_once() {
    let dir = temp("cancel-backoff");
    let mut committer = Committer::open(&dir.join("post-1.jsonl")).expect("open");
    let cancel = Cancel::new();
    let canceller = cancel.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        canceller.cancel();
    });
    let p = Policy {
        backoff_s: vec![60],
        max_restarts: 5,
        ..Policy::default()
    };
    let mut watch = Watch::default();
    let t = Instant::now();
    let err = run_batch_with(
        Path::new("this-engine-does-not-exist.exe"),
        request(),
        &mut committer,
        &dir,
        &p,
        &mut watch,
        &cancel,
    )
    .expect_err("cancelled");
    assert!(matches!(err, RunError::Cancelled { .. }));
    assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
    assert!(matches!(
        watch.notices.first(),
        Some(Notice::StartFailed(_))
    ));
}

#[test]
fn a_staged_file_keeps_a_run_killed_at_several_points_exactly_resumable() {
    let dir = temp("staged-kill");
    let path = dir.join("unit-0000.jsonl");
    let mut committer = Committer::open_staged(&path).expect("open");
    let mut watch = Watch::default();
    // The engine dies after 7 chunks, again after 5 more, then runs to the end.
    let report = run_batch(
        &mock(),
        request(),
        &mut committer,
        &dir,
        &policy(&dir, "die:7,die:5,ok", 30, 2, 4),
        &mut watch,
    )
    .expect("finished");
    assert_eq!(report.restarts, 2);
    assert_exactly_once_in_order(&segments_in(&path), 30);
    // The cursor survives a restart of the whole app: reopening the staged file gives the end of the unit.
    drop(committer);
    let reopened = Committer::open_staged(&path).expect("reopen");
    assert_eq!(
        (reopened.committed_until_ms(), reopened.segments()),
        (30_000, 30)
    );
    // And the file holds one marker per chunk.
    let text = std::fs::read_to_string(&path).expect("read");
    assert_eq!(
        text.lines().filter(|l| l.contains("chunk_end_ms")).count(),
        30
    );
}

#[test]
fn a_staged_run_restarted_after_an_app_crash_continues_from_its_last_marker() {
    let dir = temp("staged-resume");
    let path = dir.join("unit-0000.jsonl");
    {
        let mut c = Committer::open_staged(&path).expect("open");
        // The engine dies for good after 10 chunks (max_restarts 0): the app "crashes" with 10 committed.
        let r = run_batch(
            &mock(),
            request(),
            &mut c,
            &dir,
            &policy(&dir, "die:10", 25, 2, 0),
            &mut Watch::default(),
        );
        assert!(matches!(r, Err(RunError::GaveUp { .. })));
    }
    // Half of the 11th chunk is on disk when the power goes.
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("append");
    use std::io::Write;
    f.write_all(b"{\"unit\":0,\"chunk\":10,\"start_ms\":10000,\"end_ms\":10")
        .expect("torn");
    drop(f);
    let mut c = Committer::open_staged(&path).expect("reopen");
    assert_eq!(c.committed_until_ms(), 10_000);
    let report = run_batch(
        &mock(),
        request(),
        &mut c,
        &dir,
        &policy(&dir, "ok", 25, 1, 2),
        &mut Watch::default(),
    )
    .expect("finished");
    assert!(report.done.is_some());
    assert_exactly_once_in_order(&segments_in(&path), 25);
}
