//! Gate hooks: compiled only with the `gate-hooks` feature, which build.rs refuses in the release
//! profile, so no installer contains them. They let the gate scripts drive the real app without
//! clicking in the WebView:
//!
//! | Variable | Effect |
//! | --- | --- |
//! | `WB_GATE_ROOT`, `WB_GATE_BACKUP` | Recordings and backup roots for this run (not saved) |
//! | `WB_GATE_START=<title>` | Start a session with this title at launch |
//! | `WB_GATE_DEVICE=<name part>` | Input device whose name contains this text |
//! | `WB_GATE_RECOVERY=resume\|close` | Resume the newest session recovery found (closing older ones), or close them all |
//! | `WB_GATE_STOP_FILE=<path>` | When this file appears: Stop, write the report, exit |
//! | `WB_GATE_REPORT=<path>` | Where the Stop report goes (JSON) |
//! | `WB_GATE_STATUS_FILE=<path>` | Recorder and mirror status, rewritten every 5 seconds |
//! | `WB_GATE_PANIC_AT_S=<n>` | After n seconds, panic once inside the command boundary and once on a plain thread |
//! | `WB_GATE_RECORDER_PANIC_AT_S=<n>` | After n seconds, panic once inside the recorder thread itself |
//! | `WB_GATE_PART_SECONDS=<n>` | Part length for this run (kill tests aim kills at part rollovers) |

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager};
use wb_recorder::util::write_json_atomic;

use crate::recording::{self, RecoverAction, StartRequest};
use crate::state::{AppState, lock};

/// Part length override for this run (`WB_GATE_PART_SECONDS`), at least 5 seconds.
pub fn part_seconds() -> Option<u32> {
    var("WB_GATE_PART_SECONDS")
        .and_then(|s| s.parse::<u32>().ok())
        .map(|s| s.max(5))
}

fn var(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.trim().is_empty())
}

fn active() -> bool {
    ["WB_GATE_START", "WB_GATE_RECOVERY", "WB_GATE_STOP_FILE"]
        .iter()
        .any(|k| var(k).is_some())
}

/// Applies the run's folders before the start-up recovery scan reads them.
pub fn apply_overrides(app: &AppHandle) {
    if !active() {
        return;
    }
    tracing::warn!("gate hooks active (development build, gate-hooks feature)");
    let st = app.state::<AppState>();
    let mut s = lock(&st.settings);
    if let Some(root) = var("WB_GATE_ROOT") {
        s.recordings_root = root;
    }
    if let Some(b) = var("WB_GATE_BACKUP") {
        s.backup_root = Some(b);
    }
}

pub fn start(app: &AppHandle) {
    if !active() {
        return;
    }
    let app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("wb-gate".into())
        .spawn(move || {
            if let Err(e) = run(&app) {
                tracing::error!("gate hook failed: {e}");
                write_report(&serde_json::json!({ "error": e }));
                app.exit(3);
            }
        });
    if let Err(e) = spawned {
        tracing::error!("gate hooks could not start: {e}");
    }
}

fn write_report(value: &serde_json::Value) {
    if let Some(path) = var("WB_GATE_REPORT") {
        if let Err(e) = write_json_atomic(&PathBuf::from(&path), value) {
            tracing::error!("gate report {path}: {e}");
        }
    }
}

fn device_id(part: &str) -> Result<String, String> {
    let needle = part.to_lowercase();
    let inputs = wb_recorder::capture::list_inputs().map_err(|e| e.to_string())?;
    inputs
        .into_iter()
        .find(|d| d.name.to_lowercase().contains(&needle))
        .map(|d| d.id)
        .ok_or_else(|| format!("no input device whose name contains {part:?}"))
}

