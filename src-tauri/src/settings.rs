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

/// Settings fields that something other than the operator set for this run (the gate hooks' recordings and
/// backup folders and input device, `gate.rs`) and that must never be written to `settings.json`: a gate run
/// that shares the real app data folder would otherwise leave its test folders behind as the operator's
/// own. Always empty outside a gate build.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Transient {
    pub recordings_root: bool,
    pub backup_root: bool,
    pub last_device_id: bool,
}

impl Transient {
    pub fn any(self) -> bool {
        self.recordings_root || self.backup_root || self.last_device_id
    }
}

/// What may be written for `current`: the transient fields keep the value already saved on disk. `None` means
/// nothing may be written (a transient field and no saved file: a gate run never creates `settings.json`).
pub fn persistable(
    saved: Option<&Settings>,
    current: &Settings,
    transient: Transient,
) -> Option<Settings> {
    if !transient.any() {
        return Some(current.clone());
    }
    let saved = saved?;
    let mut out = current.clone();
    if transient.recordings_root {
        out.recordings_root = saved.recordings_root.clone();
    }
    if transient.backup_root {
        out.backup_root = saved.backup_root.clone();
    }
    if transient.last_device_id {
        out.last_device_id = saved.last_device_id.clone();
    }
    Some(out)
}

pub fn save(data_dir: &Path, settings: &Settings) -> Result<(), String> {
    #[cfg(feature = "gate-hooks")]
    let transient = crate::gate::transient();
    #[cfg(not(feature = "gate-hooks"))]
    let transient = Transient::default();
    save_with(data_dir, settings, transient)
}

fn save_with(data_dir: &Path, settings: &Settings, transient: Transient) -> Result<(), String> {
    let saved = if transient.any() {
        read_json::<Settings>(&data_dir.join(SETTINGS_FILE)).ok()
    } else {
        None
    };
    let Some(to_write) = persistable(saved.as_ref(), settings, transient) else {
        return Ok(());
    };
    std::fs::create_dir_all(data_dir)
        .map_err(|e| format!("Could not create {}: {e}", data_dir.display()))?;
    write_json_atomic(&data_dir.join(SETTINGS_FILE), &to_write)
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

    const REAL_ROOT: &str = "D:\\STELMAR-Transcripts";
    const GATE_ROOT: &str = "D:\\STELMAR-Gate\\dryrun-hash\\hashing-emu-low-x\\rec";
    const ALL: Transient = Transient {
        recordings_root: true,
        backup_root: true,
        last_device_id: true,
    };

    fn read(dir: &Path) -> Result<Settings, Box<dyn std::error::Error>> {
        Ok(read_json::<Settings>(&dir.join(SETTINGS_FILE))?)
    }

    fn write(dir: &Path, s: &Settings) -> Result<(), Box<dyn std::error::Error>> {
        std::fs::write(dir.join(SETTINGS_FILE), serde_json::to_string(s)?)?;
        Ok(())
    }

    /// The leak of 23 Nov 2026: a gate run on the real data folder saved its folders over the operator's.
    #[test]
    fn gate_folders_are_never_persisted() -> Result<(), Box<dyn std::error::Error>> {
        let dir = wb_recorder::testutil::TempDir::new("settings-gate-keep")?;
        let mut real = s(REAL_ROOT, None);
        real.last_device_id = Some("wasapi:real".into());
        write(dir.path(), &real)?;
        let mut gate = s(GATE_ROOT, Some("C:\\STELMAR-Gate-Backup\\x"));
        gate.last_device_id = Some("wasapi:cable".into());
        save_with(dir.path(), &gate, ALL)?;
        assert_eq!(read(dir.path())?, real);
        Ok(())
    }

    #[test]
    fn a_gate_run_never_creates_settings_json() -> Result<(), Box<dyn std::error::Error>> {
        let dir = wb_recorder::testutil::TempDir::new("settings-gate-none")?;
        save_with(dir.path(), &s(GATE_ROOT, None), ALL)?;
        assert!(!dir.path().join(SETTINGS_FILE).exists());
        Ok(())
    }

    #[test]
    fn only_the_fields_the_gate_set_are_held_back() -> Result<(), Box<dyn std::error::Error>> {
        let dir = wb_recorder::testutil::TempDir::new("settings-gate-mixed")?;
        let mut real = s(REAL_ROOT, None);
        real.last_device_id = Some("wasapi:real".into());
        write(dir.path(), &real)?;
        // The operator changes the backup folder while only the device is a gate setting.
        let mut now = s(REAL_ROOT, Some("E:\\STELMAR-Backup"));
        now.last_device_id = Some("wasapi:cable".into());
        let only_device = Transient {
            last_device_id: true,
            ..Transient::default()
        };
        save_with(dir.path(), &now, only_device)?;
        let back = read(dir.path())?;
        assert_eq!(back.backup_root.as_deref(), Some("E:\\STELMAR-Backup"));
        assert_eq!(back.last_device_id.as_deref(), Some("wasapi:real"));
        Ok(())
    }

    #[test]
    fn a_normal_run_saves_what_it_is_given() -> Result<(), Box<dyn std::error::Error>> {
        let dir = wb_recorder::testutil::TempDir::new("settings-normal")?;
        let mut now = s(REAL_ROOT, Some("E:\\STELMAR-Backup"));
        now.last_device_id = Some("wasapi:x".into());
        save_with(dir.path(), &now, Transient::default())?;
        assert_eq!(read(dir.path())?, now);
        assert_eq!(
            persistable(None, &now, Transient::default()),
            Some(now.clone())
        );
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
