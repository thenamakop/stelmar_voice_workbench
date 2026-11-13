//! Recording commands: inputs, settings, start, pause, resume, stop, events and crash recovery.
//! Each runs inside the command boundary (boundary.rs).

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager};
use wb_recorder::capture::{self, InputDevice};
use wb_recorder::recovery::{self, RecoveredSession};
use wb_recorder::session::{self, SessionSpec};
use wb_recorder::{ActiveSession, SessionStopped, StartOptions};

use crate::boundary::blocking;
use crate::settings::{self, Settings};
use crate::state::{AppState, SessionInfo, Snapshot, lock};

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// What the operator asked for when starting a session.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct StartRequest {
    pub title: String,
    #[serde(default)]
    pub hall: Option<String>,
    #[serde(default)]
    pub programme_id: Option<String>,
    #[serde(default)]
    pub scheduled: Option<String>,
    #[serde(default)]
    pub expected_languages: Option<Vec<String>>,
    /// Input device id from `list_inputs`; the last used (or the system default) when absent.
    #[serde(default)]
    pub device_id: Option<String>,
    /// `record_post` (the default everywhere) or `live_post`. Live is accepted only where the saved
    /// capability profile clears every language of the session (AGENTS.md non-negotiable 7).
    #[serde(default)]
    pub mode: Option<String>,
}

fn state(app: &AppHandle) -> tauri::State<'_, AppState> {
    app.state::<AppState>()
}

fn options(st: &AppState, device_id: String) -> StartOptions {
    #[cfg(feature = "gate-hooks")]
    {
        let mut opts = options_from_settings(st, device_id);
        if let Some(secs) = crate::gate::part_seconds() {
            opts.part_seconds = secs;
        }
        opts
    }
    #[cfg(not(feature = "gate-hooks"))]
    {
        options_from_settings(st, device_id)
    }
}

fn options_from_settings(st: &AppState, device_id: String) -> StartOptions {
    let s = lock(&st.settings).clone();
    StartOptions::new(s.recordings_root(), s.backup_root(), device_id, APP_VERSION)
}

/// The device to use: the one asked for, the last one used if it is still there, or the default.
///
/// With `asked_name` (resuming a session), a device that is gone is looked up by name (a replug
/// gives it a new id) and otherwise replaced by the last-used or default input, with a notice.
fn pick_device(
    st: &AppState,
    asked: Option<String>,
    asked_name: Option<&str>,
) -> Result<String, String> {
    let inputs = capture::list_inputs().map_err(|e| e.to_string())?;
    if let Some(id) = asked.filter(|s| !s.is_empty()) {
        if inputs.iter().any(|d| d.id == id) {
            return Ok(id);
        }
        let Some(name) = asked_name else {
            // Chosen in the UI a moment ago: let the capture thread report it if it is gone.
            return Ok(id);
        };
        if let Some(d) = inputs.iter().find(|d| d.name == name) {
            return Ok(d.id.clone());
        }
        let fallback = lock(&st.settings)
            .last_device_id
            .clone()
            .and_then(|last| inputs.iter().find(|d| d.id == last))
            .or_else(|| inputs.first())
            .ok_or_else(|| {
                "No audio input was found. Connect the audio interface and try again.".to_string()
            })?;
        let _ = st.events.send(wb_recorder::events::Notice::event(
            wb_recorder::events::NoticeLevel::Warning,
            "capture",
            format!(
                "{name} is not connected; recording from {} instead",
                fallback.name
            ),
        ));
        return Ok(fallback.id.clone());
    }
    let last = lock(&st.settings).last_device_id.clone();
    if let Some(last) = last
        && inputs.iter().any(|d| d.id == last)
    {
        return Ok(last);
    }
    inputs.first().map(|d| d.id.clone()).ok_or_else(|| {
        "No audio input was found. Connect the audio interface and try again.".to_string()
    })
}

fn remember_device(st: &AppState, id: &str) {
    let mut s = lock(&st.settings);
    if s.last_device_id.as_deref() != Some(id) {
        s.last_device_id = Some(id.to_string());
        if let Err(e) = settings::save(&st.data_dir, &s) {
            tracing::warn!("{e}");
        }
    }
}

