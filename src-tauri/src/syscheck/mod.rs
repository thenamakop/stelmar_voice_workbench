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
mod store;
#[cfg(windows)]
mod win;

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use tauri::{AppHandle, Manager};
use wb_capability::{
    Capability, CheckKind, HindiUrduScript, Options, OverrideKind, OverrideRecord, SavedFile,
    SessionPlan, Tier, assemble, plan_session, session_langs,
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
    /// The profile in use. After a fingerprint change it carries safe defaults (record + post-process only).
    pub capability: Option<Capability>,
    /// The saved checks, each with the file it lives in: the complete profile and a partial check kept apart
    /// (a partial or cancelled check never replaces a complete profile).
    pub saved_checks: Vec<SavedCheck>,
    /// Why the saved file could not be used, when it could not.
    pub error: Option<String>,
    pub running: bool,
    /// The saved profile no longer describes this computer; the UI offers a re-run.
    pub fingerprint_changes: Vec<String>,
    pub fingerprint_id: Option<String>,
    /// A development or gate build.
    pub development: bool,
}

/// One saved check, as the System screen lists it.
#[derive(Debug, Clone, Serialize)]
pub struct SavedCheck {
    pub file: SavedFile,
    pub started_at: String,
    pub kind: CheckKind,
    pub tier: Tier,
    pub partial: bool,
    pub duration_s: f64,
    pub fingerprint_id: String,
    /// This is the profile the rest of the app reads.
    pub in_use: bool,
}

fn saved_check(c: &Capability, file: SavedFile, in_use: bool) -> SavedCheck {
    SavedCheck {
        file,
        started_at: c.profile.check.started_at.clone(),
        kind: c.profile.check.kind,
        tier: c.decision.tier,
        partial: c.profile.check.partial,
        duration_s: c.profile.check.duration_s,
        fingerprint_id: c.profile.fingerprint.id(),
        in_use,
    }
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

/// What the Start Session dialog sends: the recorder's own request plus the recording mode. `recording.rs`
/// stays exactly as the Phase 1 recorder gates verified it, so the mode is handled here, in the command
/// layer that calls the recorder.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct SessionRequest {
    #[serde(flatten)]
    pub base: recording::StartRequest,
    /// `record_post` (the default everywhere) or `live_post`; live only where the saved profile clears every
    /// language of the session (AGENTS.md non-negotiable 7).
    #[serde(default)]
    pub mode: Option<String>,
    /// `devanagari` or `nastaliq`: how Hindi and Urdu speech is written (the organiser's choice).
    #[serde(default)]
    pub hindi_urdu_script: Option<String>,
}

#[tauri::command]
pub async fn start_recording(
    app: AppHandle,
    request: SessionRequest,
) -> Result<crate::state::SessionInfo, String> {
    blocking("start_recording", move || start_with_mode(&app, request)).await
}

/// Starts a recording. The recording comes first (AGENTS.md non-negotiable 2): a system check that is
/// running is cancelled at once (its profile is saved as partial, with the cells done so far) and the
/// recorder starts without waiting for anything else.
pub fn start_with_mode(
    app: &AppHandle,
    request: SessionRequest,
) -> Result<crate::state::SessionInfo, String> {
    let languages = request
        .base
        .expected_languages
        .clone()
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| vec!["en".into(), "hi".into()]);
    let mode = match request.mode.as_deref() {
        None | Some("record_post") => "record_post",
        Some("live_post") => {
            live_allowed(
                &data_dir(app),
                &languages,
                current_fingerprint(app).as_ref(),
            )?;
            "live_post"
        }
        Some(other) => return Err(format!("Unknown recording mode '{other}'.")),
    };
    cancel_check_for_recording(app);
    let info = recording::start_blocking(app, request.base)?;
    // The chosen mode goes next to the session's own files. The recorder does not carry it yet.
    let options = serde_json::json!({
        "mode": mode,
        "expected_languages": languages,
        "hindi_urdu_script": request.hindi_urdu_script,
    });
    if let Err(e) = wb_recorder::util::write_json_atomic(
        &PathBuf::from(&info.dir).join("start-options.json"),
        &options,
    ) {
        tracing::warn!("start-options.json was not written: {e}");
    }
    Ok(info)
}

