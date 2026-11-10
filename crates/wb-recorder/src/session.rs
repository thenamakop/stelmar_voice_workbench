//! Session folders and `session.json` (PLAN section 8):
//!
//! ```text
//! <recordings root>\<IST date>\<hall-slug>__<title-slug>\
//!    session.json  parts.json  audio\part-0001.wav  session.log  manifest.sha256 (at close)
//! ```

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::Error;
use crate::parts::AUDIO_DIR;
use crate::util::{
    files_under, ist_date, read_json, rel_slash, rfc3339, sha256_file, slug, write_atomic,
    write_json_atomic,
};

pub const SESSION_FILE: &str = "session.json";
pub const MANIFEST_FILE: &str = "manifest.sha256";
pub const LOG_FILE: &str = "session.log";
/// Held open without sharing while a recorder owns the session, so a second app instance (or
/// recovery) can tell a live session from a crashed one. Never mirrored or listed in the manifest.
pub const LOCK_FILE: &str = "session.lock";

/// What the operator chose when starting a session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSpec {
    pub title: String,
    #[serde(default)]
    pub hall: Option<String>,
    /// Programme entry this session records, if any (`docs/programme.json`).
    #[serde(default)]
    pub programme_id: Option<String>,
    /// Scheduled time as shown in the programme, for example `08 FEB 2027 · 12:00–13:30 IST`.
    #[serde(default)]
    pub scheduled: Option<String>,
    /// ISO 639-1 codes, English and Hindi by default.
    pub expected_languages: Vec<String>,
    /// `record_post` (default everywhere) or `live_post` (Phase 7).
    pub mode: String,
}

impl SessionSpec {
    /// An unscheduled English + Hindi session in Record + post-process mode.
    pub fn unscheduled(title: &str) -> Self {
        Self {
            title: title.to_string(),
            hall: None,
            programme_id: None,
            scheduled: None,
            expected_languages: vec!["en".into(), "hi".into()],
            mode: "record_post".into(),
        }
    }
}

/// The input a session recorded from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputInfo {
    pub device_name: String,
    pub device_id: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub sample_format: String,
}

/// Operator language lock or return to detection (Alt+1/2/3/4), at an audio time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanguageLogEntry {
    pub audio_ms: u64,
    pub at: String,
    /// `lock` or `detect`.
    pub action: String,
    #[serde(default)]
    pub lang: Option<String>,
}

/// Something that happened during the session, at a wall-clock time (IST) and an audio time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionEvent {
    pub at: String,
    pub audio_ms: u64,
    /// `opened`, `audio_started`, `paused`, `resumed`, `input_lost`, `input_restored`,
    /// `recovered`, `resumed_after_crash`, `write_error`, `closed`.
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// `session.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionMeta {
    pub version: u32,
    pub title: String,
    pub hall: Option<String>,
    pub programme_id: Option<String>,
    pub scheduled: Option<String>,
    pub expected_languages: Vec<String>,
    pub language_log: Vec<LanguageLogEntry>,
    pub mode: String,
    /// Models and devices used for transcripts (filled from Phase 4 on).
    pub models: Vec<String>,
    pub devices: Vec<String>,
    pub app_version: String,
    pub input: Option<InputInfo>,
    /// Backup root the mirror writes to, if one is set.
    pub backup_root: Option<String>,
    pub opened_at: String,
    pub closed_at: Option<String>,
    pub events: Vec<SessionEvent>,
}

impl SessionMeta {
    pub fn new(
        spec: &SessionSpec,
        app_version: &str,
        backup_root: Option<&Path>,
        now: OffsetDateTime,
    ) -> Self {
        let at = rfc3339(now);
        Self {
            version: 1,
            title: spec.title.clone(),
            hall: spec.hall.clone(),
            programme_id: spec.programme_id.clone(),
            scheduled: spec.scheduled.clone(),
            expected_languages: spec.expected_languages.clone(),
            language_log: Vec::new(),
            mode: spec.mode.clone(),
            models: Vec::new(),
            devices: Vec::new(),
            app_version: app_version.to_string(),
            input: None,
            backup_root: backup_root.map(|p| p.to_string_lossy().into_owned()),
            opened_at: at.clone(),
            closed_at: None,
            events: vec![SessionEvent {
                at,
                audio_ms: 0,
                kind: "opened".into(),
                detail: None,
            }],
        }
    }

