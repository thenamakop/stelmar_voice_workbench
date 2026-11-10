//! Crash-safe recording for the STELMAR Voice Workbench (PLAN section 8).
//!
//! Capture (cpal) → lock-free ring buffer → recorder thread (MMCSS "Pro Audio") → 16-bit WAV
//! parts of 10 minutes, flushed every second and fsynced every 5 → `parts.json` → mirror to the
//! backup drive. Crash recovery repairs headers from file lengths and rebuilds the index.
//!
//! The crate has no Tauri dependency so its logic is unit-tested on its own; the app wraps it in
//! commands and channel events.

pub mod capture;
pub mod controller;
pub mod events;
pub mod level;
pub mod mirror;
pub mod parts;
pub mod recorder;
pub mod recovery;
pub mod session;
pub mod sys;
pub mod util;
pub mod wav;
#[cfg(windows)]
mod win;

#[doc(hidden)]
pub mod testutil;

use std::path::{Path, PathBuf};

pub use controller::{ActiveSession, SessionStopped, StartOptions};
pub use events::Event;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{}: {source}", path.display())]
    Wav { path: PathBuf, source: hound::Error },
    #[error("{} is not a usable WAV file: {reason}", path.display())]
    BadWav { path: PathBuf, reason: String },
    #[error("Audio input: {0}")]
    Device(String),
    #[error("Only {free_gb:.1} GB free on {}; at least 5 GB is needed to start a session", path.display())]
    DiskFull { path: PathBuf, free_gb: f64 },
    #[error("Another copy of the app is recording {}", .0.display())]
    SessionBusy(PathBuf),
    #[error("{0}")]
    Invalid(String),
    #[error("The recorder has stopped")]
    RecorderGone,
}

impl Error {
    pub fn io(path: &Path, source: std::io::Error) -> Self {
        Self::Io {
            path: path.to_path_buf(),
            source,
        }
    }

    pub fn wav(path: &Path, source: hound::Error) -> Self {
        Self::Wav {
            path: path.to_path_buf(),
            source,
        }
    }
}
