//! The gate every engine process passes before it starts: the scheduler's pause rules (tier R, processing while
//! recording refused by the profile, free RAM under 2 GB, battery, overrun, operator pause) and the 3 GB start
//! reserve (AGENTS.md: never start an engine if free RAM after its measured peak would fall under 3 GB).
//!
//! It applies to every engine process, `wb-engine chunks` (the VAD pause finder) included: it is processing, so
//! it waits like a work unit does, and the wait and its reason are logged (owner, 14 Nov 2026).

use std::time::{Duration, Instant};

use wb_supervisor::Cancel;

/// What is about to start.
#[derive(Debug, Clone)]
pub struct StartRequest {
    /// For the log: `chunks for part 3`, `unit 4 of job 1 on the processor`.
    pub what: String,
    /// The engine's measured peak commit (an assumed value, flagged as such in the log, until one is measured).
    pub peak_commit_bytes: u64,
    /// Local memory of an integrated graphics adapter the engine will use (system RAM), else 0.
    pub gpu_local_bytes: u64,
    /// What another engine already running may still grow by (its measured peak minus what it holds).
    pub other_headroom_bytes: u64,
}

/// What the gate allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Grant {
    /// The per-process commit limit to put on the engine (`wb_queue::sched::engine_memory_limit`).
    pub memory_limit_bytes: Option<u64>,
}

/// Why the engine may not start yet. The text is logged and shown on the Queue row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wait {
    pub reason: String,
}

pub trait StartGate: Send + Sync {
    /// The scheduler's pause rules and the start reserve for one engine that is about to start.
    fn check(&self, request: &StartRequest) -> Result<Grant, Wait>;

    /// Only the pause rules (no engine in mind): why nothing may be processed now, or None. A worker asks this
    /// before it claims a unit, so a unit is not shown as running while the rules hold it back.
    fn paused(&self) -> Option<String> {
        None
    }
}

/// What waiting for the gate reports, for the scheduler log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateEvent {
    /// The engine waits (reported when the wait starts and whenever the reason changes).
    Waiting { what: String, reason: String },
    /// The wait is over.
    Cleared { what: String, waited_ms: u64 },
}

/// Waits until the gate lets the engine start. `None` when `cancel` was raised first. `poll` is how often the
/// gate is asked again.
pub fn wait_for_gate(
    gate: &dyn StartGate,
    request: &StartRequest,
    cancel: &Cancel,
    poll: Duration,
    on_event: &mut dyn FnMut(GateEvent),
) -> Option<Grant> {
    let started = Instant::now();
    let mut last_reason: Option<String> = None;
    loop {
        if cancel.is_cancelled() {
            return None;
        }
        match gate.check(request) {
            Ok(grant) => {
                if last_reason.is_some() {
                    on_event(GateEvent::Cleared {
                        what: request.what.clone(),
                        waited_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                    });
                }
                return Some(grant);
            }
            Err(wait) => {
                if last_reason.as_deref() != Some(wait.reason.as_str()) {
                    on_event(GateEvent::Waiting {
                        what: request.what.clone(),
                        reason: wait.reason.clone(),
                    });
                    last_reason = Some(wait.reason);
                }
            }
        }
        // Sleep in short steps so a cancel is seen at once.
        let until = Instant::now() + poll;
        while Instant::now() < until {
            if cancel.is_cancelled() {
                return None;
            }
            std::thread::sleep(Duration::from_millis(20).min(poll));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Script(Mutex<Vec<Result<Grant, Wait>>>);

    impl StartGate for Script {
        fn check(&self, _r: &StartRequest) -> Result<Grant, Wait> {
            let mut v = self.0.lock().unwrap();
            if v.len() > 1 {
                v.remove(0)
            } else {
                v[0].clone()
            }
        }
    }

    fn req() -> StartRequest {
        StartRequest {
            what: "chunks for part 3".into(),
            peak_commit_bytes: 1 << 30,
            gpu_local_bytes: 0,
            other_headroom_bytes: 0,
        }
    }

    fn wait(r: &str) -> Result<Grant, Wait> {
        Err(Wait { reason: r.into() })
    }

    #[test]
    fn an_open_gate_starts_at_once_and_logs_nothing() {
        let gate = Script(Mutex::new(vec![Ok(Grant {
            memory_limit_bytes: Some(5),
        })]));
        let mut log = Vec::new();
        let g = wait_for_gate(
            &gate,
            &req(),
            &Cancel::new(),
            Duration::from_millis(1),
            &mut |e| log.push(e),
        );
        assert_eq!(
            g,
            Some(Grant {
                memory_limit_bytes: Some(5)
            })
        );
        assert!(log.is_empty());
    }

    #[test]
    fn a_wait_and_every_change_of_its_reason_are_logged_once_and_the_end_too() {
        let gate = Script(Mutex::new(vec![
            wait("free memory is under 2 GB"),
            wait("free memory is under 2 GB"),
            wait("processing while recording is not cleared on this computer"),
            Ok(Grant::default()),
        ]));
        let mut log = Vec::new();
        let g = wait_for_gate(
            &gate,
            &req(),
            &Cancel::new(),
            Duration::from_millis(1),
            &mut |e| log.push(e),
        );
        assert!(g.is_some());
        assert_eq!(log.len(), 3, "{log:?}");
        assert!(matches!(&log[0], GateEvent::Waiting { reason, .. } if reason.contains("2 GB")));
        assert!(
            matches!(&log[1], GateEvent::Waiting { reason, .. } if reason.contains("while recording"))
        );
        assert!(matches!(&log[2], GateEvent::Cleared { what, .. } if what == "chunks for part 3"));
    }

    #[test]
    fn a_cancel_ends_the_wait_without_starting() {
        let gate = Script(Mutex::new(vec![wait("on battery")]));
        let cancel = Cancel::new();
        let c = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            c.cancel();
        });
        let t = Instant::now();
        let g = wait_for_gate(&gate, &req(), &cancel, Duration::from_secs(30), &mut |_| {});
        assert_eq!(g, None);
        assert!(t.elapsed() < Duration::from_secs(5));
    }
}
