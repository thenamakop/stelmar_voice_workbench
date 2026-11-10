//! A running session: capture, recorder and mirror threads, started and stopped together.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use serde::Serialize;

use crate::Error;
use crate::capture::Capture;
use crate::events::Event;
use crate::mirror::{MirrorHandle, MirrorVerify};
use crate::parts;
use crate::recorder::{
    DEFAULT_PART_SECONDS, RecorderConfig, RecorderHandle, RecorderMsg, StopReport,
};
use crate::session::{self, InputInfo, SessionLock, SessionMeta, SessionSpec};
use crate::sys;
use crate::util::now_ist;

/// New sessions are refused below this much free space.
pub const REFUSE_BELOW_BYTES: u64 = 5 * 1_000_000_000;

#[derive(Debug, Clone)]
pub struct StartOptions {
    pub recordings_root: PathBuf,
    pub backup_root: Option<PathBuf>,
    pub device_id: String,
    pub app_version: String,
    pub part_seconds: u32,
}

impl StartOptions {
    pub fn new(
        recordings_root: PathBuf,
        backup_root: Option<PathBuf>,
        device_id: String,
        app_version: &str,
    ) -> Self {
        Self {
            recordings_root,
            backup_root,
            device_id,
            app_version: app_version.to_string(),
            part_seconds: DEFAULT_PART_SECONDS,
        }
    }
}

/// Stop's full result.
#[derive(Debug, Clone, Serialize)]
pub struct SessionStopped {
    pub recorder: StopReport,
    pub mirror: MirrorVerify,
}

pub struct ActiveSession {
    dir: PathBuf,
    input: InputInfo,
    recorder: Option<RecorderHandle>,
    capture: Option<Capture>,
    mirror: Option<MirrorHandle>,
    lock: Option<SessionLock>,
}

fn check_space(root: &Path) -> Result<(), Error> {
    if let Ok(free) = sys::disk_free_bytes(root) {
        if free < REFUSE_BELOW_BYTES {
            return Err(Error::DiskFull {
                path: root.to_path_buf(),
                free_gb: free as f64 / 1e9,
            });
        }
    }
    Ok(())
}

/// Recordings stay on drives in this computer, never on network paths: a non-local recordings
/// folder refuses the session. A non-local backup folder must not stop the recording
/// (non-negotiable 2): the session records without a mirror and a notice says so.
fn local_options(opts: &StartOptions, events: &Sender<Event>) -> Result<StartOptions, Error> {
    if !sys::is_local_path(&opts.recordings_root) {
        return Err(Error::Invalid(format!(
            "{} is not on a drive in this computer. Recordings must be on an internal or USB drive, not a network folder.",
            opts.recordings_root.display()
        )));
    }
    let mut o = opts.clone();
    if o.backup_root
        .as_ref()
        .is_some_and(|b| !sys::is_local_path(b))
    {
        let _ = events.send(crate::events::Notice::event(
            crate::events::NoticeLevel::Warning,
            "mirror",
            "The backup folder is not on a drive in this computer, so this session records without a backup copy. Set a local backup folder in Settings.",
        ));
        o.backup_root = None;
    }
    Ok(o)
}

fn backup_dir_for(opts: &StartOptions, dir: &Path) -> Option<PathBuf> {
    let root = opts.backup_root.as_ref()?;
    let rel = dir.strip_prefix(&opts.recordings_root).ok()?;
    Some(root.join(rel))
}

