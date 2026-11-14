//! Import (Phase 4 step 10): register a session folder copied from another machine (a record-only machine or
//! another room) and queue it here. The folder is checked before anything is copied (`session.json`,
//! `parts.json`, every part's SHA-256), copied beside the other sessions, checked again at its new place and
//! only then moved into position, so a half-copied folder never looks like a session.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::{Error, Result, io};

#[derive(Debug, Deserialize)]
struct PartsFile {
    parts: Vec<PartEntry>,
}

#[derive(Debug, Deserialize)]
struct PartEntry {
    file: String,
    start_ms: u64,
    duration_ms: u64,
    sha256: String,
    #[serde(default)]
    needs_repair: bool,
}

/// What a valid session folder holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Validated {
    pub parts: usize,
    pub audio_ms: u64,
}

/// Files copied with the session. A lock file, a database and engine work folders are never copied: the
/// importing machine builds its own.
const COPIED: &[&str] = &[
    "session.json",
    "parts.json",
    "start-options.json",
    "language-log.jsonl",
    "session.log",
    "manifest.sha256",
];

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut f = fs::File::open(path).map_err(io(path))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).map_err(io(path))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn problems_error(problems: Vec<String>) -> Error {
    Error::Invalid(problems.join("; "))
}

/// Checks a session folder. Every problem found is reported together.
pub fn validate(dir: &Path) -> Result<Validated> {
    let mut problems = Vec::new();
    let session = dir.join("session.json");
    match fs::read(&session) {
        Ok(b) => match serde_json::from_slice::<serde_json::Value>(&b) {
            Ok(v) if v.is_object() => {
                if v.get("closed_at").is_none_or(serde_json::Value::is_null) {
                    problems.push(
                        "the session was never closed (open it on its own computer and stop it, or repair it, first)"
                            .to_owned(),
                    );
                }
            }
            Ok(_) => problems.push("session.json is not a JSON object".to_owned()),
            Err(e) => problems.push(format!("session.json does not parse ({e})")),
        },
        Err(e) => problems.push(format!("session.json cannot be read ({e})")),
    }
    let parts_path = dir.join("parts.json");
    let parts: Option<PartsFile> = match fs::read(&parts_path) {
        Ok(b) => match serde_json::from_slice(&b) {
            Ok(p) => Some(p),
            Err(e) => {
                problems.push(format!("parts.json does not parse ({e})"));
                None
            }
        },
        Err(e) => {
            problems.push(format!("parts.json cannot be read ({e})"));
            None
        }
    };
    let mut audio_ms = 0;
    let mut count = 0;
    if let Some(p) = parts {
        if p.parts.is_empty() {
            problems.push("the session has no audio parts".to_owned());
        }
        let mut expected_start = 0u64;
        for part in &p.parts {
            count += 1;
            // Parts are contiguous on the audio clock (a millisecond of rounding is allowed).
            if part.start_ms.abs_diff(expected_start) > 2 {
                problems.push(format!(
                    "{} starts at {} ms but the parts before it end at {} ms",
                    part.file, part.start_ms, expected_start
                ));
            }
            expected_start = part.start_ms + part.duration_ms;
            audio_ms = expected_start;
            if part.needs_repair {
                problems.push(format!("{} is marked as needing repair", part.file));
            }
            let rel = Path::new(&part.file);
            if rel.is_absolute()
                || rel
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                problems.push(format!("{} is not a path inside the session", part.file));
                continue;
            }
            let file = dir.join(rel);
            match sha256_file(&file) {
                Ok(h) if h.eq_ignore_ascii_case(&part.sha256) => {}
                Ok(_) => problems.push(format!("{} does not match its SHA-256", part.file)),
                Err(_) => problems.push(format!("{} is missing", part.file)),
            }
        }
    }
    if problems.is_empty() {
        Ok(Validated {
            parts: count,
            audio_ms,
        })
    } else {
        Err(problems_error(problems))
    }
}

fn copy_dir_files(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst).map_err(io(dst))?;
    for entry in fs::read_dir(src).map_err(io(src))? {
        let entry = entry.map_err(io(src))?;
        if entry.file_type().map_err(io(src))?.is_file() {
            let to = dst.join(entry.file_name());
            fs::copy(entry.path(), &to).map_err(io(&to))?;
        }
    }
    Ok(())
}

