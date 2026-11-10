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

/// Forwards recorder, mirror and capture events to the UI and the app log, keeping the snapshot
/// current. Runs on its own thread for the life of the app.
pub fn spawn_forwarder(
    rx: Receiver<Event>,
    snapshot: Arc<Mutex<Snapshot>>,
    subscribers: Arc<Mutex<Vec<Channel<serde_json::Value>>>>,
) {
    let spawned = std::thread::Builder::new()
        .name("wb-events".into())
        .spawn(move || {
            for event in rx {
                match &event {
                    Event::RecorderStatus(s) => lock(&snapshot).recorder = Some(s.as_ref().clone()),
                    Event::MirrorStatus(s) => lock(&snapshot).mirror = Some(s.clone()),
                    Event::PartClosed(p) => {
                        tracing::info!(
                            part = p.part,
                            start_ms = p.start_ms,
                            duration_ms = p.duration_ms,
                            "part closed"
                        );
                        lock(&snapshot).parts.push(p.clone());
                    }
                    Event::Notice(n) => match n.level {
                        NoticeLevel::Info => tracing::info!(source = n.source, "{}", n.message),
                        NoticeLevel::Warning => tracing::warn!(source = n.source, "{}", n.message),
                        NoticeLevel::Error => tracing::error!(source = n.source, "{}", n.message),
                    },
                    Event::Level(_) => {}
                }
                if let Ok(value) = serde_json::to_value(&event) {
                    broadcast(&subscribers, &value);
                }
            }
        });
    if let Err(e) = spawned {
        tracing::error!("could not start the event forwarder: {e}");
    }
}
