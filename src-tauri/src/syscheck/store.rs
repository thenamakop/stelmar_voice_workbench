//! `capability.json` on disk: written whole to a temporary file, flushed, then renamed over the old one,
//! so a power cut leaves either the old profile or the new one, never half of one.

use std::io::Write;
use std::path::{Path, PathBuf};

use wb_capability::{Capability, CapabilityError, Fingerprint, parse_capability};

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

pub fn save(dir: &Path, cap: &Capability) -> Result<(), StoreError> {
    let path = path_in(dir);
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
    let tmp = dir.join(format!("{CAPABILITY_FILE}.tmp"));
    {
        let mut f = std::fs::File::create(&tmp).map_err(io(&tmp))?;
        f.write_all(text.as_bytes()).map_err(io(&tmp))?;
        f.sync_all().map_err(io(&tmp))?;
    }
    std::fs::rename(&tmp, &path).map_err(io(&path))
}

/// What differs between the saved profile's machine and this one, in words; empty when the saved
/// profile still describes this machine.
pub fn fingerprint_changes(saved: &Capability, now: &Fingerprint) -> Vec<String> {
    now.differences(&saved.profile.fingerprint)
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

    #[test]
    fn garbage_is_refused_not_guessed() {
        let d = dir("garbage");
        std::fs::write(path_in(&d), "not json").unwrap();
        assert!(load(&d).is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