fn run(app: &AppHandle) -> Result<(), String> {
    let st = app.state::<AppState>();
    let wait = Instant::now();
    while !st.recovery_done.load(Ordering::Acquire) {
        if wait.elapsed() > Duration::from_secs(300) {
            return Err("recovery scan did not finish".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut recovery_report = serde_json::Value::Null;
    if let Some(action) = var("WB_GATE_RECOVERY") {
        let mut found = lock(&st.recovered).clone();
        found.sort_by(|a, b| a.opened_at.cmp(&b.opened_at));
        tracing::warn!(sessions = found.len(), action, "gate: recovery offered");
        recovery_report = serde_json::to_value(&found).unwrap_or_default();
        let newest = found.pop();
        for older in &found {
            recording::recover_blocking(app, &older.dir, RecoverAction::Close)?;
        }
        if let Some(r) = newest {
            let act = if action == "resume" {
                RecoverAction::Resume
            } else {
                RecoverAction::Close
            };
            recording::recover_blocking(app, &r.dir, act)?;
        }
    }
    if let Some(title) = var("WB_GATE_START") {
        if lock(&st.session).is_none() {
            let device = match var("WB_GATE_DEVICE") {
                Some(part) => Some(device_id(&part)?),
                None => None,
            };
            let info = recording::start_blocking(
                app,
                StartRequest {
                    title,
                    device_id: device,
                    ..StartRequest::default()
                },
            )?;
            tracing::warn!(dir = info.dir, "gate: session started");
        }
    }
    let stop_file = var("WB_GATE_STOP_FILE").map(PathBuf::from);
    let status_file = var("WB_GATE_STATUS_FILE").map(PathBuf::from);
    let panic_at = var("WB_GATE_PANIC_AT_S")
        .and_then(|s| s.parse::<u64>().ok())
        .map(Duration::from_secs);
    let started = Instant::now();
    let mut last_status = Instant::now() - Duration::from_secs(10);
    let mut panicked = false;
    let mut panic_report = serde_json::Value::Null;
    let recorder_panic_at = var("WB_GATE_RECORDER_PANIC_AT_S")
        .and_then(|s| s.parse::<u64>().ok())
        .map(Duration::from_secs);
    let mut recorder_panicked = false;
    let mut recorder_panic_report = serde_json::Value::Null;
    loop {
        if let (Some(path), true) = (
            &status_file,
            last_status.elapsed() >= Duration::from_secs(1),
        ) {
            last_status = Instant::now();
            let snap = lock(&st.snapshot).clone();
            let _ = write_json_atomic(
                path,
                &serde_json::json!({
                    "written_at": wb_recorder::util::rfc3339(wb_recorder::util::now_ist()),
                    "recorder": snap.recorder,
                    "mirror": snap.mirror,
                    "parts": snap.parts.len(),
                    "display_restarts": snap.display_restarts,
                }),
            );
        }
        if let (Some(at), false) = (panic_at, panicked) {
            if started.elapsed() >= at {
                panicked = true;
                panic_report = inject_panics();
            }
        }
        if let (Some(at), false) = (recorder_panic_at, recorder_panicked) {
            if started.elapsed() >= at {
                recorder_panicked = true;
                let sent = lock(&st.session)
                    .as_ref()
                    .map(|s| {
                        s.inject_recorder_panic(wb_recorder::recorder::PanicPoint::BeforeWrite)
                            .is_ok()
                    })
                    .unwrap_or(false);
                let when = wb_recorder::util::rfc3339(wb_recorder::util::now_ist());
                tracing::warn!(at = when, sent, "gate: recorder panic injected");
                recorder_panic_report = serde_json::json!({ "at": when, "sent": sent });
            }
        }
        if stop_file.as_ref().is_some_and(|p| p.exists()) {
            let stopped = if lock(&st.session).is_some() {
                Some(recording::stop_blocking(app)?)
            } else {
                None
            };
            let snap = lock(&st.snapshot).clone();
            write_report(&serde_json::json!({
                "stopped": stopped,
                "recovery": recovery_report,
                "panic_injection": panic_report,
                "recorder_panic_injection": recorder_panic_report,
                "display_restarts": snap.display_restarts,
            }));
            tracing::warn!("gate: stop file seen; session stopped; exiting");
            app.exit(0);
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// One panic through the command boundary (as a buggy command would) and one on a plain thread.
fn inject_panics() -> serde_json::Value {
    let boundary = tauri::async_runtime::block_on(crate::boundary::blocking(
        "gate_panic",
        || -> Result<(), String> { panic!("gate hook: simulated panic in a command handler") },
    ));
    let thread = std::thread::Builder::new()
        .name("wb-gate-panic".into())
        .spawn(|| panic!("gate hook: simulated panic on a plain thread"))
        .map(|h| h.join().is_err());
    let at = wb_recorder::util::rfc3339(wb_recorder::util::now_ist());
    tracing::warn!(at, ?boundary, ?thread, "gate: panics injected");
    serde_json::json!({
        "at": at,
        "boundary_result": format!("{boundary:?}"),
        "thread_panicked": thread.unwrap_or(false),
    })
}
