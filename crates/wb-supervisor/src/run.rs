//! Running a batch job to the end across engine failures.
//!
//! An engine that crashes, is killed or goes quiet for 60 seconds while busy is restarted from the
//! last committed segment (`resume_from_ms`), so no committed segment is repeated and none is lost
//! (AGENTS.md: restarting an engine never repeats or loses a committed segment). Every failure is
//! reported through the observer (the app turns it into a Toast); none of it reaches the recorder.

use std::path::{Path, PathBuf};
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

use wb_protocol::{BatchRequest, DoneSummary, Event};

use crate::commit::Committer;
use crate::process::{EngineProcess, Line, Priority, SpawnSpec};

/// How the supervisor reacts.
#[derive(Debug, Clone)]
pub struct Policy {
    /// No event for this long while the engine is busy: kill it and restart.
    pub watchdog: Duration,
    /// Allowance before the engine says `ready` (loading a large model from a slow disk).
    pub load_timeout: Duration,
    /// Restarts allowed in one job before it gives up.
    pub max_restarts: u32,
    /// Pause before restart number n (1-based), seconds; the last value repeats.
    pub backoff_s: Vec<u64>,
    pub priority: Priority,
    /// Extra environment for every engine started (for example `WB_MODELS_DIR`).
    pub engine_env: Vec<(String, String)>,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            watchdog: Duration::from_secs(60),
            load_timeout: Duration::from_secs(300),
            max_restarts: 8,
            backoff_s: vec![1, 2, 5, 10],
            priority: Priority::BelowNormal,
            engine_env: Vec::new(),
        }
    }
}

/// What happened, for the log and for Toasts. Implementations must not block.
pub trait Observer {
    /// Every event the engine sent, after the supervisor has acted on it.
    fn event(&mut self, _e: &Event) {}
    /// A failure or restart the operator should hear about.
    fn notice(&mut self, _n: &Notice) {}
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    /// The engine could not be started at all.
    StartFailed(String),
    /// The engine ended without finishing; `restart` is the number of the restart that follows.
    Died {
        exit: Option<i32>,
        restart: u32,
        committed_until_ms: u64,
    },
    /// The engine sent nothing for the watchdog period and was killed.
    Hung { silent_s: u64, restart: u32 },
    /// The engine reported a fatal error.
    EngineError { code: String, message: String },
    /// A line on stdout that was not an event.
    Garbled(String),
    /// A segment starting before the cursor was refused (should never happen).
    RepeatRefused { start_ms: u64 },
    /// The engine is not in the kill-on-close job.
    NotInJob(String),
    /// Gave up after too many restarts.
    GaveUp { restarts: u32 },
}

#[derive(Debug, Clone, PartialEq)]
pub struct RunReport {
    pub restarts: u32,
    pub segments_committed: u64,
    pub committed_until_ms: u64,
    pub done: Option<DoneSummary>,
}

#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error("could not write the request file {}: {source}", path.display())]
    Request {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("could not write the transcript: {0}")]
    Commit(std::io::Error),
    #[error("the engine failed {restarts} times; giving up at {committed_until_ms} ms")]
    GaveUp {
        restarts: u32,
        committed_until_ms: u64,
    },
    #[error("the engine reported a fatal error that restarting cannot fix: {0}")]
    Fatal(String),
}

/// Engine errors a restart cannot fix.
fn unrecoverable(code: &str) -> bool {
    matches!(code, "cpu_features" | "model" | "device" | "usage")
}