fn session_info(active: &ActiveSession, resumed: bool) -> Result<SessionInfo, String> {
    let meta = session::load_meta(active.dir()).map_err(|e| e.to_string())?;
    Ok(SessionInfo {
        dir: active.dir().to_string_lossy().into_owned(),
        title: meta.title,
        hall: meta.hall,
        scheduled: meta.scheduled,
        expected_languages: meta.expected_languages,
        mode: meta.mode,
        opened_at: meta.opened_at,
        input: active.input().clone(),
        backup_root: meta.backup_root,
        resumed,
    })
}

/// Records a newly started session in the state. `slot` is the held session lock.
fn opened(
    st: &AppState,
    slot: &mut Option<ActiveSession>,
    active: ActiveSession,
    resumed: bool,
) -> Result<SessionInfo, String> {
    let info = session_info(&active, resumed)?;
    remember_device(st, &info.input.device_id);
    {
        let mut snap = lock(&st.snapshot);
        let restarts = snap.display_restarts;
        let warning = snap.display_warning.take();
        *snap = Snapshot {
            session: Some(info.clone()),
            display_restarts: restarts,
            display_warning: warning,
            ..Snapshot::default()
        };
        if resumed {
            snap.parts = wb_recorder::parts::load(active.dir())
                .map(|i| i.parts)
                .unwrap_or_default();
        }
    }
    *slot = Some(active);
    st.recording.store(true, Ordering::Release);
    tracing::info!(
        dir = info.dir,
        device = info.input.device_name,
        rate = info.input.sample_rate,
        channels = info.input.channels,
        resumed,
        "session recording"
    );
    Ok(info)
}

/// Starts a new session (blocking; called inside the boundary and by the gate hooks).
pub fn start_blocking(app: &AppHandle, req: StartRequest) -> Result<SessionInfo, String> {
    let st = state(app);
    let mut current = lock(&st.session);
    if current.is_some() {
        return Err("A session is already recording. Stop it before starting another.".into());
    }
    if crate::syscheck::is_running(app) {
        return Err(
            "The system check is running. Start the recording when it has finished, or cancel it on the System screen."
                .into(),
        );
    }
    let title = req.title.trim();
    if title.is_empty() {
        return Err("The session needs a title".into());
    }
    let mut spec = SessionSpec::unscheduled(title);
    spec.hall = req.hall.filter(|h| !h.trim().is_empty());
    spec.programme_id = req.programme_id;
    spec.scheduled = req.scheduled;
    if let Some(langs) = req.expected_languages.filter(|l| !l.is_empty()) {
        spec.expected_languages = langs;
    }
    match req.mode.as_deref() {
        None | Some("record_post") => {}
        Some("live_post") => {
            crate::syscheck::live_allowed(&st.data_dir, &spec.expected_languages)?;
            spec.mode = "live_post".into();
        }
        Some(other) => return Err(format!("Unknown recording mode '{other}'.")),
    }
    let device = pick_device(&st, req.device_id, None)?;
    let opts = options(&st, device);
    let active =
        ActiveSession::start(&spec, &opts, st.events.clone()).map_err(|e| e.to_string())?;
    opened(&st, &mut current, active, false)
}

/// Stops the open session (blocking).
pub fn stop_blocking(app: &AppHandle) -> Result<SessionStopped, String> {
    let st = state(app);
    let active = lock(&st.session).take().ok_or("No session is recording")?;
    let dir = active.dir().to_path_buf();
    let result = active.stop();
    st.recording.store(false, Ordering::Release);
    let result = match result {
        Ok(r) => r,
        Err(e) => {
            // The session stays open: offer it for Resume or Close now, not only after a restart.
            tracing::error!(dir = %dir.display(), "stop failed: {e}");
            match recovery::repair(&dir) {
                Ok(r) => lock(&st.recovered).push(r),
                Err(re) => lock(&st.recovery_failed).push(format!("{}: {re}", dir.display())),
            }
            lock(&st.snapshot).session = None;
            return Err(format!(
                "{e}. The session is kept and offered for Resume or Close."
            ));
        }
    };
    tracing::info!(
        dir = %dir.display(),
        parts = result.recorder.parts.len(),
        audio_ms = result.recorder.audio_ms,
        overrun_samples = result.recorder.overrun_samples,
        xruns = result.recorder.xruns,
        mirror_all_match = result.mirror.all_match,
        "session stopped"
    );
    let mut snap = lock(&st.snapshot);
    snap.session = None;
    snap.last_stop = Some(result.clone());
    Ok(result)
}

