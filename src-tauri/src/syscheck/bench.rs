//! Runs `wb-engine bench` for one cell of the speed table and returns its report.
//!
//! The engine runs as the batch engine does in production: its own process, below-normal priority,
//! inside the kill-on-close job, with the same scrubbed environment, so the measured speed is the speed
//! the post-processing will get.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use wb_protocol::{BatchRequest, BenchReport, BenchRequest, Event as EngineEvent, Request};
use wb_supervisor::{EngineProcess, Line, Priority, SpawnSpec};

use crate::engines::scrub_engine_env;

#[derive(Debug, thiserror::Error)]
pub enum BenchError {
    #[error("could not write the bench request: {0}")]
    Request(String),
    #[error("could not start the engine: {0}")]
    Start(String),
    #[error("the engine reported: {0}")]
    Engine(String),
    #[error("the engine did not answer within {0} s")]
    Timeout(u64),
    #[error("the engine stopped without a report (exit code {0})")]
    NoReport(String),
    #[error("cancelled")]
    Cancelled,
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Where one bench cell runs and how long it may take.
pub struct Cell<'a> {
    pub exe: &'a Path,
    pub variant: &'a str,
    pub request: BenchRequest,
    pub work_dir: &'a Path,
    pub timeout: Duration,
}

pub fn run_bench(cell: &Cell<'_>, cancel: &AtomicBool) -> Result<BenchReport, BenchError> {
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let req_path: PathBuf = cell.work_dir.join(format!("bench-request-{id}.json"));
    std::fs::create_dir_all(cell.work_dir).map_err(|e| BenchError::Request(e.to_string()))?;
    let text = serde_json::to_string(&Request::Bench(cell.request.clone()))
        .map_err(|e| BenchError::Request(e.to_string()))?;
    std::fs::write(&req_path, text).map_err(|e| BenchError::Request(e.to_string()))?;
    let result = run(cell, &req_path, cancel);
    let _ = std::fs::remove_file(&req_path);
    result
}