/// Runs `request` on `exe` until it reports `done`, restarting after failures. `work_dir` holds the
/// request file and the engine's stderr log.
pub fn run_batch(
    exe: &Path,
    mut request: BatchRequest,
    committer: &mut Committer,
    work_dir: &Path,
    policy: &Policy,
    observer: &mut dyn Observer,
) -> Result<RunReport, RunError> {
    let mut restarts = 0u32;
    loop {
        // Resume exactly where the committed transcript ends.
        request.resume_from_ms = committer.committed_until_ms();
        let request_path = work_dir.join(format!("batch-request-{restarts}.json"));
        std::fs::create_dir_all(work_dir).map_err(|source| RunError::Request {
            path: request_path.clone(),
            source,
        })?;
        let json = serde_json::to_vec_pretty(&wb_protocol::Request::Batch(request.clone()))
            .map_err(|e| RunError::Request {
                path: request_path.clone(),
                source: std::io::Error::other(e),
            })?;
        std::fs::write(&request_path, json).map_err(|source| RunError::Request {
            path: request_path.clone(),
            source,
        })?;
        let spec = SpawnSpec {
            exe: exe.to_path_buf(),
            args: vec![
                "batch".into(),
                "--request".into(),
                request_path.to_string_lossy().into_owned(),
            ],
            priority: policy.priority,
            stderr_log: work_dir.join("engine.log"),
            env: policy.engine_env.clone(),
        };
        let mut engine = match EngineProcess::spawn(&spec) {
            Ok(e) => e,
            Err(e) => {
                observer.notice(&Notice::StartFailed(format!("{}: {e}", exe.display())));
                restarts += 1;
                if restarts > policy.max_restarts {
                    observer.notice(&Notice::GaveUp { restarts });
                    return Err(RunError::GaveUp {
                        restarts,
                        committed_until_ms: committer.committed_until_ms(),
                    });
                }
                std::thread::sleep(backoff(policy, restarts));
                continue;
            }
        };
        if let Some(w) = &engine.job_warning {
            observer.notice(&Notice::NotInJob(w.clone()));
        }

        let mut last_event = Instant::now();
        let mut ready = false;
        let mut done: Option<DoneSummary> = None;
        let mut fatal: Option<(String, String)> = None;
        let mut hung = false;
        loop {
            match engine.events.recv_timeout(Duration::from_millis(500)) {
                Ok(Line::Event(e)) => {
                    last_event = Instant::now();
                    match &*e {
                        Event::Ready { .. } => ready = true,
                        Event::Segment(seg) => match committer.commit(seg) {
                            Ok(true) => {}
                            Ok(false) => observer.notice(&Notice::RepeatRefused {
                                start_ms: seg.start_ms,
                            }),
                            Err(err) => {
                                engine.kill();
                                return Err(RunError::Commit(err));
                            }
                        },
                        Event::Done(summary) => done = Some(summary.clone()),
                        Event::Error {
                            code,
                            message,
                            fatal: true,
                        } => {
                            observer.notice(&Notice::EngineError {
                                code: code.clone(),
                                message: message.clone(),
                            });
                            fatal = Some((code.clone(), message.clone()));
                        }
                        _ => {}
                    }
                    observer.event(&e);
                }
                Ok(Line::Garbled(s)) => {
                    last_event = Instant::now();
                    observer.notice(&Notice::Garbled(s));
                }
                Err(RecvTimeoutError::Timeout) => {
                    let limit = if ready {
                        policy.watchdog
                    } else {
                        policy.load_timeout
                    };
                    if done.is_none() && last_event.elapsed() > limit {
                        observer.notice(&Notice::Hung {
                            silent_s: last_event.elapsed().as_secs(),
                            restart: restarts + 1,
                        });
                        engine.kill();
                        hung = true;
                        break;
                    }
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        let exit = if hung { None } else { engine.wait() };
        if let Some(summary) = done {
            return Ok(RunReport {
                restarts,
                segments_committed: committer.segments(),
                committed_until_ms: committer.committed_until_ms(),
                done: Some(summary),
            });
        }
        if let Some((code, message)) = fatal
            && unrecoverable(&code)
        {
            return Err(RunError::Fatal(format!("{code}: {message}")));
        }
        restarts += 1;
        if !hung {
            observer.notice(&Notice::Died {
                exit,
                restart: restarts,
                committed_until_ms: committer.committed_until_ms(),
            });
        }
        if restarts > policy.max_restarts {
            observer.notice(&Notice::GaveUp { restarts });
            return Err(RunError::GaveUp {
                restarts,
                committed_until_ms: committer.committed_until_ms(),
            });
        }
        std::thread::sleep(backoff(policy, restarts));
    }
}

fn backoff(policy: &Policy, restart: u32) -> Duration {
    let i = (restart as usize)
        .saturating_sub(1)
        .min(policy.backoff_s.len().saturating_sub(1));
    Duration::from_secs(policy.backoff_s.get(i).copied().unwrap_or(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_then_repeats_the_last_value() {
        let p = Policy {
            backoff_s: vec![1, 2, 5],
            ..Policy::default()
        };
        let secs = |n| backoff(&p, n).as_secs();
        assert_eq!(
            (secs(1), secs(2), secs(3), secs(4), secs(40)),
            (1, 2, 5, 5, 5)
        );
        assert_eq!(
            backoff(
                &Policy {
                    backoff_s: vec![],
                    ..Policy::default()
                },
                3
            )
            .as_secs(),
            1
        );
    }

    #[test]
    fn only_errors_a_restart_cannot_fix_are_fatal() {
        for c in ["cpu_features", "model", "device", "usage"] {
            assert!(unrecoverable(c));
        }
        for c in ["error", "io", "oom"] {
            assert!(!unrecoverable(c));
        }
    }
}
