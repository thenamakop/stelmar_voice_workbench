//! The recorder thread: drains the ring buffer, writes 10-minute WAV parts, flushes every second,
//! fsyncs every 5 seconds, measures the level and reports status.
//!
//! It runs on its own thread with raised priority (MMCSS "Pro Audio") and shares no lock with any
//! other thread: commands arrive on a channel and events leave on a channel. It alone writes the
//! WAV parts and `session.json`; it and the closer thread append lines to `session.log` (each
//! append is one write of a whole line). `parts.json` is written only by the closer, which hashes
//! each closed part off the recorder's path.
//!
//! Panics: a panic anywhere else in the process cannot stop this thread (the release profile
//! unwinds). A panic inside it is caught per loop step: the open part is closed, a
//! `recorder_restarted` event is logged and recording continues in a new part.
//!
//! Audio clock: `start_ms` and `audio_ms` count recorded audio only. Pauses, input losses and
//! crash downtime are not on the clock; they are logged in `session.json` events with their
//! wall-clock times (IST) and the audio time at which they happened.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ringbuf::traits::{Consumer, Observer};
use serde::Serialize;

use crate::Error;
use crate::capture::{PositionKind, RING_SECONDS, StreamHandle};
use crate::dropout::{DropoutDetector, ZeroGap};
use crate::events::{Event, Notice, NoticeLevel, RecState, RecorderStatus};
use crate::level::LevelMeter;
use crate::parts::{self, AUDIO_DIR, PartRecord, PartsIndex};
use crate::repeat::{RepeatDetector, RepeatedBlock};
use crate::session::{self, InputInfo, SessionMeta};
use crate::sys::{self, AwakeGuard};
use crate::util::{now_ist, rfc3339};
use crate::wav::{self, PCM16_HEADER_BYTES, PartWriter, Salvage, f32_to_i16, frames_to_ms};

/// 10-minute parts (PLAN D10).
pub const DEFAULT_PART_SECONDS: u32 = 600;
pub const FLUSH_INTERVAL: Duration = Duration::from_secs(1);
pub const SYNC_INTERVAL: Duration = Duration::from_secs(5);
const STATUS_INTERVAL: Duration = Duration::from_millis(250);
const DISK_CHECK_INTERVAL: Duration = Duration::from_secs(30);
const IDLE_SLEEP: Duration = Duration::from_millis(10);
const WRITE_RETRY: Duration = Duration::from_secs(1);
/// An open stream that delivers nothing for this long counts as lost (the capture thread's own
/// watchdog is 3 seconds; this one also covers a capture thread that died).
const NO_AUDIO_LIMIT: Duration = Duration::from_secs(5);
/// Overrun and xrun events go to session.json at most this often.
const COUNTER_EVENT_INTERVAL: Duration = Duration::from_secs(1);
/// `gap` events go to session.json at most this often (session.log gets each one).
const GAP_EVENT_INTERVAL: Duration = Duration::from_secs(30);
/// Warn below this much free space on the recording drive.
pub const DISK_WARN_BYTES: u64 = 20 * 1_000_000_000;
/// `session.json` keeps at most this many xrun events and this many repeated-block events per
/// session (it is rewritten whole each time); `session.log` and the counters get every one.
const MAX_XRUN_EVENTS: usize = 50;
const MAX_REPEAT_EVENTS: usize = 200;
/// Events that are not urgent are written to `session.json` at most this often.
const META_SAVE_INTERVAL: Duration = Duration::from_secs(2);
/// A repeated block raises a Toast at most this often.
const REPEAT_NOTICE_INTERVAL: Duration = Duration::from_secs(60);

pub enum RecorderMsg {
    /// A new input stream (first open or reconnect). Always starts a new part.
    Stream(StreamHandle),
    InputLost {
        reason: String,
    },
    Pause,
    Resume,
    Stop(SyncSender<Result<StopReport, Error>>),
    /// Test and gate-hook use only: panic inside the recorder thread, to prove it recovers.
    #[doc(hidden)]
    InjectPanic(PanicPoint),
    /// Test use only: make a detector panic once or run slowly, to prove the recording is
    /// unaffected.
    #[doc(hidden)]
    InjectDetectorFault(DetectorFault),
}

/// The two audio-level detectors that look at a block after it is written.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectorKind {
    Repeat,
    ZeroGap,
}

impl DetectorKind {
    fn name(self) -> &'static str {
        match self {
            Self::Repeat => "repeated_block",
            Self::ZeroGap => "zero_gap",
        }
    }
}

/// A fault injected into a detector (tests only).
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectorFault {
    /// The detector panics the next time it is fed.
    Panic(DetectorKind),
    /// Every feed of the detector sleeps this long first (a detector that has become slow).
    Sleep(DetectorKind, Duration),
}

/// Detector time per block is limited to this fraction of the block's audio duration (1/20),
/// but never below `DETECTOR_BUDGET_MIN`.
const DETECTOR_BUDGET_DIVISOR: u32 = 20;
const DETECTOR_BUDGET_MIN: Duration = Duration::from_millis(2);
/// The detectors are fed in slices of this much audio, so the budget is checked between slices.
const DETECTOR_SLICE_MS: u64 = 10;

/// The time the detectors may spend on a block of `block_frames` frames at `rate`.
fn detector_budget(block_frames: u64, rate: u32) -> Duration {
    let block = Duration::from_micros(block_frames * 1_000_000 / u64::from(rate.max(1)));
    (block / DETECTOR_BUDGET_DIVISOR).max(DETECTOR_BUDGET_MIN)
}

fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".into())
}

/// Where an injected recorder panic happens (tests and the gate hook only).
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanicPoint {
    /// While handling the message itself.
    InMessage,
    /// At the start of the next batch write, before any of it is written.
    BeforeWrite,
    /// In the middle of a batch that spans a part rollover, after its first part was written.
    MidBatch,
    /// Before writing, on each of the next N attempts (a batch that keeps panicking).
    BeforeWriteRepeated(u8),
}

/// After this many panics in a row on the same batch, the rest of that batch is discarded and
/// counted in `dropped_samples`, so one poisonous batch cannot hold the recorder in a restart loop.
const MAX_PANICS_PER_BATCH: u8 = 3;
/// `recorder_restarted` events (session.json, toast) are written at most this often; the restart
/// counter still counts every restart.
const RESTART_EVENT_EVERY: Duration = Duration::from_secs(5);

/// What the recorder left behind at Stop.
#[derive(Debug, Clone, Serialize)]
pub struct StopReport {
    pub session_dir: String,
    pub parts: Vec<PartRecord>,
    pub audio_ms: u64,
    pub overrun_samples: u64,
    pub xruns: u64,
    /// Capture position breaks: the device time left the line the frames predict and stayed off.
    pub position_breaks: u64,
    /// Single late timestamps that came back (no audio affected); reported, never gated.
    pub timestamp_outliers: u64,
    /// What the old pairwise check would have counted (diagnostic only).
    pub position_pair_flags: u64,
    /// Repeated blocks found in the recorded audio (`repeated_block` events in session.json).
    pub repeated_blocks: u64,
    /// Zero gaps found in the recorded audio (`zero_gap` events in session.json).
    pub zero_gaps: u64,
    /// Samples taken from the ring that could not be written (disk full, drive gone).
    pub dropped_samples: u64,
    /// Panics caught inside the recorder thread.
    pub recorder_restarts: u32,
    pub recorder_cpu_ms: Option<u64>,
    pub manifest_files: usize,
    /// Blocks on which the detectors ran out of their time budget and skipped the rest.
    pub detector_over_budget_blocks: u64,
    /// Audio the detectors did not look at because of the budget, in milliseconds.
    pub detector_skipped_ms: u64,
    /// Longest time the detectors took on one block, in microseconds.
    pub detector_max_block_us: u64,
    /// Detectors that panicked and were switched off for the rest of the session.
    pub detectors_disabled: Vec<String>,
}

pub struct RecorderConfig {
    pub session_dir: PathBuf,
    pub part_seconds: u32,
    pub meta: SessionMeta,
    /// Parts already closed (a resumed session); new parts continue after them.
    pub index: PartsIndex,
}

pub struct RecorderHandle {
    tx: Sender<RecorderMsg>,
    join: Option<JoinHandle<()>>,
}

