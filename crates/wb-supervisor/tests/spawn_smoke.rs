//! Smoke test of the spawn path with the REAL engine: `wb-engine bench` is started through
//! `EngineProcess::spawn_limited` (suspended, in a Job Object of its own, which nests inside the emulation
//! job when run under `wb-emulate`), first without a memory limit to measure the engine's peak commit, then
//! with the limit the queue would give it (`wb_queue::sched::engine_memory_limit`, owner's amendment D of
//! 14 Nov 2026). Both runs must finish, report a bench result, and the second must stay at or under 70% of its
//! limit. The peak is logged against the limit. Ignored by default: `scripts/gates/spawn-smoke.ps1` runs it.
//!
//! Environment: WB_SMOKE_ENGINE (engine exe), WB_SMOKE_MODEL, WB_SMOKE_VAD, WB_SMOKE_CLIP (WAV), WB_SMOKE_OUT
//! (folder for the report), WB_SMOKE_THREADS (default 3), WB_SMOKE_LANG (default hi),
//! WB_SMOKE_AVAILABLE_MB (the machine's available RAM, for the cap; default 11264).

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use wb_protocol::{
    BenchMode, BenchReport, BenchRequest, Device, Event, ModelRef, Request, Settings,
};
use wb_queue::sched::{engine_memory_limit, over_safe_share};
use wb_supervisor::{EngineProcess, Line, Priority, SpawnSpec};

const MIB: u64 = 1 << 20;

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set"))
}

struct Outcome {
    report: BenchReport,
    job_peak: Option<u64>,
    job_warning: Option<String>,
    seconds: f64,
}

fn run_bench(limit: Option<u64>, tag: &str, out: &std::path::Path) -> Outcome {
    let m = |id: &str, p: String| ModelRef {
        id: id.into(),
        path: p,
    };
    let request = Request::Bench(BenchRequest {
        model: m("smoke-model", env("WB_SMOKE_MODEL")),
        vad_model: m("silero-v6.2.0", env("WB_SMOKE_VAD")),
        lang: std::env::var("WB_SMOKE_LANG").unwrap_or_else(|_| "hi".into()),
        clip: env("WB_SMOKE_CLIP"),
        mode: BenchMode::NearLive,
        device: Device::Cpu,
        threads: std::env::var("WB_SMOKE_THREADS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3),
        runs: 1,
        single_run_above_rtf: Some(2.0),
        settings: Settings::default(),
    });
    let req_path = out.join(format!("smoke-request-{tag}.json"));
    std::fs::write(&req_path, serde_json::to_string(&request).unwrap()).unwrap();
    let spec = SpawnSpec {
        exe: PathBuf::from(env("WB_SMOKE_ENGINE")),
        args: vec![
            "bench".into(),
            "--request".into(),
            req_path.to_string_lossy().into_owned(),
        ],
        priority: Priority::BelowNormal,
        stderr_log: out.join(format!("smoke-engine-{tag}.log")),
        env: Vec::new(),
        // As the app starts its engines in a release build.
        scrub_env: true,
    };
    let started = Instant::now();
    let mut proc = EngineProcess::spawn_limited(&spec, limit).expect("the engine starts");
    let mut report = None;
    let mut error = None;
    loop {
        assert!(
            started.elapsed() < Duration::from_secs(900),
            "the bench took over 15 minutes"
        );
        match proc.events.recv_timeout(Duration::from_millis(250)) {
            Ok(Line::Event(e)) => match *e {
                Event::Bench(r) => report = Some(*r),
                Event::Error { code, message, .. } => error = Some(format!("{code}: {message}")),
                _ => {}
            },
            Ok(Line::Garbled(_)) | Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let exit = proc.wait();
    let seconds = started.elapsed().as_secs_f64();
    let report = report.unwrap_or_else(|| {
        panic!(
            "no bench report (exit {exit:?}, engine error {error:?}); see smoke-engine-{tag}.log"
        )
    });
    Outcome {
        report,
        job_peak: proc.peak_memory_bytes(),
        job_warning: proc.job_warning.clone(),
        seconds,
    }
}

#[cfg(windows)]
#[test]
#[ignore = "needs a built engine, a model and a clip; run scripts/gates/spawn-smoke.ps1"]
fn a_real_engine_bench_starts_suspended_in_its_own_job_and_reports_its_peak_against_its_limit() {
    let out = PathBuf::from(env("WB_SMOKE_OUT"));
    std::fs::create_dir_all(&out).unwrap();
    let available_mb: u64 = std::env::var("WB_SMOKE_AVAILABLE_MB")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(11_264);
    let mut text = String::new();
    let emulation = std::env::var("WB_EMULATE").unwrap_or_else(|_| "none (native)".into());
    writeln!(text, "emulation profile seen by the test: {emulation}").unwrap();
    writeln!(
        text,
        "available RAM given to the limit formula: {available_mb} MiB"
    )
    .unwrap();

    // 1. No limit: the engine's own peak commit, from the job and from the engine's report.
    let first = run_bench(None, "unlimited", &out);
    let job_peak = first.job_peak.expect("the job reports a peak on Windows");
    writeln!(
        text,
        "run 1, no limit: {:.1} s, rtf {:.3}, engine reports peak commit {:.0} MiB, job reports {:.0} MiB, job warning {:?}",
        first.seconds,
        first.report.rtf_median,
        first.report.peak_commit_bytes as f64 / MIB as f64,
        job_peak as f64 / MIB as f64,
        first.job_warning
    )
    .unwrap();
    assert!(
        first.job_warning.is_none(),
        "the engine is in its job: {:?}",
        first.job_warning
    );
    assert!(first.report.segments > 0, "the bench decoded something");

    // 2. The limit the queue would set from that peak.
    let limit = engine_memory_limit(job_peak, available_mb * MIB);
    let second = run_bench(Some(limit), "limited", &out);
    let peak2 = second.job_peak.expect("peak");
    let share = peak2 as f64 * 100.0 / limit as f64;
    writeln!(
        text,
        "run 2, limit {:.0} MiB: {:.1} s, rtf {:.3}, job peak {:.0} MiB = {:.1}% of the limit (smoke bar: at most 70%), job warning {:?}",
        limit as f64 / MIB as f64,
        second.seconds,
        second.report.rtf_median,
        peak2 as f64 / MIB as f64,
        share,
        second.job_warning
    )
    .unwrap();
    std::fs::write(out.join("report.txt"), &text).unwrap();
    println!("{text}");
    assert!(second.job_warning.is_none());
    assert!(second.report.segments > 0);
    assert!(
        !over_safe_share(peak2, limit),
        "peak {peak2} is over 70% of the limit {limit}"
    );
}
