//! The per-engine memory limit and the suspended start, against the mock engine only: an engine that asks for
//! more than its limit dies and is reported as a memory-limit death, one that stays under it finishes, and the
//! run reports each engine's peak so it can be logged next to the limit.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use wb_protocol::{BatchRequest, Device, Event, ModelRef, Settings, Source};
use wb_supervisor::{Committer, Notice, Observer, Policy, RunError, run_batch};

const MIB: u64 = 1 << 20;

fn mock() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_mock_engine"))
}

fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wb-supervisor-mem-{name}-{}", std::process::id()));
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

fn policy(dir: &Path, alloc_mb: u64, limit: Option<u64>, max_restarts: u32) -> Policy {
    Policy {
        watchdog: Duration::from_secs(20),
        load_timeout: Duration::from_secs(20),
        max_restarts,
        backoff_s: vec![0],
        engine_env: vec![
            ("MOCK_PLAN".into(), "ok".into()),
            (
                "MOCK_COUNTER".into(),
                dir.join("launches").to_string_lossy().into_owned(),
            ),
            ("MOCK_CHUNKS".into(), "5".into()),
            ("MOCK_DELAY_MS".into(), "1".into()),
            ("MOCK_ALLOC_MB".into(), alloc_mb.to_string()),
        ],
        memory_limit_bytes: limit,
        ..Policy::default()
    }
}

#[cfg(windows)]
#[test]
fn an_engine_that_reaches_its_memory_limit_dies_and_is_reported_as_a_memory_limit_death() {
    let dir = temp("over");
    let mut committer = Committer::open(&dir.join("post-1.jsonl")).expect("open");
    let mut watch = Watch::default();
    // The engine wants 400 MiB and may have 150: it can never finish.
    let err = run_batch(
        &mock(),
        request(),
        &mut committer,
        &dir,
        &policy(&dir, 400, Some(150 * MIB), 1),
        &mut watch,
    )
    .expect_err("over the limit");
    assert!(
        matches!(err, RunError::GaveUp { restarts: 2, .. }),
        "{err:?}"
    );
    let deaths: Vec<_> = watch
        .notices
        .iter()
        .filter_map(|n| match n {
            Notice::MemoryLimit {
                peak_bytes,
                limit_bytes,
                ..
            } => Some((*peak_bytes, *limit_bytes)),
            _ => None,
        })
        .collect();
    assert_eq!(deaths.len(), 2, "{:?}", watch.notices);
    for (peak, limit) in deaths {
        assert_eq!(limit, 150 * MIB);
        assert!(
            peak <= limit && peak * 100 >= limit * 95,
            "peak {peak} against limit {limit}"
        );
    }
    assert!(
        !watch
            .notices
            .iter()
            .any(|n| matches!(n, Notice::Died { .. })),
        "a memory-limit death is not reported as an ordinary death: {:?}",
        watch.notices
    );
}

#[cfg(windows)]
#[test]
fn an_engine_under_its_limit_finishes_and_the_run_reports_its_peak() {
    let dir = temp("under");
    let mut committer = Committer::open(&dir.join("post-1.jsonl")).expect("open");
    let report = run_batch(
        &mock(),
        request(),
        &mut committer,
        &dir,
        &policy(&dir, 60, Some(400 * MIB), 1),
        &mut Watch::default(),
    )
    .expect("finished");
    assert!(report.done.is_some());
    assert_eq!(report.memory_limit_deaths, 0);
    let peak = report.peak_memory_bytes.expect("peak is known on Windows");
    assert!(
        peak >= 60 * MIB && peak * 100 < 400 * MIB * 70,
        "{peak} is the engine's commit, over the 60 MiB it held and under 70% of its limit"
    );
}

#[test]
fn without_a_limit_nothing_changes() {
    let dir = temp("nolimit");
    let mut committer = Committer::open(&dir.join("post-1.jsonl")).expect("open");
    let report = run_batch(
        &mock(),
        request(),
        &mut committer,
        &dir,
        &policy(&dir, 200, None, 0),
        &mut Watch::default(),
    )
    .expect("finished");
    assert!(report.done.is_some());
    assert_eq!(report.memory_limit_deaths, 0);
}
