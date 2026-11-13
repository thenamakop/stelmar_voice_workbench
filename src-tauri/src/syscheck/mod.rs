//! The system check (Phase 3): probe this computer, benchmark the ranked models on each device
//! configuration, test that the recorder stays safe, and save `capability.json`. The rules live in
//! `crates/wb-capability`; this module measures and stores.
//!
//! The check never runs while a session is recording, and it runs on its own thread: the recorder is
//! not touched. Engines run below normal priority inside the kill-on-close job, as in production.

pub mod bench;
pub mod check;
pub mod excerpt;
pub mod probe;
pub mod rankings;
pub mod safety;
pub mod store;
#[cfg(windows)]
mod win;

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use tauri::{AppHandle, Manager};
use wb_capability::{
    Capability, CheckKind, HindiUrduScript, Options, OverrideKind, OverrideRecord, SessionPlan,
    assemble, plan_session, session_langs,
};
use wb_models::Manifest;

use crate::boundary::blocking;
use crate::engines::{self, EngineState, Registry};
use crate::recording;
use crate::state::{AppState, lock};

/// Which check is running, and the flag that cancels it.
#[derive(Default)]
pub struct CheckState {
    running: Mutex<Option<std::sync::Arc<AtomicBool>>>,
    /// Why the last check failed (read by the gate hook; the UI gets the `check_failed` event).
    #[cfg_attr(not(feature = "gate-hooks"), allow(dead_code))]
    last_error: Mutex<Option<String>>,
}

/// What the System screen and the Start Session dialog read.
#[derive(Debug, Clone, Serialize)]
pub struct CapabilityView {
    pub capability: Option<Capability>,
    /// Why the saved file could not be used, when it could not.
    pub error: Option<String>,
    pub running: bool,
    /// The saved profile no longer describes this computer; the UI offers a re-run.
    pub fingerprint_changes: Vec<String>,
    pub fingerprint_id: Option<String>,
    /// A development or gate build.
    pub development: bool,
}

fn state(app: &AppHandle) -> tauri::State<'_, AppState> {
    app.state::<AppState>()
}

fn data_dir(app: &AppHandle) -> PathBuf {
    state(app).data_dir.clone()
}

/// The folder with the bundled bench clips: next to the app in an install, in the repository in development.
pub fn locate_bench_dir() -> Option<PathBuf> {
    if cfg!(any(debug_assertions, feature = "gate-hooks"))
        && let Ok(d) = std::env::var("WB_BENCH_DIR")
    {
        return Some(PathBuf::from(d));
    }
    engines::exe_ancestors().into_iter().find_map(|d| {
        [d.join("resources").join("bench"), d.join("bench")]
            .into_iter()
            .find(|c| c.join("bench-en.wav").is_file())
    })
}

fn load_manifest() -> Result<(PathBuf, Manifest, String), String> {
    let dir =
        engines::locate_models_dir().ok_or("No models folder with a manifest.json was found.")?;
    let path = dir.join("manifest.json");
    let manifest = Manifest::load(&path).map_err(|e| e.to_string())?;
    let hash = wb_models::sha256_file(&path).map_err(|e| e.to_string())?;
    Ok((dir, manifest, hash))
}

fn now_iso() -> String {
    // Wall-clock time is shown in IST (AGENTS.md); the file stores the offset explicitly.
    wb_recorder::util::rfc3339(wb_recorder::util::now_ist())
}

/// The fingerprint this computer has now, from the engine registry (no new probe).
fn current_fingerprint(app: &AppHandle) -> Option<wb_capability::Fingerprint> {
    let snap = app.state::<Registry>().snapshot();
    if !snap.engines_done
        || !snap
            .engines
            .iter()
            .any(|e| matches!(e.state, EngineState::Ok))
    {
        return None;
    }
    let (_, _, hash) = load_manifest().ok()?;
    let st = state(app);
    let s = lock(&st.settings).clone();
    let recordings = s.recordings_root();
    let roots = probe::Roots {
        recordings: &recordings,
        backup: None,
    };
    let machine = crate::machine::get();
    let (hw, _) = probe::hardware(machine, &snap.engines, &roots);
    #[allow(unused_mut)]
    let mut fp = probe::fingerprint(
        &hw,
        recording::APP_VERSION,
        &hash,
        machine.emulated.as_deref(),
    );
    // Test hook (development and gate builds): pretend the machine changed.
    #[cfg(any(debug_assertions, feature = "gate-hooks"))]
    if let Ok(v) = std::env::var("WB_TEST_FINGERPRINT_CHANGE") {
        fp.os = format!("{} ({v})", fp.os);
    }
    Some(fp)
}