#[tauri::command]
pub async fn list_inputs() -> Result<Vec<InputDevice>, String> {
    blocking("list_inputs", || {
        capture::list_inputs().map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
pub async fn get_settings(app: AppHandle) -> Result<Settings, String> {
    blocking("get_settings", move || {
        Ok(lock(&state(&app).settings).clone())
    })
    .await
}

#[tauri::command]
pub async fn save_settings(app: AppHandle, settings: Settings) -> Result<Settings, String> {
    blocking("save_settings", move || {
        let mut settings = settings;
        settings.backup_root = settings.backup_root.filter(|b| !b.trim().is_empty());
        settings.validate()?;
        let st = state(&app);
        settings::save(&st.data_dir, &settings)?;
        *lock(&st.settings) = settings.clone();
        Ok(settings)
    })
    .await
}

#[tauri::command]
pub async fn start_session(app: AppHandle, request: StartRequest) -> Result<SessionInfo, String> {
    blocking("start_session", move || start_blocking(&app, request)).await
}

#[tauri::command]
pub async fn pause(app: AppHandle) -> Result<(), String> {
    blocking("pause", move || {
        let st = state(&app);
        let guard = lock(&st.session);
        guard
            .as_ref()
            .ok_or("No session is recording")?
            .pause()
            .map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
pub async fn resume(app: AppHandle) -> Result<(), String> {
    blocking("resume", move || {
        let st = state(&app);
        let guard = lock(&st.session);
        guard
            .as_ref()
            .ok_or("No session is recording")?
            .resume_recording()
            .map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
pub async fn stop_session(app: AppHandle) -> Result<SessionStopped, String> {
    blocking("stop_session", move || stop_blocking(&app)).await
}

/// Registers a channel for `level`, `recorder_status`, `mirror_status`, `part_closed`, `notice`
/// and `display_recovered` events.
#[tauri::command]
pub async fn subscribe_events(
    app: AppHandle,
    on_event: Channel<serde_json::Value>,
) -> Result<(), String> {
    blocking("subscribe_events", move || {
        lock(&state(&app).subscribers).push(on_event);
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn recorder_snapshot(app: AppHandle) -> Result<Snapshot, String> {
    blocking("recorder_snapshot", move || {
        Ok(lock(&state(&app).snapshot).clone())
    })
    .await
}

#[derive(Debug, Clone, Serialize)]
pub struct RecoveryList {
    /// False while the start-up scan is still repairing files.
    pub done: bool,
    pub sessions: Vec<RecoveredSession>,
    /// Sessions a crash left open that could not be repaired (folder and reason).
    pub failed: Vec<String>,
}

#[tauri::command]
pub async fn recovery_list(app: AppHandle) -> Result<RecoveryList, String> {
    blocking("recovery_list", move || {
        let st = state(&app);
        Ok(RecoveryList {
            done: st.recovery_done.load(Ordering::Acquire),
            sessions: lock(&st.recovered).clone(),
            failed: lock(&st.recovery_failed).clone(),
        })
    })
    .await
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecoverAction {
    Resume,
    Close,
}

/// Resumes or closes a session recovery found (blocking).
pub fn recover_blocking(
    app: &AppHandle,
    dir: &str,
    action: RecoverAction,
) -> Result<Option<SessionInfo>, String> {
    let st = state(app);
    let path = PathBuf::from(dir);
    if !lock(&st.recovered)
        .iter()
        .any(|r| Path::new(&r.dir) == path)
    {
        return Err("That session is not waiting for recovery".into());
    }
    let result = match action {
        RecoverAction::Close => {
            recovery::close(&path).map_err(|e| e.to_string())?;
            tracing::info!(dir, "recovered session closed");
            sync_backup_after_close(&st, &path);
            None
        }
        RecoverAction::Resume => {
            let mut current = lock(&st.session);
            if current.is_some() {
                return Err(
                    "Another session is recording. Stop it before resuming this one.".into(),
                );
            }
            let meta = session::load_meta(&path).map_err(|e| e.to_string())?;
            let (asked_id, asked_name) = meta
                .input
                .as_ref()
                .map(|i| (Some(i.device_id.clone()), Some(i.device_name.clone())))
                .unwrap_or((None, None));
            let device = pick_device(&st, asked_id, asked_name.as_deref())?;
            let mut opts = options(&st, device);
            // Keep the session's own backup root so the mirror continues in the same place.
            // A backup root that is not on a local drive is dropped (never mirror over the network).
            if let Some(b) = meta.backup_root.map(PathBuf::from) {
                if wb_recorder::sys::is_local_path(&b) {
                    opts.backup_root = Some(b);
                } else {
                    tracing::warn!(backup = %b.display(), "resume: backup root is not a local drive; mirroring off");
                    let _ = st.events.send(wb_recorder::events::Notice::event(
                        wb_recorder::events::NoticeLevel::Warning,
                        "mirror",
                        "The session's backup folder is not on a drive in this computer, so no backup copy is made. Set a local backup folder in Settings.",
                    ));
                    opts.backup_root = None;
                }
            }
            if let Some(root) = path.parent().and_then(Path::parent) {
                opts.recordings_root = root.to_path_buf();
            }
            let active = ActiveSession::resume(&path, &opts, st.events.clone())
                .map_err(|e| e.to_string())?;
            Some(opened(&st, &mut current, active, true)?)
        }
    };
    lock(&st.recovered).retain(|r| Path::new(&r.dir) != path);
    Ok(result)
}

#[tauri::command]
pub async fn recover_session(
    app: AppHandle,
    dir: String,
    action: RecoverAction,
) -> Result<Option<SessionInfo>, String> {
    blocking("recover_session", move || {
        recover_blocking(&app, &dir, action)
    })
    .await
}

/// Finds and repairs sessions a crash left open (start-up, on a background thread).
pub fn scan_for_recovery(app: &AppHandle) {
    let st = state(app);
    let root = lock(&st.settings).recordings_root();
    if !wb_recorder::sys::is_local_path(&root) {
        // Never list a network folder (settings::load already replaces one; this is the backstop).
        tracing::warn!(folder = %root.display(), "recovery scan skipped: not a local drive");
        st.recovery_done.store(true, Ordering::Release);
        return;
    }
    let mut found = Vec::new();
    let mut failed = Vec::new();
    match recovery::scan(&root) {
        Ok(dirs) => {
            for dir in dirs {
                match recovery::repair(&dir) {
                    Ok(r) => {
                        tracing::warn!(
                            dir = r.dir,
                            parts = r.parts,
                            audio_ms = r.audio_ms,
                            "recovery: session left open by a crash; offering Resume or Close"
                        );
                        found.push(r);
                    }
                    Err(e) => {
                        tracing::error!(dir = %dir.display(), "recovery: could not repair: {e}");
                        failed.push(format!("{}: {e}", dir.display()));
                    }
                }
            }
        }
        Err(e) => tracing::error!("recovery scan of {} failed: {e}", root.display()),
    }
    *lock(&st.recovered) = found;
    *lock(&st.recovery_failed) = failed;
    st.recovery_done.store(true, Ordering::Release);
}

/// True while a session is open (used to keep the window from closing mid-recording). Reads a
/// flag, never the session lock, so the UI thread cannot wait behind a slow start or stop.
pub fn is_recording(app: &AppHandle) -> bool {
    state(app).recording.load(Ordering::Acquire)
}

/// After Close on a recovered session: bring its backup copy up to date and verify it.
fn sync_backup_after_close(st: &AppState, dir: &Path) {
    let Ok(meta) = session::load_meta(dir) else {
        return;
    };
    let Some(root) = meta.backup_root.map(PathBuf::from) else {
        return;
    };
    if !wb_recorder::sys::is_local_path(&root) {
        return;
    }
    let (Some(session_name), Some(day)) = (dir.file_name(), dir.parent().and_then(Path::file_name))
    else {
        return;
    };
    let backup = root.join(day).join(session_name);
    match wb_recorder::mirror::MirrorHandle::spawn(
        dir.to_path_buf(),
        Some(backup),
        st.events.clone(),
    ) {
        Ok(mut m) => {
            let verify = m.final_sync();
            m.stop();
            tracing::info!(
                all_match = verify.all_match,
                "backup copy after recovery close"
            );
            if !verify.all_match {
                let _ = st.events.send(wb_recorder::events::Notice::event(
                    wb_recorder::events::NoticeLevel::Warning,
                    "mirror",
                    verify.message.unwrap_or_else(|| {
                        "The backup copy of the closed session could not be verified".into()
                    }),
                ));
            }
        }
        Err(e) => tracing::error!("backup after recovery close: {e}"),
    }
}
