//! `capability.json` on disk: written whole to a temporary file, flushed, then renamed over the old one,
//! so a power cut leaves either the old profile or the new one, never half of one.

use std::io::Write;
use std::path::{Path, PathBuf};

use wb_capability::{
    Capability, CapabilityError, Fingerprint, InUse, SavedFile, choose_in_use, parse_capability,
    route_new_check, unidentified,
};

use crate::capability::CAPABILITY_FILE;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{0}")]
    Format(#[from] CapabilityError),
    #[error("could not write the profile: {0}")]
    Serialize(String),
}

pub fn path_in(dir: &Path) -> PathBuf {
    dir.join(CAPABILITY_FILE)
}

/// The saved profile; `Ok(None)` when none exists. A file that cannot be read as this version's profile
/// is an error, and the caller re-runs the check instead of guessing.
pub fn load(dir: &Path) -> Result<Option<Capability>, StoreError> {
    let path = path_in(dir);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(StoreError::Io { path, source }),
    };
    Ok(Some(parse_capability(&text)?))
}

/// A partial or cancelled check, kept apart so that it never replaces a complete profile.
pub const PARTIAL_FILE: &str = "capability.partial.json";

pub fn partial_path_in(dir: &Path) -> PathBuf {
    dir.join(PARTIAL_FILE)
}

/// The separately saved partial check. A file that cannot be read is logged and treated as absent: the
/// profile in use does not depend on it.
pub fn load_partial(dir: &Path) -> Option<Capability> {
    let path = partial_path_in(dir);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            tracing::warn!("{}: {e}", path.display());
            return None;
        }
    };
    match parse_capability(&text) {
        Ok(c) => Some(c),
        Err(e) => {
            tracing::warn!("{}: {e}", path.display());
            None
        }
    }
}

/// The profile in use and why (see `wb_capability::inuse`), for decisions: starting live text, changing an
/// override. `now` is this computer's fingerprint; when it could not be taken the profile is not known to
/// fit, and carries safe defaults.
pub fn in_use(dir: &Path, now: Option<&Fingerprint>) -> Result<Option<InUse>, StoreError> {
    let used = choose_in_use(load(dir)?, load_partial(dir), now);
    Ok(match now {
        Some(_) => used,
        None => used.map(unidentified),
    })
}

/// Saves a new check's profile where it belongs: a partial or cancelled check goes to the partial file when a
/// complete profile is in use, anything else becomes the profile in use (and the partial file, which belonged to
/// the profile it replaces, goes).
pub fn save_routed(dir: &Path, new: &Capability) -> Result<SavedFile, StoreError> {
    let existing = load(dir).ok().flatten();
    let target = route_new_check(existing.as_ref(), new);
    match target {
        SavedFile::Main => {
            save(dir, new)?;
            match std::fs::remove_file(partial_path_in(dir)) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => tracing::warn!("capability.partial.json was not removed: {e}"),
            }
        }
        SavedFile::Partial => write_file(dir, PARTIAL_FILE, new)?,
    }
    Ok(target)
}

pub fn save(dir: &Path, cap: &Capability) -> Result<(), StoreError> {
    write_file(dir, CAPABILITY_FILE, cap)
}

fn write_file(dir: &Path, name: &str, cap: &Capability) -> Result<(), StoreError> {
    let path = dir.join(name);
    let io = |p: &Path| {
        let p = p.to_path_buf();
        move |source| StoreError::Io {
            path: p.clone(),
            source,
        }
    };
    std::fs::create_dir_all(dir).map_err(io(dir))?;
    let text =
        serde_json::to_string_pretty(cap).map_err(|e| StoreError::Serialize(e.to_string()))?;
    let tmp = dir.join(format!("{name}.tmp"));
    {
        let mut f = std::fs::File::create(&tmp).map_err(io(&tmp))?;
        f.write_all(text.as_bytes()).map_err(io(&tmp))?;
        f.sync_all().map_err(io(&tmp))?;
    }
    std::fs::rename(&tmp, &path).map_err(io(&path))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wb-store-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn no_file_is_not_an_error() {
        let d = dir("none");
        assert!(load(&d).unwrap().is_none());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_file_of_another_version_is_refused() {
        let d = dir("version");
        std::fs::write(path_in(&d), r#"{"schema_version": 0}"#).unwrap();
        assert!(matches!(
            load(&d),
            Err(StoreError::Format(CapabilityError::Schema(0)))
        ));
        std::fs::remove_dir_all(&d).unwrap();
    }

    fn sample(partial: bool) -> Capability {
        let text = include_str!("../../../crates/wb-capability/fixtures/tier-c-low.json");
        let mut c = parse_capability(text).unwrap();
        c.profile.check.partial = partial;
        c
    }

    #[test]
    fn a_partial_check_is_saved_apart_and_the_complete_profile_stays_in_use() {
        let d = dir("routed");
        let complete = sample(false);
        assert_eq!(save_routed(&d, &complete).unwrap(), SavedFile::Main);
        let mut partial = sample(true);
        partial.profile.created_at = "later".into();
        assert_eq!(save_routed(&d, &partial).unwrap(), SavedFile::Partial);
        assert_eq!(
            load(&d).unwrap().unwrap(),
            complete,
            "the complete profile is untouched"
        );
        assert_eq!(load_partial(&d).unwrap(), partial);
        let used = in_use(&d, None).unwrap().unwrap();
        assert_eq!(used.file, SavedFile::Main);
        // A complete check later replaces the profile and the partial file goes.
        assert_eq!(save_routed(&d, &complete).unwrap(), SavedFile::Main);
        assert!(load_partial(&d).is_none());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn with_no_complete_profile_a_partial_check_is_the_profile_in_use() {
        let d = dir("only-partial");
        assert_eq!(save_routed(&d, &sample(true)).unwrap(), SavedFile::Main);
        assert!(load(&d).unwrap().unwrap().profile.check.partial);
        assert!(load_partial(&d).is_none());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn an_unreadable_partial_file_does_not_stop_the_profile_in_use() {
        let d = dir("bad-partial");
        save(&d, &sample(false)).unwrap();
        std::fs::write(partial_path_in(&d), "not json").unwrap();
        assert!(load_partial(&d).is_none());
        assert!(in_use(&d, None).unwrap().is_some());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn garbage_is_refused_not_guessed() {
        let d = dir("garbage");
        std::fs::write(path_in(&d), "not json").unwrap();
        assert!(load(&d).is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