/// True while a check is running. A recording never starts during one: the check opens the input and runs
/// engines of its own.
pub fn is_running(app: &AppHandle) -> bool {
    lock(&app.state::<CheckState>().running).is_some()
}

/// The message of the last failed check, if it failed (gate hook).
#[cfg(feature = "gate-hooks")]
pub fn last_error(app: &AppHandle) -> Option<String> {
    lock(&app.state::<CheckState>().last_error).clone()
}

pub fn view(app: &AppHandle) -> CapabilityView {
    let running = lock(&app.state::<CheckState>().running).is_some();
    let development = cfg!(any(debug_assertions, feature = "gate-hooks"));
    match store::load(&data_dir(app)) {
        Ok(Some(cap)) => {
            let changes = current_fingerprint(app)
                .map(|now| store::fingerprint_changes(&cap, &now))
                .unwrap_or_default();
            CapabilityView {
                fingerprint_id: Some(cap.profile.fingerprint.id()),
                capability: Some(cap),
                error: None,
                running,
                fingerprint_changes: changes,
                development,
            }
        }
        Ok(None) => CapabilityView {
            capability: None,
            error: None,
            running,
            fingerprint_changes: vec![],
            fingerprint_id: None,
            development,
        },
        Err(e) => CapabilityView {
            capability: None,
            error: Some(e.to_string()),
            running,
            fingerprint_changes: vec![],
            fingerprint_id: None,
            development,
        },
    }
}

#[tauri::command]
pub async fn get_capability(app: AppHandle) -> Result<CapabilityView, String> {
    blocking("get_capability", move || Ok(view(&app))).await
}

/// Starts the check on a thread of its own; progress and the result arrive as app events
/// (`check_progress`, `check_done`, `check_failed`).
#[tauri::command]
pub async fn run_system_check(app: AppHandle, full: bool) -> Result<(), String> {
    blocking("run_system_check", move || {
        start_check(
            &app,
            if full {
                CheckKind::Full
            } else {
                CheckKind::Quick
            },
        )
    })
    .await
}

#[tauri::command]
pub async fn cancel_system_check(app: AppHandle) -> Result<(), String> {
    blocking("cancel_system_check", move || {
        if let Some(flag) = lock(&app.state::<CheckState>().running).as_ref() {
            flag.store(true, Ordering::Relaxed);
        }
        Ok(())
    })
    .await
}

pub fn start_check(app: &AppHandle, kind: CheckKind) -> Result<(), String> {
    if recording::is_recording(app) {
        return Err("The system check does not run while a session is recording.".into());
    }
    let st = state(app);
    let (models_dir, manifest, manifest_hash) = load_manifest()?;
    let engines_dir =
        engines::locate_engines_dir().ok_or("No speech engine was found next to the app.")?;
    let bench_dir = locate_bench_dir()
        .ok_or("The bench clips (resources/bench) were not found next to the app.")?;
    let settings = lock(&st.settings).clone();
    let cancel = std::sync::Arc::new(AtomicBool::new(false));
    {
        let check_state = app.state::<CheckState>();
        let mut running = lock(&check_state.running);
        if running.is_some() {
            return Err("The system check is already running.".into());
        }
        *running = Some(cancel.clone());
    }
    *lock(&app.state::<CheckState>().last_error) = None;
    let env = check::CheckEnv {
        engines_dir,
        models_dir,
        bench_dir,
        work_dir: st.data_dir.join("check"),
        model_cache: st.data_dir.join("verified-models.json"),
        recordings_root: settings.recordings_root(),
        backup_root: settings.backup_root(),
        app_version: recording::APP_VERSION.to_string(),
        machine: crate::machine::get().clone(),
        rankings: rankings::from_manifest(&manifest),
        manifest,
        manifest_hash,
        options: Options::default(),
        started_at: now_iso(),
    };
    let handle = app.clone();
    let spawned = std::thread::Builder::new()
        .name("wb-system-check".into())
        .spawn(move || {
            wb_recorder::sys::lower_current_thread();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_check(&handle, &env, kind, &cancel)
            }));
            *lock(&handle.state::<CheckState>().running) = None;
            let message = match result {
                Ok(Ok(())) => return,
                Ok(Err(m)) => m,
                Err(p) => format!(
                    "The system check stopped unexpectedly: {}",
                    crate::boundary::panic_message(p.as_ref())
                ),
            };
            tracing::error!("system check failed: {message}");
            *lock(&handle.state::<CheckState>().last_error) = Some(message.clone());
            state(&handle)
                .broadcast(serde_json::json!({ "kind": "check_failed", "message": message }));
        });
    if let Err(e) = spawned {
        *lock(&app.state::<CheckState>().running) = None;
        return Err(format!("Could not start the system check: {e}"));
    }
    Ok(())
}

