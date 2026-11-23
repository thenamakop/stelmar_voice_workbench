//! App state shared by commands, the event forwarder and the display guard. Nothing here is
//! shared with the recorder thread: it receives commands on its channel and sends events on
//! another, so a panic that poisons one of these locks cannot reach it. Locks are taken with
//! `lock()`, which recovers from poisoning.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde::Serialize;
use tauri::ipc::Channel;
use wb_recorder::events::{Event, MirrorStatus, NoticeLevel, RecorderStatus};
use wb_recorder::parts::PartRecord;
use wb_recorder::recovery::RecoveredSession;
use wb_recorder::session::InputInfo;
use wb_recorder::{ActiveSession, SessionStopped};

use crate::settings::Settings;

/// Locks a mutex, recovering the data if a panic poisoned it.
pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The open session as the UI shows it.
#[derive(Debug, Clone, Serialize)]
pub struct SessionInfo {
    pub dir: String,
    pub title: String,
    pub hall: Option<String>,
    pub scheduled: Option<String>,
    pub expected_languages: Vec<String>,
    pub mode: String,
    pub opened_at: String,
    pub input: InputInfo,
    pub backup_root: Option<String>,
    /// Resumed after a crash.
    pub resumed: bool,
}

/// Everything a freshly loaded UI needs (after a start, or after the display recovered).
#[derive(Debug, Clone, Default, Serialize)]
pub struct Snapshot {
    pub session: Option<SessionInfo>,
    pub recorder: Option<RecorderStatus>,
    pub mirror: Option<MirrorStatus>,
    pub parts: Vec<PartRecord>,
    pub last_stop: Option<SessionStopped>,
    /// Times the WebView2 display was restarted since launch.
    pub display_restarts: u32,
    /// Shown when the display restarted repeatedly.
    pub display_warning: Option<String>,
}

pub struct AppState {
    pub data_dir: PathBuf,
    pub settings: Mutex<Settings>,
    pub session: Mutex<Option<ActiveSession>>,
    pub snapshot: Arc<Mutex<Snapshot>>,
    pub subscribers: Arc<Mutex<Vec<Channel<serde_json::Value>>>>,
    pub recovered: Mutex<Vec<RecoveredSession>>,
    pub recovery_done: AtomicBool,
    /// Sessions a crash left open that the start-up scan could not repair.
    pub recovery_failed: Mutex<Vec<String>>,
    /// A session is open. Read by the window-close and display handlers instead of the session
    /// lock, which a starting or stopping session holds for seconds.
    pub recording: AtomicBool,
    pub events: Sender<Event>,
}

impl AppState {
    /// Creates the state and the receiving end of the recorder event channel.
    pub fn new(data_dir: PathBuf, settings: Settings) -> (Self, Receiver<Event>) {
        let (tx, rx) = mpsc::channel();
        (
            Self {
                data_dir,
                settings: Mutex::new(settings),
                session: Mutex::new(None),
                snapshot: Arc::new(Mutex::new(Snapshot::default())),
                subscribers: Arc::new(Mutex::new(Vec::new())),
                recovered: Mutex::new(Vec::new()),
                recovery_done: AtomicBool::new(false),
                recovery_failed: Mutex::new(Vec::new()),
                recording: AtomicBool::new(false),
                events: tx,
            },
            rx,
        )
    }

    /// Sends an app-level event (not from the recorder) to every subscribed UI.
    pub fn broadcast(&self, value: serde_json::Value) {
        broadcast(&self.subscribers, &value);
    }
}

fn broadcast(subscribers: &Mutex<Vec<Channel<serde_json::Value>>>, value: &serde_json::Value) {
    let mut subs = lock(subscribers);
    // A channel whose webview is gone fails to send: drop it.
    subs.retain(|c| c.send(value.clone()).is_ok());
}

