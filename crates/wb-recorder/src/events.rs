//! Events the recorder, capture and mirror threads send to the app, which forwards them to the UI
//! over a Tauri channel. The threads only ever `send` on an unbounded channel, which never blocks.

use serde::Serialize;

use crate::level::LevelReading;
use crate::parts::PartRecord;
use crate::session::InputInfo;
use crate::sys::PriorityMode;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    /// About 20 a second while a stream is open.
    Level(LevelReading),
    /// About 4 a second.
    RecorderStatus(Box<RecorderStatus>),
    /// About once a second.
    MirrorStatus(MirrorStatus),
    PartClosed(PartRecord),
    Notice(Notice),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecState {
    Recording,
    Paused,
    /// The input device disappeared; retrying every 2 seconds.
    InputLost,
    Stopped,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecorderStatus {
    pub state: RecState,
    pub session_dir: String,
    /// Audio-clock position: everything recorded so far, in milliseconds.
    pub audio_ms: u64,
    /// Part being written, if any.
    pub part: Option<u32>,
    pub part_audio_ms: u64,
    pub part_seconds: u32,
    /// Samples the audio callback could not fit in the ring buffer (zero is the gate).
    pub overrun_samples: u64,
    /// Capture glitches the audio driver reported.
    pub xruns: u64,
    /// Samples taken from the ring that could not be written (disk full, drive gone).
    pub dropped_samples: u64,
    /// Panics caught inside the recorder thread (each continued in a new part).
    pub recorder_restarts: u32,
    /// Capture callbacks whose device time did not follow on from the previous one.
    pub position_breaks: u64,
    pub ring_seconds: u32,
    /// Highest ring-buffer fill seen so far, in percent.
    pub ring_max_fill_pct: f32,
    pub priority: PriorityMode,
    pub priority_note: Option<String>,
    /// Priority of the audio driver's capture thread (`mmcss`, `highest`, `normal`, `unknown`).
    pub capture_priority: Option<String>,
    /// Device buffer the capture stream opened with: "fixed 100 ms" or "device default".
    pub capture_buffer: Option<String>,
    /// The machine is being kept awake.
    pub awake: bool,
    pub disk_free_bytes: Option<u64>,
    /// Under 20 GB free on the recording drive.
    pub disk_warning: bool,
    pub input: Option<InputInfo>,
    /// When the input was lost (IST), while it is lost.
    pub input_lost_since: Option<String>,
    /// CPU time the recorder thread has used.
    pub thread_cpu_ms: Option<u64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MirrorState {
    /// No backup folder is set.
    NotSet,
    InSync,
    /// More than a pass behind but under the 30-second warning.
    Behind,
    /// The backup drive or folder cannot be reached.
    Missing,
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct MirrorStatus {
    pub state: MirrorState,
    pub backup_dir: Option<String>,
    /// Audio not yet on the backup, in milliseconds.
    pub lag_ms: u64,
    /// Missing, or more than 30 seconds behind.
    pub warn: bool,
    pub last_sync_at: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeLevel {
    Info,
    Warning,
    Error,
}

/// A message for the app log and, for warnings and errors, a Toast.
#[derive(Debug, Clone, Serialize)]
pub struct Notice {
    pub level: NoticeLevel,
    pub source: &'static str,
    pub message: String,
}

impl Notice {
    pub fn event(level: NoticeLevel, source: &'static str, message: impl Into<String>) -> Event {
        Event::Notice(Self {
            level,
            source,
            message: message.into(),
        })
    }
}
