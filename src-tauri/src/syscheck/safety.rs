//! The recorder safety run: a short test recording through the recorder's own code path (a throw-away
//! session in the check's folder) while engines load the machine. A configuration that makes the
//! recorder drop samples is ruled out (AGENTS.md non-negotiable 2).
//!
//! This uses the recorder only through its public API; nothing in `wb-recorder` changed for it.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use wb_capability::{DeviceConfig, RecorderSafety};
use wb_recorder::capture;
use wb_recorder::events::Event;
use wb_recorder::session::SessionSpec;
use wb_recorder::{ActiveSession, StartOptions};

#[derive(Debug, thiserror::Error)]
pub enum SafetyError {
    #[error("no audio input to test the recorder with")]
    NoInput,
    #[error("could not start the test recording: {0}")]
    Start(String),
    #[error("could not stop the test recording: {0}")]
    Stop(String),
}

#[derive(Debug, Default, Clone, Copy)]
struct Counters {
    overruns: u64,
    xruns: u64,
    dropped: u64,
}

pub struct SafetyRun {
    session: ActiveSession,
    root: PathBuf,
    reader: JoinHandle<Counters>,
    done: Arc<AtomicBool>,
    started: Instant,
    config: DeviceConfig,
}

impl SafetyRun {
    /// Starts the test recording under `<work_dir>/recorder-test`.
    pub fn start(
        work_dir: &Path,
        app_version: &str,
        config: DeviceConfig,
    ) -> Result<Self, SafetyError> {
        let inputs = capture::list_inputs().map_err(|e| SafetyError::Start(e.to_string()))?;
        let input = inputs
            .iter()
            .find(|d| d.is_default)
            .or_else(|| inputs.first())
            .ok_or(SafetyError::NoInput)?;
        let root = work_dir.join("recorder-test");
        let _ = std::fs::remove_dir_all(&root);
        let (tx, rx) = mpsc::channel::<Event>();
        let opts = StartOptions::new(root.clone(), None, input.id.clone(), app_version);
        let spec = SessionSpec::unscheduled("System check: recorder safety run");
        let session = ActiveSession::start(&spec, &opts, tx)
            .map_err(|e| SafetyError::Start(e.to_string()))?;
        let done = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&done);
        let reader = std::thread::Builder::new()
            .name("wb-check-recorder-events".into())
            .spawn(move || {
                let mut c = Counters::default();
                loop {
                    match rx.recv_timeout(Duration::from_millis(200)) {
                        Ok(Event::RecorderStatus(s)) => {
                            c = Counters {
                                overruns: s.overrun_samples,
                                xruns: s.xruns,
                                dropped: s.dropped_samples,
                            };
                        }
                        Ok(_) => {}
                        Err(mpsc::RecvTimeoutError::Timeout) if !flag.load(Ordering::Acquire) => {}
                        Err(_) => break,
                    }
                }
                c
            })
            .map_err(|e| SafetyError::Start(e.to_string()))?;
        Ok(Self {
            session,
            root,
            reader,
            done,
            started: Instant::now(),
            config,
        })
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// Stops the recording, deletes it and reports the counters.
    pub fn finish(self) -> Result<RecorderSafety, SafetyError> {
        let duration_s = self.started.elapsed().as_secs_f64();
        let stopped = self
            .session
            .stop()
            .map_err(|e| SafetyError::Stop(e.to_string()))?;
        self.done.store(true, Ordering::Release);
        let streamed = self.reader.join().unwrap_or_default();
        let r = &stopped.recorder;
        // The stop report is final; the last status event can lag it by a quarter second.
        let overruns = r.overrun_samples.max(streamed.overruns);
        let dropped = r.dropped_samples.max(streamed.dropped);
        let xruns = r.xruns.max(streamed.xruns);
        let _ = std::fs::remove_dir_all(&self.root);
        Ok(RecorderSafety {
            config: self.config,
            duration_s,
            overruns: (overruns.min(u64::from(u32::MAX)) as u32)
                .saturating_add(dropped.min(u64::from(u32::MAX)) as u32),
            driver_xruns: xruns.min(u64::from(u32::MAX)) as u32,
            passed: overruns == 0 && dropped == 0 && xruns == 0,
        })
    }
}