impl RecorderHandle {
    pub fn spawn(cfg: RecorderConfig, events: Sender<Event>) -> Result<Self, Error> {
        let (tx, rx) = mpsc::channel();
        let join = std::thread::Builder::new()
            .name("wb-recorder".into())
            .spawn(move || run(cfg, &rx, events))
            .map_err(|e| Error::Invalid(format!("could not start the recorder thread: {e}")))?;
        Ok(Self {
            tx,
            join: Some(join),
        })
    }

    pub fn sender(&self) -> Sender<RecorderMsg> {
        self.tx.clone()
    }

    pub fn send(&self, msg: RecorderMsg) -> Result<(), Error> {
        self.tx.send(msg).map_err(|_| Error::RecorderGone)
    }

    /// Stops recording, closes the last part, writes `closed_at` and the manifest.
    pub fn stop(mut self) -> Result<StopReport, Error> {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.send(RecorderMsg::Stop(reply_tx))?;
        let report = reply_rx.recv().map_err(|_| Error::RecorderGone)?;
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
        report
    }

    pub fn is_running(&self) -> bool {
        self.join.as_ref().is_some_and(|j| !j.is_finished())
    }
}

/// Work for the closer thread: hash a closed part and add it to `parts.json`.
struct CloseJob {
    part: u32,
    start_ms: u64,
}

/// Adds a closed part to `parts.json`. False when the file could not be read.
fn index_part(
    dir: &std::path::Path,
    events: &Sender<Event>,
    index: &mut PartsIndex,
    job: &CloseJob,
    report: bool,
) -> bool {
    match parts::record_for_file(dir, job.part, job.start_ms) {
        Ok(rec) => {
            index.parts.push(rec.clone());
            index.parts.sort_by_key(|p| p.part);
            if let Err(e) = parts::save(dir, index) {
                let _ = events.send(Notice::event(
                    NoticeLevel::Error,
                    "recorder",
                    format!("Could not write parts.json: {e}"),
                ));
            }
            session::append_log(
                dir,
                now_ist(),
                &format!(
                    "part_closed part={} start_ms={} duration_ms={} sha256={}{}",
                    rec.part,
                    rec.start_ms,
                    rec.duration_ms,
                    rec.sha256,
                    if rec.needs_repair {
                        " needs_repair=true"
                    } else {
                        ""
                    }
                ),
            );
            let _ = events.send(Event::PartClosed(rec));
            true
        }
        Err(e) => {
            if report {
                let _ = events.send(Notice::event(
                    NoticeLevel::Error,
                    "recorder",
                    format!("Could not index part {}: {e}", job.part),
                ));
            }
            false
        }
    }
}

/// Whole frames a part file holds, from its length (the header is not trusted).
fn file_frames(path: &std::path::Path, channels: u16) -> Option<u64> {
    if let Ok(info) = wav::inspect(path) {
        return Some(info.frames_on_disk);
    }
    let len = std::fs::metadata(path).ok()?.len();
    Some(len.saturating_sub(PCM16_HEADER_BYTES) / (u64::from(channels.max(1)) * 2))
}

/// What `open_part` did.
enum OpenResult {
    Opened,
    /// A write failed a moment ago and the retry is not due yet.
    Blocked,
    /// The file could not be created.
    Failed,
}

struct Closer {
    tx: Option<Sender<CloseJob>>,
    join: Option<JoinHandle<PartsIndex>>,
}

impl Closer {
    fn spawn(dir: PathBuf, mut index: PartsIndex, events: Sender<Event>) -> Result<Self, Error> {
        let (tx, rx) = mpsc::channel::<CloseJob>();
        let join = std::thread::Builder::new()
            .name("wb-part-closer".into())
            .spawn(move || {
                sys::lower_current_thread();
                // Parts that could not be read when they closed (the fault may still have been
                // there) are tried again with every later part and once more at the end.
                let mut retry: Vec<CloseJob> = Vec::new();
                for job in rx {
                    for earlier in std::mem::take(&mut retry) {
                        if !index_part(&dir, &events, &mut index, &earlier, false) {
                            retry.push(earlier);
                        }
                    }
                    if !index_part(&dir, &events, &mut index, &job, true) {
                        retry.push(job);
                    }
                }
                for last in retry {
                    if !index_part(&dir, &events, &mut index, &last, false) {
                        let _ = events.send(Notice::event(
                            NoticeLevel::Error,
                            "recorder",
                            format!(
                                "Part {} is on disk but not in parts.json; recovery will index it",
                                last.part
                            ),
                        ));
                    }
                }
                // Parts closed with a header that did not match their length: one more try at
                // the repair now that the session is ending.
                let mut changed = false;
                for rec in index.parts.iter_mut().filter(|r| r.needs_repair) {
                    let path = dir.join(&rec.file);
                    if wav::repair_header(&path).is_ok()
                        && let Ok(fixed) = parts::record_for_file(&dir, rec.part, rec.start_ms)
                    {
                        *rec = fixed;
                        changed = true;
                    }
                }
                if changed {
                    let _ = parts::save(&dir, &index);
                }
                index
            })
            .map_err(|e| Error::Invalid(format!("could not start the part closer thread: {e}")))?;
        Ok(Self {
            tx: Some(tx),
            join: Some(join),
        })
    }

    fn close(&self, job: CloseJob) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(job);
        }
    }

    /// Waits for every queued part to be indexed; returns the final index.
    fn finish(&mut self) -> Option<PartsIndex> {
        self.tx.take();
        self.join.take().and_then(|j| j.join().ok())
    }
}

struct Recorder {
    dir: PathBuf,
    events: Sender<Event>,
    meta: SessionMeta,
    part_seconds: u32,
    closer: Closer,

    stream: Option<StreamHandle>,
    meter: Option<LevelMeter>,
    scratch: Vec<f32>,
    /// Samples (interleaved, 16-bit) that a failed part owes the recording: they go first into
    /// the next part. Small: at most about a second plus the rest of one batch.
    carry: Vec<i16>,

    writer: Option<PartWriter>,
    /// Number of the open part, or of the next one to open.
    part_no: u32,
    /// Audio-clock start of the open (or next) part.
    next_start_ms: u64,

    paused: bool,
    input_ok: bool,
    /// A stream was open earlier in this run (so a new one is a restore, not the first).
    had_stream: bool,
    input_lost_since: Option<String>,
    audio_started_logged: bool,
    /// Frames queued in the ring when the first audio of this run was drained (for back-dating).
    first_queued_frames: Option<u64>,
    /// Driver callbacks seen and when that count last changed (the no-audio watchdog).
    last_callbacks: (u64, Instant),
    /// The batch popped from the ring into `scratch` and not yet fully handled: `batch_len`
    /// samples, of which `batch_done` are written (or deliberately discarded). A panic leaves
    /// both as they were, and the restart finishes the batch in the new part, so a panic loses
    /// and duplicates nothing.
    batch_len: usize,
    batch_done: usize,
    /// Panics in a row on the current batch (reset when a batch completes).
    batch_panics: u8,
    /// When `recorder_restarted` was last written, and restarts since then not yet reported.
    last_restart_event: Option<Instant>,
    restarts_unreported: u32,
    /// Test hook: where to panic next (see `PanicPoint`).
    panic_point: Option<PanicPoint>,
    /// A Stop being handled, so a panic during it can still reply.
    pending_stop: Option<SyncSender<Result<StopReport, Error>>>,
    /// The open part has had no write yet: flush right after the first one, so a crash in the
    /// part's first second still leaves a playable file.
    first_write_pending: bool,
    /// Last `gap` event written to session.json (at most one every 30 s; session.log gets each).
    last_gap_event: Option<Instant>,
    write_blocked_until: Option<Instant>,

