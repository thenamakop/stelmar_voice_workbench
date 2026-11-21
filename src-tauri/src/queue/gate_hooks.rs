//! Test hooks of the queue for gate builds (`--profile gate`, feature `gate-hooks`); none of this is compiled into
//! an installer build, and the offline check scans the release binary for `WB_GATE_` to prove it.
//!
//! | Variable | Effect |
//! | --- | --- |
//! | `WB_GATE_DAY_PLAN=<json file>` | The app starts and stops the day's sessions by itself (see [`DayPlan`]), without quitting between them, then waits for `WB_GATE_DAY_STOP_FILE` and writes the queue report |
//! | `WB_GATE_DAY_STOP_FILE=<path>` | When this file appears the day plan ends: the report is written and the app exits |
//! | `WB_GATE_CLOCK_OFFSET_S=<n>` | Moves the scheduler's clock by n seconds (so a short run stands for the day: the first session maps to 10:00 IST); read in `queue::clock_ms` |
//! | `WB_GATE_FORCE_HYBRID=1` | The queue works as if the profile had chosen a hybrid (processor and graphics engines at once). For the kill test only: neither emulation profile chose a hybrid, so the report labels the run as forced |
//! | `WB_GATE_QUEUE_LOG=<path>` | Every line of the scheduler log is also appended to this file, with the scheduler's IST time |
//! | `WB_GATE_NIGHT_WAIT_S=<n>` | The night pass waits n seconds (not 30 minutes) after the last session ended; for the smoke run only (it also turns off the 20:00 IST rule for a day the programme does not list) |
//!
//! The day plan may also carry `actions` (see [`PlanAction`]): the operator's pause and resume, a language lock
//! and an import of a copy of a finished session, each fired at a time relative to a session's start or stop. They
//! run the same functions the commands run, and each one's outcome goes into the report.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Deserialize;
use tauri::{AppHandle, Manager};

use crate::recording;
use crate::state::{AppState, lock};

fn var(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.trim().is_empty())
}

/// `WB_GATE_FORCE_HYBRID` is set.
pub fn force_hybrid() -> bool {
    var("WB_GATE_FORCE_HYBRID").is_some_and(|v| v != "0")
}

/// How much earlier than 30 minutes after the last session the night pass may start (`WB_GATE_NIGHT_WAIT_S`), in
/// milliseconds: the queue adds it to the clock it passes to the 30-minute rule. Zero when the variable is not set.
pub fn night_wait_shift_ms() -> u64 {
    var("WB_GATE_NIGHT_WAIT_S")
        .and_then(|v| v.parse::<u64>().ok())
        .map_or(0, |wait| {
            30u64.saturating_mul(60).saturating_sub(wait) * 1_000
        })
}

/// The programme's entry, as the app itself reads it, for the day the scheduler's clock shows now (with
/// `WB_GATE_CLOCK_OFFSET_S` applied). The day simulation's pre-flight check (`WB_GATE_CHECK=programme`) refuses to
/// start unless this lists the four sessions of the day.
pub fn programme_day_report() -> serde_json::Value {
    let now = super::clock_ms();
    let date = wb_queue::night::ist_date(now);
    match wb_queue::night::programme_day(super::PROGRAMME_JSON, now) {
        Some(day) => serde_json::json!({
            "date": date,
            "sessions": day.titles.len(),
            "titles": day.titles,
            "last_end_ist": wb_queue::estimate::format_ist_hhmm(day.last_end_ms),
        }),
        None => serde_json::json!({
            "date": date,
            "sessions": 0,
            "titles": [],
            "last_end_ist": null,
        }),
    }
}

/// Appends a scheduler log line to `WB_GATE_QUEUE_LOG`.
pub fn queue_log(text: &str) {
    let Some(path) = var("WB_GATE_QUEUE_LOG") else {
        return;
    };
    let line = format!(
        "{}\t{}\n",
        wb_queue::estimate::format_ist_hhmm(super::clock_ms()),
        text.replace(['\r', '\n'], " ")
    );
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = f.write_all(line.as_bytes());
    }
}

