//! Starting one engine process and reading its events.
//!
//! The engine runs with `CREATE_NO_WINDOW`, a priority class (below normal for post-processing,
//! normal for live; the recorder thread is raised above both) and in the app's kill-on-close Job
//! Object. Its stdout is read line by line on a thread of its own and turned into events; stderr
//! goes to a log file. Nothing here can block or slow the recorder: it shares no lock with it.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread::JoinHandle;
use std::time::Instant;

use wb_protocol::Event;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
mod job {
    use std::sync::OnceLock;

    use super::Child;
    use crate::win::Job;

    static JOB: OnceLock<Result<Job, String>> = OnceLock::new();

    /// Adds the child to the app-wide job. Returns the reason when that was not possible (the
    /// engine still runs, but would outlive the app).
    pub fn assign(child: &Child) -> Result<(), String> {
        match JOB.get_or_init(Job::new) {
            Ok(j) => j.assign(child),
            Err(e) => Err(e.clone()),
        }
    }
}

#[cfg(not(windows))]
mod job {
    pub fn assign(_child: &super::Child) -> Result<(), String> {
        Ok(())
    }
}

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;
const NORMAL_PRIORITY_CLASS: u32 = 0x0000_0020;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    /// Rolling and night passes, translation: below normal, so the recorder and the UI win.
    BelowNormal,
    /// Live and near-live engines.
    Normal,
}

#[derive(Debug, Clone)]
pub struct SpawnSpec {
    pub exe: PathBuf,
    /// Arguments after the program name, for example `["batch", "--request", "req.json"]`.
    pub args: Vec<String>,
    pub priority: Priority,
    /// Where the engine's stderr goes.
    pub stderr_log: PathBuf,
    /// Extra environment variables.
    pub env: Vec<(String, String)>,
    /// Remove every `WB_*` variable from the child's environment (inherited ones and `env` entries alike).
    /// Release builds of the app turn this on: a stray `WB_EMULATE*`, `WB_ENGINES_DIR` or `WB_MODELS_DIR` in
    /// an operator's environment must not change what an engine believes about the machine or where it
    /// looks for models. Gate and debug builds leave it off because they pass those variables on purpose.
    pub scrub_env: bool,
}

/// True for the variables the app family uses to steer development and gate runs.
pub fn is_wb_variable(name: &str) -> bool {
    // Bytes, not a string slice: a name such as "ab€" has no character boundary at byte 3.
    name.as_bytes().get(..3).is_some_and(|p| p.eq_ignore_ascii_case(b"WB_"))
}

/// One line the engine wrote to stdout.
#[derive(Debug, Clone)]
pub enum Line {
    Event(Box<Event>),
    /// A line that is not a valid event (kept for the log; never fatal).
    Garbled(String),
}

pub struct EngineProcess {
    child: Child,
    pub events: Receiver<Line>,
    reader: Option<JoinHandle<()>>,
    pub started: Instant,
    pub pid: u32,
    /// Why the process is not in the kill-on-close job, if it is not.
    pub job_warning: Option<String>,
}

impl EngineProcess {
    pub fn spawn(spec: &SpawnSpec) -> std::io::Result<Self> {
        if let Some(dir) = spec.stderr_log.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let stderr = File::options()
            .create(true)
            .append(true)
            .open(&spec.stderr_log)?;
        let mut cmd = Command::new(&spec.exe);
        cmd.args(&spec.args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(stderr));
        if spec.scrub_env {
            for (k, _) in std::env::vars_os() {
                if k.to_str().is_some_and(is_wb_variable) {
                    cmd.env_remove(&k);
                }
            }
        }
        for (k, v) in &spec.env {
            if spec.scrub_env && is_wb_variable(k) {
                continue;
            }
            cmd.env(k, v);
        }
        #[cfg(windows)]
        cmd.creation_flags(
            CREATE_NO_WINDOW
                | match spec.priority {
                    Priority::BelowNormal => BELOW_NORMAL_PRIORITY_CLASS,
                    Priority::Normal => NORMAL_PRIORITY_CLASS,
                },
        );
        let mut child = cmd.spawn()?;
        let job_warning = job::assign(&child).err();
        let pid = child.id();
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| std::io::Error::other("no stdout pipe"))?;
        let (tx, rx) = mpsc::channel();
        let reader = std::thread::Builder::new()
            .name(format!("wb-engine-{pid}-stdout"))
            .spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    if line.trim().is_empty() {
                        continue;
                    }
                    let msg = match Event::from_line(&line) {
                        Ok(e) => Line::Event(Box::new(e)),
                        Err(_) => Line::Garbled(line),
                    };
                    if tx.send(msg).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Self {
            child,
            events: rx,
            reader: Some(reader),
            started: Instant::now(),
            pid,
            job_warning,
        })
    }

    /// The exit code if the process has ended.
    pub fn try_exit(&mut self) -> Option<i32> {
        self.child
            .try_wait()
            .ok()
            .flatten()
            .map(|s| s.code().unwrap_or(-1))
    }

    /// Ends the process and waits for it. Safe to call on one that has already ended.
    pub fn kill(&mut self) -> Option<i32> {
        let _ = self.child.kill();
        let code = self.child.wait().ok().map(|s| s.code().unwrap_or(-1));
        if let Some(r) = self.reader.take() {
            let _ = r.join();
        }
        code
    }

    /// Waits for the process to end on its own.
    pub fn wait(&mut self) -> Option<i32> {
        let code = self.child.wait().ok().map(|s| s.code().unwrap_or(-1));
        if let Some(r) = self.reader.take() {
            let _ = r.join();
        }
        code
    }
}

impl Drop for EngineProcess {
    fn drop(&mut self) {
        // A supervisor that goes away takes its engine with it.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The path of the engine for `variant` next to `dir`: `wb-engine-<variant>-x86_64-pc-windows-msvc.exe`.
pub fn sidecar_path(dir: &Path, variant: &str) -> PathBuf {
    dir.join(format!("wb-engine-{variant}-x86_64-pc-windows-msvc.exe"))
}