/// Copies a valid session folder to `dest` (which must not exist), checks the copy, and moves it into place.
/// Returns what the copy holds. Nothing at `dest` changes if anything fails.
pub fn import_session(src: &Path, dest: &Path) -> Result<Validated> {
    validate(src)?;
    if dest.exists() {
        return Err(Error::Invalid(format!(
            "{} already exists; a session is never overwritten",
            dest.display()
        )));
    }
    let parent = dest
        .parent()
        .ok_or_else(|| Error::Invalid("the destination has no parent folder".into()))?;
    fs::create_dir_all(parent).map_err(io(parent))?;
    let name = dest
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| Error::Invalid("the destination has no name".into()))?;
    let staging: PathBuf = parent.join(format!("{name}.importing"));
    let _ = fs::remove_dir_all(&staging);
    let result = (|| {
        fs::create_dir_all(&staging).map_err(io(&staging))?;
        for f in COPIED {
            let from = src.join(f);
            if from.is_file() {
                let to = staging.join(f);
                fs::copy(&from, &to).map_err(io(&to))?;
            }
        }
        copy_dir_files(&src.join("audio"), &staging.join("audio"))?;
        let v = validate(&staging)?;
        fs::rename(&staging, dest).map_err(io(dest))?;
        Ok(v)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

/// An import is refused on a computer that only records (tier R): it could not process the session either.
pub fn refuse_on_record_only(record_only: bool) -> Result<()> {
    if record_only {
        Err(Error::Invalid(
            "this computer only records; import the session on a stronger machine".into(),
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::temp;

    /// A small valid session folder: two parts of 10 minutes (the audio bytes are stand-ins).
    fn make_session(dir: &Path, closed: bool) {
        fs::create_dir_all(dir.join("audio")).unwrap();
        let mut parts = Vec::new();
        for (i, body) in [b"first part".as_slice(), b"second part".as_slice()]
            .iter()
            .enumerate()
        {
            let rel = format!("audio/part-{:04}.wav", i + 1);
            fs::write(dir.join(&rel), body).unwrap();
            parts.push(serde_json::json!({
                "part": i + 1, "file": rel, "start_ms": i as u64 * 600_000, "duration_ms": 600_000,
                "frames": 1, "sample_rate": 48_000, "channels": 1,
                "sha256": sha256_file(&dir.join(&rel)).unwrap(),
            }));
        }
        fs::write(
            dir.join("parts.json"),
            serde_json::json!({"version": 1, "parts": parts}).to_string(),
        )
        .unwrap();
        let closed_at = if closed {
            serde_json::json!("2027-02-08T13:30:00+05:30")
        } else {
            serde_json::Value::Null
        };
        fs::write(
            dir.join("session.json"),
            serde_json::json!({"title": "t", "closed_at": closed_at}).to_string(),
        )
        .unwrap();
        fs::write(dir.join("session.lock"), "pid").unwrap();
    }

    #[test]
    fn a_closed_session_with_matching_hashes_is_valid() {
        let d = temp("imp-ok");
        make_session(&d, true);
        assert_eq!(
            validate(&d).unwrap(),
            Validated {
                parts: 2,
                audio_ms: 1_200_000
            }
        );
    }

    #[test]
    fn an_unclosed_session_a_changed_part_and_a_gap_are_each_reported() {
        let d = temp("imp-bad");
        make_session(&d, false);
        fs::write(d.join("audio/part-0002.wav"), b"tampered").unwrap();
        let e = validate(&d).unwrap_err().to_string();
        assert!(e.contains("never closed"), "{e}");
        assert!(
            e.contains("part-0002.wav does not match its SHA-256"),
            "{e}"
        );

        let d = temp("imp-gap");
        make_session(&d, true);
        let mut p: serde_json::Value =
            serde_json::from_slice(&fs::read(d.join("parts.json")).unwrap()).unwrap();
        p["parts"][1]["start_ms"] = serde_json::json!(700_000);
        fs::write(d.join("parts.json"), p.to_string()).unwrap();
        assert!(
            validate(&d)
                .unwrap_err()
                .to_string()
                .contains("starts at 700000 ms")
        );
    }

    #[test]
    fn a_part_path_that_leaves_the_session_is_refused() {
        let d = temp("imp-path");
        make_session(&d, true);
        let mut p: serde_json::Value =
            serde_json::from_slice(&fs::read(d.join("parts.json")).unwrap()).unwrap();
        p["parts"][0]["file"] = serde_json::json!("../outside.wav");
        fs::write(d.join("parts.json"), p.to_string()).unwrap();
        assert!(
            validate(&d)
                .unwrap_err()
                .to_string()
                .contains("not a path inside the session")
        );
    }

    #[test]
    fn an_import_copies_the_session_without_the_lock_and_checks_the_copy() {
        let src = temp("imp-src");
        make_session(&src, true);
        let root = temp("imp-root");
        let dest = root.join("2027-02-08").join("hall-b__session");
        let v = import_session(&src, &dest).unwrap();
        assert_eq!(v.parts, 2);
        assert!(dest.join("audio/part-0001.wav").is_file());
        assert!(!dest.join("session.lock").exists());
        assert!(
            !root
                .join("2027-02-08")
                .join("hall-b__session.importing")
                .exists()
        );
        // The source is untouched, and an existing session is never overwritten.
        assert!(src.join("session.lock").exists());
        assert!(
            import_session(&src, &dest)
                .unwrap_err()
                .to_string()
                .contains("already exists")
        );
    }

    #[test]
    fn a_failed_import_leaves_nothing_behind() {
        let src = temp("imp-fail");
        make_session(&src, false);
        let root = temp("imp-fail-root");
        let dest = root.join("s");
        assert!(import_session(&src, &dest).is_err());
        assert!(!dest.exists());
        assert!(!root.join("s.importing").exists());
    }

    #[test]
    fn a_record_only_computer_refuses_an_import() {
        assert!(refuse_on_record_only(true).is_err());
        assert!(refuse_on_record_only(false).is_ok());
    }
}