impl ActiveSession {
    /// Creates the session folder and starts recording from `opts.device_id`.
    pub fn start(
        spec: &SessionSpec,
        opts: &StartOptions,
        events: Sender<Event>,
    ) -> Result<Self, Error> {
        let opts = &local_options(opts, &events)?;
        std::fs::create_dir_all(&opts.recordings_root)
            .map_err(|e| Error::io(&opts.recordings_root, e))?;
        check_space(&opts.recordings_root)?;
        let now = now_ist();
        let dir = session::create_session_dir(&opts.recordings_root, spec, now)?;
        // Lock before session.json exists, so a recovery scan in another instance never sees
        // this session as one a crash left open.
        let lock = match SessionLock::acquire(&dir) {
            Ok(l) => l,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&dir);
                return Err(e);
            }
        };
        let meta = SessionMeta::new(spec, &opts.app_version, opts.backup_root.as_deref(), now);
        if let Err(e) = session::save_meta(&dir, &meta) {
            lock.release();
            let _ = std::fs::remove_dir_all(&dir);
            return Err(e);
        }
        match Self::launch(&dir, meta, parts::PartsIndex::new(), opts, events, lock) {
            Ok(s) => Ok(s),
            Err(e) => {
                // Nothing was recorded: remove the empty folder so it does not show as a crash.
                let _ = std::fs::remove_dir_all(&dir);
                Err(e)
            }
        }
    }

    /// Resumes a recovered session (run `recovery::repair` first): a new part continues the
    /// audio clock where the last one ended.
    pub fn resume(dir: &Path, opts: &StartOptions, events: Sender<Event>) -> Result<Self, Error> {
        let opts = &local_options(opts, &events)?;
        check_space(dir)?;
        let lock = SessionLock::acquire(dir)?;
        let mut meta = session::load_meta(dir)?;
        if meta.closed_at.is_some() {
            return Err(Error::Invalid(format!(
                "{} is already closed",
                dir.display()
            )));
        }
        let index = parts::load(dir)?;
        meta.push_event(now_ist(), index.end_ms(), "resumed_after_crash", None);
        session::save_meta(dir, &meta)?;
        Self::launch(dir, meta, index, opts, events, lock)
    }

    fn launch(
        dir: &Path,
        meta: SessionMeta,
        index: parts::PartsIndex,
        opts: &StartOptions,
        events: Sender<Event>,
        lock: SessionLock,
    ) -> Result<Self, Error> {
        let recorder = match RecorderHandle::spawn(
            RecorderConfig {
                session_dir: dir.to_path_buf(),
                part_seconds: opts.part_seconds,
                meta,
                index,
            },
            events.clone(),
        ) {
            Ok(r) => r,
            Err(e) => {
                lock.release();
                return Err(e);
            }
        };
        let (capture, input) =
            match Capture::start(opts.device_id.clone(), recorder.sender(), events.clone()) {
                Ok(c) => c,
                Err(e) => {
                    let _ = recorder.stop();
                    lock.release();
                    return Err(e);
                }
            };
        let mirror = MirrorHandle::spawn(dir.to_path_buf(), backup_dir_for(opts, dir), events)
            .map_err(|e| Error::io(dir, e))?;
        Ok(Self {
            dir: dir.to_path_buf(),
            input,
            recorder: Some(recorder),
            capture: Some(capture),
            mirror: Some(mirror),
            lock: Some(lock),
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn input(&self) -> &InputInfo {
        &self.input
    }

    fn send(&self, msg: RecorderMsg) -> Result<(), Error> {
        self.recorder.as_ref().ok_or(Error::RecorderGone)?.send(msg)
    }

    pub fn pause(&self) -> Result<(), Error> {
        self.send(RecorderMsg::Pause)
    }

    pub fn resume_recording(&self) -> Result<(), Error> {
        self.send(RecorderMsg::Resume)
    }

    /// Test and gate-hook use only: makes the recorder thread panic once, to prove it recovers
    /// and continues in a new part.
    #[doc(hidden)]
    pub fn inject_recorder_panic(&self, point: crate::recorder::PanicPoint) -> Result<(), Error> {
        self.send(RecorderMsg::InjectPanic(point))
    }

    /// True while the recorder thread is alive.
    pub fn recorder_running(&self) -> bool {
        self.recorder
            .as_ref()
            .is_some_and(RecorderHandle::is_running)
    }

    /// Stops recording, closes the session, finishes the mirror and verifies it.
    pub fn stop(mut self) -> Result<SessionStopped, Error> {
        let recorder = self.recorder.take().ok_or(Error::RecorderGone)?;
        // The recorder drains the ring first, then the stream closes.
        let report = recorder.stop();
        if let Some(mut c) = self.capture.take() {
            c.stop();
        }
        let report = report?;
        let mirror = match self.mirror.as_mut() {
            Some(m) => m.final_sync(),
            None => MirrorVerify {
                backup_dir: None,
                files: Vec::new(),
                all_match: false,
                message: None,
            },
        };
        if let Some(mut m) = self.mirror.take() {
            m.stop();
        }
        if let Some(lock) = self.lock.take() {
            lock.release();
        }
        Ok(SessionStopped {
            recorder: report,
            mirror,
        })
    }
}

impl Drop for ActiveSession {
    fn drop(&mut self) {
        // Dropped without Stop (app exit): the recorder closes its part and leaves the session
        // open, so the next start offers Resume. The lock is released by the OS with the handle.
        if let Some(mut c) = self.capture.take() {
            c.stop();
        }
        self.recorder.take();
        self.mirror.take();
    }
}
