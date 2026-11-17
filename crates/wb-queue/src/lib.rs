//! The logic of the post-processing queue (PLAN section 4, Phase 4), with no Tauri and no engine in it, so
//! every rule can be tested alone:
//!
//! * [`store`]: jobs, work units, segments, revisions and gaps in the session's `transcript.db` (SQLite, WAL);
//! * [`revision`]: the single writer of `post-1.jsonl` and `post-2.jsonl`; the JSONL is the truth and SQLite
//!   follows it;
//! * [`order`]: units finish in any order on two engines and commit in audio order;
//! * [`sched`]: priorities, pause rules and the memory rules, as functions of a resource snapshot;
//! * [`estimate`]: "all transcripts ready by 19:40";
//! * [`speed`]: the speed this computer reaches, from the units it has finished, against the profile's;
//! * [`night`]: the night pass plan (decision D6);
//! * [`import`]: registering a session folder copied from another machine.
//!
//! * [`units`]: cutting a job into work units of about 5 minutes at VAD pauses given as plain data.

pub mod estimate;
pub mod import;
pub mod night;
pub mod order;
pub mod revision;
pub mod sched;
pub mod speed;
pub mod store;
pub mod units;

/// Everything that can go wrong in this crate. The app shows it in a Toast and the queue goes on.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("database: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("file {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn io(path: &std::path::Path) -> impl FnOnce(std::io::Error) -> Error + '_ {
    move |source| Error::Io {
        path: path.display().to_string(),
        source,
    }
}

/// Milliseconds in a day, for IST clock arithmetic.
pub const DAY_MS: u64 = 86_400_000;
/// IST is UTC+05:30 all year (no daylight saving).
pub const IST_OFFSET_MS: u64 = 19_800_000;

#[cfg(test)]
pub(crate) mod testutil {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    use crate::store::{Device, Segment};

    pub fn seg(rev: &str, start: u64, end: u64, text: &str) -> Segment {
        Segment {
            revision: rev.into(),
            start_ms: start,
            end_ms: end,
            text: text.into(),
            lang: "hi".into(),
            lang_prob: 0.9,
            mixed: false,
            dir: "ltr".into(),
            avg_logprob: -0.3,
            no_speech_prob: 0.01,
            model: "m1".into(),
            device: Device::Cpu,
            flags: vec![],
            hint_text: None,
            created_at: String::new(),
        }
    }

    /// A fresh empty folder under the system temp dir, removed first if it exists.
    pub fn temp(name: &str) -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let d = std::env::temp_dir().join(format!(
            "wb-queue-{name}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }
}
