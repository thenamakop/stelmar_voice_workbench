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

#[cfg(windows)]
#[test]
fn two_engines_at_once_each_have_their_own_limit() {
    // The same 100 MiB ask: the engine with room for it finishes while the one beside it, with a smaller
    // limit, dies. A limit shared through one job would treat them alike.
    let run = |name: &'static str, limit: u64| {
        std::thread::spawn(move || {
            let dir = temp(name);
            let mut committer = Committer::open(&dir.join("post-1.jsonl")).expect("open");
            let mut watch = Watch::default();
            let result = run_batch(
                &mock(),
                request(),
                &mut committer,
                &dir,
                &policy(&dir, 100, Some(limit), 0),
                &mut watch,
            );
            (result, watch.notices)
        })
    };
    let roomy = run("pair-roomy", 300 * MIB);
    let tight = run("pair-tight", 70 * MIB);
    let (roomy_result, _) = roomy.join().expect("roomy thread");
    let (tight_result, tight_notices) = tight.join().expect("tight thread");
    assert!(
        roomy_result
            .expect("the roomy engine finishes")
            .done
            .is_some()
    );
    assert!(matches!(tight_result, Err(RunError::GaveUp { .. })));
    assert!(
        tight_notices.iter().any(
            |n| matches!(n, Notice::MemoryLimit { limit_bytes, .. } if *limit_bytes == 70 * MIB)
        ),
        "{tight_notices:?}"
    );
}

/// Row 106 (GATES): under emulation the app's machine view must count what its engines hold. Each engine runs in a
/// Job Object of its own, nested inside the emulation job the app lives in; the app's view (`wb_machine::probe`,
/// the job's commit used) has to include those children, or the scheduler would think there is room that the
/// engines already use. Needs the app to be in a job, so it checks only under `wb-emulate` (the spawn-smoke gate
/// runs it there); natively it says so and passes.
#[cfg(windows)]
#[test]
fn the_apps_job_view_counts_an_engine_running_in_its_own_nested_job() {
    let before = wb_machine::probe();
    let Some(base) = before.job_commit_used_bytes else {
        eprintln!("not inside a job (no wb-emulate): the job view cannot be checked here");
        return;
    };
    struct Peek {
        seen: Vec<u64>,
        free: Vec<u64>,
    }
    impl Observer for Peek {
        fn event(&mut self, _e: &Event) {
            let m = wb_machine::probe();
            if let Some(used) = m.job_commit_used_bytes {
                self.seen.push(used);
            }
            self.free.push(m.available_ram_bytes);
        }
        fn notice(&mut self, _n: &Notice) {}
    }
    let dir = temp("view");
    let mut committer = Committer::open(&dir.join("post-1.jsonl")).expect("open");
    let mut peek = Peek {
        seen: Vec::new(),
        free: Vec::new(),
    };
    // The mock holds 300 MiB for the whole run (40 chunks of 50 ms).
    let mut pol = policy(&dir, 300, Some(900 * MIB), 0);
    for (k, v) in pol.engine_env.iter_mut() {
        if k == "MOCK_CHUNKS" {
            *v = "40".into();
        }
        if k == "MOCK_DELAY_MS" {
            *v = "50".into();
        }
    }
    run_batch(
        &mock(),
        request(),
        &mut committer,
        &dir.join("work"),
        &pol,
        &mut peek,
    )
    .expect("the mock finishes under its limit");
    let most = peek.seen.iter().copied().max().unwrap_or(0);
    eprintln!(
        "job commit used before the engine {} MiB, most while it ran {} MiB, free RAM in the view {:?} MiB",
        base / MIB,
        most / MIB,
        peek.free
            .iter()
            .map(|f| f / MIB)
            .collect::<Vec<_>>()
            .iter()
            .take(3)
            .collect::<Vec<_>>()
    );
    assert!(
        most >= base + 250 * MIB,
        "the app's view saw {} MiB while the engine held 300 MiB on top of {} MiB",
        most / MIB,
        base / MIB
    );
}
