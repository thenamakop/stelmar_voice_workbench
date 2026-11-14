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

use std::io::Write;
use std::path::PathBuf;
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

/// One session of the day plan: it starts `at_s` seconds after the plan starts and is stopped `duration_s` later.
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
}

#[derive(Debug, Clone, Deserialize)]
pub struct DayPlan {
    pub sessions: Vec<PlanSession>,
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
        let end = Instant::now() + Duration::from_secs(s.duration_s);
        while Instant::now() < end {
            std::thread::sleep(Duration::from_millis(500));
        }
        let stopped = recording::stop_blocking(app)?;
        let stopped_ist = wb_recorder::util::rfc3339(wb_recorder::util::now_ist());
        queue_log(&format!("DAY: stopped {:?}", s.title));
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
    }
}
