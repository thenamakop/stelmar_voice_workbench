//! Starting one engine process and reading its events.
//!
//! The engine runs with `CREATE_NO_WINDOW`, a priority class (below normal for post-processing,
//! normal for live; the recorder thread is raised above both) and in the app's kill-on-close Job
//! Object. Its stdout is read line by line on a thread of its own and turned into events; stderr
//! goes to a log file. Nothing here can block or slow the recorder: it shares no lock with it.

use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread::JoinHandle;
use std::time::Instant;

use wb_protocol::Event;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// The job of one engine. Each engine has its own, so each can have its own memory limit; closing the handle
/// (the app exits or crashes, or the engine is dropped) ends the engine.
#[cfg(windows)]
mod job {
    use super::Child;
    use crate::win::{self, Job};

    pub struct EngineJob(Result<Job, String>);

    impl EngineJob {
        pub fn new(memory_limit_bytes: Option<u64>) -> Self {
            EngineJob(Job::with_memory_limit(memory_limit_bytes))
        }

        /// Puts the child in the job. The reason when that was not possible (the engine still runs, but
        /// would outlive the app).
        pub fn assign(&self, child: &Child) -> Result<(), String> {
            match &self.0 {
                Ok(j) => j.assign(child),
                Err(e) => Err(e.clone()),
            }
        }

        pub fn peak_memory(&self) -> Option<u64> {
            self.0.as_ref().ok().and_then(Job::peak_process_memory)
        }

        pub fn resume(pid: u32) -> Result<(), String> {
            win::resume_process(pid).map(|_| ())
        }
    }
}

#[cfg(not(windows))]
mod job {
    use super::Child;

    pub struct EngineJob;

    impl EngineJob {
        pub fn new(_memory_limit_bytes: Option<u64>) -> Self {
            EngineJob
        }
        pub fn assign(&self, _child: &Child) -> Result<(), String> {
            Ok(())
        }
        pub fn peak_memory(&self) -> Option<u64> {
            None
        }
        pub fn resume(_pid: u32) -> Result<(), String> {
            Ok(())
        }
    }
}

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// The process is created with its main thread suspended; it is resumed after it is in its job.
const CREATE_SUSPENDED: u32 = 0x0000_0004;
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
    name.as_bytes()
        .get(..3)
        .is_some_and(|p| p.eq_ignore_ascii_case(b"WB_"))
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
    /// The per-process commit limit of its job, if one was set.
    pub memory_limit_bytes: Option<u64>,
    job: job::EngineJob,
}

impl EngineProcess {
    pub fn spawn(spec: &SpawnSpec) -> std::io::Result<Self> {
        Self::spawn_limited(spec, None)
    }