/// Cancels a running system check and waits a few seconds for it to end. The recording starts either way.
fn cancel_check_for_recording(app: &AppHandle) {
    let flag = lock(&app.state::<CheckState>().running).clone();
    let Some(flag) = flag else {
        return;
    };
    tracing::warn!("a recording is starting: cancelling the system check");
    flag.store(true, Ordering::Relaxed);
    let waited = std::time::Instant::now();
    while is_running(app) && waited.elapsed() < std::time::Duration::from_secs(5) {
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    if is_running(app) {
        tracing::warn!(
            "the system check was still stopping after 5 s; the recording starts anyway"
        );
    }
}

/// True while a check is running. Starting a recording cancels it (the recording comes first).
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
    let dir = data_dir(app);
    let now = current_fingerprint(app);
    let main = match store::load(&dir) {
        Ok(m) => m,
        Err(e) => {
            return CapabilityView {
                capability: None,
                saved_checks: vec![],
                error: Some(e.to_string()),
                running,
                fingerprint_changes: vec![],
                fingerprint_id: None,
                development,
            };
        }
    };
    let partial = store::load_partial(&dir);
    let mut saved_checks = Vec::new();
    let used = wb_capability::choose_in_use(main.clone(), partial.clone(), now.as_ref());
    let used_file = used.as_ref().map(|u| u.file);
    if let Some(m) = &main {
        saved_checks.push(saved_check(
            m,
            SavedFile::Main,
            used_file == Some(SavedFile::Main),
        ));
    }
    if let Some(p) = &partial {
        saved_checks.push(saved_check(
            p,
            SavedFile::Partial,
            used_file == Some(SavedFile::Partial),
        ));
    }
    match used {
        Some(u) => CapabilityView {
            fingerprint_id: Some(u.cap.profile.fingerprint.id()),
            capability: Some(u.cap),
            saved_checks,
            error: None,
            running,
            fingerprint_changes: u.changes,
            development,
        },
        None => CapabilityView {
            capability: None,
            saved_checks,
            error: None,
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
    let outcome = match check::run(env, kind, cancel, &emit) {
        Ok(o) => o,
        // Cancelled before the first step finished: nothing was measured and the saved profile stays.
        Err(check::CheckError::Cancelled) => {
            tracing::info!("system check cancelled before it measured anything");
            state(app).broadcast(serde_json::json!({ "kind": "check_done" }));
            return Ok(());
        }
        Err(e) => return Err(e.to_string()),
    };
    if outcome.cancelled && outcome.profile.measurements.is_empty() {
        tracing::info!(
            "system check cancelled before it measured anything; the saved profile stays"
        );
        state(app).broadcast(serde_json::json!({ "kind": "check_done" }));
        return Ok(());
    }
    // The operator's overrides were confirmed for one state of the machine: they carry to the new profile only
    // while the fingerprint is the same.
    let old = store::load(&data_dir(app)).ok().flatten();
    let overrides = old
        .filter(|c| c.profile.fingerprint == outcome.profile.fingerprint)
        .map(|c| c.overrides)
        .unwrap_or_default();
    let cap = assemble(outcome.profile, &env.rankings, &env.options, overrides);
    let file = store::save_routed(&data_dir(app), &cap).map_err(|e| e.to_string())?;
    tracing::info!(
        file = ?file,
        tier = cap.decision.tier.label(),
        seconds = cap.profile.check.duration_s,
        partial = cap.profile.check.partial,
        "system check done; profile saved"
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
        // Overrides belong to the profile in use as it fits this computer: after a fingerprint change none can
        // be set until a new check has run.
        let used = store::in_use(&dir, current_fingerprint(&app).as_ref())
            .map_err(|e| e.to_string())?
            .ok_or("There is no capability profile yet.")?;
        if !used.changes.is_empty() {
            return Err(
                "This computer has changed since the last check, so no setting can be changed until the check has run again."
                    .into(),
            );
        }
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
        // Processing while the recorder runs is allowed only for a setup whose recorder safety test passed: a
        // setup without a passed test cannot be switched on by an override.
        if kind == OverrideKind::ProcessWhileRecording
            && enabled
            && !cap.decision.process_while_recording
        {
            return Err(
                "The recorder safety test has not passed for this setup, so processing while recording stays off. Run the system check again."
                    .into(),
            );
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
        let Some(used) = store::in_use(&data_dir(&app), current_fingerprint(&app).as_ref())
            .map_err(|e| e.to_string())?
        else {
            return Ok(None);
        };
        Ok(Some(plan_session(
            &used.cap,
            &session_langs(english, hindi_urdu, script),
        )))
    })
    .await
}

/// Refuses live text unless the saved profile clears every language of the session.
/// A profile that no longer fits this computer (its fingerprint changed) clears nothing: it carries safe
/// defaults, which refuse live text.
pub fn live_allowed(
    data_dir: &std::path::Path,
    languages: &[String],
    now: Option<&wb_capability::Fingerprint>,
) -> Result<(), String> {
    let cap = store::in_use(data_dir, now)
        .map_err(|e| e.to_string())?
        .ok_or("Live captions are not offered until the system check has run.")?
        .cap;
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wb-mod-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn tier_g() -> Capability {
        let text = include_str!("../../../crates/wb-capability/fixtures/tier-g.json");
        wb_capability::parse_capability(text).unwrap()
    }

    #[test]
    fn live_text_is_refused_after_a_fingerprint_change_until_a_new_check_has_run() {
        let d = dir("live-stale");
        let cap = tier_g();
        store::save(&d, &cap).unwrap();
        let langs = vec!["en".to_string()];
        let same = cap.profile.fingerprint.clone();
        assert!(
            live_allowed(&d, &langs, Some(&same)).is_ok(),
            "fits: English live is cleared"
        );
        let unknown = live_allowed(&d, &langs, None).unwrap_err();
        assert!(
            unknown.contains("not been identified"),
            "no fingerprint yet: live text is refused, {unknown}"
        );
        let mut changed = same;
        changed.os = format!("{} (updated)", changed.os);
        let refused = live_allowed(&d, &langs, Some(&changed)).unwrap_err();
        assert!(refused.contains("changed"), "{refused}");
        // A new complete check saved on the changed computer clears it again.
        let mut fresh = cap;
        fresh.profile.fingerprint = changed.clone();
        store::save_routed(&d, &fresh).unwrap();
        assert!(live_allowed(&d, &langs, Some(&changed)).is_ok());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_stale_profile_gives_the_dialog_no_live_option_but_keeps_post_processing() {
        let d = dir("plan-stale");
        let cap = tier_g();
        store::save(&d, &cap).unwrap();
        let mut changed = cap.profile.fingerprint.clone();
        changed.os = format!("{} (updated)", changed.os);
        let used = store::in_use(&d, Some(&changed)).unwrap().unwrap();
        assert!(!used.changes.is_empty());
        let plan = plan_session(&used.cap, &[wb_capability::Lang::En]);
        assert!(!plan.live.selectable);
        assert!(!plan.record_only);
        assert!(!used.cap.decision.process_while_recording);
        std::fs::remove_dir_all(&d).unwrap();
    }
}