/// Hands one recorder event on: to the post-processing queue's tap, the snapshot, the app log and every
/// subscribed UI. The tap comes first and cannot block (`queue::feed::Feed::tap` uses `try_send` and an atomic),
/// so a queue that is stalled, slow or gone never delays the recorder's events (owner, 14 Nov 2026).
pub(crate) fn forward(
    event: &Event,
    snapshot: &Mutex<Snapshot>,
    subscribers: &Mutex<Vec<Channel<serde_json::Value>>>,
    feed: Option<&crate::queue::feed::Feed>,
) {
    if let Some(feed) = feed {
        feed.tap(event);
    }
    match event {
        Event::RecorderStatus(s) => lock(snapshot).recorder = Some(s.as_ref().clone()),
        Event::MirrorStatus(s) => lock(snapshot).mirror = Some(s.clone()),
        Event::PartClosed(p) => {
            tracing::info!(
                part = p.part,
                start_ms = p.start_ms,
                duration_ms = p.duration_ms,
                "part closed"
            );
            lock(snapshot).parts.push(p.clone());
        }
        Event::Notice(n) => match n.level {
            NoticeLevel::Info => tracing::info!(source = n.source, "{}", n.message),
            NoticeLevel::Warning => tracing::warn!(source = n.source, "{}", n.message),
            NoticeLevel::Error => tracing::error!(source = n.source, "{}", n.message),
        },
        Event::Level(_) => {}
    }
    if let Ok(value) = serde_json::to_value(event) {
        broadcast(subscribers, &value);
    }
}

/// Forwards recorder, mirror and capture events to the UI and the app log, keeping the snapshot
/// current. Runs on its own thread for the life of the app.
pub fn spawn_forwarder(
    rx: Receiver<Event>,
    snapshot: Arc<Mutex<Snapshot>>,
    subscribers: Arc<Mutex<Vec<Channel<serde_json::Value>>>>,
    feed: Option<Arc<crate::queue::feed::Feed>>,
) {
    let spawned = std::thread::Builder::new()
        .name("wb-events".into())
        .spawn(move || {
            for event in rx {
                forward(&event, &snapshot, &subscribers, feed.as_deref());
            }
        });
    if let Err(e) = spawned {
        tracing::error!("could not start the event forwarder: {e}");
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::queue::feed::Feed;
    use std::time::{Duration, Instant};

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

    /// Owner's amendment B (14 Nov 2026): the queue's tap in the forwarder must never block or slow the
    /// recorder's events. The queue's receiver is never read; a million events go through the real forwarding
    /// step; every one still reaches the snapshot, in order, and the slowest step stays far under what a
    /// recorder event could tolerate (the recorder's channel is unbounded and nothing waits for the forwarder,
    /// but a slow tap would still let events pile up behind it).
    #[test]
    fn a_stalled_queue_never_slows_the_forwarding_of_recorder_events() {
        let (feed, _never_read) = Feed::new();
        let snapshot = Mutex::new(Snapshot::default());
        let subscribers: Mutex<Vec<Channel<serde_json::Value>>> = Mutex::new(Vec::new());
        let total: u64 = 1_000_000;
        let mut worst = Duration::ZERO;
        let started = Instant::now();
        for i in 0..total {
            let event = if i % 1_000 == 0 {
                Event::PartClosed(part((i / 1_000) as u32))
            } else {
                Event::RecorderStatus(Box::new(crate::queue::feed::test_status(i, i / 100_000)))
            };
            let t = Instant::now();
            forward(&event, &snapshot, &subscribers, Some(&feed));
            worst = worst.max(t.elapsed());
        }
        let snap = lock(&snapshot);
        assert_eq!(
            snap.parts.len(),
            1_000,
            "every closed part reached the snapshot"
        );
        assert!(
            snap.parts.windows(2).all(|w| w[0].part + 1 == w[1].part),
            "in order"
        );
        assert_eq!(
            snap.recorder.as_ref().map(|r| r.audio_ms),
            Some(total - 1),
            "the latest status is the last one sent"
        );
        // 1,000 parts offered, 256 fit in the queue's channel, the rest dropped and counted (the queue
        // finds them again from parts.json).
        assert_eq!(feed.dropped_parts(), 1_000 - 256);
        assert_eq!(
            feed.recorder_faults(),
            9,
            "the fault total follows the last status"
        );
        eprintln!(
            "stalled queue: {total} events forwarded in {:?}, slowest single step {worst:?}",
            started.elapsed()
        );
        assert!(worst < Duration::from_millis(100), "slowest step {worst:?}");
        assert!(
            started.elapsed() < Duration::from_secs(120),
            "a million events took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn the_forwarder_works_without_a_queue() {
        let snapshot = Mutex::new(Snapshot::default());
        let subscribers: Mutex<Vec<Channel<serde_json::Value>>> = Mutex::new(Vec::new());
        forward(&Event::PartClosed(part(0)), &snapshot, &subscribers, None);
        assert_eq!(lock(&snapshot).parts.len(), 1);
    }
}
