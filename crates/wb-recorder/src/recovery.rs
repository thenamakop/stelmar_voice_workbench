//! Crash recovery at start-up (PLAN section 8): sessions without `closed_at` get their WAV
//! headers repaired from the file lengths and `parts.json` rebuilt from the files. The operator
//! then chooses Resume (a new part continues the audio clock) or Close.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::Error;
use crate::parts::{self, AUDIO_DIR, PartsIndex};
use crate::session::{self, SESSION_FILE, SessionMeta};
use crate::util::{now_ist, rfc3339};
use crate::wav::{self, RepairReport};

/// A session left open by a crash, after repair.
#[derive(Debug, Clone, Serialize)]
pub struct RecoveredSession {
    pub dir: String,
    pub title: String,
    pub hall: Option<String>,
    pub opened_at: String,
    pub audio_ms: u64,
    pub parts: usize,
    pub repairs: Vec<RepairReport>,
    /// Part files with no audio in them (a crash right after opening), removed.
    pub removed_empty: Vec<String>,
    /// Part files that could not be read even after repair, renamed to `*.wav.bad` and left out
    /// of the index (kept for manual recovery).
    pub quarantined: Vec<String>,
}

fn is_date_folder(name: &str) -> bool {
    let b = name.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

/// Session folders under `root` (`<root>\<YYYY-MM-DD>\<session>`) whose `session.json` has no
/// `closed_at` and that no live process holds.
pub fn scan(root: &Path) -> Result<Vec<PathBuf>, Error> {
    let mut found = Vec::new();
    if !root.is_dir() {
        return Ok(found);
    }
    for day in std::fs::read_dir(root)
        .map_err(|e| Error::io(root, e))?
        .flatten()
    {
        let name = day.file_name();
        if !day.path().is_dir() || !name.to_str().is_some_and(is_date_folder) {
            continue;
        }
        let Ok(sessions) = std::fs::read_dir(day.path()) else {
            continue;
        };
        for s in sessions.flatten() {
            let dir = s.path();
            if !dir.join(SESSION_FILE).is_file() {
                continue;
            }
            match session::load_meta(&dir) {
                Ok(meta) if meta.closed_at.is_none() && !session::is_live(&dir) => found.push(dir),
                _ => {}
            }
        }
    }
    found.sort();
    Ok(found)
}

/// Repairs every part's header, removes parts with no audio, rebuilds `parts.json` and notes the
/// recovery in `session.json`. Safe to run twice.
pub fn repair(dir: &Path) -> Result<RecoveredSession, Error> {
    if session::is_live(dir) {
        return Err(Error::SessionBusy(dir.to_path_buf()));
    }
    let mut meta = session::load_meta(dir)?;
    let mut repairs = Vec::new();
    let mut removed_empty = Vec::new();
    let mut quarantined = Vec::new();
    for n in parts::part_numbers_on_disk(dir)? {
        let path = dir.join(AUDIO_DIR).join(parts::part_file_name(n));
        match wav::repair_header(&path) {
            Ok(r) if r.frames == 0 => {
                std::fs::remove_file(&path).map_err(|e| Error::io(&path, e))?;
                removed_empty.push(r.file);
            }
            Ok(r) => repairs.push(r),
            Err(Error::BadWav { .. }) => {
                // Shorter than a header: the crash came before any audio reached the file.
                let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                if len <= 4096 {
                    std::fs::remove_file(&path).map_err(|e| Error::io(&path, e))?;
                    removed_empty.push(parts::part_file_name(n));
                } else {
                    // Keep the bytes for manual recovery, but do not let one damaged part block
                    // the whole session.
                    let mut bad = path.as_os_str().to_os_string();
                    bad.push(".bad");
                    // Its length is unknown, so later parts' start_ms move back by it; record an
                    // estimate from the file size and the session's input format.
                    let est_ms = meta.input.as_ref().map_or(0, |i| {
                        let bytes_per_ms =
                            u64::from(i.sample_rate) * u64::from(i.channels) * 2 / 1000;
                        len.saturating_sub(44) / bytes_per_ms.max(1)
                    });
                    std::fs::rename(&path, &bad).map_err(|e| Error::io(&path, e))?;
                    quarantined.push(format!(
                        "{} (about {est_ms} ms, not on the clock)",
                        parts::part_file_name(n)
                    ));
                }
            }
            Err(e) => return Err(e),
        }
    }
    let index = parts::rebuild_from_folder(dir)?;
    parts::save(dir, &index)?;
    let fixed = repairs.iter().filter(|r| !r.already_valid).count();
    let detail = format!(
        "parts={} headers_repaired={fixed} empty_removed={} quarantined={}",
        index.parts.len(),
        removed_empty.len(),
        quarantined.join(",")
    );
    let now = now_ist();
    meta.push_event(now, index.end_ms(), "recovered", Some(detail.clone()));
    session::save_meta(dir, &meta)?;
    session::append_log(dir, now, &format!("recovered {detail}"));
    Ok(summary(
        dir,
        &meta,
        &index,
        repairs,
        removed_empty,
        quarantined,
    ))
}

fn summary(
    dir: &Path,
    meta: &SessionMeta,
    index: &PartsIndex,
    repairs: Vec<RepairReport>,
    removed_empty: Vec<String>,
    quarantined: Vec<String>,
) -> RecoveredSession {
    RecoveredSession {
        dir: dir.to_string_lossy().into_owned(),
        title: meta.title.clone(),
        hall: meta.hall.clone(),
        opened_at: meta.opened_at.clone(),
        audio_ms: index.end_ms(),
        parts: index.parts.len(),
        repairs,
        removed_empty,
        quarantined,
    }
}

/// Closes a recovered session without resuming: `closed_at` and the manifest.
pub fn close(dir: &Path) -> Result<(), Error> {
    if session::is_live(dir) {
        return Err(Error::SessionBusy(dir.to_path_buf()));
    }
    let mut meta = session::load_meta(dir)?;
    if meta.closed_at.is_some() {
        return Ok(());
    }
    let index = parts::load(dir)?;
    let now = now_ist();
    meta.closed_at = Some(rfc3339(now));
    meta.push_event(
        now,
        index.end_ms(),
        "closed",
        Some("closed after recovery".into()),
    );
    session::save_meta(dir, &meta)?;
    session::append_log(dir, now, "closed after recovery");
    session::write_manifest(dir)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::SessionSpec;
    use crate::testutil::TempDir;
    use crate::wav::{PartWriter, append_bytes};

    fn crashed_session(root: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
        let spec = SessionSpec::unscheduled("Crash Test");
        let now = now_ist();
        let dir = session::create_session_dir(root, &spec, now)?;
        session::save_meta(&dir, &SessionMeta::new(&spec, "0.1.0", None, now))?;
        // Part 1 closed normally.
        let mut w = PartWriter::create(&dir.join("audio/part-0001.wav"), 48_000, 1)?;
        w.write_f32(&vec![0.1; 48_000 * 2])?;
        w.finish()?;
        let mut index = PartsIndex::new();
        index.parts.push(parts::record_for_file(&dir, 1, 0)?);
        parts::save(&dir, &index)?;
        // Part 2 open at the crash: header says 1 s, the file holds 1.5 s and a stray byte.
        let mut w = PartWriter::create(&dir.join("audio/part-0002.wav"), 48_000, 1)?;
        w.write_f32(&vec![0.2; 48_000])?;
        w.flush()?;
        std::mem::forget(w); // no finalize, as after a kill
        append_bytes(&dir.join("audio/part-0002.wav"), &vec![0u8; 48_000 + 1])?;
        // Part 3 created a moment before the crash: header only.
        let mut w = PartWriter::create(&dir.join("audio/part-0003.wav"), 48_000, 1)?;
        w.flush()?;
        std::mem::forget(w);
        Ok(dir)
    }

    #[test]
    fn crashed_session_is_found_repaired_and_closed() -> Result<(), Box<dyn std::error::Error>> {
        let root = TempDir::new("recovery")?;
        let dir = crashed_session(root.path())?;
        // A closed session next to it is ignored.
        let closed =
            session::create_session_dir(root.path(), &SessionSpec::unscheduled("Done"), now_ist())?;
        let mut m = SessionMeta::new(&SessionSpec::unscheduled("Done"), "0.1.0", None, now_ist());
        m.closed_at = Some(rfc3339(now_ist()));
        session::save_meta(&closed, &m)?;

        assert_eq!(scan(root.path())?, vec![dir.clone()]);
        let rec = repair(&dir)?;
        assert_eq!(rec.parts, 2);
        assert_eq!(rec.audio_ms, 3_500);
        assert_eq!(rec.removed_empty, vec!["part-0003.wav".to_string()]);
        assert!(
            rec.repairs
                .iter()
                .any(|r| r.file == "part-0002.wav" && !r.already_valid && r.truncated_bytes == 1)
        );
        let index = parts::load(&dir)?;
        assert_eq!(index, parts::rebuild_from_folder(&dir)?);
        assert_eq!(index.parts[1].start_ms, 2_000);
        assert_eq!(index.parts[1].duration_ms, 1_500);
        assert!(wav::inspect(&dir.join("audio/part-0002.wav"))?.header_matches_length());
        assert!(
            session::load_meta(&dir)?
                .events
                .iter()
                .any(|e| e.kind == "recovered")
        );

        // Repair is idempotent.
        assert_eq!(repair(&dir)?.parts, 2);

        close(&dir)?;
        assert!(session::load_meta(&dir)?.closed_at.is_some());
        assert!(dir.join(session::MANIFEST_FILE).is_file());
        assert!(scan(root.path())?.is_empty());
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn a_live_session_is_not_touched() -> Result<(), Box<dyn std::error::Error>> {
        let root = TempDir::new("recovery-live")?;
        let dir = crashed_session(root.path())?;
        let lock = session::SessionLock::acquire(&dir)?;
        assert!(scan(root.path())?.is_empty());
        assert!(matches!(repair(&dir), Err(Error::SessionBusy(_))));
        lock.release();
        assert_eq!(scan(root.path())?.len(), 1);
        Ok(())
    }

    #[test]
    fn a_damaged_part_is_quarantined_and_the_rest_recovered()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = TempDir::new("recovery-bad")?;
        let dir = crashed_session(root.path())?;
        // Part 4: 8 KB of bytes that are not a WAV file.
        std::fs::write(dir.join("audio/part-0004.wav"), vec![0x5au8; 8192])?;
        let rec = repair(&dir)?;
        assert_eq!(rec.quarantined.len(), 1);
        assert!(
            rec.quarantined[0].starts_with("part-0004.wav (about "),
            "{:?}",
            rec.quarantined
        );
        assert_eq!(rec.parts, 2);
        assert!(dir.join("audio/part-0004.wav.bad").is_file());
        assert!(!dir.join("audio/part-0004.wav").exists());
        assert_eq!(parts::load(&dir)?, parts::rebuild_from_folder(&dir)?);
        Ok(())
    }

    #[test]
    fn date_folders() {
        assert!(is_date_folder("2027-02-08"));
        assert!(!is_date_folder("2027-2-08"));
        assert!(!is_date_folder("notes"));
    }
}