    overruns_before: u64,
    xruns_before: u64,
    position_breaks_before: u64,
    timestamp_outliers_before: u64,
    pair_flags_before: u64,
    /// Detector for repeated blocks in the recorded audio (one per stream).
    repeat: Option<RepeatDetector>,
    /// Detector for zero gaps (short digital silence inside live audio), one per stream.
    dropout: Option<DropoutDetector>,
    /// A detector that panicked stays off for the rest of the session (a new stream does not
    /// bring it back).
    repeat_off: bool,
    dropout_off: bool,
    /// Test hooks: a detector panic due on the next feed, and a delay added to every feed.
    fault_panic: Option<DetectorKind>,
    fault_sleep: Option<(DetectorKind, Duration)>,
    detector_over_budget_blocks: u64,
    detector_skipped_ms: u64,
    detector_max_block_us: u64,
    detectors_disabled: Vec<String>,
    last_budget_log: Option<Instant>,
    zero_gaps: u64,
    zero_gap_events_in_meta: usize,
    last_zero_gap_notice: Option<Instant>,
    repeated_blocks: u64,
    repeat_events_in_meta: usize,
    last_repeat_notice: Option<Instant>,
    /// Frames taken from the open stream's ring so far (to place xrun marks on the audio clock).
    stream_popped_frames: u64,
    xrun_seq: u64,
    xrun_events_in_meta: usize,
    /// `session.json` has events not yet saved.
    meta_dirty: bool,
    last_meta_save: Instant,
    dropped_samples: u64,
    logged_overruns: u64,
    logged_xruns: u64,
    logged_dropped: u64,
    last_counter_event: Instant,
    restarts: u32,
    max_fill_pct: f32,
    disk_free: Option<u64>,
    disk_warning: bool,
    last_error: Option<String>,
    priority: sys::PriorityGuard,
    awake: AwakeGuard,
}

struct Timers {
    flush: Instant,
    sync: Instant,
    status: Instant,
    disk: Instant,
}

enum Step {
    Continue,
    Exit,
}

fn run(cfg: RecorderConfig, rx: &Receiver<RecorderMsg>, events: Sender<Event>) {
    // Raise priority and keep the machine awake from this thread (both are per-thread).
    let priority = sys::raise_recorder_priority();
    let awake = AwakeGuard::acquire();
    let dir = cfg.session_dir.clone();
    session::append_log(
        &dir,
        now_ist(),
        &format!(
            "recorder_started priority={:?} awake={} {}",
            priority.mode,
            awake.active,
            priority.fallback_reason.as_deref().unwrap_or("")
        ),
    );
    if let Some(reason) = &priority.fallback_reason {
        let _ = events.send(Notice::event(
            NoticeLevel::Warning,
            "recorder",
            format!("Recorder priority: {reason}"),
        ));
    }
    let closer = match Closer::spawn(dir.clone(), cfg.index.clone(), events.clone()) {
        Ok(c) => c,
        Err(e) => {
            let _ = events.send(Notice::event(NoticeLevel::Error, "recorder", e.to_string()));
            return;
        }
    };
    let now = Instant::now();
    let mut rec = Recorder {
        dir,
        events,
        meta: cfg.meta,
        part_seconds: cfg.part_seconds.max(1),
        closer,
        stream: None,
        meter: None,
        scratch: Vec::new(),
        carry: Vec::new(),
        writer: None,
        part_no: cfg.index.next_part(),
        next_start_ms: cfg.index.end_ms(),
        paused: false,
        input_ok: false,
        had_stream: false,
        input_lost_since: None,
        audio_started_logged: false,
        first_queued_frames: None,
        last_callbacks: (0, now),
        batch_len: 0,
        batch_done: 0,
        batch_panics: 0,
        last_restart_event: None,
        restarts_unreported: 0,
        panic_point: None,
        pending_stop: None,
        first_write_pending: false,
        last_gap_event: None,
        write_blocked_until: None,
        overruns_before: 0,
        xruns_before: 0,
        position_breaks_before: 0,
        timestamp_outliers_before: 0,
        pair_flags_before: 0,
        repeat: None,
        dropout: None,
        repeat_off: false,
        dropout_off: false,
        fault_panic: None,
        fault_sleep: None,
        detector_over_budget_blocks: 0,
        detector_skipped_ms: 0,
        detector_max_block_us: 0,
        detectors_disabled: Vec::new(),
        last_budget_log: None,
        zero_gaps: 0,
        zero_gap_events_in_meta: 0,
        last_zero_gap_notice: None,
        repeated_blocks: 0,
        repeat_events_in_meta: 0,
        last_repeat_notice: None,
        stream_popped_frames: 0,
        xrun_seq: 0,
        xrun_events_in_meta: 0,
        meta_dirty: false,
        last_meta_save: now,
        dropped_samples: 0,
        logged_overruns: 0,
        logged_xruns: 0,
        logged_dropped: 0,
        last_counter_event: now,
        restarts: 0,
        max_fill_pct: 0.0,
        disk_free: None,
        disk_warning: false,
        last_error: None,
        priority,
        awake,
    };
    rec.check_disk();
    let mut timers = Timers {
        flush: now,
        sync: now,
        status: now,
        disk: now,
    };
    loop {
        match catch_unwind(AssertUnwindSafe(|| rec.step(rx, &mut timers))) {
            Ok(Step::Continue) => {}
            Ok(Step::Exit) => return,
            Err(payload) => {
                let msg = panic_text(payload.as_ref());
                if rec.after_panic(&msg) {
                    return;
                }
            }
        }
        std::thread::sleep(IDLE_SLEEP);
    }
}

