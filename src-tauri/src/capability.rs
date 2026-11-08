//! The capability profile (`capability.json`) written by the system check (Phase 3).
//!
//! Phase 0 only needs to know whether it exists: without it the app opens on the System screen.

use std::path::Path;

use tauri::Manager;

/// File name of the capability profile inside the app data folder.
pub const CAPABILITY_FILE: &str = "capability.json";

/// True when `dir` holds a capability profile file.
pub fn profile_exists_in(dir: &Path) -> bool {
    dir.join(CAPABILITY_FILE).is_file()
}

/// Tells the UI whether the first-launch system check has saved a profile on this machine.
#[tauri::command]
pub fn has_capability_profile(app: tauri::AppHandle) -> Result<bool, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|err| format!("Could not find the app data folder: {err}"))?;
    Ok(profile_exists_in(&dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> std::io::Result<std::path::PathBuf> {
        let dir =
            std::env::temp_dir().join(format!("wb-capability-test-{name}-{}", std::process::id()));
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
        }
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    #[test]
    fn missing_profile_means_first_launch() -> std::io::Result<()> {
        let dir = temp_dir("missing")?;
        assert!(!profile_exists_in(&dir));
        std::fs::remove_dir_all(&dir)
    }

    #[test]
    fn present_profile_is_found() -> std::io::Result<()> {
        let dir = temp_dir("present")?;
        std::fs::write(dir.join(CAPABILITY_FILE), "{}")?;
        assert!(profile_exists_in(&dir));
        std::fs::remove_dir_all(&dir)
    }

    #[test]
    fn a_folder_named_like_the_profile_does_not_count() -> std::io::Result<()> {
        let dir = temp_dir("folder")?;
        std::fs::create_dir(dir.join(CAPABILITY_FILE))?;
        assert!(!profile_exists_in(&dir));
        std::fs::remove_dir_all(&dir)
    }
}
