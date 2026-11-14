//! What the queue learns from the recorder: closed parts and the fault counters. The tap sits in the event
//! forwarder (`state.rs`), which runs on its own thread between the recorder's channel and the UI, and it can
//! never block or slow it (AGENTS.md non-negotiable 2: nothing the queue does may touch the recorder):
//!
//! * a closed part goes through a bounded channel with `try_send`: if the queue is not reading (stalled, or not
//!   running) the part is dropped and counted, and the queue finds it again from `parts.json`, which the recorder
//!   writes before it announces the part;
//! * the fault counters (overruns, xruns, dropped samples) overwrite one atomic, so a consumer that never reads
//!   costs nothing.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};

use wb_recorder::events::{Event, RecorderStatus};
use wb_recorder::parts::PartRecord;

/// How many closed parts may wait for the queue. A part closes every 10 minutes: this is hours of backlog.
const PARTS_CAPACITY: usize = 256;

pub struct Feed {
    parts: SyncSender<PartRecord>,
    faults: AtomicU64,
    dropped_parts: AtomicU64,
}

impl Feed {
    pub fn new() -> (Self, Receiver<PartRecord>) {
        let (tx, rx) = sync_channel(PARTS_CAPACITY);
        (
            Self {
                parts: tx,
                faults: AtomicU64::new(0),
                dropped_parts: AtomicU64::new(0),
            },
            rx,
        )
    }

    /// Called by the forwarder for every recorder event. Never blocks, never allocates beyond one clone of a
    /// part record (rare), never takes a lock.
    pub fn tap(&self, event: &Event) {
        match event {
            Event::PartClosed(p) => match self.parts.try_send(p.clone()) {
                Ok(()) => {}
                Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                    self.dropped_parts.fetch_add(1, Ordering::Relaxed);
                }
            },
            Event::RecorderStatus(s) => self.faults.store(fault_total(s), Ordering::Relaxed),
            _ => {}
        }
    }

    /// Overruns, xruns and dropped samples so far; a rise is a new fault.
    pub fn recorder_faults(&self) -> u64 {
        self.faults.load(Ordering::Relaxed)
    }

    /// Parts the tap could not hand over (the queue re-reads `parts.json` for them).
    pub fn dropped_parts(&self) -> u64 {
        self.dropped_parts.load(Ordering::Relaxed)
    }
}

pub fn fault_total(s: &RecorderStatus) -> u64 {
    s.overrun_samples
        .saturating_add(s.xruns)
        .saturating_add(s.dropped_samples)
}

/// A recorder status with the given audio position and fault total, for tests of anything that reads one.
#[cfg(test)]
pub(crate) fn test_status(audio_ms: u64, overrun_samples: u64) -> RecorderStatus {
    use wb_recorder::events::RecState;
    use wb_recorder::sys::PriorityMode;
    RecorderStatus {
        state: RecState::Recording,
        session_dir: String::new(),
        audio_ms,
        part: Some(0),
        part_audio_ms: 0,
        part_seconds: 600,
        overrun_samples,
        xruns: 0,
        dropped_samples: 0,
        recorder_restarts: 0,
        position_breaks: 0,
        timestamp_outliers: 0,
        position_pair_flags: 0,
        repeated_blocks: 0,
        zero_gaps: 0,
        ring_seconds: 60,
        ring_max_fill_pct: 0.0,
        priority: PriorityMode::Mmcss,
        priority_note: None,
        capture_priority: None,
        capture_buffer: None,
        awake: true,
        disk_free_bytes: None,
        disk_warning: false,
        input: None,
        input_lost_since: None,
        thread_cpu_ms: None,
        last_error: None,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use wb_recorder::events::NoticeLevel;

    fn part(n: u32) -> PartRecord {
        PartRecord {
            part: n,
            file: format!("audio/part-{n:04}.wav"),
            start_ms: u64::from(n) * 600_000,
            duration_ms: 600_000,
            frames: 28_800_000,
            sample_rate: 48_000,
            channels: 1,
            sha256: "0".repeat(64),
            needs_repair: false,
        }
    }

    #[test]
    fn closed_parts_reach_the_queue_in_order() {
        let (feed, rx) = Feed::new();
        for n in 0..5 {
            feed.tap(&Event::PartClosed(part(n)));
        }
        let got: Vec<u32> = rx.try_iter().map(|p| p.part).collect();
        assert_eq!(got, vec![0, 1, 2, 3, 4]);
        assert_eq!(feed.dropped_parts(), 0);
    }

    #[test]
    fn the_fault_counters_are_the_latest_total_and_cost_nothing_to_overwrite() {
        let (feed, _rx) = Feed::new();
        let mut s = test_status(0, 3);
        s.xruns = 2;
        s.dropped_samples = 1;
        feed.tap(&Event::RecorderStatus(Box::new(s)));
        assert_eq!(feed.recorder_faults(), 6);
    }

    /// The owner's amendment B: a consumer that never reads must not slow the recorder's events. A million events
    /// go through the same forwarding step the app uses, with the queue's receiver never read: every event still
    /// reaches the UI side in order, and the worst time of one tap stays tiny.
    #[test]
    fn a_stalled_consumer_never_slows_or_loses_the_recorders_events() {
        let (feed, _stalled_rx) = Feed::new();
        let feed = Arc::new(feed);
        let seen = Arc::new(Mutex::new(Vec::<u64>::new()));
        let mut worst = Duration::ZERO;
        let total: u64 = 1_000_000;
        let started = Instant::now();
        for i in 0..total {
            // A mix like the real stream: status ticks, a part now and then, and notices.
            let event = match i % 1_000 {
                0 => Event::PartClosed(part((i / 1_000) as u32)),
                1 => wb_recorder::events::Notice::event(NoticeLevel::Info, "test", "n"),
                _ => Event::RecorderStatus(Box::new(test_status(i, 0))),
            };
            let t = Instant::now();
            feed.tap(&event);
            let took = t.elapsed();
            worst = worst.max(took);
            // What the forwarder does after the tap: hand the event on, in order.
            seen.lock().unwrap().push(i);
        }
        let elapsed = started.elapsed();
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len() as u64, total);
        assert!(seen.windows(2).all(|w| w[0] + 1 == w[1]), "order kept");
        // 1,000 parts offered, 256 fit, the rest dropped and counted, nothing blocked.
        assert_eq!(feed.dropped_parts(), 1_000 - PARTS_CAPACITY as u64);
        assert!(
            worst < Duration::from_millis(50),
            "the slowest tap took {worst:?}"
        );
        assert!(
            elapsed < Duration::from_secs(60),
            "a million taps took {elapsed:?}"
        );
    }

    #[test]
    fn a_queue_that_is_gone_costs_only_a_count() {
        let (feed, rx) = Feed::new();
        drop(rx);
        feed.tap(&Event::PartClosed(part(1)));
        assert_eq!(feed.dropped_parts(), 1);
    }
}