    pub fn push_event(
        &mut self,
        now: OffsetDateTime,
        audio_ms: u64,
        kind: &str,
        detail: Option<String>,
    ) {
        self.events.push(SessionEvent {
            at: rfc3339(now),
            audio_ms,
            kind: kind.to_string(),
            detail,
        });
    }
}

pub fn load_meta(session_dir: &Path) -> Result<SessionMeta, Error> {
    let path = session_dir.join(SESSION_FILE);
    read_json(&path).map_err(|e| Error::io(&path, e))
}

pub fn save_meta(session_dir: &Path, meta: &SessionMeta) -> Result<(), Error> {
    let path = session_dir.join(SESSION_FILE);
    write_json_atomic(&path, meta).map_err(|e| Error::io(&path, e))
}

/// Folder name for a session: `<hall-slug>__<title-slug>`, or the title slug alone.
pub fn folder_name(spec: &SessionSpec) -> String {
    let title = slug(&spec.title, 60);
    let title = if title.is_empty() {
        "session".to_string()
    } else {
        title
    };
    match spec
        .hall
        .as_deref()
        .map(|h| slug(h, 30))
        .filter(|h| !h.is_empty())
    {
        Some(hall) => format!("{hall}__{title}"),
        None => title,
    }
}

/// Creates `<root>\<IST date>\<folder name>` (with `-2`, `-3`, … if taken) and its `audio\`.
pub fn create_session_dir(
    root: &Path,
    spec: &SessionSpec,
    now: OffsetDateTime,
) -> Result<PathBuf, Error> {
    let day = root.join(ist_date(now));
    std::fs::create_dir_all(&day).map_err(|e| Error::io(&day, e))?;
    let base = folder_name(spec);
    for n in 1..1000u32 {
        let name = if n == 1 {
            base.clone()
        } else {
            format!("{base}-{n}")
        };
        let dir = day.join(&name);
        match std::fs::create_dir(&dir) {
            Ok(()) => {
                let audio = dir.join(AUDIO_DIR);
                std::fs::create_dir(&audio).map_err(|e| Error::io(&audio, e))?;
                return Ok(dir);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(Error::io(&dir, e)),
        }
    }
    Err(Error::Invalid(format!(
        "too many sessions named {base} on this day"
    )))
}

/// The exclusive lock a live recorder holds on its session.
pub struct SessionLock {
    _file: File,
    path: PathBuf,
}

impl SessionLock {
    /// Takes the lock, failing if another process holds it.
    pub fn acquire(session_dir: &Path) -> Result<Self, Error> {
        let path = session_dir.join(LOCK_FILE);
        let file = open_exclusive(&path).map_err(|e| {
            if is_sharing_violation(&e) {
                Error::SessionBusy(session_dir.to_path_buf())
            } else {
                Error::io(&path, e)
            }
        })?;
        Ok(Self { _file: file, path })
    }

    /// Releases the lock and removes the file (clean close).
    pub fn release(self) {
        let path = self.path.clone();
        drop(self);
        let _ = std::fs::remove_file(path);
    }
}

fn open_exclusive(path: &Path) -> std::io::Result<File> {
    let mut opts = OpenOptions::new();
    opts.read(true).write(true).create(true).truncate(false);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        opts.share_mode(0);
    }
    opts.open(path)
}

fn is_sharing_violation(e: &std::io::Error) -> bool {
    // ERROR_SHARING_VIOLATION
    e.raw_os_error() == Some(32)
}

/// True when another live process holds the session's lock.
pub fn is_live(session_dir: &Path) -> bool {
    let path = session_dir.join(LOCK_FILE);
    if !path.exists() {
        return false;
    }
    match open_exclusive(&path) {
        Ok(_) => false,
        Err(e) => is_sharing_violation(&e),
    }
}

/// Files that belong in the manifest and the mirror (everything but the lock and temp files).
pub fn session_files(session_dir: &Path) -> Result<Vec<PathBuf>, Error> {
    let files = files_under(session_dir).map_err(|e| Error::io(session_dir, e))?;
    Ok(files
        .into_iter()
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            name != LOCK_FILE && name != MANIFEST_FILE && !name.ends_with(".tmp")
        })
        .collect())
}