/// One session of the day plan: it starts `at_s` seconds after the plan starts (or when the one before it has stopped) and
/// ends when its `stop_file` appears, or `duration_s` after it started when that is not set or never appears.
#[derive(Debug, Clone, Deserialize)]
pub struct PlanSession {
    pub title: String,
    #[serde(default)]
    pub hall: Option<String>,
    pub at_s: u64,
    pub duration_s: u64,
    /// `en`, `hi`, `ur`; default English and Hindi.
    #[serde(default)]
    pub languages: Option<Vec<String>>,
    /// `devanagari` or `nastaliq`.
    #[serde(default)]
    pub script: Option<String>,
    /// Input device whose name contains this text (the VB-CABLE endpoint of the rig).
    #[serde(default)]
    pub device: Option<String>,
    /// The session ends when this file appears (the day script writes it 15 s after its player has finished playing);
    /// `duration_s` is then only the safety net for a player that died (20 Nov 2026, see `day_stop`).
    #[serde(default)]
    pub stop_file: Option<PathBuf>,
}

/// One thing the plan does to the queue, `after_s` seconds after the named session started (`on` = `start`, the
/// default) or stopped (`on` = `stop`).
#[derive(Debug, Clone, Deserialize)]
pub struct PlanAction {
    /// Title of the session the time is relative to.
    pub session: String,
    #[serde(default)]
    pub on: Option<String>,
    #[serde(default)]
    pub after_s: u64,
    /// `lock_language` (needs `lang`: a language, or null/absent to return to detection, at the recorder's current
    /// audio time), `queue_pause`, `queue_resume`, or `import_session` (needs `from`: the title of a finished
    /// session; its folder is copied beside the recordings and the copy is imported).
    #[serde(rename = "do")]
    pub kind: String,
    #[serde(default)]
    pub lang: Option<String>,
    #[serde(default)]
    pub from: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DayPlan {
    pub sessions: Vec<PlanSession>,
    #[serde(default)]
    pub actions: Vec<PlanAction>,
}

/// When each session of the plan started and stopped, and where its folder is.
#[derive(Default)]
struct Events {
    started: BTreeMap<String, Instant>,
    stopped: BTreeMap<String, Instant>,
    dirs: BTreeMap<String, PathBuf>,
}

/// Starts the day-plan thread if `WB_GATE_DAY_PLAN` is set.
pub fn start_day_plan(app: &AppHandle) {
    let Some(path) = var("WB_GATE_DAY_PLAN") else {
        return;
    };
    let app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("wb-gate-day".into())
        .spawn(move || {
            if let Err(e) = run_day(&app, &PathBuf::from(path)) {
                tracing::error!("gate day plan: {e}");
                queue_log(&format!("DAY PLAN ERROR: {e}"));
            }
        });
    if let Err(e) = spawned {
        tracing::error!("gate day plan thread: {e}");
    }
}

fn run_day(app: &AppHandle, path: &std::path::Path) -> Result<(), String> {
    let plan: DayPlan = serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let st = app.state::<AppState>();
    let wait = Instant::now();
    while !st.recovery_done.load(std::sync::atomic::Ordering::Acquire) {
        if wait.elapsed() > Duration::from_secs(300) {
            return Err("recovery scan did not finish".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    // The queue starts in `setup`; give its monitor a moment to bring up the planner.
    let t0 = Instant::now();
    let events: Arc<Mutex<Events>> = Arc::new(Mutex::new(Events::default()));
    let outcomes: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
    if !plan.actions.is_empty() {
        let (app2, events2, outcomes2, actions) = (
            app.clone(),
            events.clone(),
            outcomes.clone(),
            plan.actions.clone(),
        );
        let spawned = std::thread::Builder::new()
            .name("wb-gate-actions".into())
            .spawn(move || run_actions(&app2, &events2, &outcomes2, actions));
        if let Err(e) = spawned {
            return Err(format!("the plan's action thread did not start: {e}"));
        }
    }
    let mut done: Vec<serde_json::Value> = Vec::new();
    for s in &plan.sessions {
        let due = Duration::from_secs(s.at_s);
        while t0.elapsed() < due {
            std::thread::sleep(Duration::from_millis(200));
        }
        let device_id = match &s.device {
            Some(part) => Some(crate::gate::device_id(part)?),
            None => None,
        };
        let request = crate::syscheck::SessionRequest {
            base: recording::StartRequest {
                title: s.title.clone(),
                hall: s.hall.clone(),
                expected_languages: s.languages.clone(),
                device_id,
                ..recording::StartRequest::default()
            },
            mode: Some("record_post".into()),
            hindi_urdu_script: s.script.clone(),
        };
        let started_ist = wb_recorder::util::rfc3339(wb_recorder::util::now_ist());
        let info = crate::syscheck::start_with_mode(app, request)?;
        queue_log(&format!("DAY: started {:?} in {}", s.title, info.dir));
        {
            let mut ev = plock(&events);
            ev.started.insert(s.title.clone(), Instant::now());
            ev.dirs.insert(s.title.clone(), PathBuf::from(&info.dir));
        }
        let end = Instant::now() + Duration::from_secs(s.duration_s);
        let reason =
            super::day_stop::wait_for_stop(s.stop_file.as_deref(), end, Duration::from_millis(500));
        if s.stop_file.is_some() && reason == super::day_stop::StopReason::Timer {
            queue_log(&format!(
                "DAY: {:?} was stopped by the safety timer: no stop file within {} s",
                s.title, s.duration_s
            ));
        }
        let stopped = recording::stop_blocking(app)?;
        let stopped_ist = wb_recorder::util::rfc3339(wb_recorder::util::now_ist());
        queue_log(&format!("DAY: stopped {:?}", s.title));
        plock(&events)
            .stopped
            .insert(s.title.clone(), Instant::now());
        done.push(serde_json::json!({
            "title": s.title,
            "dir": info.dir,
            "started_ist": started_ist,
            "stopped_ist": stopped_ist,
            "stopped": stopped,
        }));
    }
    queue_log("DAY: every session of the plan was recorded; waiting for the stop file");
    let stop_file = var("WB_GATE_DAY_STOP_FILE").map(PathBuf::from);
    loop {
        if stop_file.as_ref().is_some_and(|p| p.exists()) {
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    let report = serde_json::json!({
        "sessions": done,
        "forced_hybrid": force_hybrid(),
        "actions": plock(&outcomes).clone(),
        "queue": super::report_json(app),
        "snapshot": lock(&st.snapshot).clone(),
    });
    if let Some(p) = var("WB_GATE_REPORT") {
        let _ = wb_recorder::util::write_json_atomic(&PathBuf::from(p), &report);
    }
    queue_log("DAY: report written; exiting");
    app.exit(0);
    Ok(())
}

fn plock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Fires each action when its reference moment has come. Every outcome (done, or the error the command gave) is
/// recorded; a failing action never stops the plan.
fn run_actions(
    app: &AppHandle,
    events: &Mutex<Events>,
    outcomes: &Mutex<Vec<serde_json::Value>>,
    actions: Vec<PlanAction>,
) {
    let mut pending: Vec<PlanAction> = actions;
    while !pending.is_empty() {
        std::thread::sleep(Duration::from_millis(250));
        let due: Vec<usize> = {
            let ev = plock(events);
            pending
                .iter()
                .enumerate()
                .filter(|(_, a)| {
                    let table = if a.on.as_deref() == Some("stop") {
                        &ev.stopped
                    } else {
                        &ev.started
                    };
                    table
                        .get(&a.session)
                        .is_some_and(|t| t.elapsed() >= Duration::from_secs(a.after_s))
                })
                .map(|(i, _)| i)
                .collect()
        };
        for i in due.into_iter().rev() {
            let action = pending.remove(i);
            let (result, detail) = fire(app, events, &action);
            let at = wb_recorder::util::rfc3339(wb_recorder::util::now_ist());
            queue_log(&format!(
                "ACTION {} on {:?} -> {}{}",
                action.kind,
                action.session,
                match &result {
                    Ok(()) => "ok".to_owned(),
                    Err(e) => format!("ERROR {e}"),
                },
                detail
                    .as_ref()
                    .map(|d| format!(" ({d})"))
                    .unwrap_or_default()
            ));
            plock(outcomes).push(serde_json::json!({
                "do": action.kind,
                "session": action.session,
                "lang": action.lang,
                "at_ist": at,
                "ok": result.is_ok(),
                "error": result.err(),
                "detail": detail,
            }));
        }
    }
}

fn fire(
    app: &AppHandle,
    events: &Mutex<Events>,
    action: &PlanAction,
) -> (Result<(), String>, Option<String>) {
    match action.kind.as_str() {
        "queue_pause" => (super::pause_now(app, None), None),
        "queue_resume" => (super::resume_now(app, None), None),
        "lock_language" => {
            let st = app.state::<AppState>();
            let audio_ms = lock(&st.snapshot)
                .recorder
                .as_ref()
                .map_or(0, |r| r.audio_ms);
            (
                super::lock_language_now(app, action.lang.as_deref(), audio_ms),
                Some(format!("lang {:?} at audio {audio_ms} ms", action.lang)),
            )
        }
        "import_session" => {
            let Some(from) = action.from.as_deref() else {
                return (Err("import_session needs `from`".into()), None);
            };
            let Some(dir) = plock(events).dirs.get(from).cloned() else {
                return (
                    Err(format!("no session titled {from:?} was recorded")),
                    None,
                );
            };
            // A copy under another name stands for a folder carried over from another computer.
            let copy_root = std::env::var("WB_GATE_ROOT")
                .map(PathBuf::from)
                .unwrap_or_else(|_| std::env::temp_dir())
                .join("import-source");
            let name = format!(
                "{}-copy",
                dir.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("session")
            );
            let copy = copy_root.join(&name);
            if let Err(e) = copy_session(&dir, &copy) {
                return (Err(format!("copying {}: {e}", dir.display())), None);
            }
            match super::import_now(app, &copy.to_string_lossy()) {
                Ok(imported) => (Ok(()), Some(format!("imported as {imported}"))),
                Err(e) => (Err(e), Some(format!("source {}", copy.display()))),
            }
        }
        other => (Err(format!("unknown action {other:?}")), None),
    }
}

/// Copies a finished session's recording (`session.json`, `parts.json`, `audio/`, `start-options.json`) and leaves
/// out what the queue made (`work/`, the transcript files), as a folder from another computer would arrive.
fn copy_session(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    if to.exists() {
        std::fs::remove_dir_all(to)?;
    }
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_s = name.to_string_lossy();
        if name_s == "work"
            || name_s.starts_with("transcript.db")
            || name_s.starts_with("post-")
            || name_s == "language-log.jsonl"
        {
            continue;
        }
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            copy_dir(&path, &to.join(&name))?;
        } else {
            std::fs::copy(&path, to.join(&name))?;
        }
    }
    Ok(())
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &to.join(entry.file_name()))?;
        } else {
            std::fs::copy(entry.path(), to.join(entry.file_name()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn a_day_plan_reads_with_its_defaults() {
        let plan: DayPlan = serde_json::from_str(
            r#"{"sessions":[{"title":"Inaugural","at_s":0,"duration_s":5400},
                {"title":"Panel","hall":"Hall B","at_s":21600,"duration_s":5400,
                 "languages":["en","hi","ur"],"script":"nastaliq","device":"CABLE"}]}"#,
        )
        .unwrap();
        assert_eq!(plan.sessions.len(), 2);
        assert!(plan.sessions[0].languages.is_none());
        assert_eq!(plan.sessions[1].script.as_deref(), Some("nastaliq"));
        assert!(plan.actions.is_empty());
    }

    #[test]
    fn a_day_plan_reads_its_actions() {
        let plan: DayPlan = serde_json::from_str(
            r#"{"sessions":[{"title":"A","at_s":0,"duration_s":60}],
                "actions":[{"session":"A","after_s":30,"do":"lock_language","lang":"hi"},
                           {"session":"A","on":"stop","do":"import_session","from":"A"}]}"#,
        )
        .unwrap();
        assert_eq!(plan.actions.len(), 2);
        assert_eq!(plan.actions[0].kind, "lock_language");
        assert_eq!(plan.actions[0].on, None);
        assert_eq!(plan.actions[1].on.as_deref(), Some("stop"));
        assert_eq!(plan.actions[1].after_s, 0);
    }

    #[test]
    fn a_copied_session_leaves_out_what_the_queue_made() {
        let tmp = std::env::temp_dir().join(format!("wb-copy-{}", std::process::id()));
        let from = tmp.join("from");
        std::fs::create_dir_all(from.join("audio")).unwrap();
        std::fs::create_dir_all(from.join("work/post-1-unit-0000")).unwrap();
        for f in [
            "session.json",
            "parts.json",
            "transcript.db",
            "post-1.jsonl",
            "language-log.jsonl",
            "audio/part-0001.wav",
            "work/post-1-unit-0000/engine.log",
        ] {
            std::fs::write(from.join(f), b"x").unwrap();
        }
        let to = tmp.join("to");
        copy_session(&from, &to).unwrap();
        assert!(to.join("session.json").exists());
        assert!(to.join("parts.json").exists());
        assert!(to.join("audio/part-0001.wav").exists());
        assert!(!to.join("work").exists());
        assert!(!to.join("transcript.db").exists());
        assert!(!to.join("post-1.jsonl").exists());
        assert!(!to.join("language-log.jsonl").exists());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
