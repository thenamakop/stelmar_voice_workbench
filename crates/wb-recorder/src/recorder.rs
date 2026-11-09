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
use crate::capture::{RING_SECONDS, StreamHandle};
use crate::events::{Event, Notice, NoticeLevel, RecState, RecorderStatus};
use crate::level::LevelMeter;
use crate::parts::{self, AUDIO_DIR, PartRecord, PartsIndex};
use crate::session::{self, InputInfo, SessionMeta};
use crate::sys::{self, AwakeGuard};
use crate::util::{now_ist, rfc3339};
use crate::wav::{PartWriter, frames_to_ms};

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
    /// Capture callbacks whose device time did not follow on from the previous one.
    pub position_breaks: u64,
    /// Samples taken from the ring that could not be written (disk full, drive gone).
    pub dropped_samples: u64,
    /// Panics caught inside the recorder thread.
    pub recorder_restarts: u32,
    pub recorder_cpu_ms: Option<u64>,
    pub manifest_files: usize,
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
                for job in rx {
                    match parts::record_for_file(&dir, job.part, job.start_ms) {
                        Ok(rec) => {
                            index.parts.push(rec.clone());
                            if let Err(e) = parts::save(&dir, &index) {
                                let _ = events.send(Notice::event(
                                    NoticeLevel::Error,
                                    "recorder",
                                    format!("Could not write parts.json: {e}"),
                                ));
                            }
                            session::append_log(
                                &dir,
                                now_ist(),
                                &format!(
                                    "part_closed part={} start_ms={} duration_ms={} sha256={}",
                                    rec.part, rec.start_ms, rec.duration_ms, rec.sha256
                                ),
                            );
                            let _ = events.send(Event::PartClosed(rec));
                        }
                        Err(e) => {
                            let _ = events.send(Notice::event(
                                NoticeLevel::Error,
                                "recorder",
                                format!("Could not index part {}: {e}", job.part),
                            ));
                        }
                    }
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
                let msg = payload
                    .downcast_ref::<&str>()
                    .map(|s| (*s).to_string())
                    .or_else(|| payload.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "unknown panic".into());
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
        self.watch_for_silence();
        let now = Instant::now();
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
        }
        self.meter = None;
    }

    fn lose_input(&mut self, reason: String) {
        self.drain();
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
                self.close_part();
                self.take_stream();
                self.meter = Some(LevelMeter::new(
                    handle.input.sample_rate,
                    handle.input.channels,
                    20,
                ));
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
            RecorderMsg::Stop(_) | RecorderMsg::InjectPanic(_) => {}
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
        let part_frames = u64::from(self.part_seconds) * u64::from(input.sample_rate);
        while *done < samples.len() {
            let rest = &samples[*done..];
            if self.writer.is_none() && !self.open_part(&input) {
                // Could not open a part: this batch is lost. It is counted and logged; the ring
                // holds what follows until the retry.
                self.dropped_samples += rest.len() as u64;
                *done = samples.len();
                return;
            }
            let Some(writer) = self.writer.as_mut() else {
                *done = samples.len();
                return;
            };
            let room = part_frames.saturating_sub(writer.frames()) as usize * channels;
            let take = rest.len().min(room.max(channels));
            let (now, _) = rest.split_at(take);
            if let Err(e) = writer.write_f32(now) {
                self.dropped_samples += rest.len() as u64;
                *done = samples.len();
                self.write_failed(e);
                return;
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
                    return;
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

    fn open_part(&mut self, input: &InputInfo) -> bool {
        if self.write_blocked_until.is_some_and(|t| Instant::now() < t) {
            return false;
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
                true
            }
            Err(e) => {
                // A leftover file with this number (should not happen): skip the number.
                if path.exists() {
                    self.part_no += 1;
                }
                self.write_blocked_until = Some(Instant::now() + WRITE_RETRY);
                self.error(format!("Could not open part {}: {e}", path.display()));
                false
            }
        }
    }

    fn write_failed(&mut self, e: Error) {
        self.error(format!("Write failed, starting a new part: {e}"));
        self.close_part();
        self.write_blocked_until = Some(Instant::now() + WRITE_RETRY);
        self.event("write_error", Some(e.to_string()));
    }

    fn with_writer(&mut self, f: fn(&mut PartWriter) -> Result<(), Error>) {
        if let Some(w) = self.writer.as_mut() {
            if let Err(e) = f(w) {
                self.write_failed(e);
            }
        }
    }

    /// Finishes the open part (if any) and queues it for indexing.
    fn close_part(&mut self) {
        let Some(w) = self.writer.take() else { return };
        let path: PathBuf = w.path().to_path_buf();
        let part = self.part_no;
        let start_ms = self.next_start_ms;
        // Advance the number first, so a failure below never reuses it.
        self.part_no += 1;
        let frames = match w.finish() {
            Ok((frames, rate, _)) => {
                self.next_start_ms += frames_to_ms(frames, rate);
                frames
            }
            Err(e) => {
                self.error(format!("Could not finish part {part}: {e}"));
                // The data that reached the file is still indexed; recovery repairs the header.
                let info = crate::wav::repair_header(&path).ok();
                let frames = info.as_ref().map_or(0, |r| r.frames);
                if let Some(r) = info {
                    self.next_start_ms += frames_to_ms(r.frames, r.sample_rate);
                }
                frames
            }
        };
        if frames == 0 {
            let _ = std::fs::remove_file(&path);
            return;
        }
        self.closer.close(CloseJob { part, start_ms });
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
        self.drain();
        self.close_part();
        self.log_counters_now();
        let (overrun_samples, xruns) = self.counters();
        let position_breaks = self.position_breaks();
        let cpu = sys::thread_cpu_ms();
        let final_status = self.status(RecState::Stopped);
        self.take_stream();
        let index = self.closer.finish().unwrap_or_default();
        let audio_ms = self.next_start_ms;
        self.meta.closed_at = Some(rfc3339(now_ist()));
        self.event(
            "closed",
            Some(format!(
                "overrun_samples={overrun_samples} xruns={xruns} dropped_samples={} recorder_restarts={} recorder_cpu_ms={}",
                self.dropped_samples,
                self.restarts,
                cpu.unwrap_or(0)
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
            dropped_samples: self.dropped_samples,
            recorder_restarts: self.restarts,
            recorder_cpu_ms: cpu,
            manifest_files,
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