impl Recorder {
    /// One pass of the loop: messages, drain, flush, sync, disk check, status.
    fn step(&mut self, rx: &Receiver<RecorderMsg>, t: &mut Timers) -> Step {
        loop {
            match rx.try_recv() {
                Ok(RecorderMsg::Stop(reply)) => {
                    // Kept on self, so a panic during stop can still answer it (after_panic).
                    self.pending_stop = Some(reply);
                    let result = self.stop();
                    if let Some(reply) = self.pending_stop.take() {
                        let _ = reply.send(result);
                    }
                    return Step::Exit;
                }
                Ok(RecorderMsg::InjectPanic(PanicPoint::InMessage)) => {
                    panic!("injected recorder panic (test hook)")
                }
                Ok(RecorderMsg::InjectPanic(point)) => self.panic_point = Some(point),
                Ok(RecorderMsg::InjectDetectorFault(fault)) => match fault {
                    DetectorFault::Panic(kind) => self.fault_panic = Some(kind),
                    DetectorFault::Sleep(kind, d) => self.fault_sleep = Some((kind, d)),
                },
                Ok(msg) => self.handle(msg),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    // The app dropped us without Stop: close the part cleanly; leave the session
                    // open so recovery offers Resume.
                    self.drain();
                    self.close_part();
                    self.closer.finish();
                    return Step::Exit;
                }
            }
        }
        self.drain();
        self.drain_marks();
        self.watch_for_silence();
        let now = Instant::now();
        if self.meta_dirty && now.duration_since(self.last_meta_save) >= META_SAVE_INTERVAL {
            self.save_meta();
        }
        if now.duration_since(t.flush) >= FLUSH_INTERVAL {
            t.flush = now;
            if now.duration_since(t.sync) >= SYNC_INTERVAL {
                t.sync = now;
                self.with_writer(PartWriter::sync);
            } else {
                self.with_writer(PartWriter::flush);
            }
        }
        if now.duration_since(t.disk) >= DISK_CHECK_INTERVAL {
            t.disk = now;
            self.check_disk();
        }
        if now.duration_since(t.status) >= STATUS_INTERVAL {
            t.status = now;
            self.log_counters();
            self.send_status();
        }
        Step::Continue
    }

    /// After a panic inside the loop: close the open part (or abandon it if that panics too),
    /// log the restart and carry on in a new part.
    /// Returns true when the thread should end (a panic during Stop: the caller is told).
    fn after_panic(&mut self, msg: &str) -> bool {
        self.restarts += 1;
        self.restarts_unreported += 1;
        let mut discarded = 0u64;
        if self.batch_len > 0 && self.batch_panics >= MAX_PANICS_PER_BATCH {
            // The same batch has panicked repeatedly: drop what is left of it (counted) so the
            // recorder goes on with the audio behind it.
            discarded = self.batch_len.saturating_sub(self.batch_done) as u64;
            self.dropped_samples += discarded;
            self.batch_len = 0;
            self.batch_done = 0;
            self.batch_panics = 0;
        }
        // The drain buffer may have been moved out when the panic hit: rebuild it.
        self.rebuild_scratch();
        if let Some(reply) = self.pending_stop.take() {
            let _ = reply.send(Err(Error::Invalid(format!(
                "The recorder failed while stopping ({msg}); the session stays open and the next start offers Resume"
            ))));
            return true;
        }
        let closed = catch_unwind(AssertUnwindSafe(|| self.close_part()));
        if closed.is_err() {
            if let Some(w) = self.writer.take() {
                let path = w.path().to_path_buf();
                // Leak the writer rather than run its finaliser again; the bytes on disk stay
                // and the header is repaired from the file length.
                std::mem::forget(w);
                let _ = crate::wav::repair_header(&path);
            }
            self.part_no += 1;
        }
        let due = self
            .last_restart_event
            .is_none_or(|t| t.elapsed() >= RESTART_EVENT_EVERY);
        if due {
            self.last_restart_event = Some(Instant::now());
            let count = std::mem::take(&mut self.restarts_unreported);
            let _ = catch_unwind(AssertUnwindSafe(|| {
                self.error(format!(
                    "The recorder hit an internal error and continues in a new part: {msg}"
                ));
                let mut detail = msg.to_string();
                if count > 1 {
                    detail.push_str(&format!(" ({count} restarts since the last report)"));
                }
                if discarded > 0 {
                    detail.push_str(&format!(
                        "; {discarded} samples of a batch that kept failing were dropped"
                    ));
                }
                self.event("recorder_restarted", Some(detail));
            }));
        }
        false
    }

    /// Half a second of whole frames for the open stream.
    fn rebuild_scratch(&mut self) {
        if let Some(stream) = &self.stream {
            let channels = usize::from(stream.input.channels.max(1));
            let len = (stream.input.sample_rate as usize / 2).max(1) * channels;
            if self.scratch.len() != len {
                self.scratch = vec![0.0; len];
            }
        }
    }

    fn audio_ms(&self) -> u64 {
        self.next_start_ms
            + self
                .writer
                .as_ref()
                .map_or(0, |w| frames_to_ms(w.frames(), w.sample_rate()))
    }

    fn state(&self) -> RecState {
        if !self.input_ok {
            RecState::InputLost
        } else if self.paused {
            RecState::Paused
        } else {
            RecState::Recording
        }
    }

    fn event(&mut self, kind: &str, detail: Option<String>) {
        let now = now_ist();
        let audio_ms = self.audio_ms();
        self.meta.push_event(now, audio_ms, kind, detail.clone());
        session::append_log(
            &self.dir,
            now,
            &format!("{kind} audio_ms={audio_ms} {}", detail.unwrap_or_default()),
        );
        self.save_meta();
    }

    fn save_meta(&mut self) {
        self.meta_dirty = false;
        self.last_meta_save = Instant::now();
        if let Err(e) = session::save_meta(&self.dir, &self.meta) {
            self.error(format!("Could not write session.json: {e}"));
        }
    }

    fn error(&mut self, message: String) {
        session::append_log(&self.dir, now_ist(), &format!("error {message}"));
        let _ = self.events.send(Notice::event(
            NoticeLevel::Error,
            "recorder",
            message.clone(),
        ));
        self.last_error = Some(message);
    }

    fn take_stream(&mut self) {
        if let Some(old) = self.stream.take() {
            self.overruns_before += old.counters.overrun_samples.load(Ordering::Relaxed);
            self.xruns_before += old.counters.xruns.load(Ordering::Relaxed);
            self.position_breaks_before += old.counters.position_breaks.load(Ordering::Relaxed);
            self.timestamp_outliers_before +=
                old.counters.timestamp_outliers.load(Ordering::Relaxed);
            self.pair_flags_before += old.counters.position_pair_flags.load(Ordering::Relaxed);
        }
        self.meter = None;
        self.repeat = None;
        self.dropout = None;
        self.stream_popped_frames = 0;
    }

    fn lose_input(&mut self, reason: String) {
        self.drain();
        self.drain_marks();
        self.finish_repeat();
        self.close_part();
        self.take_stream();
        if self.input_ok || self.input_lost_since.is_none() {
            self.input_ok = false;
            self.input_lost_since = Some(rfc3339(now_ist()));
            self.event("input_lost", Some(reason));
        }
    }

    fn handle(&mut self, msg: RecorderMsg) {
        match msg {
            RecorderMsg::Stream(handle) => {
                // Finish the old stream's audio in its own part.
                self.drain();
                self.drain_marks();
                self.finish_repeat();
                self.close_part();
                self.take_stream();
                self.meter = Some(LevelMeter::new(
                    handle.input.sample_rate,
                    handle.input.channels,
                    20,
                ));
                self.repeat =
                    (!self.repeat_off).then(|| RepeatDetector::new(handle.input.channels));
                self.dropout = (!self.dropout_off)
                    .then(|| DropoutDetector::new(handle.input.channels, handle.input.sample_rate));
                let restored = self.had_stream && !self.input_ok;
                let input = handle.input.clone();
                self.stream = Some(handle);
                self.rebuild_scratch();
                self.had_stream = true;
                self.input_ok = true;
                self.input_lost_since = None;
                self.last_callbacks = (0, Instant::now());
                self.meta.input = Some(input.clone());
                if restored {
                    self.event(
                        "input_restored",
                        Some(format!(
                            "{} {} Hz {} ch",
                            input.device_name, input.sample_rate, input.channels
                        )),
                    );
                } else {
                    self.save_meta();
                }
            }
            RecorderMsg::InputLost { reason } => self.lose_input(reason),
            RecorderMsg::Pause => {
                if !self.paused {
                    self.drain();
                    self.drain_marks();
                    self.finish_repeat();
                    self.close_part();
                    self.paused = true;
                    self.event("paused", None);
                }
            }
            RecorderMsg::Resume => {
                if self.paused {
                    self.paused = false;
                    self.event("resumed", None);
                }
            }
            RecorderMsg::Stop(_)
            | RecorderMsg::InjectPanic(_)
            | RecorderMsg::InjectDetectorFault(_) => {}
        }
        self.send_status();
    }

    /// While an open stream delivers nothing (the driver's callbacks stopped), mark the stream
    /// lost so the capture thread tears it down and reopens the device, and say so. Based on
    /// callbacks arriving, not on this thread draining (which pauses while writing is blocked).
    fn watch_for_silence(&mut self) {
        if !self.input_ok {
            return;
        }
        let Some(stream) = &self.stream else { return };
        let callbacks = stream.counters.callbacks.load(Ordering::Relaxed);
        if callbacks != self.last_callbacks.0 {
            self.last_callbacks = (callbacks, Instant::now());
            return;
        }
        if self.last_callbacks.1.elapsed() < NO_AUDIO_LIMIT {
            return;
        }
        stream.counters.lost_kind.store(0, Ordering::Relaxed);
        stream.counters.lost.store(true, Ordering::Release);
        let _ = self.events.send(Notice::event(
            NoticeLevel::Warning,
            "recorder",
            "No audio has arrived for 5 seconds; reopening the input",
        ));
        self.lose_input("no audio arrived for 5 seconds".into());
    }

    /// True while samples cannot be written and should stay in the ring (it holds 20 seconds).
    fn holding(&self) -> bool {
        self.writer.is_none()
            && !self.paused
            && self.input_ok
            && self.write_blocked_until.is_some_and(|t| Instant::now() < t)
    }

    /// Moves everything in the ring buffer to the current part.
    fn drain(&mut self) {
        loop {
            if self.holding() {
                return;
            }
            if self.batch_len > 0 {
                // A panic interrupted this batch: finish it first, in order.
                self.run_batch();
                continue;
            }
            if self.scratch.is_empty() {
                // Never sized (or lost): rebuild before popping.
                self.rebuild_scratch();
                if self.scratch.is_empty() {
                    return;
                }
            }
            let Some(stream) = self.stream.as_mut() else {
                return;
            };
            let occupied = stream.consumer.occupied_len();
            let fill = occupied as f32 * 100.0 / stream.capacity.max(1) as f32;
            if fill > self.max_fill_pct {
                self.max_fill_pct = fill;
            }
            if occupied > 0 && !self.audio_started_logged && self.first_queued_frames.is_none() {
                let channels = usize::from(stream.input.channels.max(1));
                self.first_queued_frames = Some((occupied / channels) as u64);
            }
            let n = stream.consumer.pop_slice(&mut self.scratch);
            if n == 0 {
                return;
            }
            self.stream_popped_frames += (n / usize::from(stream.input.channels.max(1))) as u64;
            let full = n == self.scratch.len();
            self.batch_len = n;
            self.batch_done = 0;
            self.run_batch();
            if !full {
                return;
            }
        }
    }

    /// Handles the pending batch from its cursor. A panic inside leaves the batch and cursor in
    /// place (the buffer is put back first) and then continues unwinding to the recorder loop.
    fn run_batch(&mut self) {
        let buf = std::mem::take(&mut self.scratch);
        let len = self.batch_len.min(buf.len());
        let mut done = self.batch_done.min(len);
        let result = catch_unwind(AssertUnwindSafe(|| self.process(&buf[..len], &mut done)));
        self.scratch = buf;
        self.batch_done = done;
        match result {
            Ok(()) => {
                self.batch_len = 0;
                self.batch_done = 0;
                self.batch_panics = 0;
            }
            Err(payload) => {
                self.batch_panics = self.batch_panics.saturating_add(1);
                std::panic::resume_unwind(payload)
            }
        }
    }

    /// Writes `samples` from the cursor `done` on, advancing it after each successful write.
    fn process(&mut self, samples: &[f32], done: &mut usize) {
        if let Some(PanicPoint::BeforeWriteRepeated(n)) = self.panic_point {
            self.panic_point = (n > 1).then_some(PanicPoint::BeforeWriteRepeated(n - 1));
            panic!("injected recorder panic before writing audio, repeated (test hook)");
        }
        if self.panic_point == Some(PanicPoint::BeforeWrite) {
            self.panic_point = None;
            panic!("injected recorder panic before writing audio (test hook)");
        }
        if *done == 0 {
            // Not again when a restart finishes the batch: the meter has seen it.
            if let Some(meter) = self.meter.as_mut() {
                let events = &self.events;
                meter.push(samples, |r| {
                    let _ = events.send(Event::Level(r));
                });
            }
        }
        if self.paused || !self.input_ok {
            *done = samples.len();
            return;
        }
        let Some(input) = self.stream.as_ref().map(|s| s.input.clone()) else {
            *done = samples.len();
            return;
        };
        let channels = usize::from(input.channels.max(1));
        if !self.audio_started_logged {
            // Back-date by everything that was queued in the ring when it was first drained,
            // so `audio_started` marks when the first recorded sample arrived (used to measure
            // loss in the kill test). Device latency before the callback is not included.
            self.audio_started_logged = true;
            let queued = self
                .first_queued_frames
                .unwrap_or((samples.len() / channels) as u64);
            let back = Duration::from_millis(frames_to_ms(queued, input.sample_rate));
            let at = now_ist() - back;
            let audio_ms = self.audio_ms();
            self.meta.push_event(
                at,
                audio_ms,
                "audio_started",
                Some(format!(
                    "{} {} Hz {} ch",
                    input.device_name, input.sample_rate, input.channels
                )),
            );
            session::append_log(&self.dir, at, &format!("audio_started audio_ms={audio_ms}"));
            self.save_meta();
        }
        self.write_samples(samples, done, &input, channels);
        // The block is on its way to disk (written, or carried to the next part). Only now do
        // the detectors look at it: a slow or failing detector can no longer delay the write.
        // A restart that finishes a batch gets here once, when the batch is complete.
        self.run_detectors(samples, &input);
    }

    /// Writes `samples` from the cursor `done` on into the open part, opening the next part at a
    /// rollover and carrying samples over a failed write.
    fn write_samples(
        &mut self,
        samples: &[f32],
        done: &mut usize,
        input: &InputInfo,
        channels: usize,
    ) {
        let part_frames = u64::from(self.part_seconds) * u64::from(input.sample_rate);
        while *done < samples.len() {
            let rest = &samples[*done..];
            if self.writer.is_none() {
                match self.open_part(input) {
                    OpenResult::Opened => {
                        if !self.write_carry() {
                            // The new part failed at once; write_failed has handled it.
                            continue;
                        }
                    }
                    OpenResult::Blocked => {
                        // A write just failed and the retry is not due: keep the rest of this
                        // batch (it goes into the next part, after the carried samples).
                        self.carry.extend(rest.iter().map(|&s| f32_to_i16(s)));
                        *done = samples.len();
                        return;
                    }
                    OpenResult::Failed => {
                        // The next part cannot be opened either: what the failed part owed and
                        // this batch are lost. They are counted and logged; the ring holds what
                        // follows until the retry.
                        self.dropped_samples += (self.carry.len() + rest.len()) as u64;
                        self.carry.clear();
                        *done = samples.len();
                        return;
                    }
                }
            }
            let Some(writer) = self.writer.as_mut() else {
                *done = samples.len();
                return;
            };
            let room = part_frames.saturating_sub(writer.frames()) as usize * channels;
            let take = rest.len().min(room.max(channels));
            let (now, _) = rest.split_at(take);
            if let Err(e) = writer.write_f32(now) {
                // The chunk is in the writer's pending copy: write_failed carries what did not
                // reach the disk into the next part; the rest of the batch follows it.
                *done += take;
                self.write_failed(e);
                continue;
            }
            *done += take;
            if self.panic_point == Some(PanicPoint::MidBatch) && *done < samples.len() {
                self.panic_point = None;
                panic!("injected recorder panic in the middle of a batch (test hook)");
            }
            if self.first_write_pending {
                // Header now covers the first samples: the new part plays even if the app dies
                // before the next one-second flush.
                self.first_write_pending = false;
                if let Some(Err(e)) = self.writer.as_mut().map(PartWriter::flush) {
                    self.write_failed(e);
                    continue;
                }
            }
            if self
                .writer
                .as_ref()
                .is_some_and(|w| w.frames() >= part_frames)
            {
                self.close_part();
            }
        }
    }

    fn open_part(&mut self, input: &InputInfo) -> OpenResult {
        if self.write_blocked_until.is_some_and(|t| Instant::now() < t) {
            return OpenResult::Blocked;
        }
        let path = self
            .dir
            .join(AUDIO_DIR)
            .join(parts::part_file_name(self.part_no));
        match PartWriter::create(&path, input.sample_rate, input.channels) {
            Ok(w) => {
                self.write_blocked_until = None;
                self.writer = Some(w);
                self.first_write_pending = true;
                session::append_log(
                    &self.dir,
                    now_ist(),
                    &format!(
                        "part_opened part={} start_ms={}",
                        self.part_no, self.next_start_ms
                    ),
                );
                OpenResult::Opened
            }
            Err(e) => {
                // A leftover file with this number (should not happen): skip the number.
                if path.exists() {
                    self.part_no += 1;
                }
                self.write_blocked_until = Some(Instant::now() + WRITE_RETRY);
                self.error(format!("Could not open part {}: {e}", path.display()));
                OpenResult::Failed
            }
        }
    }

    /// A write or flush failed. Nothing already captured may be lost: the part is closed with
    /// whatever reached the disk (the file is kept even if its header cannot be rewritten), the
    /// samples that did not reach it are carried into the next part, and only samples the next
    /// part also fails to take are counted in `dropped_samples`.
    fn write_failed(&mut self, e: Error) {
        self.error(format!("Write failed, starting a new part: {e}"));
        let salvage = self.writer.as_mut().map(PartWriter::take_salvage);
        let on_disk = self.close_part_inner(true);
        if let Some(s) = salvage {
            self.carry_over(&s, on_disk);
        }
        self.write_blocked_until = Some(Instant::now() + WRITE_RETRY);
        self.event("write_error", Some(e.to_string()));
    }

    /// Works out, from the frames the failed file really holds, which of the writer's pending
    /// samples still have to be written.
    fn carry_over(&mut self, s: &Salvage, on_disk: u64) {
        let ch = usize::from(s.channels.max(1));
        let pending_frames = (s.pending.len() / ch) as u64;
        let first_pending = s.accepted_frames.saturating_sub(pending_frames);
        // Frames the last successful flush covered that are not in the file: gone (counted).
        let missing_before = first_pending.saturating_sub(on_disk);
        // Pending frames that did reach the file are not written twice.
        let start = on_disk.max(first_pending);
        let skip = (start - first_pending).min(pending_frames) as usize;
        let owed = &s.pending[skip * ch..];
        // Samples at the start of this part came from an earlier failed part. If this part
        // failed to take them too, they are lost and counted.
        let carried_again = (s.carry_frames.saturating_sub(start) as usize).min(owed.len() / ch);
        let lost_frames = missing_before + carried_again as u64;
        self.dropped_samples += lost_frames * ch as u64;
        let mut next: Vec<i16> = owed[carried_again * ch..].to_vec();
        next.append(&mut self.carry);
        self.carry = next;
    }

    /// Writes the carried samples at the start of the part that was just opened. False when
    /// that write failed (`write_failed` has dealt with it).
    fn write_carry(&mut self) -> bool {
        if self.carry.is_empty() {
            return true;
        }
        let carry = std::mem::take(&mut self.carry);
        let Some(w) = self.writer.as_mut() else {
            self.carry = carry;
            return false;
        };
        match w.write_carry(&carry) {
            Ok(()) => true,
            Err(e) => {
                self.write_failed(e);
                false
            }
        }
    }

    /// At Stop: whatever a failed part still owes the recording gets one last try in a new
    /// part, and is counted as dropped if that fails too.
    fn flush_carry_at_stop(&mut self) {
        if self.carry.is_empty() {
            return;
        }
        if self.writer.is_none() {
            let input = self
                .stream
                .as_ref()
                .map(|s| s.input.clone())
                .or_else(|| self.meta.input.clone());
            if let Some(input) = input
                && matches!(self.open_part(&input), OpenResult::Opened)
            {
                self.write_carry();
            }
        }
        if !self.carry.is_empty() {
            let lost = self.carry.len();
            self.dropped_samples += lost as u64;
            self.carry.clear();
            self.error(format!(
                "{lost} samples could not be written before Stop and were dropped"
            ));
        }
    }

    fn with_writer(&mut self, f: fn(&mut PartWriter) -> Result<(), Error>) {
        if let Some(w) = self.writer.as_mut()
            && let Err(e) = f(w)
        {
            self.write_failed(e);
        }
    }

    /// Finishes the open part (if any) and queues it for indexing.
    fn close_part(&mut self) {
        self.close_part_inner(false);
    }

    /// Closes the open part and queues it for indexing; returns the whole frames the file
    /// holds. After a failed write (`after_failure`) that count comes from the file's length,
    /// not from the samples handed to the writer.
    ///
    /// A part with audio in it is never deleted: if the header cannot be finished or repaired
    /// (the fault that broke the write may still be there) the file stays as it is, is indexed
    /// with `needs_repair`, and recovery rewrites its header. Only a file with no whole frame
    /// is removed.
    fn close_part_inner(&mut self, after_failure: bool) -> u64 {
        let Some(w) = self.writer.take() else {
            return 0;
        };
        let path: PathBuf = w.path().to_path_buf();
        let part = self.part_no;
        let start_ms = self.next_start_ms;
        let (rate, channels, accepted) = (w.sample_rate(), w.channels(), w.frames());
        // Advance the number first, so a failure below never reuses it.
        self.part_no += 1;
        let mut frames = match w.finish() {
            Ok((frames, _, _)) => frames,
            Err(e) => {
                self.error(format!("Could not finish part {part}: {e}"));
                match wav::repair_header(&path) {
                    Ok(r) => r.frames,
                    Err(_) => file_frames(&path, channels).unwrap_or(accepted),
                }
            }
        };
        if after_failure {
            match file_frames(&path, channels) {
                Some(on_disk) => frames = on_disk,
                None => {
                    // The file cannot be read at all: leave it for recovery, do not guess.
                    self.error(format!(
                        "Part {part} could not be read after a write failure; it is left for recovery"
                    ));
                    return 0;
                }
            }
        }
        if frames == 0 {
            // No whole frame in the file: nothing to keep.
            let _ = std::fs::remove_file(&path);
            return 0;
        }
        self.next_start_ms += frames_to_ms(frames, rate);
        self.closer.close(CloseJob { part, start_ms });
        frames
    }

    fn check_disk(&mut self) {
        match sys::disk_free_bytes(&self.dir) {
            Ok(free) => {
                let warn = free < DISK_WARN_BYTES;
                if warn && !self.disk_warning {
                    let _ = self.events.send(Notice::event(
                        NoticeLevel::Warning,
                        "recorder",
                        format!(
                            "Only {:.1} GB free on the recording drive",
                            free as f64 / 1e9
                        ),
                    ));
                }
                self.disk_free = Some(free);
                self.disk_warning = warn;
            }
            Err(_) => self.disk_free = None,
        }
    }

    fn position_breaks(&self) -> u64 {
        self.position_breaks_before
            + self
                .stream
                .as_ref()
                .map_or(0, |s| s.counters.position_breaks.load(Ordering::Relaxed))
    }

    fn timestamp_outliers(&self) -> u64 {
        self.timestamp_outliers_before
            + self
                .stream
                .as_ref()
                .map_or(0, |s| s.counters.timestamp_outliers.load(Ordering::Relaxed))
    }

    fn position_pair_flags(&self) -> u64 {
        self.pair_flags_before
            + self.stream.as_ref().map_or(0, |s| {
                s.counters.position_pair_flags.load(Ordering::Relaxed)
            })
    }

    /// Shows a block that has been written to the repeated-block and zero-gap detectors, in
    /// slices of 10 ms of audio.
    ///
    /// The recording never waits for this and never depends on it. Each detector runs inside its
    /// own `catch_unwind`: a panic switches that detector off for the rest of the session and
    /// records one `detector_disabled` event. The time spent on a block is limited to 1/20 of the
    /// block's duration (at least 2 ms): when it runs out, the rest of the block is not shown to
    /// the detectors, they start afresh on the next block, and the skip is counted. The budget
    /// is checked between slices, so one call that hangs is not interrupted (it blocks the
    /// recorder thread; the ring holds 20 seconds).
    fn run_detectors(&mut self, samples: &[f32], input: &InputInfo) {
        if self.repeat.is_none() && self.dropout.is_none() {
            return;
        }
        let channels = usize::from(input.channels.max(1));
        let rate = input.sample_rate;
        let block_frames = samples.len() / channels;
        if block_frames == 0 {
            return;
        }
        // The block is written, so the audio clock now stands at its end.
        let origin_ms = self
            .audio_ms()
            .saturating_sub(frames_to_ms(block_frames as u64, rate));
        let budget = detector_budget(block_frames as u64, rate);
        let slice_frames = ((u64::from(rate) * DETECTOR_SLICE_MS / 1000) as usize).max(1);
        let started = Instant::now();
        let mut resynced = [false; 2];
        let mut any_skipped = false;
        let mut offset = 0usize;
        while offset < block_frames {
            let n = slice_frames.min(block_frames - offset);
            let slice = &samples[offset * channels..(offset + n) * channels];
            let slice_ms = origin_ms + frames_to_ms(offset as u64, rate);
            let out_of_time = started.elapsed() >= budget;
            let mut skipped = false;
            skipped |= self.feed_repeat_slice(slice, slice_ms, rate, out_of_time, &mut resynced[0]);
            skipped |=
                self.feed_zero_gap_slice(slice, slice_ms, rate, out_of_time, &mut resynced[1]);
            if skipped {
                any_skipped = true;
                self.detector_skipped_ms += frames_to_ms(n as u64, rate);
            }
            offset += n;
        }
        let spent = started.elapsed();
        self.detector_max_block_us = self.detector_max_block_us.max(spent.as_micros() as u64);
        if any_skipped {
            self.detector_over_budget_blocks += 1;
            let due = self
                .last_budget_log
                .is_none_or(|t| t.elapsed() >= GAP_EVENT_INTERVAL);
            if due {
                self.last_budget_log = Some(Instant::now());
                session::append_log(
                    &self.dir,
                    now_ist(),
                    &format!(
                        "detector_over_budget audio_ms={} spent_us={} budget_us={} blocks_so_far={} skipped_ms_so_far={}",
                        self.audio_ms(),
                        spent.as_micros(),
                        budget.as_micros(),
                        self.detector_over_budget_blocks,
                        self.detector_skipped_ms
                    ),
                );
            }
        }
    }

    /// One slice for the repeated-block detector. True when the slice was skipped for lack of time.
    fn feed_repeat_slice(
        &mut self,
        slice: &[f32],
        slice_ms: u64,
        rate: u32,
        out_of_time: bool,
        resynced: &mut bool,
    ) -> bool {
        let kind = DetectorKind::Repeat;
        let panic_now = self.take_panic_fault(kind);
        let delay = self.sleep_fault(kind);
        let Some(det) = self.repeat.as_mut() else {
            return false;
        };
        let first_frame = det.frames_seen();
        let mut found: Vec<RepeatedBlock> = Vec::new();
        let result = catch_unwind(AssertUnwindSafe(|| {
            if out_of_time {
                // End what the detector follows, then start afresh after the gap.
                if !*resynced {
                    *resynced = true;
                    det.flush(|b| found.push(b));
                    det.reset();
                }
                return;
            }
            *resynced = false;
            if let Some(d) = delay {
                std::thread::sleep(d);
            }
            if panic_now {
                panic!("injected detector panic (test hook)");
            }
            det.push(slice, |b| found.push(b));
        }));
        match result {
            Ok(()) => {
                for b in found {
                    self.note_repeated_block(b, first_frame, slice_ms, rate);
                }
            }
            Err(p) => self.disable_detector(kind, &panic_text(p.as_ref())),
        }
        out_of_time
    }

    /// One slice for the zero-gap detector. True when the slice was skipped for lack of time.
    fn feed_zero_gap_slice(
        &mut self,
        slice: &[f32],
        slice_ms: u64,
        rate: u32,
        out_of_time: bool,
        resynced: &mut bool,
    ) -> bool {
        let kind = DetectorKind::ZeroGap;
        let panic_now = self.take_panic_fault(kind);
        let delay = self.sleep_fault(kind);
        let Some(det) = self.dropout.as_mut() else {
            return false;
        };
        let first_frame = det.frames_seen();
        let mut found: Vec<ZeroGap> = Vec::new();
        let result = catch_unwind(AssertUnwindSafe(|| {
            if out_of_time {
                if !*resynced {
                    *resynced = true;
                    det.flush();
                    det.reset();
                }
                return;
            }
            *resynced = false;
            if let Some(d) = delay {
                std::thread::sleep(d);
            }
            if panic_now {
                panic!("injected detector panic (test hook)");
            }
            det.push(slice, |g| found.push(g));
        }));
        match result {
            Ok(()) => {
                for g in found {
                    self.note_zero_gap(g, first_frame, slice_ms, rate);
                }
            }
            Err(p) => self.disable_detector(kind, &panic_text(p.as_ref())),
        }
        out_of_time
    }

    fn take_panic_fault(&mut self, kind: DetectorKind) -> bool {
        if self.fault_panic == Some(kind) {
            self.fault_panic = None;
            true
        } else {
            false
        }
    }

    fn sleep_fault(&self, kind: DetectorKind) -> Option<Duration> {
        self.fault_sleep.filter(|(k, _)| *k == kind).map(|(_, d)| d)
    }

    /// A detector panicked: it stays off for the rest of the session, and that is recorded once.
    /// The recording itself is not touched.
    fn disable_detector(&mut self, kind: DetectorKind, msg: &str) {
        match kind {
            DetectorKind::Repeat => {
                self.repeat = None;
                self.repeat_off = true;
            }
            DetectorKind::ZeroGap => {
                self.dropout = None;
                self.dropout_off = true;
            }
        }
        self.detectors_disabled.push(kind.name().to_string());
        let _ = self.events.send(Notice::event(
            NoticeLevel::Warning,
            "recorder",
            format!(
                "The {} check stopped after an internal error and is off for the rest of this session. The recording is not affected.",
                kind.name()
            ),
        ));
        self.event(
            "detector_disabled",
            Some(format!("detector={} panic={msg}", kind.name())),
        );
    }

    /// Ends a repeat the detector is still following (stream change, pause, Stop). Call after
    /// everything drained has been written, so the audio clock and the detector agree.
    fn finish_repeat(&mut self) {
        let rate = self.stream.as_ref().map_or(0, |s| s.input.sample_rate);
        let audio_ms = self.audio_ms();
        let dropout_result = self.dropout.as_mut().map(|det| {
            catch_unwind(AssertUnwindSafe(|| {
                det.flush();
                det.reset();
            }))
        });
        if let Some(Err(p)) = dropout_result {
            self.disable_detector(DetectorKind::ZeroGap, &panic_text(p.as_ref()));
        }
        let mut found: Vec<RepeatedBlock> = Vec::new();
        let mut seen = 0u64;
        let repeat_result = self.repeat.as_mut().map(|det| {
            seen = det.frames_seen();
            catch_unwind(AssertUnwindSafe(|| {
                det.flush(|b| found.push(b));
                det.reset();
            }))
        });
        match repeat_result {
            Some(Ok(())) => {
                for b in found {
                    self.note_repeated_block(b, seen, audio_ms, rate);
                }
            }
            Some(Err(p)) => self.disable_detector(DetectorKind::Repeat, &panic_text(p.as_ref())),
            None => {}
        }
    }

    /// Records a repeated block: `session.log` always, `session.json` up to a limit, and a Toast
    /// at most once a minute. `ref_frame` and `ref_ms` are one point on the detector's frame
    /// count and the audio clock, from which the block's audio time is worked out.
    fn note_repeated_block(&mut self, b: RepeatedBlock, ref_frame: u64, ref_ms: u64, rate: u32) {
        let rate = i64::from(rate.max(1));
        let offset = b.start_frame as i64 - ref_frame as i64;
        let at_ms = (ref_ms as i64 + offset * 1000 / rate).max(0) as u64;
        self.repeated_blocks += 1;
        let detail = format!(
            "start_frame={} len_frames={} lag_frames={} level_dbfs={:.1} rate={rate}",
            b.start_frame, b.len, b.lag, b.level_dbfs
        );
        let now = now_ist();
        session::append_log(
            &self.dir,
            now,
            &format!("repeated_block audio_ms={at_ms} {detail}"),
        );
        if self.repeat_events_in_meta < MAX_REPEAT_EVENTS {
            self.repeat_events_in_meta += 1;
            self.meta
                .push_event(now, at_ms, "repeated_block", Some(detail));
            self.meta_dirty = true;
        }
        let due = self
            .last_repeat_notice
            .is_none_or(|t| t.elapsed() >= REPEAT_NOTICE_INTERVAL);
        if due {
            self.last_repeat_notice = Some(Instant::now());
            let _ = self.events.send(Notice::event(
                NoticeLevel::Warning,
                "recorder",
                "The audio input repeated a block of samples (the driver or the audio engine, not the recorder). It is recorded as received and marked in session.json.",
            ));
        }
    }

    /// Records a zero gap (a run of exact digital silence inside live audio, one WASAPI period
    /// long on the Legion): `session.log` always, `session.json` up to a limit, a Toast at most
    /// once a minute. The audio is recorded as received.
    fn note_zero_gap(&mut self, g: ZeroGap, ref_frame: u64, ref_ms: u64, rate: u32) {
        let rate = i64::from(rate.max(1));
        let offset = g.start_frame as i64 - ref_frame as i64;
        let at_ms = (ref_ms as i64 + offset * 1000 / rate).max(0) as u64;
        self.zero_gaps += 1;
        let detail = format!(
            "start_frame={} len_frames={} len_ms={:.1} rate={rate}",
            g.start_frame,
            g.len,
            f64::from(g.len) * 1000.0 / rate as f64
        );
        let now = now_ist();
        session::append_log(
            &self.dir,
            now,
            &format!("zero_gap audio_ms={at_ms} {detail}"),
        );
        if self.zero_gap_events_in_meta < MAX_REPEAT_EVENTS {
            self.zero_gap_events_in_meta += 1;
            self.meta.push_event(now, at_ms, "zero_gap", Some(detail));
            self.meta_dirty = true;
        }
        let due = self
            .last_zero_gap_notice
            .is_none_or(|t| t.elapsed() >= REPEAT_NOTICE_INTERVAL);
        if due {
            self.last_zero_gap_notice = Some(Instant::now());
            let _ = self.events.send(Notice::event(
                NoticeLevel::Warning,
                "recorder",
                "The audio input delivered a short stretch of digital silence in the middle of the sound (the driver or the audio engine, not the recorder). It is recorded as received and marked in session.json.",
            ));
        }
    }

    /// Reads the xrun and position marks the capture callbacks left and logs them with their
    /// place on the audio clock.
    fn drain_marks(&mut self) {
        let mut xruns = Vec::new();
        let mut positions = Vec::new();
        let mut rate = 0u32;
        if let Some(s) = self.stream.as_mut() {
            rate = s.input.sample_rate;
            if let Some(c) = s.xrun_log.as_mut() {
                while let Some(m) = c.try_pop() {
                    xruns.push(m);
                }
            }
            if let Some(c) = s.position_log.as_mut() {
                while let Some(m) = c.try_pop() {
                    positions.push(m);
                }
            }
        }
        if xruns.is_empty() && positions.is_empty() {
            return;
        }
        let rate = u64::from(rate.max(1));
        let now_audio = self.audio_ms();
        let popped = self.stream_popped_frames;
        let hz = sys::qpc_hz();
        let now = now_ist();
        // Frames the stream has delivered but this thread has not taken yet are behind `popped`:
        // an estimate of the mark's place on the audio clock, good to the ring's read-ahead.
        let place = |frames: u64| -> u64 {
            let behind = popped.saturating_sub(frames);
            now_audio.saturating_sub(behind * 1000 / rate)
        };
        for m in xruns {
            self.xrun_seq += 1;
            let at_ms = place(m.frames);
            let detail = format!(
                "cause=xrun n={} callback={} frames={} audio_ms={at_ms} qpc={} qpc_hz={hz}",
                self.xrun_seq, m.callbacks, m.frames, m.qpc
            );
            session::append_log(&self.dir, now, &format!("gap {detail}"));
            if self.xrun_events_in_meta < MAX_XRUN_EVENTS {
                self.xrun_events_in_meta += 1;
                self.meta.push_event(now, at_ms, "gap", Some(detail));
                self.meta_dirty = true;
            }
        }
        for m in positions {
            let at_ms = place(m.frames);
            let kind = match m.kind {
                PositionKind::Break => "position_break",
                PositionKind::Outlier => "timestamp_outlier",
                PositionKind::PairFlag => "position_pair_flag",
            };
            let detail = format!(
                "{kind} callback={} frames={} audio_ms={at_ms} ts_ns={} dev_ns={} pair_diff_ns={}",
                m.callback, m.frames, m.ts_ns, m.dev_ns, m.pair_diff_ns
            );
            session::append_log(&self.dir, now, &detail);
            if m.kind == PositionKind::Break && self.xrun_events_in_meta < MAX_XRUN_EVENTS {
                self.xrun_events_in_meta += 1;
                self.meta
                    .push_event(now, at_ms, "gap", Some(format!("cause={detail}")));
                self.meta_dirty = true;
            }
        }
    }

    fn counters(&self) -> (u64, u64) {
        let (o, x) = self.stream.as_ref().map_or((0, 0), |s| {
            (
                s.counters.overrun_samples.load(Ordering::Relaxed),
                s.counters.xruns.load(Ordering::Relaxed),
            )
        });
        (self.overruns_before + o, self.xruns_before + x)
    }

    /// Logs a `gap` event whenever overruns, driver xruns or dropped samples grow, so the place
    /// where audio was lost is on record at its audio time (at most once a second).
    fn log_counters(&mut self) {
        let (overruns, xruns) = self.counters();
        let dropped = self.dropped_samples;
        if (overruns, xruns, dropped)
            == (self.logged_overruns, self.logged_xruns, self.logged_dropped)
            || self.last_counter_event.elapsed() < COUNTER_EVENT_INTERVAL
        {
            return;
        }
        let detail = format!(
            "overrun_samples+{} xruns+{} dropped_samples+{} (totals {overruns}, {xruns}, {dropped})",
            overruns - self.logged_overruns,
            xruns - self.logged_xruns,
            dropped - self.logged_dropped
        );
        self.logged_overruns = overruns;
        self.logged_xruns = xruns;
        self.logged_dropped = dropped;
        self.last_counter_event = Instant::now();
        let _ = self.events.send(Notice::event(
            NoticeLevel::Warning,
            "recorder",
            format!("Audio was lost: {detail}"),
        ));
        // session.log gets every one; session.json (rewritten whole) at most every 30 s.
        let audio_ms = self.audio_ms();
        session::append_log(
            &self.dir,
            now_ist(),
            &format!("gap audio_ms={audio_ms} {detail}"),
        );
        if self
            .last_gap_event
            .is_none_or(|t| t.elapsed() >= GAP_EVENT_INTERVAL)
        {
            self.last_gap_event = Some(Instant::now());
            self.meta
                .push_event(now_ist(), audio_ms, "gap", Some(detail));
            self.save_meta();
        }
    }

    fn status(&self, state: RecState) -> RecorderStatus {
        let (overrun_samples, xruns) = self.counters();
        RecorderStatus {
            state,
            session_dir: self.dir.to_string_lossy().into_owned(),
            audio_ms: self.audio_ms(),
            part: self.writer.as_ref().map(|_| self.part_no),
            part_audio_ms: self
                .writer
                .as_ref()
                .map_or(0, |w| frames_to_ms(w.frames(), w.sample_rate())),
            part_seconds: self.part_seconds,
            overrun_samples,
            xruns,
            dropped_samples: self.dropped_samples,
            recorder_restarts: self.restarts,
            position_breaks: self.position_breaks(),
            timestamp_outliers: self.timestamp_outliers(),
            position_pair_flags: self.position_pair_flags(),
            repeated_blocks: self.repeated_blocks,
            zero_gaps: self.zero_gaps,
            ring_seconds: RING_SECONDS,
            ring_max_fill_pct: self.max_fill_pct,
            priority: self.priority.mode,
            priority_note: self.priority.fallback_reason.clone(),
            capture_buffer: self.stream.as_ref().map(|s| s.buffer.to_string()),
            capture_priority: self
                .stream
                .as_ref()
                .map(|s| s.counters.capture_priority_name().to_string()),
            awake: self.awake.active,
            disk_free_bytes: self.disk_free,
            disk_warning: self.disk_warning,
            input: self
                .stream
                .as_ref()
                .map(|s| s.input.clone())
                .or_else(|| self.meta.input.clone()),
            input_lost_since: self.input_lost_since.clone(),
            thread_cpu_ms: sys::thread_cpu_ms(),
            last_error: self.last_error.clone(),
        }
    }

    fn send_status(&self) {
        let _ = self
            .events
            .send(Event::RecorderStatus(Box::new(self.status(self.state()))));
    }

    fn stop(&mut self) -> Result<StopReport, Error> {
        // A write failure may have the ring on hold: try again now rather than lose what it holds.
        self.write_blocked_until = None;
        self.drain();
        self.flush_carry_at_stop();
        self.close_part();
        self.drain_marks();
        self.finish_repeat();
        self.log_counters_now();
        let (overrun_samples, xruns) = self.counters();
        let position_breaks = self.position_breaks();
        let timestamp_outliers = self.timestamp_outliers();
        let position_pair_flags = self.position_pair_flags();
        let repeated_blocks = self.repeated_blocks;
        let zero_gaps = self.zero_gaps;
        let cpu = sys::thread_cpu_ms();
        let final_status = self.status(RecState::Stopped);
        self.take_stream();
        let index = self.closer.finish().unwrap_or_default();
        let audio_ms = self.next_start_ms;
        self.meta.closed_at = Some(rfc3339(now_ist()));
        self.event(
            "closed",
            Some(format!(
                "overrun_samples={overrun_samples} xruns={xruns} dropped_samples={} recorder_restarts={} recorder_cpu_ms={} detector_over_budget_blocks={} detector_skipped_ms={} detector_max_block_us={} detectors_disabled={}",
                self.dropped_samples,
                self.restarts,
                cpu.unwrap_or(0),
                self.detector_over_budget_blocks,
                self.detector_skipped_ms,
                self.detector_max_block_us,
                self.detectors_disabled.len()
            )),
        );
        let manifest_files = session::write_manifest(&self.dir)?;
        let _ = self
            .events
            .send(Event::RecorderStatus(Box::new(final_status)));
        Ok(StopReport {
            session_dir: self.dir.to_string_lossy().into_owned(),
            parts: index.parts,
            audio_ms,
            overrun_samples,
            xruns,
            position_breaks,
            timestamp_outliers,
            position_pair_flags,
            repeated_blocks,
            zero_gaps,
            dropped_samples: self.dropped_samples,
            recorder_restarts: self.restarts,
            recorder_cpu_ms: cpu,
            manifest_files,
            detector_over_budget_blocks: self.detector_over_budget_blocks,
            detector_skipped_ms: self.detector_skipped_ms,
            detector_max_block_us: self.detector_max_block_us,
            detectors_disabled: self.detectors_disabled.clone(),
        })
    }

    /// `log_counters` without the once-a-second limit (at Stop).
    fn log_counters_now(&mut self) {
        self.last_gap_event = None;
        self.last_counter_event = Instant::now()
            .checked_sub(COUNTER_EVENT_INTERVAL)
            .unwrap_or_else(Instant::now);
        self.log_counters();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_detector_budget_is_a_twentieth_of_the_block_with_a_floor() {
        // A usual batch: half a second at 48 kHz gives 25 ms.
        assert_eq!(detector_budget(24_000, 48_000), Duration::from_millis(25));
        // A 10 ms block would give 0.5 ms; the floor is 2 ms.
        assert_eq!(detector_budget(480, 48_000), Duration::from_millis(2));
        // A zero rate does not divide by zero.
        assert!(detector_budget(100, 0) >= DETECTOR_BUDGET_MIN);
    }
}
