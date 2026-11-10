//! Operator settings kept in the app data folder: where recordings go, the backup root the mirror
//! writes to, and the last input device.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use wb_recorder::util::{read_json, write_json_atomic};

pub const SETTINGS_FILE: &str = "settings.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    pub recordings_root: String,
    #[serde(default)]
    pub backup_root: Option<String>,
    #[serde(default)]
    pub last_device_id: Option<String>,
}

impl Settings {
    /// `D:\STELMAR-Transcripts` when a local D: drive exists (PLAN section 8), otherwise
    /// `<Documents>\STELMAR-Transcripts`, unless Documents is synced by OneDrive (its client
    /// would upload session audio whenever the machine is online): then
    /// `<system drive>\STELMAR-Transcripts`.
    pub fn defaults(documents: Option<PathBuf>) -> Self {
        let d = Path::new("D:\\");
        let root = if cfg!(windows) && d.is_dir() && wb_recorder::sys::is_local_path(d) {
            d.join("STELMAR-Transcripts")
        } else {
            match documents.filter(|p| !is_onedrive(p)) {
                Some(docs) => docs.join("STELMAR-Transcripts"),
                None if cfg!(windows) => {
                    let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
                    PathBuf::from(format!("{drive}\\")).join("STELMAR-Transcripts")
                }
                None => std::env::temp_dir().join("STELMAR-Transcripts"),
            }
        };
        Self {
            recordings_root: root.to_string_lossy().into_owned(),
            backup_root: None,
            last_device_id: None,
        }
    }

    pub fn recordings_root(&self) -> PathBuf {
        PathBuf::from(&self.recordings_root)
    }

    pub fn backup_root(&self) -> Option<PathBuf> {
        self.backup_root
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .map(PathBuf::from)
    }

    /// Checks the folders: both absolute, and the backup neither the recordings folder nor
    /// inside it (or the reverse).
    pub fn validate(&self) -> Result<(), String> {
        let rec = self.recordings_root();
        if self.recordings_root.trim().is_empty() || !rec.is_absolute() {
            return Err(
                "The recordings folder must be a full path, for example D:\\STELMAR-Transcripts"
                    .into(),
            );
        }
        const NOT_LOCAL: &str = "Recordings and backups must be on a drive in this computer (internal or USB), not a network folder";
        if !wb_recorder::sys::is_local_path(&rec) {
            return Err(NOT_LOCAL.into());
        }
        if let Some(b) = self.backup_root() {
            if !wb_recorder::sys::is_local_path(&b) && b.is_absolute() {
                return Err(NOT_LOCAL.into());
            }
            if !b.is_absolute() {
                return Err(
                    "The backup folder must be a full path, for example E:\\STELMAR-Backup".into(),
                );
            }
            if b.starts_with(&rec) || rec.starts_with(&b) {
                return Err("The backup folder must be separate from the recordings folder, ideally on another drive".into());
            }
        }
        Ok(())
    }
}

/// Loads the saved settings. Folders that are not on a drive in this computer (a settings file
/// saved before this check, edited by hand or copied in by a setup script) are replaced: the
/// recordings folder by the default, the backup folder by none. Nothing is ever listed or
/// written over the network.
/// True when `path` is inside a OneDrive-synced folder (the OneDrive client uploads it).
fn is_onedrive(path: &Path) -> bool {
    ["OneDrive", "OneDriveCommercial", "OneDriveConsumer"]
        .iter()
        .filter_map(std::env::var_os)
        .filter(|v| !v.is_empty())
        .any(|root| path.starts_with(Path::new(&root)))
}

pub fn load(data_dir: &Path, documents: Option<PathBuf>) -> Settings {
    let defaults = Settings::defaults(documents);
    let Ok(mut s) = read_json::<Settings>(&data_dir.join(SETTINGS_FILE)) else {
        return defaults;
    };
    if !wb_recorder::sys::is_local_path(&s.recordings_root()) {
        tracing::warn!(
            folder = s.recordings_root,
            "saved recordings folder is not on a local drive; using the default"
        );
        s.recordings_root = defaults.recordings_root;
    }
    if s.backup_root()
        .is_some_and(|b| !wb_recorder::sys::is_local_path(&b))
    {
        tracing::warn!("saved backup folder is not on a local drive; backup turned off");
        s.backup_root = None;
    }
    s
}

pub fn save(data_dir: &Path, settings: &Settings) -> Result<(), String> {
    std::fs::create_dir_all(data_dir)
        .map_err(|e| format!("Could not create {}: {e}", data_dir.display()))?;
    write_json_atomic(&data_dir.join(SETTINGS_FILE), settings)
        .map_err(|e| format!("Could not save settings: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(rec: &str, backup: Option<&str>) -> Settings {
        Settings {
            recordings_root: rec.into(),
            backup_root: backup.map(Into::into),
            last_device_id: None,
        }
    }

    #[cfg(windows)]
    #[test]
    fn validation() {
        assert!(s("D:\\STELMAR-Transcripts", None).validate().is_ok());
        assert!(
            s("D:\\STELMAR-Transcripts", Some("E:\\Backup"))
                .validate()
                .is_ok()
        );
        assert!(s("D:\\STELMAR-Transcripts", Some("")).validate().is_ok());
        assert!(s("relative", None).validate().is_err());
        assert!(s(r"\\srv\share\rec", None).validate().is_err());
        assert!(s(r"\\?\UNC\srv\share\rec", None).validate().is_err());
        assert!(
            s("D:\\STELMAR-Transcripts", Some(r"\\srv\share\backup"))
                .validate()
                .is_err()
        );
        assert!(
            s(
                "D:\\STELMAR-Transcripts",
                Some("D:\\STELMAR-Transcripts\\copy")
            )
            .validate()
            .is_err()
        );
        assert!(
            s("D:\\STELMAR-Transcripts", Some("D:\\"))
                .validate()
                .is_err()
        );
        assert!(
            s("D:\\STELMAR-Transcripts", Some("backup"))
                .validate()
                .is_err()
        );
    }

    #[test]
    fn network_folders_in_a_saved_file_are_replaced() -> Result<(), Box<dyn std::error::Error>> {
        let dir = wb_recorder::testutil::TempDir::new("settings-unc")?;
        std::fs::write(
            dir.path().join(SETTINGS_FILE),
            r#"{"recordings_root":"\\\\srv\\share\\rec","backup_root":"\\\\?\\UNC\\srv\\share\\bak","last_device_id":null}"#,
        )?;
        let s = load(dir.path(), Some(dir.path().to_path_buf()));
        assert!(wb_recorder::sys::is_local_path(&s.recordings_root()));
        assert!(s.recordings_root.ends_with("STELMAR-Transcripts"));
        assert_eq!(s.backup_root, None);
        Ok(())
    }

    #[test]
    fn round_trip_and_defaults() -> Result<(), Box<dyn std::error::Error>> {
        let dir = wb_recorder::testutil::TempDir::new("settings")?;
        let loaded = load(dir.path(), Some(dir.path().to_path_buf()));
        assert!(loaded.recordings_root.ends_with("STELMAR-Transcripts"));
        let mut changed = loaded.clone();
        changed.last_device_id = Some("wasapi:{x}".into());
        save(dir.path(), &changed)?;
        assert_eq!(load(dir.path(), None), changed);
        Ok(())
    }
}
