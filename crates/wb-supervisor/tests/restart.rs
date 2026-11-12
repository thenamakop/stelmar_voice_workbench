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

#[test]
fn an_engine_killed_between_the_two_halves_of_a_split_chunk_loses_and_repeats_no_half() {
    let d = temp("split");
    let mut c = Committer::open(&d.join("post-1.jsonl")).expect("open");
    let mut obs = Recorder::default();
    // Every chunk is two segments (a split chunk). The first launch emits the first half of chunk 3 and dies
    // before the chunk is finished; the second finishes the job.
    let mut policy = with_plan(fast(4), &d, "die_between:3,ok", 8);
    policy.engine_env.push(("MOCK_SPLIT".into(), "1".into()));
    let report = run_batch(&mock(), request(), &mut c, &d, &policy, &mut obs).expect("run");
    assert_eq!(report.restarts, 1);
    let segs = committed(c.path());
    assert_eq!(segs.len(), 16, "two halves of each of 8 chunks");
    for (i, s) in segs.iter().enumerate() {
        let (chunk, half) = (i as u64 / 2, i as u64 % 2);
        assert_eq!(s.chunk as u64, chunk, "in order, no half missing or repeated");
        assert_eq!(s.start_ms, chunk * 1000 + half * 500);
        assert_eq!(s.end_ms, chunk * 1000 + 500 + half * 500);
    }
    assert_eq!(c.repeats_refused(), 0, "the half that was never committed is not refused later");
    // The cursor when the engine died was the end of the last finished chunk, not the middle of chunk 3.
    assert!(
        obs.notices.iter().any(|n| matches!(
            n,
            Notice::Died { committed_until_ms: 3_000, restart: 1, .. }
        )),
        "{:?}",
        obs.notices
    );
}

#[test]
fn a_chunk_is_shown_only_after_all_of_it_is_committed() {
    struct Order {
        shown: Vec<(u32, u64)>,
        file: PathBuf,
        committed_when_shown: Vec<usize>,
    }
    impl Observer for Order {
        fn event(&mut self, e: &Event) {
            if let Event::Segment(s) = e {
                self.shown.push((s.chunk, s.start_ms));
                self.committed_when_shown.push(committed(&self.file).len());
            }
        }
    }
    let d = temp("shown");
    let mut c = Committer::open(&d.join("post-1.jsonl")).expect("open");
    let mut obs = Order {
        shown: vec![],
        file: c.path().to_path_buf(),
        committed_when_shown: vec![],
    };
    let mut policy = with_plan(fast(1), &d, "ok", 3);
    policy.engine_env.push(("MOCK_SPLIT".into(), "1".into()));
    run_batch(&mock(), request(), &mut c, &d, &policy, &mut obs).expect("run");
    assert_eq!(obs.shown.len(), 6);
    // When the first half of a chunk is shown, both halves are already on disk (fsynced before shown).
    for (k, n) in obs.committed_when_shown.iter().enumerate() {
        assert!(*n >= (k / 2 + 1) * 2, "segment {k} shown with only {n} on disk");
    }
}

#[test]
fn a_long_decode_that_sends_heartbeats_is_not_a_hang() {
    let d = temp("heartbeat");
    let mut c = Committer::open(&d.join("post-1.jsonl")).expect("open");
    let mut obs = Recorder::default();
    // The first chunk takes 5 s of "decoding" with no segment, a 2 s watchdog, and a heartbeat every 200 ms.
    let policy = with_plan(fast(3), &d, "slow:5", 4);
    let report = run_batch(&mock(), request(), &mut c, &d, &policy, &mut obs).expect("run");
    assert_eq!(report.restarts, 0, "{:?}", obs.notices);
    assert!(obs.notices.is_empty(), "{:?}", obs.notices);
    assert_exactly_once_in_order(&committed(c.path()), 4);
}

#[test]
fn control_a_long_silent_decode_is_a_hang_and_is_restarted() {
    let d = temp("silent");
    let mut c = Committer::open(&d.join("post-1.jsonl")).expect("open");
    let mut obs = Recorder::default();
    let policy = with_plan(fast(3), &d, "slow_silent:5,ok", 4);
    let report = run_batch(&mock(), request(), &mut c, &d, &policy, &mut obs).expect("run");
    assert_eq!(report.restarts, 1);
    assert!(
        obs.notices
            .iter()
            .any(|n| matches!(n, Notice::Hung { restart: 1, .. })),
        "{:?}",
        obs.notices
    );
    assert_exactly_once_in_order(&committed(c.path()), 4);
}

#[test]
fn a_chunk_with_no_segment_still_moves_the_cursor_so_nothing_is_decoded_twice() {
    let d = temp("skip");
    let mut c = Committer::open(&d.join("post-1.jsonl")).expect("open");
    let mut obs = Recorder::default();
    // Chunk 5 is too short to decode (a warning, no segment); the engine dies right after it, in chunk 6.
    let mut policy = with_plan(fast(3), &d, "die:6,ok", 10);
    policy.engine_env.push(("MOCK_SKIP".into(), "5".into()));
    let report = run_batch(&mock(), request(), &mut c, &d, &policy, &mut obs).expect("run");
    assert_eq!(report.restarts, 1);
    let segs = committed(c.path());
    assert_eq!(segs.len(), 9, "ten chunks, one of them has no segment");
    assert!(segs.iter().all(|s| s.chunk != 5));
    assert!(
        obs.notices.iter().any(|n| matches!(
            n,
            Notice::Died { committed_until_ms: 6_000, .. }
        )),
        "the cursor passed the empty chunk: {:?}",
        obs.notices
    );
    assert_eq!(c.repeats_refused(), 0);
}
