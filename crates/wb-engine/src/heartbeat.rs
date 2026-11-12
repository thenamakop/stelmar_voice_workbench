//! Proof of life while a long decode runs.
//!
//! One chunk can take longer than the supervisor's watchdog on a slow machine (large-v3 q5_0 on the `emu-low`
//! CPU needs about 107 s for a 15 s chunk of Hindi) and the engine sends no segment until the chunk is done.
//! whisper.cpp's callbacks (the abort callback, which ggml's CPU backend polls between graph nodes on every
//! thread, and the progress callback, once per decoding window) call [`tick`]; a thread of its own sends a
//! `heartbeat` event every 2 s while ticks keep arriving. A compute that really hangs stops ticking, the
//! heartbeats stop, and the supervisor's watchdog restarts the engine as before.
//!
//! The callbacks only read and now and then write one atomic, so they cost nothing measurable (checked on the
//! speed cell of `docs/GATES.md`, 12 Nov 2026).

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use wb_protocol::Event;

use crate::out;

static EPOCH: OnceLock<Instant> = OnceLock::new();
static LAST_TICK_MS: AtomicU64 = AtomicU64::new(0);
static UNIT: AtomicU32 = AtomicU32::new(0);
/// While a model call that has no callback of its own runs (the language-detection encoder pass), heartbeats
/// continue until this time (ms on the same clock), bounded so that a call that really hangs is still caught.
static HOLD_UNTIL_MS: AtomicU64 = AtomicU64::new(0);

/// A tick older than this is not "computing" any more: no heartbeat.
const FRESH_MS: u64 = 20_000;
/// Heartbeats are sent this often while ticks are fresh (the contract is at least every 10 s).
const EVERY: Duration = Duration::from_secs(2);

fn now_ms() -> u64 {
    // +1 so that 0 means "never ticked".
    EPOCH.get_or_init(Instant::now).elapsed().as_millis() as u64 + 1
}

/// Called from whisper.cpp's callbacks (any thread) and at the start and end of each model call.
pub fn tick() {
    let now = now_ms();
    let last = LAST_TICK_MS.load(Ordering::Relaxed);
    if last == 0 || now.saturating_sub(last) >= 250 {
        LAST_TICK_MS.store(now, Ordering::Relaxed);
    }
}

/// The unit the heartbeats name.
pub fn set_unit(unit: u32) {
    UNIT.store(unit, Ordering::Relaxed);
}

/// True when a tick arrived recently enough for a heartbeat, or a [`hold`] is still running.
fn computing(now: u64, last: u64, hold_until: u64) -> bool {
    (last != 0 && now.saturating_sub(last) <= FRESH_MS) || now <= hold_until
}

/// Keeps the heartbeats going for at most `max` while one model call runs whose encoder pass has no whisper
/// callback: the language-detection pass (`whisper_lang_detect`) and the first part of every `full` call
/// (the abort callback is polled in the decoder only; a timestamped trace on 12 Nov 2026 showed 38 s without
/// heartbeats in each chunk's encoder for large-v3 q5_0 on the `emu-low` CPU). A call that hangs for longer than `max` stops the heartbeats and the watchdog acts. Ends when
/// the guard is dropped.
pub fn hold(max: Duration) -> Hold {
    tick();
    HOLD_UNTIL_MS.store(now_ms() + max.as_millis() as u64, Ordering::Relaxed);
    Hold
}

pub struct Hold;

impl Drop for Hold {
    fn drop(&mut self) {
        HOLD_UNTIL_MS.store(0, Ordering::Relaxed);
        tick();
    }
}

/// Stops the heartbeat thread when dropped.
pub struct Ticker {
    stop: std::sync::Arc<AtomicBool>,
}

impl Drop for Ticker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Starts the heartbeat thread. The thread writes straight to stdout (one line per event, like every other
/// event) and ends with the returned guard.
pub fn start() -> Ticker {
    let stop = std::sync::Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let _ = std::thread::Builder::new()
        .name("wb-engine-heartbeat".into())
        .spawn(move || {
            while !flag.load(Ordering::Relaxed) {
                std::thread::sleep(EVERY);
                if flag.load(Ordering::Relaxed) {
                    break;
                }
                if computing(
                    now_ms(),
                    LAST_TICK_MS.load(Ordering::Relaxed),
                    HOLD_UNTIL_MS.load(Ordering::Relaxed),
                ) {
                    let sent = out::emit(&Event::Heartbeat {
                        unit: UNIT.load(Ordering::Relaxed),
                        phase: "compute".into(),
                    });
                    if sent.is_err() {
                        break; // the app went away
                    }
                }
            }
        });
    Ticker { stop }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_tick_is_computing_and_a_stale_or_missing_one_is_not() {
        assert!(!computing(5_000, 0, 0), "never ticked");
        assert!(computing(5_000, 4_000, 0));
        assert!(computing(25_000, 5_000, 0), "exactly the freshness limit");
        assert!(!computing(25_001, 5_000, 0), "stale: a hung compute stops the heartbeats");
    }

    #[test]
    fn a_hold_keeps_the_heartbeats_going_without_ticks_but_only_until_its_limit() {
        // Last tick long ago (the detection pass has no callback), hold until 400 s.
        assert!(computing(100_000, 1_000, 400_000), "inside the hold");
        assert!(computing(400_000, 1_000, 400_000), "at the end of the hold");
        assert!(!computing(400_001, 1_000, 400_000), "past the hold: a real hang is caught");
    }

    #[test]
    fn ticks_move_the_clock_but_not_more_often_than_every_250_ms() {
        tick();
        let a = LAST_TICK_MS.load(Ordering::Relaxed);
        assert!(a > 0);
        tick();
        assert_eq!(LAST_TICK_MS.load(Ordering::Relaxed), a, "a second tick right after does not store");
    }
}