    /// Starts the engine suspended, puts it in a job of its own (with a per-process commit limit when
    /// `memory_limit_bytes` is given) and only then lets it run, so none of it runs outside the job.
    pub fn spawn_limited(
        spec: &SpawnSpec,
        memory_limit_bytes: Option<u64>,
    ) -> std::io::Result<Self> {
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
                | CREATE_SUSPENDED
                | match spec.priority {
                    Priority::BelowNormal => BELOW_NORMAL_PRIORITY_CLASS,
                    Priority::Normal => NORMAL_PRIORITY_CLASS,
                },
        );
        let mut child = cmd.spawn()?;
        let pid = child.id();
        let engine_job = job::EngineJob::new(memory_limit_bytes);
        let mut job_warning = engine_job.assign(&child).err();
        // Whatever happened to the job, the engine must run: a suspended process would hang the batch.
        if let Err(e) = job::EngineJob::resume(pid) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::other(e));
        }
        if job_warning.is_some() && memory_limit_bytes.is_some() {
            job_warning = job_warning.map(|w| format!("{w} (the memory limit is not in force)"));
        }
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| std::io::Error::other("no stdout pipe"))?;
        let (tx, rx) = mpsc::channel();
        let reader = std::thread::Builder::new()
            .name(format!("wb-engine-{pid}-stdout"))
            .spawn(move || read_events(stdout, |msg| tx.send(msg).is_ok()))?;
        Ok(Self {
            child,
            events: rx,
            reader: Some(reader),
            started: Instant::now(),
            pid,
            job_warning,
            memory_limit_bytes,
            job: engine_job,
        })
    }

    /// The largest commit the engine has reached (readable after it has ended), when its job can tell.
    pub fn peak_memory_bytes(&self) -> Option<u64> {
        self.job.peak_memory()
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

/// Reads an engine's stdout line by line and hands each line to `send` (which returns false when nobody is
/// listening any more). The reader works on bytes: a line that is not valid UTF-8 becomes [`Line::Garbled`]
/// (with the bad bytes shown as U+FFFD, and never parsed as an event, so damaged text cannot slip into a
/// transcript as a segment) and the stream goes on. Only a read error or the end of the stream stops it.
pub(crate) fn read_events<R: Read>(reader: R, mut send: impl FnMut(Line) -> bool) {
    let mut reader = BufReader::new(reader);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let trimmed = buf.trim_ascii();
        if trimmed.is_empty() {
            continue;
        }
        let msg = match std::str::from_utf8(trimmed) {
            Ok(text) => match Event::from_line(text) {
                Ok(e) => Line::Event(Box::new(e)),
                Err(_) => Line::Garbled(text.to_owned()),
            },
            Err(_) => Line::Garbled(String::from_utf8_lossy(trimmed).into_owned()),
        };
        if !send(msg) {
            break;
        }
    }
}

/// The path of the engine for `variant` next to `dir`: `wb-engine-<variant>-x86_64-pc-windows-msvc.exe`.
pub fn sidecar_path(dir: &Path, variant: &str) -> PathBuf {
    dir.join(format!("wb-engine-{variant}-x86_64-pc-windows-msvc.exe"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(bytes: &[u8]) -> Vec<Line> {
        let mut out = Vec::new();
        read_events(bytes, |l| {
            out.push(l);
            true
        });
        out
    }

    fn warning(code: &str) -> Vec<u8> {
        let mut v = Event::Warning {
            code: code.into(),
            message: "m".into(),
        }
        .to_line()
        .expect("line")
        .into_bytes();
        v.push(b'\n');
        v
    }

    #[test]
    fn a_line_with_invalid_utf8_is_garbled_and_the_stream_continues() {
        let mut data = warning("first");
        data.extend_from_slice(b"{\"not\":\"an event \xff\xfe broken\"}\n");
        data.extend(warning("second"));
        let lines = collect(&data);
        assert_eq!(lines.len(), 3);
        assert!(
            matches!(&lines[0], Line::Event(e) if matches!(**e, Event::Warning { ref code, .. } if code == "first"))
        );
        assert!(matches!(&lines[1], Line::Garbled(t) if t.contains('\u{FFFD}')));
        assert!(
            matches!(&lines[2], Line::Event(e) if matches!(**e, Event::Warning { ref code, .. } if code == "second"))
        );
    }

    #[test]
    fn an_event_line_damaged_by_invalid_utf8_is_not_parsed_as_an_event() {
        // The text of a segment with a broken byte inside: never shown as transcript text.
        let mut line = warning("w");
        let at = line.len() - 8;
        line.insert(at, 0xff);
        let lines = collect(&line);
        assert!(matches!(&lines[0], Line::Garbled(_)));
    }

    #[test]
    fn blank_lines_are_skipped_and_a_last_line_without_a_newline_is_read() {
        let mut data = b"\n  \r\n".to_vec();
        let mut w = warning("last");
        w.pop();
        data.extend(w);
        let lines = collect(&data);
        assert_eq!(lines.len(), 1);
        assert!(matches!(&lines[0], Line::Event(_)));
    }

    #[test]
    fn the_reader_stops_when_nobody_is_listening() {
        let mut data = warning("a");
        data.extend(warning("b"));
        let mut seen = 0;
        read_events(data.as_slice(), |_| {
            seen += 1;
            false
        });
        assert_eq!(seen, 1);
    }
}
