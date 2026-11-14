//! STELMAR Voice Workbench app process.
//!
//! Phase 1: audio capture and crash-safe recording (crates/wb-recorder) behind Tauri commands
//! and channel events, crash recovery at start-up, and a display guard that recovers from
//! WebView2 process failures without touching the recording.

mod boundary;
mod capability;
mod display;
mod engines;
#[cfg(feature = "gate-hooks")]
mod gate;
mod machine;
// `start_session` here is replaced by `syscheck::start_recording` (the command layer that cancels a running
// system check first); this file stays byte-identical to the Phase 1 gate build, so its old command is unused.
#[allow(dead_code)]
mod recording;
mod settings;
mod state;
mod syscheck;
#[cfg(windows)]
mod win;

use std::fs::OpenOptions;
use std::sync::Mutex;

use tauri::{Manager, RunEvent, WindowEvent};

use crate::state::AppState;

/// Worker threads of the async runtime, from the affinity mask (logged at start).
static RUNTIME_WORKERS: std::sync::OnceLock<usize> = std::sync::OnceLock::new();

/// Errors that stop the app from starting.
#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error("Tauri runtime error: {0}")]
    Tauri(#[from] tauri::Error),
}

fn init_logging(app: &tauri::App) {
    let Ok(dir) = app.path().app_log_dir() else {
        return;
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let Ok(file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("app.log"))
    else {
        return;
    };
    let _ = tracing_subscriber::fmt()
        .with_writer(Mutex::new(file))
        .with_ansi(false)
        .with_thread_names(true)
        .with_target(false)
        .try_init();
}

fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    init_logging(app);
    boundary::install_panic_hook();
    tracing::info!(
        version = recording::APP_VERSION,
        pid = std::process::id(),
        gate_hooks = cfg!(feature = "gate-hooks"),
        "app started"
    );
    // Emulation (wb-emulate) is a development aid: only development and gate builds read it.
    #[cfg(any(debug_assertions, feature = "gate-hooks"))]
    tracing::info!(
        emulated = std::env::var("WB_EMULATE").unwrap_or_default(),
        "emulation profile"
    );
    let machine = machine::get();
    tracing::info!(
        runtime_workers = RUNTIME_WORKERS.get().copied().unwrap_or(0),
        logical_processors = machine.logical_processors,
        physical_cores = machine.physical_cores,
        std_available_parallelism = machine.std_available_parallelism,
        total_ram_bytes = machine.total_ram_bytes,
        available_ram_bytes = machine.available_ram_bytes,
        job_memory_limit_bytes = machine.job_memory_limit_bytes.unwrap_or(0),
        "machine view (from the affinity mask and the job limit, not from the host)"
    );
    let data_dir = app.path().app_data_dir()?;
    #[cfg(feature = "gate-hooks")]
    let data_dir = gate::data_dir_override().unwrap_or(data_dir);
    let settings = settings::load(&data_dir, app.path().document_dir().ok());
    let (state, events_rx) = AppState::new(data_dir, settings);
    state::spawn_forwarder(events_rx, state.snapshot.clone(), state.subscribers.clone());
    app.manage(state);
    app.manage(engines::Registry::default());
    app.manage(syscheck::CheckState::default());
    #[cfg(feature = "gate-hooks")]
    gate::apply_overrides(app.handle());

    engines::start(app.handle());

    let handle = app.handle().clone();
    std::thread::Builder::new()
        .name("wb-recovery-scan".into())
        .spawn(move || recording::scan_for_recovery(&handle))?;

    for window in app.webview_windows().values() {
        display::guard(window);
    }
    #[cfg(feature = "gate-hooks")]
    gate::start(app.handle());
    Ok(())
}

/// Starts the app and blocks until it exits.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() -> Result<(), RunError> {
    if let Some(workers) = machine::install_runtime() {
        let _ = RUNTIME_WORKERS.set(workers);
    }
    let app = tauri::Builder::default()
        .setup(setup)
        .on_window_event(|window, event| {
            // Closing the window mid-session would end the recording: ask for Stop first.
            if let WindowEvent::CloseRequested { api, .. } = event
                && recording::is_recording(window.app_handle())
            {
                api.prevent_close();
                tracing::warn!("window close refused while recording");
                window
                    .app_handle()
                    .state::<AppState>()
                    .broadcast(serde_json::json!({ "kind": "close_blocked" }));
            }
        })
        .invoke_handler(tauri::generate_handler![
            capability::has_capability_profile,
            recording::list_inputs,
            recording::get_settings,
            recording::save_settings,
            syscheck::start_recording,
            recording::pause,
            recording::resume,
            recording::stop_session,
            recording::subscribe_events,
            recording::recorder_snapshot,
            recording::recovery_list,
            recording::recover_session,
            engines::list_engines,
            engines::recheck_registry,
            syscheck::get_capability,
            syscheck::run_system_check,
            syscheck::cancel_system_check,
            syscheck::set_override,
            syscheck::session_plan,
        ])
        .build(tauri::generate_context!())?;
    app.run(|_handle, event| {
        if let RunEvent::Exit = event {
            tracing::info!("app exiting");
        }
    });
    Ok(())
}