fn run(cell: &Cell<'_>, req_path: &Path, cancel: &AtomicBool) -> Result<BenchReport, BenchError> {
    let spec = SpawnSpec {
        exe: cell.exe.to_path_buf(),
        args: vec![
            "bench".into(),
            "--request".into(),
            req_path.to_string_lossy().into_owned(),
        ],
        priority: Priority::BelowNormal,
        stderr_log: cell.work_dir.join(format!("bench-{}.log", cell.variant)),
        env: Vec::new(),
        scrub_env: scrub_engine_env(),
    };
    let mut proc = EngineProcess::spawn(&spec).map_err(|e| BenchError::Start(e.to_string()))?;
    let started = Instant::now();
    let mut report: Option<BenchReport> = None;
    let mut error: Option<String> = None;
    loop {
        if cancel.load(Ordering::Relaxed) {
            proc.kill();
            return Err(BenchError::Cancelled);
        }
        if started.elapsed() > cell.timeout {
            proc.kill();
            return Err(BenchError::Timeout(cell.timeout.as_secs()));
        }
        match proc.events.recv_timeout(Duration::from_millis(250)) {
            Ok(Line::Event(e)) => match *e {
                EngineEvent::Bench(r) => report = Some(*r),
                EngineEvent::Error { code, message, .. } => {
                    error = Some(format!("{code}: {message}"))
                }
                _ => {}
            },
            Ok(Line::Garbled(_)) | Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let exit = proc.wait();
    match (report, error) {
        (Some(r), _) => Ok(r),
        (None, Some(e)) => Err(BenchError::Engine(e)),
        (None, None) => Err(BenchError::NoReport(
            exit.map_or("?".into(), |c| c.to_string()),
        )),
    }
}

/// What a batch decode of a clip came to: the seconds the engine says it took (models loaded before it starts), the audio
/// decoded, and how many chunks had their language detected.
#[derive(Debug, Clone, PartialEq)]
pub struct BatchTiming {
    pub elapsed_s: f64,
    pub audio_s: f64,
    pub chunks: u32,
    pub detections: u32,
    pub truncated_chunks: u32,
    /// The largest working set the engine reported while it decoded.
    pub peak_working_set_bytes: u64,
}

impl BatchTiming {
    /// Processing seconds per second of audio.
    pub fn rtf(&self) -> f64 {
        if self.audio_s > 0.0 {
            self.elapsed_s / self.audio_s
        } else {
            0.0
        }
    }
}

/// A batch decode of one WAV file the way the post-processing runs it (several languages, language detection on, the
/// models of `request.models_by_lang`), for the pair cells of the system check: the engine's own `bench` has no language
/// detection and no second model.
pub fn run_batch(
    exe: &Path,
    variant: &str,
    request: &BatchRequest,
    work_dir: &Path,
    timeout: Duration,
    cancel: &AtomicBool,
) -> Result<BatchTiming, BenchError> {
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let req_path: PathBuf = work_dir.join(format!("batch-request-{id}.json"));
    std::fs::create_dir_all(work_dir).map_err(|e| BenchError::Request(e.to_string()))?;
    let text = serde_json::to_string(&Request::Batch(request.clone()))
        .map_err(|e| BenchError::Request(e.to_string()))?;
    std::fs::write(&req_path, text).map_err(|e| BenchError::Request(e.to_string()))?;
    let result = run_batch_process(exe, variant, &req_path, work_dir, timeout, cancel);
    let _ = std::fs::remove_file(&req_path);
    result
}

fn run_batch_process(
    exe: &Path,
    variant: &str,
    req_path: &Path,
    work_dir: &Path,
    timeout: Duration,
    cancel: &AtomicBool,
) -> Result<BatchTiming, BenchError> {
    let spec = SpawnSpec {
        exe: exe.to_path_buf(),
        args: vec![
            "batch".into(),
            "--request".into(),
            req_path.to_string_lossy().into_owned(),
        ],
        priority: Priority::BelowNormal,
        stderr_log: work_dir.join(format!("batch-{variant}.log")),
        env: Vec::new(),
        scrub_env: scrub_engine_env(),
    };
    let mut proc = EngineProcess::spawn(&spec).map_err(|e| BenchError::Start(e.to_string()))?;
    let started = Instant::now();
    let mut done: Option<wb_protocol::DoneSummary> = None;
    let mut detections = 0u32;
    let mut peak_ws = 0u64;
    let mut error: Option<String> = None;
    loop {
        if cancel.load(Ordering::Relaxed) {
            proc.kill();
            return Err(BenchError::Cancelled);
        }
        if started.elapsed() > timeout {
            proc.kill();
            return Err(BenchError::Timeout(timeout.as_secs()));
        }
        match proc.events.recv_timeout(Duration::from_millis(250)) {
            Ok(Line::Event(e)) => match *e {
                EngineEvent::Done(d) => done = Some(d),
                EngineEvent::Stats(s) => {
                    detections = detections.saturating_add(s.detections);
                    peak_ws = peak_ws.max(s.peak_working_set_bytes);
                }
                EngineEvent::Error { code, message, .. } => {
                    error = Some(format!("{code}: {message}"))
                }
                _ => {}
            },
            Ok(Line::Garbled(_)) | Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let exit = proc.wait();
    match (done, error) {
        (Some(d), _) => Ok(BatchTiming {
            elapsed_s: d.elapsed_s,
            audio_s: d.audio_ms as f64 / 1000.0,
            chunks: d.chunks,
            detections,
            truncated_chunks: d.truncated_chunks,
            peak_working_set_bytes: peak_ws,
        }),
        (None, Some(e)) => Err(BenchError::Engine(e)),
        (None, None) => Err(BenchError::NoReport(
            exit.map_or("?".into(), |c| c.to_string()),
        )),
    }
}