/// Writes `manifest.sha256` (`<sha256>  <relative path>` per line, sha256sum format) for every
/// session file. Returns the number of files.
pub fn write_manifest(session_dir: &Path) -> Result<usize, Error> {
    let mut lines = String::new();
    let files = session_files(session_dir)?;
    for f in &files {
        let hash = sha256_file(f).map_err(|e| Error::io(f, e))?;
        let rel = rel_slash(session_dir, f).unwrap_or_default();
        lines.push_str(&format!("{hash}  {rel}\n"));
    }
    let path = session_dir.join(MANIFEST_FILE);
    write_atomic(&path, lines.as_bytes()).map_err(|e| Error::io(&path, e))?;
    Ok(files.len())
}

/// Appends a line to `session.log`. Best effort: a failure here never stops the recording.
pub fn append_log(session_dir: &Path, now: OffsetDateTime, line: &str) {
    let path = session_dir.join(LOG_FILE);
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{} {line}", rfc3339(now));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;
    use crate::util::now_ist;

    #[test]
    fn folder_names() {
        let mut spec = SessionSpec::unscheduled("Inaugural Session: Inaugural Keynote");
        spec.hall = Some("Hall A".into());
        assert_eq!(
            folder_name(&spec),
            "hall-a__inaugural-session-inaugural-keynote"
        );
        spec.hall = None;
        assert_eq!(folder_name(&spec), "inaugural-session-inaugural-keynote");
        let hindi = SessionSpec::unscheduled("उद्घाटन सत्र");
        assert_eq!(folder_name(&hindi), "session");
    }

    #[test]
    fn session_dirs_do_not_collide() -> Result<(), Box<dyn std::error::Error>> {
        let root = TempDir::new("session-dirs")?;
        let spec = SessionSpec::unscheduled("Talk");
        let now = now_ist();
        let a = create_session_dir(root.path(), &spec, now)?;
        let b = create_session_dir(root.path(), &spec, now)?;
        assert_ne!(a, b);
        assert!(
            b.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with("talk-2"))
        );
        assert!(a.join(AUDIO_DIR).is_dir());
        assert_eq!(
            a.parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str()),
            Some(ist_date(now).as_str())
        );
        Ok(())
    }

    #[test]
    fn meta_round_trip() -> Result<(), Box<dyn std::error::Error>> {
        let dir = TempDir::new("session-meta")?;
        let spec = SessionSpec::unscheduled("Talk");
        let mut meta = SessionMeta::new(&spec, "0.1.0", None, now_ist());
        meta.push_event(now_ist(), 1234, "paused", None);
        save_meta(dir.path(), &meta)?;
        assert_eq!(load_meta(dir.path())?, meta);
        assert_eq!(meta.expected_languages, vec!["en", "hi"]);
        assert_eq!(meta.mode, "record_post");
        assert!(meta.closed_at.is_none());
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn lock_marks_a_session_live() -> Result<(), Box<dyn std::error::Error>> {
        let dir = TempDir::new("session-lock")?;
        assert!(!is_live(dir.path()));
        let lock = SessionLock::acquire(dir.path())?;
        assert!(is_live(dir.path()));
        assert!(matches!(
            SessionLock::acquire(dir.path()),
            Err(Error::SessionBusy(_))
        ));
        lock.release();
        assert!(!is_live(dir.path()));
        assert!(!dir.path().join(LOCK_FILE).exists());
        Ok(())
    }

    #[test]
    fn manifest_lists_every_file_but_the_lock() -> Result<(), Box<dyn std::error::Error>> {
        let dir = TempDir::new("session-manifest")?;
        std::fs::create_dir_all(dir.path().join("audio"))?;
        std::fs::write(dir.path().join("audio/part-0001.wav"), b"abc")?;
        std::fs::write(dir.path().join(SESSION_FILE), b"{}")?;
        std::fs::write(dir.path().join(LOCK_FILE), b"")?;
        assert_eq!(write_manifest(dir.path())?, 2);
        let text = std::fs::read_to_string(dir.path().join(MANIFEST_FILE))?;
        assert!(text.contains(
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  audio/part-0001.wav"
        ));
        assert!(!text.contains(LOCK_FILE));
        Ok(())
    }
}
