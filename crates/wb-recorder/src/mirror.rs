//! The mirror thread keeps a copy of the session folder on the backup drive, at the same
//! relative path under the backup root. It copies closed parts, follows the open part by
//! appending new bytes and rewriting its header every second, and reports how far behind it is.
//! It runs below normal priority and only reads the session folder.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::events::{Event, MirrorState, MirrorStatus, Notice, NoticeLevel};
use crate::session::{self, MANIFEST_FILE};
use crate::sys;
use crate::util::{now_ist, rel_slash, rfc3339, sha256_file, write_atomic};
use crate::wav::{HEADER_REWRITE_BYTES, inspect};

pub const PASS_INTERVAL: Duration = Duration::from_secs(1);
/// Warn when the mirror is this far behind.
pub const WARN_LAG: Duration = Duration::from_secs(30);
const FSYNC_INTERVAL: Duration = Duration::from_secs(5);
/// Stop waits at most this long for the final copy and hash comparison.
const FINAL_SYNC_TIMEOUT: Duration = Duration::from_secs(300);

/// One file's comparison after the final sync.
#[derive(Debug, Clone, Serialize)]
pub struct FileCheck {
    pub file: String,
    pub sha256: String,
    pub mirror_sha256: Option<String>,
    pub matches: bool,
}

/// The mirror's state after Stop.
#[derive(Debug, Clone, Serialize)]
pub struct MirrorVerify {
    pub backup_dir: Option<String>,
    pub files: Vec<FileCheck>,
    pub all_match: bool,
    pub message: Option<String>,
}

enum MirrorMsg {
    FinalSync(SyncSender<MirrorVerify>),
}

pub struct MirrorHandle {
    tx: Option<Sender<MirrorMsg>>,
    join: Option<JoinHandle<()>>,
    session_dir: PathBuf,
}

impl MirrorHandle {
    /// Starts mirroring `session_dir` to `backup_dir` (or reports "not set" when `None`).
    pub fn spawn(
        session_dir: PathBuf,
        backup_dir: Option<PathBuf>,
        events: Sender<Event>,
    ) -> io::Result<Self> {
        let (tx, rx) = mpsc::channel();
        let dir = session_dir.clone();
        let join = std::thread::Builder::new()
            .name("wb-mirror".into())
            .spawn(move || {
                sys::lower_current_thread();
                let mut m = Mirror::new(dir, backup_dir, events);
                m.run(&rx);
            })?;
        Ok(Self {
            tx: Some(tx),
            join: Some(join),
            session_dir,
        })
    }

    /// Copies everything that is left, then compares SHA-256 of every file with its copy.
    pub fn final_sync(&mut self) -> MirrorVerify {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        let fallback = MirrorVerify {
            backup_dir: None,
            files: Vec::new(),
            all_match: false,
            message: Some("the mirror thread has stopped".into()),
        };
        let Some(tx) = &self.tx else { return fallback };
        if tx.send(MirrorMsg::FinalSync(reply_tx)).is_err() {
            return fallback;
        }
        // The final pass copies what is left and hashes every file on both sides: allow 5
        // minutes plus a second per 10 MB of session data (a slow USB drive reads ~20 MB/s).
        let bytes: u64 = session::session_files(&self.session_dir)
            .unwrap_or_default()
            .iter()
            .filter_map(|f| std::fs::metadata(f).ok())
            .map(|m| m.len())
            .sum();
        let timeout = FINAL_SYNC_TIMEOUT + Duration::from_secs(bytes / 10_000_000);
        match reply_rx.recv_timeout(timeout) {
            Ok(v) => v,
            Err(RecvTimeoutError::Timeout) => {
                // A hung backup drive must not hang Stop: leave the thread to finish on its own.
                self.tx.take();
                self.join.take();
                MirrorVerify {
                    backup_dir: None,
                    files: Vec::new(),
                    all_match: false,
                    message: Some(
                        "The backup copy did not finish in time; it may be incomplete. Check the backup drive."
                            .into(),
                    ),
                }
            }
            Err(RecvTimeoutError::Disconnected) => fallback,
        }
    }

