//! Load for the recorder load test: every logical CPU the process may use kept busy at normal
//! priority, and a compute load on one GPU adapter (the integrated GPU under emulation).

use std::hint::black_box;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::Result;
use serde::Serialize;
use wb_recorder::util::{now_ist, rfc3339};

#[derive(Debug, Serialize)]
pub struct StressReport {
    pub started_at: String,
    pub ended_at: String,
    pub seconds: f64,
    pub cpu_threads: u32,
    /// Millions of loop iterations per thread per second (a steady value means no throttling).
    pub cpu_mloops_per_thread_s: f64,
    pub gpu_adapter: Option<String>,
    pub gpu_dispatches: u64,
    pub gpu_error: Option<String>,
}

fn spin(stop: &AtomicBool, counter: &AtomicU64) {
    let mut x = 1.000_1f64;
    while !stop.load(Ordering::Relaxed) {
        for _ in 0..1_000_000 {
            x = black_box(x * 1.000_000_1 + 1e-9);
        }
        counter.fetch_add(1, Ordering::Relaxed);
    }
    black_box(x);
}

/// Runs until `seconds` pass or `stop_file` appears.
pub fn run(
    threads: u32,
    gpu: Option<&str>,
    seconds: u64,
    stop_file: Option<&Path>,
) -> Result<StressReport> {
    let stop = Arc::new(AtomicBool::new(false));
    let loops = Arc::new(AtomicU64::new(0));
    let started_at = rfc3339(now_ist());
    let start = Instant::now();
    let mut handles = Vec::new();
    for _ in 0..threads {
        let (s, l) = (Arc::clone(&stop), Arc::clone(&loops));
        handles.push(std::thread::spawn(move || spin(&s, &l)));
    }
    let gpu_handle = gpu.map(|name| {
        let name = name.to_string();
        let s = Arc::clone(&stop);
        std::thread::spawn(move || {
            #[cfg(windows)]
            {
                crate::win::gpu_load(&name, &s)
                    .map(|(a, n, _)| (a.name, n))
                    .map_err(|e| e.to_string())
            }
            #[cfg(not(windows))]
            {
                let _ = (&name, &s);
                Err::<(String, u64), String>("GPU load needs Windows".into())
            }
        })
    });
    let deadline = Duration::from_secs(seconds);
    while start.elapsed() < deadline && !stop_file.is_some_and(Path::exists) {
        std::thread::sleep(Duration::from_millis(200));
    }
    stop.store(true, Ordering::Relaxed);
    for h in handles {
        let _ = h.join();
    }
    let secs = start.elapsed().as_secs_f64();
    let (gpu_adapter, gpu_dispatches, gpu_error) = match gpu_handle.map(|h| h.join()) {
        Some(Ok(Ok((adapter, n)))) => (Some(adapter), n, None),
        Some(Ok(Err(e))) => (None, 0, Some(e)),
        Some(Err(_)) => (None, 0, Some("GPU thread panicked".into())),
        None => (None, 0, None),
    };
    Ok(StressReport {
        started_at,
        ended_at: rfc3339(now_ist()),
        seconds: secs,
        cpu_threads: threads,
        cpu_mloops_per_thread_s: loops.load(Ordering::Relaxed) as f64
            / f64::from(threads.max(1))
            / secs,
        gpu_adapter,
        gpu_dispatches,
        gpu_error,
    })
}
