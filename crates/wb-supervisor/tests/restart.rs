//! The supervisor against a mock engine that crashes, hangs and fails on demand: no committed
//! segment is lost or repeated, a hung engine is killed and restarted, failures are reported, and an
//! error a restart cannot fix stops the job at once.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use wb_protocol::{BatchRequest, Device, Event, ModelRef, SegmentEvent, Settings, Source};
use wb_supervisor::{Committer, Notice, Observer, Policy, RunError, run_batch};

fn mock() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_mock_engine"))
}

fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "wb-supervisor-restart-{name}-{}",
        std::process::id()
    ));
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
struct Recorder {
    notices: Vec<Notice>,
    segments_seen: u32,
}

impl Observer for Recorder {
    fn event(&mut self, e: &Event) {
        if matches!(e, Event::Segment(_)) {
            self.segments_seen += 1;
        }
    }
    fn notice(&mut self, n: &Notice) {
        self.notices.push(n.clone());
    }
}

fn fast(max_restarts: u32) -> Policy {
    Policy {
        watchdog: Duration::from_secs(2),
        load_timeout: Duration::from_secs(5),
        max_restarts,
        backoff_s: vec![0],
        ..Policy::default()
    }
}

fn with_plan(mut p: Policy, dir: &Path, plan: &str, chunks: u32) -> Policy {
    p.engine_env = vec![
        ("MOCK_PLAN".into(), plan.into()),
        (
            "MOCK_COUNTER".into(),
            dir.join("launches").to_string_lossy().into_owned(),
        ),
        ("MOCK_CHUNKS".into(), chunks.to_string()),
    ];
    p
}

fn committed(path: &Path) -> Vec<SegmentEvent> {
    std::fs::read_to_string(path)
        .expect("read")
        .lines()
        .map(|l| serde_json::from_str(l).expect("a complete segment per line"))
        .collect()
}

fn assert_exactly_once_in_order(segs: &[SegmentEvent], chunks: u32) {
    assert_eq!(segs.len(), chunks as usize, "every chunk exactly once");
    for (i, s) in segs.iter().enumerate() {
        assert_eq!(
            s.chunk as usize,
            i,
            "in audio order: {:?}",
            segs.iter().map(|s| s.chunk).collect::<Vec<_>>()
        );
        assert_eq!(
            (s.start_ms, s.end_ms),
            (i as u64 * 1000, (i as u64 + 1) * 1000)
        );
        assert!(s.text.contains("नमस्ते"));
    }
}

#[test]
fn a_clean_run_commits_every_segment_once_with_no_restart() {
    let d = temp("clean");
    let mut c = Committer::open(&d.join("post-1.jsonl")).expect("open");
    let mut obs = Recorder::default();
    let policy = with_plan(fast(3), &d, "ok", 20);
    let report = run_batch(&mock(), request(), &mut c, &d, &policy, &mut obs)
        .unwrap_or_else(|e| panic!("run: {e:?}; notices {:?}", obs.notices));
    assert_eq!(report.restarts, 0);
    assert!(report.done.is_some());
    assert_eq!(report.committed_until_ms, 20_000);
    assert!(obs.notices.is_empty(), "{:?}", obs.notices);
    assert_exactly_once_in_order(&committed(c.path()), 20);
}

#[test]
fn an_engine_killed_at_several_points_loses_nothing_and_repeats_nothing() {
    let d = temp("die");
    let mut c = Committer::open(&d.join("post-1.jsonl")).expect("open");
    let mut obs = Recorder::default();
    // Dies after 3 segments, then after 7 more, then after 1, then finishes.
    let policy = with_plan(fast(8), &d, "die:3,die:7,die:1,ok", 20);
    let report = run_batch(&mock(), request(), &mut c, &d, &policy, &mut obs).expect("run");
    assert_eq!(report.restarts, 3);
    assert_exactly_once_in_order(&committed(c.path()), 20);
    assert_eq!(c.repeats_refused(), 0);
    let died: Vec<_> = obs
        .notices
        .iter()
        .filter(|n| matches!(n, Notice::Died { .. }))
        .collect();
    assert_eq!(died.len(), 3, "{:?}", obs.notices);
    // The cursor at each death is where the committed transcript ended.
    assert!(
        matches!(
            died[0],
            Notice::Died {
                committed_until_ms: 3_000,
                restart: 1,
                ..
            }
        ),
        "{:?}",
        died[0]
    );
    assert!(
        matches!(
            died[1],
            Notice::Died {
                committed_until_ms: 10_000,
                restart: 2,
                ..
            }
        ),
        "{:?}",
        died[1]
    );
}