    pub fn stop(&mut self) {
        self.tx.take();
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

impl Drop for MirrorHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

struct Mirror {
    src: PathBuf,
    dst: Option<PathBuf>,
    events: Sender<Event>,
    byte_rates: HashMap<PathBuf, u64>,
    last_ok: Option<Instant>,
    last_ok_at: Option<String>,
    last_fsync: Instant,
    started: Instant,
    warned: bool,
}

impl Mirror {
    fn new(src: PathBuf, dst: Option<PathBuf>, events: Sender<Event>) -> Self {
        Self {
            src,
            dst,
            events,
            byte_rates: HashMap::new(),
            last_ok: None,
            last_ok_at: None,
            last_fsync: Instant::now(),
            started: Instant::now(),
            warned: false,
        }
    }

    fn run(&mut self, rx: &Receiver<MirrorMsg>) {
        loop {
            match rx.recv_timeout(PASS_INTERVAL) {
                Ok(MirrorMsg::FinalSync(reply)) => {
                    let _ = reply.send(self.final_sync());
                }
                Err(RecvTimeoutError::Timeout) => {
                    let status = self.pass();
                    let _ = self.events.send(Event::MirrorStatus(status));
                }
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    fn files(&self) -> Vec<PathBuf> {
        let mut files = session::session_files(&self.src).unwrap_or_default();
        let manifest = self.src.join(MANIFEST_FILE);
        if manifest.is_file() {
            files.push(manifest);
        }
        files
    }

    fn status(&self, state: MirrorState, lag_ms: u64, message: Option<String>) -> MirrorStatus {
        let warn = matches!(state, MirrorState::Missing | MirrorState::Error)
            || lag_ms > WARN_LAG.as_millis() as u64;
        MirrorStatus {
            state,
            backup_dir: self.dst.as_ref().map(|d| d.to_string_lossy().into_owned()),
            lag_ms,
            warn,
            last_sync_at: self.last_ok_at.clone(),
            message,
        }
    }

    /// One pass: copy what changed. Returns the status to report.
    fn pass(&mut self) -> MirrorStatus {
        let Some(dst_root) = self.dst.clone() else {
            return self.status(MirrorState::NotSet, 0, None);
        };
        let since_ok = self.last_ok.unwrap_or(self.started).elapsed();
        if let Err(e) = fs::create_dir_all(&dst_root) {
            return self.unreachable(
                MirrorState::Missing,
                since_ok,
                format!("Backup folder unreachable: {e}"),
            );
        }
        let mut behind_bytes_ms = 0u64;
        for src in self.files() {
            let Some(rel) = rel_slash(&self.src, &src) else {
                continue;
            };
            let dst = dst_root.join(Path::new(&rel));
            match self.sync_one(&src, &dst) {
                Ok(lag) => behind_bytes_ms += lag,
                Err(e) => {
                    return self.unreachable(
                        MirrorState::Error,
                        since_ok,
                        format!("Could not copy {rel}: {e}"),
                    );
                }
            }
        }
        let fsync = self.last_fsync.elapsed() >= FSYNC_INTERVAL;
        if fsync {
            self.last_fsync = Instant::now();
        }
        self.last_ok = Some(Instant::now());
        self.last_ok_at = Some(rfc3339(now_ist()));
        if self.warned {
            self.warned = false;
            let _ = self.events.send(Notice::event(
                NoticeLevel::Info,
                "mirror",
                "Backup copy is in sync again",
            ));
        }
        let state = if behind_bytes_ms > PASS_INTERVAL.as_millis() as u64 * 2 {
            MirrorState::Behind
        } else {
            MirrorState::InSync
        };
        self.status(state, behind_bytes_ms, None)
    }

    fn unreachable(
        &mut self,
        state: MirrorState,
        since_ok: Duration,
        message: String,
    ) -> MirrorStatus {
        if !self.warned {
            self.warned = true;
            let _ = self.events.send(Notice::event(
                NoticeLevel::Warning,
                "mirror",
                message.clone(),
            ));
        }
        self.status(state, since_ok.as_millis() as u64, Some(message))
    }

    /// Brings `dst` up to date with `src`. Returns how much audio (ms) was behind before the copy.
    fn sync_one(&mut self, src: &Path, dst: &Path) -> io::Result<u64> {
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        let is_wav = src
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("wav"));
        if !is_wav {
            let bytes = fs::read(src)?;
            if fs::read(dst).ok().as_deref() != Some(&bytes[..]) {
                write_atomic(dst, &bytes)?;
            }
            return Ok(0);
        }
        let src_len = fs::metadata(src)?.len();
        let mut dst_len = fs::metadata(dst).map(|m| m.len()).unwrap_or(0);
        let mut out = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(dst)?;
        if dst_len > src_len {
            // The source shrank (header repair cut a partial frame): copy again from the start.
            out.set_len(0)?;
            dst_len = 0;
        }
        let behind = src_len - dst_len;
        let rate = match self.byte_rates.get(src) {
            Some(r) => *r,
            None => {
                let r = inspect(src).map(|i| i.byte_rate()).unwrap_or(0);
                if r > 0 {
                    self.byte_rates.insert(src.to_path_buf(), r);
                }
                r
            }
        };
        let mut input = File::open(src)?;
        if behind > 0 {
            input.seek(SeekFrom::Start(dst_len))?;
            out.seek(SeekFrom::Start(dst_len))?;
            copy_exact(&mut input, &mut out, behind)?;
        }
        // The RIFF and data sizes change as the part grows: rewrite the header region.
        let head = HEADER_REWRITE_BYTES.min(src_len) as usize;
        let mut buf = vec![0u8; head];
        input.seek(SeekFrom::Start(0))?;
        input.read_exact(&mut buf)?;
        out.seek(SeekFrom::Start(0))?;
        out.write_all(&buf)?;
        if self.last_fsync.elapsed() >= FSYNC_INTERVAL {
            out.sync_data()?;
        }
        Ok((behind * 1000).checked_div(rate).unwrap_or(0))
    }

    fn final_sync(&mut self) -> MirrorVerify {
        let Some(dst_root) = self.dst.clone() else {
            return MirrorVerify {
                backup_dir: None,
                files: Vec::new(),
                all_match: false,
                message: Some("No backup folder is set".into()),
            };
        };
        // Two passes: the second catches anything the recorder wrote during the first.
        for _ in 0..2 {
            let status = self.pass();
            if matches!(status.state, MirrorState::Missing | MirrorState::Error) {
                let _ = self.events.send(Event::MirrorStatus(status.clone()));
                return MirrorVerify {
                    backup_dir: status.backup_dir,
                    files: Vec::new(),
                    all_match: false,
                    message: status.message,
                };
            }
        }
        let mut files = Vec::new();
        for src in self.files() {
            let Some(rel) = rel_slash(&self.src, &src) else {
                continue;
            };
            let dst = dst_root.join(Path::new(&rel));
            let sha = sha256_file(&src).unwrap_or_default();
            let mut mirror_sha = sha256_file(&dst).ok();
            if mirror_sha.as_deref() != Some(sha.as_str()) {
                // Copy the whole file again and check once more.
                if fs::copy(&src, &dst).is_ok() {
                    if let Ok(f) = OpenOptions::new().write(true).open(&dst) {
                        let _ = f.sync_all();
                    }
                }
                mirror_sha = sha256_file(&dst).ok();
            } else if let Ok(f) = OpenOptions::new().write(true).open(&dst) {
                let _ = f.sync_all();
            }
            let matches = mirror_sha.as_deref() == Some(sha.as_str());
            files.push(FileCheck {
                file: rel,
                sha256: sha,
                mirror_sha256: mirror_sha,
                matches,
            });
        }
        let all_match = !files.is_empty() && files.iter().all(|f| f.matches);
        let status = self.status(
            if all_match {
                MirrorState::InSync
            } else {
                MirrorState::Error
            },
            0,
            None,
        );
        let _ = self.events.send(Event::MirrorStatus(status));
        MirrorVerify {
            backup_dir: Some(dst_root.to_string_lossy().into_owned()),
            files,
            all_match,
            message: None,
        }
    }
}

fn copy_exact(input: &mut File, out: &mut File, mut n: u64) -> io::Result<()> {
    let mut buf = vec![0u8; 1 << 20];
    while n > 0 {
        let want = n.min(buf.len() as u64) as usize;
        let got = input.read(&mut buf[..want])?;
        if got == 0 {
            break; // the source is shorter than its length said a moment ago
        }
        out.write_all(&buf[..got])?;
        n -= got as u64;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;
    use crate::wav::PartWriter;

    #[test]
    fn mirror_follows_a_growing_part_and_verifies() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = TempDir::new("mirror")?;
        let src = tmp.path().join("rec/2027-02-08/talk");
        let dst = tmp.path().join("backup/2027-02-08/talk");
        fs::create_dir_all(src.join("audio"))?;
        fs::write(src.join("session.json"), b"{\"v\":1}")?;
        let (tx, rx) = mpsc::channel();
        let mut m = Mirror::new(src.clone(), Some(dst.clone()), tx);

        let mut w = PartWriter::create(&src.join("audio/part-0001.wav"), 16_000, 1)?;
        w.write_f32(&vec![0.1; 16_000])?;
        w.flush()?;
        let s1 = m.pass();
        assert_eq!(s1.state, MirrorState::InSync);
        assert_eq!(
            fs::read(src.join("audio/part-0001.wav"))?,
            fs::read(dst.join("audio/part-0001.wav"))?
        );

        // The part grows by 40 seconds while the mirror is not looking: the next pass reports
        // the lag and catches up, header included.
        w.write_f32(&vec![-0.2; 16_000 * 40])?;
        w.flush()?;
        let s2 = m.pass();
        assert!(s2.lag_ms >= 39_000, "{s2:?}");
        assert!(s2.warn);
        assert_eq!(
            fs::read(src.join("audio/part-0001.wav"))?,
            fs::read(dst.join("audio/part-0001.wav"))?
        );
        w.finish()?;
        fs::write(src.join("session.json"), b"{\"v\":2}")?;
        fs::write(src.join(session::LOCK_FILE), b"")?;

        let verify = m.final_sync();
        assert!(verify.all_match, "{verify:?}");
        assert_eq!(verify.files.len(), 2);
        assert_eq!(fs::read(dst.join("session.json"))?, b"{\"v\":2}");
        assert!(!dst.join(session::LOCK_FILE).exists());
        drop(rx);
        Ok(())
    }

    #[test]
    fn missing_backup_drive_warns() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = TempDir::new("mirror-missing")?;
        let src = tmp.path().join("rec");
        fs::create_dir_all(&src)?;
        // A file where the backup folder should be makes it unreachable.
        let blocker = tmp.path().join("backup");
        fs::write(&blocker, b"")?;
        let (tx, rx) = mpsc::channel();
        let mut m = Mirror::new(src, Some(blocker.join("talk")), tx);
        let s = m.pass();
        assert_eq!(s.state, MirrorState::Missing);
        assert!(s.warn);
        assert!(rx.try_iter().any(|e| matches!(e, Event::Notice(_))));
        Ok(())
    }

    #[test]
    fn not_set_is_reported() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = TempDir::new("mirror-none")?;
        let (tx, _rx) = mpsc::channel();
        let mut m = Mirror::new(tmp.path().to_path_buf(), None, tx);
        assert_eq!(m.pass().state, MirrorState::NotSet);
        assert!(!m.final_sync().all_match);
        Ok(())
    }
}
