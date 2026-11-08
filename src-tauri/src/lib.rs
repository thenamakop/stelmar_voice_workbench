//! STELMAR Voice Workbench app process.
//!
//! Phase 0: the window, the strict offline configuration and one command that tells the UI
//! whether the first-launch system check has run. Audio, engines and the scheduler come later.

mod capability;

/// Errors that stop the app from starting.
#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error("Tauri runtime error: {0}")]
    Tauri(#[from] tauri::Error),
}

/// Starts the app and blocks until the last window closes.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() -> Result<(), RunError> {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![capability::has_capability_profile])
        .run(tauri::generate_context!())?;
    Ok(())
}