#[test]
fn a_hung_engine_is_killed_by_the_watchdog_and_the_job_finishes() {
    let d = temp("hang");
    let mut c = Committer::open(&d.join("post-1.jsonl")).expect("open");
    let mut obs = Recorder::default();
    let policy = with_plan(fast(3), &d, "hang:5,ok", 12);
    let report = run_batch(&mock(), request(), &mut c, &d, &policy, &mut obs).expect("run");
    assert_eq!(report.restarts, 1);
    assert!(
        obs.notices
            .iter()
            .any(|n| matches!(n, Notice::Hung { restart: 1, .. })),
        "{:?}",
        obs.notices
    );
    assert_exactly_once_in_order(&committed(c.path()), 12);
}

#[test]
fn an_engine_that_never_gets_anywhere_is_given_up_on() {
    let d = temp("giveup");
    let mut c = Committer::open(&d.join("post-1.jsonl")).expect("open");
    let mut obs = Recorder::default();
    let policy = with_plan(fast(2), &d, "die:0", 5);
    match run_batch(&mock(), request(), &mut c, &d, &policy, &mut obs) {
        Err(RunError::GaveUp {
            restarts,
            committed_until_ms,
        }) => assert_eq!((restarts, committed_until_ms), (3, 0)),
        other => panic!("expected GaveUp, got {other:?}"),
    }
    assert!(
        obs.notices
            .iter()
            .any(|n| matches!(n, Notice::GaveUp { .. }))
    );
}

#[test]
fn an_error_a_restart_cannot_fix_stops_the_job_without_restarting() {
    let d = temp("fatal");
    let mut c = Committer::open(&d.join("post-1.jsonl")).expect("open");
    let mut obs = Recorder::default();
    let policy = with_plan(fast(5), &d, "fatal:model", 5);
    match run_batch(&mock(), request(), &mut c, &d, &policy, &mut obs) {
        Err(RunError::Fatal(m)) => assert!(m.contains("model"), "{m}"),
        other => panic!("expected Fatal, got {other:?}"),
    }
    assert_eq!(
        std::fs::read_to_string(d.join("launches"))
            .expect("counter")
            .trim(),
        "1",
        "one launch, no restart"
    );
    assert!(
        obs.notices
            .iter()
            .any(|n| matches!(n, Notice::EngineError { code, .. } if code == "model"))
    );
}

#[test]
fn an_engine_that_cannot_be_started_is_reported_and_retried_then_given_up_on() {
    let d = temp("nostart");
    let mut c = Committer::open(&d.join("post-1.jsonl")).expect("open");
    let mut obs = Recorder::default();
    let missing = d.join("wb-engine-nowhere.exe");
    match run_batch(&missing, request(), &mut c, &d, &fast(1), &mut obs) {
        Err(RunError::GaveUp { restarts, .. }) => assert_eq!(restarts, 2),
        other => panic!("expected GaveUp, got {other:?}"),
    }
    assert_eq!(
        obs.notices
            .iter()
            .filter(|n| matches!(n, Notice::StartFailed(_)))
            .count(),
        2
    );
}

#[test]
fn a_resumed_job_continues_after_what_an_earlier_run_committed() {
    let d = temp("resume");
    let path = d.join("post-1.jsonl");
    {
        // An earlier app run committed the first 6 chunks and was closed.
        let mut c = Committer::open(&path).expect("open");
        let mut obs = Recorder::default();
        let policy = with_plan(fast(3), &d, "die:6,die:0", 20);
        let _ = run_batch(&mock(), request(), &mut c, &d, &policy, &mut obs);
        assert_eq!(c.committed_until_ms(), 6_000);
    }
    let _ = std::fs::remove_file(d.join("launches"));
    let mut c = Committer::open(&path).expect("reopen");
    assert_eq!(c.committed_until_ms(), 6_000);
    let mut obs = Recorder::default();
    let policy = with_plan(fast(3), &d, "ok", 20);
    run_batch(&mock(), request(), &mut c, &d, &policy, &mut obs).expect("run");
    assert_exactly_once_in_order(&committed(&path), 20);
}