fn run_check(
    app: &AppHandle,
    env: &check::CheckEnv,
    kind: CheckKind,
    cancel: &AtomicBool,
) -> Result<(), String> {
    tracing::info!(?kind, "system check started");
    let emit = |p: check::Progress| {
        state(app).broadcast(serde_json::json!({ "kind": "check_progress", "progress": p }));
    };
    let outcome = check::run(env, kind, cancel, &emit).map_err(|e| e.to_string())?;
    let old = store::load(&data_dir(app)).ok().flatten();
    let overrides = old.map(|c| c.overrides).unwrap_or_default();
    let cap = assemble(outcome.profile, &env.rankings, &env.options, overrides);
    store::save(&data_dir(app), &cap).map_err(|e| e.to_string())?;
    tracing::info!(
        tier = cap.decision.tier.label(),
        seconds = cap.profile.check.duration_s,
        partial = cap.profile.check.partial,
        "system check done; capability.json saved"
    );
    state(app).broadcast(serde_json::json!({ "kind": "check_done" }));
    Ok(())
}

/// Turns an override on or off. The log keeps every change with its time; a change that goes beyond the
/// verdict needs the operator's confirmation.
#[tauri::command]
pub async fn set_override(
    app: AppHandle,
    kind: OverrideKind,
    enabled: bool,
    confirmed: bool,
) -> Result<CapabilityView, String> {
    blocking("set_override", move || {
        let dir = data_dir(&app);
        let mut cap = store::load(&dir)
            .map_err(|e| e.to_string())?
            .ok_or("There is no capability profile yet.")?;
        // Going beyond the verdict: allowing live text the check did not clear, or switching a safeguard
        // (processing while recording, pausing on battery) off.
        let beyond_verdict = match kind {
            OverrideKind::AllowLiveAnyway => enabled,
            _ => !enabled,
        };
        if beyond_verdict && !confirmed {
            return Err("Confirm the warning before turning this on.".into());
        }
        cap.overrides.push(OverrideRecord {
            kind,
            enabled,
            at: now_iso(),
            confirmed,
        });
        store::save(&dir, &cap).map_err(|e| e.to_string())?;
        Ok(view(&app))
    })
    .await
}

/// What the Start Session dialog shows for the ticked languages.
#[tauri::command]
pub async fn session_plan(
    app: AppHandle,
    english: bool,
    hindi_urdu: bool,
    script: HindiUrduScript,
) -> Result<Option<SessionPlan>, String> {
    blocking("session_plan", move || {
        let Some(cap) = store::load(&data_dir(&app)).map_err(|e| e.to_string())? else {
            return Ok(None);
        };
        Ok(Some(plan_session(
            &cap,
            &session_langs(english, hindi_urdu, script),
        )))
    })
    .await
}

/// Refuses live text unless the saved profile clears every language of the session.
pub fn live_allowed(data_dir: &std::path::Path, languages: &[String]) -> Result<(), String> {
    let cap = store::load(data_dir)
        .map_err(|e| e.to_string())?
        .ok_or("Live captions are not offered until the system check has run.")?;
    let langs: Vec<wb_capability::Lang> = languages
        .iter()
        .filter_map(|l| wb_capability::Lang::parse(l))
        .collect();
    let plan = plan_session(&cap, &langs);
    if plan.live.selectable {
        Ok(())
    } else {
        Err(plan
            .live
            .reason
            .unwrap_or_else(|| "Live captions are not offered on this machine.".into()))
    }
}
