//! Audio capture through cpal (WASAPI shared mode on Windows).
//!
//! The data callback only copies whole frames into a lock-free ring buffer and counts what did
//! not fit. It never allocates, locks, logs or does I/O (AGENTS.md, audio callback). A capture
//! thread owns the stream, watches for device loss and reopens the device every 2 seconds; each
//! new stream reaches the recorder with its own ring buffer, so the recorder starts a new part.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{DeviceId, ErrorKind, SampleFormat, StreamConfig};
use ringbuf::traits::{Observer, Producer, Split};
use ringbuf::{HeapCons, HeapProd, HeapRb};
use serde::Serialize;

use crate::Error;
use crate::events::{Event, Notice, NoticeLevel};
use crate::recorder::RecorderMsg;
use crate::session::InputInfo;

/// Seconds of audio the ring buffer holds (the prompt asks for at least 10).
pub const RING_SECONDS: u32 = 20;
/// How often a lost device is retried.
pub const RETRY_INTERVAL: Duration = Duration::from_secs(2);
const PREFERRED_RATE: u32 = 48_000;

/// Counters shared between the audio callback, the error callback and the recorder.
#[derive(Debug, Default)]
pub struct StreamCounters {
    /// Samples dropped because the ring buffer was full.
    pub overrun_samples: AtomicU64,
    /// Glitches the driver reported (cpal `ErrorKind::Xrun`).
    pub xruns: AtomicU64,
    /// The device went away or the stream must be rebuilt.
    pub lost: AtomicBool,
    /// `ErrorKind` of the last fatal error, as `lost_reason` decodes it.
    pub lost_kind: AtomicU8,
    /// Callbacks received (a stream that never calls back is reported as lost).
    pub callbacks: AtomicU64,
    /// Priority of the audio driver's capture thread: 0 not yet known, 1 MMCSS "Pro Audio",
    /// 2 THREAD_PRIORITY_HIGHEST, 3 normal.
    pub capture_priority: AtomicU8,
    /// Capture position breaks: the device capture time left the line the frames delivered so
    /// far predict and stayed off it for [`BREAK_CALLBACKS`] callbacks in a row (a jump or a
    /// repeat on the capture side, even when the driver reports no xrun).
    pub position_breaks: AtomicU64,
    /// Single timestamps (or runs shorter than a break) that left the line and came back: jitter
    /// in the timestamp, not in the audio. Reported, never gated.
    pub timestamp_outliers: AtomicU64,
    /// What the old pairwise check would have counted: each callback compared with the one before
    /// it, so one outlier counts twice (out and back). Kept for the diagnostic only.
    pub position_pair_flags: AtomicU64,
    /// Frames the data callback has delivered (including any it could not fit in the ring).
    pub frames: AtomicU64,
}

/// A driver xrun, as the error callback saw it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XrunMark {
    /// Data callbacks received before the error callback ran.
    pub callbacks: u64,
    /// Frames delivered before it.
    pub frames: u64,
    /// `QueryPerformanceCounter` ticks when the error callback ran (0 where unavailable).
    pub qpc: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PositionKind {
    /// Counted in `position_breaks`.
    Break,
    /// Counted in `timestamp_outliers`.
    Outlier,
    /// Counted in `position_pair_flags`.
    PairFlag,
}

/// A position-check finding, for session.log and the diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PositionMark {
    pub kind: PositionKind,
    /// Index of the data callback it was found in (0 is the first).
    pub callback: u64,
    /// Frames delivered before that callback.
    pub frames: u64,
    /// Capture time of that callback in ns since the first callback.
    pub ts_ns: i64,
    /// How far it was from the line the earlier frames predict (the anchored check), in ns.
    pub dev_ns: i64,
    /// How far it was from the previous callback's timestamp plus that callback's frames (the old
    /// pairwise check), in ns.
    pub pair_diff_ns: i64,
}

/// Callbacks in a row that must stay off the predicted line for it to be a break.
pub const BREAK_CALLBACKS: u8 = 3;
const MIN_TOLERANCE_NS: i64 = 2_000_000;

/// What the anchored check made of one callback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PositionVerdict {
    /// On the line.
    Ok,
    /// Off the line; not yet known whether it comes back.
    Deviating,
    /// Off the line for [`BREAK_CALLBACKS`] callbacks in a row: the audio clock moved.
    Break { dev_ns: i64 },
    /// Was off the line for `callbacks` callbacks (fewer than a break) and is back on it.
    Outlier { dev_ns: i64, callbacks: u8 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PositionStep {
    pub verdict: PositionVerdict,
    /// This callback's distance from the predicted line, in ns.
    pub dev_ns: i64,
    /// Set when the old pairwise check would have flagged this callback (its distance, in ns).
    pub pair_flag: Option<i64>,
}

/// Checks that the capture timestamps follow the frames delivered.
///
/// The pairwise check this replaces compared each callback with the one before it, so a single
/// late timestamp was flagged twice (out and back) although no audio was affected: the 10 Nov
/// 2026 runs counted 8 breaks (webview 2, load 2, soak 4) while the recordings equalled the
/// reference capture exactly. This one keeps the line the delivered frames predict. A callback
/// that is back on the line after a short excursion is an outlier; one that stays off it for
/// [`BREAK_CALLBACKS`] callbacks in a row is a break and the line is re-drawn from there. While
/// callbacks are on the line the prediction is re-anchored to each one, so a slow clock drift
/// never accumulates. Arithmetic only: safe for the audio callback.
#[derive(Debug, Clone)]
pub struct PositionTracker {
    rate: u64,
    /// Where the next callback should be, in ns since the first one.
    expected: Option<i64>,
    pending: u8,
    pending_dev: i64,
    /// The previous callback, for the old pairwise check: (timestamp ns, frames).
    prev: Option<(i64, u64)>,
}

impl PositionTracker {
    pub fn new(rate: u32) -> Self {
        Self {
            rate: u64::from(rate.max(1)),
            expected: None,
            pending: 0,
            pending_dev: 0,
            prev: None,
        }
    }

    fn frames_ns(&self, frames: u64) -> i64 {
        (u128::from(frames) * 1_000_000_000 / u128::from(self.rate)) as i64
    }

    /// `ts_ns` is the callback's capture time in ns since the first callback; `frames` how many
    /// frames it delivered.
    #[inline]
    pub fn check(&mut self, ts_ns: i64, frames: u64) -> PositionStep {
        let period = self.frames_ns(frames);
        let tol = MIN_TOLERANCE_NS.max(period / 4);

        let mut pair_flag = None;
        if let Some((prev_ts, prev_frames)) = self.prev {
            let prev_period = self.frames_ns(prev_frames);
            let diff = ts_ns - prev_ts - prev_period;
            if diff.abs() > MIN_TOLERANCE_NS && diff.abs() > prev_period / 4 {
                pair_flag = Some(diff);
            }
        }
        self.prev = Some((ts_ns, frames));

        let Some(expected) = self.expected else {
            self.expected = Some(ts_ns + period);
            return PositionStep {
                verdict: PositionVerdict::Ok,
                dev_ns: 0,
                pair_flag,
            };
        };
        let dev = ts_ns - expected;
        let verdict = if dev.abs() <= tol {
            let v = if self.pending > 0 {
                PositionVerdict::Outlier {
                    dev_ns: self.pending_dev,
                    callbacks: self.pending,
                }
            } else {
                PositionVerdict::Ok
            };
            self.pending = 0;
            self.expected = Some(ts_ns + period);
            v
        } else if self.pending > 0 && (dev - self.pending_dev).abs() <= tol {
            self.pending += 1;
            if self.pending >= BREAK_CALLBACKS {
                self.pending = 0;
                self.expected = Some(ts_ns + period);
                PositionVerdict::Break { dev_ns: dev }
            } else {
                self.expected = Some(expected + period);
                PositionVerdict::Deviating
            }
        } else {
            // A first excursion, or one that differs from the pending one: the pending one was
            // an outlier that did not come back to the line before the next one began.
            let v = if self.pending > 0 {
                PositionVerdict::Outlier {
                    dev_ns: self.pending_dev,
                    callbacks: self.pending,
                }
            } else {
                PositionVerdict::Deviating
            };
            self.pending = 1;
            self.pending_dev = dev;
            self.expected = Some(expected + period);
            v
        };
        PositionStep {
            verdict,
            dev_ns: dev,
            pair_flag,
        }
    }
}

/// Wraps [`PositionTracker`] for the data callback: converts the stream timestamp, counts, and
/// leaves a mark in a ring for the recorder thread. Allocation-free.
struct PositionCheck {
    tracker: PositionTracker,
    first: Option<cpal::StreamInstant>,
    marks: HeapProd<PositionMark>,
}

impl PositionCheck {
    fn new(rate: u32, marks: HeapProd<PositionMark>) -> Self {
        Self {
            tracker: PositionTracker::new(rate),
            first: None,
            marks,
        }
    }

    #[inline]
    fn check(&mut self, info: &cpal::InputCallbackInfo, frames: u64, counters: &StreamCounters) {
        let ts = info.timestamp().capture;
        let first = *self.first.get_or_insert(ts);
        let ts_ns = if ts >= first {
            ts.saturating_duration_since(first).as_nanos() as i64
        } else {
            -(first.saturating_duration_since(ts).as_nanos() as i64)
        };
        let callback = counters.callbacks.load(Ordering::Relaxed);
        let delivered = counters.frames.load(Ordering::Relaxed);
        let step = self.tracker.check(ts_ns, frames);
        let mut mark = |kind: PositionKind, counter: &AtomicU64, dev_ns: i64, pair_diff_ns: i64| {
            counter.fetch_add(1, Ordering::Relaxed);
            let _ = self.marks.try_push(PositionMark {
                kind,
                callback,
                frames: delivered,
                ts_ns,
                dev_ns,
                pair_diff_ns,
            });
        };
        let pair = step.pair_flag.unwrap_or(0);
        match step.verdict {
            PositionVerdict::Break { dev_ns } => {
                mark(PositionKind::Break, &counters.position_breaks, dev_ns, pair)
            }
            PositionVerdict::Outlier { dev_ns, .. } => {
                mark(PositionKind::Outlier, &counters.timestamp_outliers, dev_ns, pair)
            }
            PositionVerdict::Ok | PositionVerdict::Deviating => {}
        }
        if let Some(diff) = step.pair_flag {
            mark(
                PositionKind::PairFlag,
                &counters.position_pair_flags,
                step.dev_ns,
                diff,
            );
        }
    }
}

impl StreamCounters {
    pub fn capture_priority_name(&self) -> &'static str {
        match self.capture_priority.load(Ordering::Relaxed) {
            1 => "mmcss",
            2 => "highest",
            3 => "normal",
            _ => "unknown",
        }
    }

    pub fn lost_reason(&self) -> &'static str {
        match self.lost_kind.load(Ordering::Relaxed) {
            1 => "the device was disconnected",
            2 => "the stream became invalid (format or route changed)",
            3 => "the audio driver reported an error",
            4 => "the device is busy",
            5 => "the audio route changed",
            _ => "the audio stream stopped",
        }
    }
}

/// One open stream as the recorder sees it.
pub struct StreamHandle {
    pub consumer: HeapCons<f32>,
    pub input: InputInfo,
    pub counters: Arc<StreamCounters>,
    /// Ring buffer size in samples.
    pub capacity: usize,
    /// Device buffer the stream opened with: "fixed 100 ms" or "device default".
    pub buffer: &'static str,
    /// Driver xruns with their position, left by the error callback (none for a manual handle).
    pub xrun_log: Option<HeapCons<XrunMark>>,
    /// Position-check findings left by the data callback (none for a manual handle).
    pub position_log: Option<HeapCons<PositionMark>>,
}

impl StreamHandle {
    /// A handle fed by the caller instead of a device (tests and the panic-isolation check).
    pub fn manual(input: InputInfo, seconds: u32) -> (HeapProd<f32>, Self) {
        let capacity = (input.sample_rate * u32::from(input.channels) * seconds) as usize;
        let (prod, consumer) = HeapRb::<f32>::new(capacity).split();
        (
            prod,
            Self {
                consumer,
                input,
                counters: Arc::new(StreamCounters::default()),
                capacity,
                buffer: "manual",
                xrun_log: None,
                position_log: None,
            },
        )
    }
}

/// What `StreamHandle::manual_with_marks` returns besides the handle: producers for the audio
/// ring and for the xrun and position rings (tests of the recorder's mark logging).
#[doc(hidden)]
pub struct ManualProducers {
    pub audio: HeapProd<f32>,
    pub xruns: HeapProd<XrunMark>,
    pub positions: HeapProd<PositionMark>,
}

impl StreamHandle {
    /// Like [`StreamHandle::manual`], with xrun and position rings the caller can fill.
    #[doc(hidden)]
    pub fn manual_with_marks(input: InputInfo, seconds: u32) -> (ManualProducers, Self) {
        let (audio, mut handle) = Self::manual(input, seconds);
        let (xruns, xrun_cons) = HeapRb::<XrunMark>::new(MARK_RING).split();
        let (positions, position_cons) = HeapRb::<PositionMark>::new(MARK_RING).split();
        handle.xrun_log = Some(xrun_cons);
        handle.position_log = Some(position_cons);
        (
            ManualProducers {
                audio,
                xruns,
                positions,
            },
            handle,
        )
    }
}

/// Slots in the xrun and position rings: far more than a healthy stream ever leaves between two
/// looks of the recorder thread (about 4 a second); a full ring drops the mark, never blocks.
const MARK_RING: usize = 256;

/// An input device for the device picker.
#[derive(Debug, Clone, Serialize)]
pub struct InputDevice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
    /// The configuration a session would open: 48 kHz when the device offers it.
    pub sample_rate: u32,
    pub channels: u16,
    pub sample_format: String,
}

fn host() -> cpal::Host {
    cpal::default_host()
}

fn device_id_string(device: &cpal::Device) -> String {
    device
        .id()
        .map(|id| id.to_string())
        .unwrap_or_else(|_| device.to_string())
}

fn is_supported_format(f: SampleFormat) -> bool {
    matches!(f, SampleFormat::F32 | SampleFormat::I16 | SampleFormat::I32)
}

fn format_rank(f: SampleFormat) -> u8 {
    match f {
        SampleFormat::F32 => 3,
        SampleFormat::I32 => 2,
        SampleFormat::I16 => 1,
        _ => 0,
    }
}

/// Picks the input configuration: 48 kHz if any range offers it (channels as the device's
/// default, float preferred), otherwise the device's default rate.
fn choose_config(device: &cpal::Device) -> Result<(StreamConfig, SampleFormat), Error> {
    let default = device
        .default_input_config()
        .map_err(|e| Error::Device(format!("{device}: {e}")))?;
    let mut best: Option<(u8, cpal::SupportedStreamConfig)> = None;
    if let Ok(ranges) = device.supported_input_configs() {
        for range in ranges {
            if !is_supported_format(range.sample_format()) || !range.contains_rate(PREFERRED_RATE) {
                continue;
            }
            let score = format_rank(range.sample_format())
                + if range.channels() == default.channels() {
                    10
                } else {
                    0
                };
            if best.as_ref().is_none_or(|(s, _)| score > *s) {
                best = Some((score, range.with_sample_rate(PREFERRED_RATE)));
            }
        }
    }
    let chosen = match best {
        Some((_, cfg)) => cfg,
        None if is_supported_format(default.sample_format()) => default,
        None => {
            return Err(Error::Device(format!(
                "{device}: sample format {:?} is not supported",
                default.sample_format()
            )));
        }
    };
    Ok((chosen.config(), chosen.sample_format()))
}

/// Lists audio inputs with the configuration a session would use.
pub fn list_inputs() -> Result<Vec<InputDevice>, Error> {
    let host = host();
    let default_id = host.default_input_device().map(|d| device_id_string(&d));
    let devices = host
        .input_devices()
        .map_err(|e| Error::Device(e.to_string()))?;
    let mut out = Vec::new();
    for device in devices {
        let Ok((cfg, fmt)) = choose_config(&device) else {
            continue;
        };
        let id = device_id_string(&device);
        out.push(InputDevice {
            is_default: default_id.as_deref() == Some(id.as_str()),
            name: device.to_string(),
            id,
            sample_rate: cfg.sample_rate,
            channels: cfg.channels,
            sample_format: format!("{fmt:?}").to_lowercase(),
        });
    }
    out.sort_by(|a, b| {
        b.is_default
            .cmp(&a.is_default)
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(out)
}

/// Finds a device by the id `list_inputs` reported, then by its exact name (an interface
/// plugged back into another USB port comes back with a new endpoint id).
fn find_device(id: &str, name: Option<&str>) -> Option<cpal::Device> {
    let host = host();
    if let Ok(parsed) = id.parse::<DeviceId>() {
        if let Some(d) = host.device_by_id(&parsed) {
            return Some(d);
        }
    }
    let devices: Vec<cpal::Device> = host.input_devices().ok()?.collect();
    if let Some(d) = devices.iter().find(|d| device_id_string(d) == id) {
        return Some(d.clone());
    }
    let name = name?;
    devices.into_iter().find(|d| d.to_string() == name)
}

/// On the first callback only: raises the driver's capture thread (cpal's WASAPI thread, which
/// must drain the device in time) to MMCSS "Pro Audio", as the recorder thread is. This one-time
/// registration is the single exception to "the callback only copies samples": every later call
/// returns at the first line. The guard is leaked on purpose; the registration ends with the
/// thread, when the stream closes.
#[inline]
fn register_capture_thread(done: &mut bool, counters: &StreamCounters) {
    if *done {
        return;
    }
    *done = true;
    let guard = crate::sys::raise_recorder_priority();
    let code = match guard.mode {
        crate::sys::PriorityMode::Mmcss => 1,
        crate::sys::PriorityMode::Highest => 2,
        crate::sys::PriorityMode::Normal => 3,
    };
    counters.capture_priority.store(code, Ordering::Relaxed);
    std::mem::forget(guard);
}

/// Copies whole frames into the ring; counts the rest as overrun. No allocation, locks or I/O.
#[inline]
fn push_frames<T: Copy>(
    prod: &mut HeapProd<f32>,
    data: &[T],
    channels: usize,
    conv: impl Fn(T) -> f32,
    counters: &StreamCounters,
) {
    let vacant = prod.vacant_len();
    let fit = data.len().min(vacant - vacant % channels);
    let pushed = prod.push_iter(data[..fit].iter().map(|&s| conv(s)));
    let dropped = data.len() - pushed;
    if dropped > 0 {
        counters
            .overrun_samples
            .fetch_add(dropped as u64, Ordering::Relaxed);
    }
    counters
        .frames
        .fetch_add((data.len() / channels) as u64, Ordering::Relaxed);
    counters.callbacks.fetch_add(1, Ordering::Relaxed);
}

/// Opens the device with `buffer` (a fixed size, or the device default).
fn try_open(
    device: &cpal::Device,
    base: StreamConfig,
    format: SampleFormat,
    buffer: cpal::BufferSize,
    buffer_label: &'static str,
) -> Result<(cpal::Stream, StreamHandle), Error> {
    let config = StreamConfig {
        buffer_size: buffer,
        ..base
    };
    let channels = usize::from(config.channels.max(1));
    let capacity = (config.sample_rate as usize) * channels * RING_SECONDS as usize;
    let (mut prod, consumer) = HeapRb::<f32>::new(capacity).split();
    let counters = Arc::new(StreamCounters::default());

    let (mut xrun_prod, xrun_cons) = HeapRb::<XrunMark>::new(MARK_RING).split();
    let (position_prod, position_cons) = HeapRb::<PositionMark>::new(MARK_RING).split();
    let err_counters = Arc::clone(&counters);
    let on_error = move |err: cpal::Error| {
        let kind = match err.kind() {
            ErrorKind::Xrun => {
                // Where it happened: the data callbacks and frames before it, and the clock.
                let mark = XrunMark {
                    callbacks: err_counters.callbacks.load(Ordering::Relaxed),
                    frames: err_counters.frames.load(Ordering::Relaxed),
                    qpc: crate::sys::qpc_ticks(),
                };
                err_counters.xruns.fetch_add(1, Ordering::Relaxed);
                let _ = xrun_prod.try_push(mark);
                return;
            }
            ErrorKind::DeviceNotAvailable => 1,
            ErrorKind::StreamInvalidated => 2,
            ErrorKind::BackendError => 3,
            ErrorKind::DeviceBusy => 4,
            ErrorKind::DeviceChanged => 5,
            _ => 6,
        };
        err_counters.lost_kind.store(kind, Ordering::Relaxed);
        err_counters.lost.store(true, Ordering::Release);
    };

    let cb = Arc::clone(&counters);
    let mut registered = false;
    let mut position = PositionCheck::new(config.sample_rate, position_prod);
    let timeout = Some(Duration::from_secs(5));
    let stream = match format {
        SampleFormat::F32 => device.build_input_stream::<f32, _, _>(
            config,
            move |data: &[f32], info| {
                register_capture_thread(&mut registered, &cb);
                position.check(info, (data.len() / channels) as u64, &cb);
                push_frames(&mut prod, data, channels, |s| s, &cb)
            },
            on_error,
            timeout,
        ),
        SampleFormat::I16 => device.build_input_stream::<i16, _, _>(
            config,
            move |data: &[i16], info| {
                register_capture_thread(&mut registered, &cb);
                position.check(info, (data.len() / channels) as u64, &cb);
                push_frames(&mut prod, data, channels, |s| f32::from(s) / 32_768.0, &cb)
            },
            on_error,
            timeout,
        ),
        SampleFormat::I32 => device.build_input_stream::<i32, _, _>(
            config,
            move |data: &[i32], info| {
                register_capture_thread(&mut registered, &cb);
                position.check(info, (data.len() / channels) as u64, &cb);
                push_frames(
                    &mut prod,
                    data,
                    channels,
                    |s| s as f32 / 2_147_483_648.0,
                    &cb,
                )
            },
            on_error,
            timeout,
        ),
        other => {
            return Err(Error::Device(format!(
                "{device}: sample format {other:?} is not supported"
            )));
        }
    }
    .map_err(|e| Error::Device(format!("{device}: {e}")))?;

    let input = InputInfo {
        device_name: device.to_string(),
        device_id: device_id_string(device),
        sample_rate: config.sample_rate,
        channels: config.channels,
        sample_format: format!("{format:?}").to_lowercase(),
    };
    Ok((
        stream,
        StreamHandle {
            consumer,
            input,
            counters,
            capacity,
            buffer: buffer_label,
            xrun_log: Some(xrun_cons),
            position_log: Some(position_cons),
        },
    ))
}

/// Opens the device by id (or, after a replug, by name) with a 100 ms device buffer, so the
/// driver's capture thread has slack under load; falls back to the device default when the
/// driver refuses that size.
fn open_stream(device_id: &str, name: Option<&str>) -> Result<(cpal::Stream, StreamHandle), Error> {
    let device = find_device(device_id, name).ok_or_else(|| {
        Error::Device(format!(
            "input device not found: {}",
            name.unwrap_or(device_id)
        ))
    })?;
    let (config, format) = choose_config(&device)?;
    let fixed = cpal::BufferSize::Fixed((config.sample_rate / 10).max(1));
    match try_open(&device, config, format, fixed, "fixed 100 ms") {
        Ok(opened) => Ok(opened),
        Err(_) => try_open(
            &device,
            config,
            format,
            cpal::BufferSize::Default,
            "device default",
        ),
    }
}

/// The running capture thread.
pub struct Capture {
    stop: Option<Sender<()>>,
    join: Option<JoinHandle<()>>,
}

impl Capture {
    /// Opens `device_id`, hands its stream to the recorder and keeps it open, reopening it after
    /// a loss (by id, or by the device's name after a replug). Fails if the device cannot be
    /// opened the first time. A panic in the capture thread is caught: the input is reported
    /// lost and the thread starts over.
    pub fn start(
        device_id: String,
        to_recorder: Sender<RecorderMsg>,
        events: Sender<Event>,
    ) -> Result<(Self, InputInfo), Error> {
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<InputInfo, Error>>(1);
        let join = std::thread::Builder::new()
            .name("wb-capture".into())
            .spawn(move || {
                let mut ready = Some(ready_tx);
                let mut name: Option<String> = None;
                loop {
                    let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        capture_loop(&device_id, &mut name, &to_recorder, &events, &stop_rx, &mut ready)
                    }));
                    if run.is_ok() {
                        return;
                    }
                    let _ = events.send(Notice::event(
                        NoticeLevel::Error,
                        "capture",
                        "The audio capture thread failed and is restarting; recording continues in a new part",
                    ));
                    let _ = to_recorder.send(RecorderMsg::InputLost {
                        reason: "the capture thread failed and restarted".into(),
                    });
                    if let Some(r) = ready.take() {
                        let _ = r.send(Err(Error::Device("the capture thread failed while starting".into())));
                        return;
                    }
                    if wait_stop(&stop_rx, RETRY_INTERVAL) {
                        return;
                    }
                }
            })
            .map_err(|e| Error::Device(format!("could not start the capture thread: {e}")))?;
        let mut capture = Self {
            stop: Some(stop_tx),
            join: Some(join),
        };
        match ready_rx.recv_timeout(Duration::from_secs(15)) {
            Ok(Ok(info)) => Ok((capture, info)),
            Ok(Err(e)) => {
                capture.stop();
                Err(e)
            }
            Err(_) => {
                // The driver did not answer: do not join a thread that may be stuck in it.
                capture.detach();
                Err(Error::Device(
                    "the input device did not start within 15 seconds".into(),
                ))
            }
        }
    }

    /// Closes the stream and ends the thread.
    pub fn stop(&mut self) {
        self.stop.take();
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }

    /// Asks the thread to end without waiting for it.
    fn detach(&mut self) {
        self.stop.take();
        self.join.take();
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Waits up to `d` for a stop request; true if the thread should end.
fn wait_stop(stop: &Receiver<()>, d: Duration) -> bool {
    !matches!(stop.recv_timeout(d), Err(RecvTimeoutError::Timeout))
}

fn capture_loop(
    device_id: &str,
    name: &mut Option<String>,
    to_recorder: &Sender<RecorderMsg>,
    events: &Sender<Event>,
    stop: &Receiver<()>,
    ready: &mut Option<mpsc::SyncSender<Result<InputInfo, Error>>>,
) {
    let mut lost_reported = false;
    loop {
        match open_stream(device_id, name.as_deref()) {
            Ok((stream, handle)) => {
                let counters = Arc::clone(&handle.counters);
                let info = handle.input.clone();
                if name.is_none() {
                    *name = Some(info.device_name.clone());
                }
                if to_recorder.send(RecorderMsg::Stream(handle)).is_err() {
                    return;
                }
                if let Err(e) = stream.play() {
                    let err = Error::Device(format!("{}: could not start: {e}", info.device_name));
                    if let Some(r) = ready.take() {
                        let _ = r.send(Err(err));
                        return;
                    }
                    let _ = to_recorder.send(RecorderMsg::InputLost {
                        reason: err.to_string(),
                    });
                    lost_reported = true;
                } else {
                    if let Some(r) = ready.take() {
                        let _ = r.send(Ok(info.clone()));
                    } else {
                        let substituted = info.device_id != device_id;
                        let _ = events.send(Notice::event(
                            NoticeLevel::Info,
                            "capture",
                            if substituted {
                                format!(
                                    "Input {} is back under a new device id; recording continues in a new part",
                                    info.device_name
                                )
                            } else {
                                format!(
                                    "Input {} is back; recording continues in a new part",
                                    info.device_name
                                )
                            },
                        ));
                    }
                    // Watch the stream until it is lost or we are told to stop. A stream that
                    // delivers no callback for 3 seconds counts as lost too.
                    let mut last_callbacks = 0;
                    let mut silent_ticks = 0;
                    loop {
                        if wait_stop(stop, Duration::from_millis(100)) {
                            drop(stream);
                            return;
                        }
                        if counters.lost.load(Ordering::Acquire) {
                            break;
                        }
                        let now = counters.callbacks.load(Ordering::Relaxed);
                        if now == last_callbacks {
                            silent_ticks += 1;
                            if silent_ticks >= 30 {
                                counters.lost_kind.store(0, Ordering::Relaxed);
                                break;
                            }
                        } else {
                            silent_ticks = 0;
                            last_callbacks = now;
                        }
                    }
                    drop(stream);
                    let reason = counters.lost_reason().to_string();
                    let _ = events.send(Notice::event(
                        NoticeLevel::Warning,
                        "capture",
                        format!(
                            "Input {} lost: {reason}; retrying every 2 seconds",
                            info.device_name
                        ),
                    ));
                    let _ = to_recorder.send(RecorderMsg::InputLost { reason });
                    lost_reported = true;
                }
            }
            Err(e) => {
                if let Some(r) = ready.take() {
                    let _ = r.send(Err(e));
                    return;
                }
                if !lost_reported {
                    let _ = to_recorder.send(RecorderMsg::InputLost {
                        reason: e.to_string(),
                    });
                    lost_reported = true;
                }
            }
        }
        if wait_stop(stop, RETRY_INTERVAL) {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: i64 = 1_000_000;
    /// 480 frames at 48 kHz.
    const PERIOD: i64 = 10 * MS;

    #[derive(Default, Debug)]
    struct Tally {
        breaks: u32,
        outliers: u32,
        pair_flags: u32,
    }

    /// Runs a timeline of (timestamp offset from nominal, frames) through the tracker.
    fn run(steps: &[(i64, u64)]) -> Tally {
        let mut tracker = PositionTracker::new(48_000);
        let mut t = Tally::default();
        let mut nominal = 0i64;
        for &(offset, frames) in steps {
            let step = tracker.check(nominal + offset, frames);
            match step.verdict {
                PositionVerdict::Break { .. } => t.breaks += 1,
                PositionVerdict::Outlier { .. } => t.outliers += 1,
                _ => {}
            }
            if step.pair_flag.is_some() {
                t.pair_flags += 1;
            }
            nominal += frames as i64 * 1_000_000_000 / 48_000;
        }
        t
    }

    fn steady(n: usize) -> Vec<(i64, u64)> {
        (0..n).map(|_| (0, 480)).collect()
    }

    #[test]
    fn steady_callbacks_with_small_jitter_are_quiet() {
        let steps: Vec<(i64, u64)> = (0..2_000)
            .map(|i| (((i * 7919) % 9) as i64 * 100_000 - 400_000, 480))
            .collect();
        let t = run(&steps);
        assert_eq!((t.breaks, t.outliers, t.pair_flags), (0, 0, 0));
    }

    #[test]
    fn one_late_timestamp_is_one_outlier_where_the_pair_check_counted_two() {
        // Callback 100 reports a time 4 ms late; the next one is back on the line. Nothing in the
        // audio moved. The old check flagged both pairs (out and back).
        let mut steps = steady(300);
        steps[100].0 = 4 * MS;
        let t = run(&steps);
        assert_eq!(t.breaks, 0);
        assert_eq!(t.outliers, 1);
        assert_eq!(t.pair_flags, 2);
    }

    #[test]
    fn two_late_timestamps_in_a_row_are_still_an_outlier() {
        let mut steps = steady(300);
        steps[100].0 = 4 * MS;
        steps[101].0 = 4 * MS;
        let t = run(&steps);
        assert_eq!((t.breaks, t.outliers), (0, 1));
    }

    #[test]
    fn a_skip_of_20_ms_that_stays_is_one_break() {
        // The device clock jumps forward 20 ms (audio was skipped) and keeps going from there.
        let mut steps = steady(300);
        for s in steps.iter_mut().skip(100) {
            s.0 = 20 * MS;
        }
        let t = run(&steps);
        assert_eq!((t.breaks, t.outliers), (1, 0));
        assert_eq!(t.pair_flags, 1);
    }

    #[test]
    fn a_repeat_where_the_clock_steps_back_is_one_break() {
        let mut steps = steady(300);
        for s in steps.iter_mut().skip(100) {
            s.0 = -10 * MS;
        }
        let t = run(&steps);
        assert_eq!((t.breaks, t.outliers), (1, 0));
    }

    #[test]
    fn a_break_re_anchors_so_the_next_one_is_counted_separately() {
        let mut steps = steady(600);
        for s in steps.iter_mut().skip(100) {
            s.0 = 20 * MS;
        }
        for s in steps.iter_mut().skip(300) {
            s.0 = 45 * MS;
        }
        let t = run(&steps);
        assert_eq!((t.breaks, t.outliers), (2, 0));
    }

    #[test]
    fn a_clock_drift_of_100_ppm_over_ten_minutes_is_not_a_break() {
        // 60,000 callbacks of 10 ms; the device clock runs 100 ppm fast, so each timestamp is 1 us
        // earlier than the last one's prediction. 60 ms of drift in all.
        let steps: Vec<(i64, u64)> = (0..60_000).map(|i| (-(i as i64) * 1_000, 480)).collect();
        let t = run(&steps);
        assert_eq!((t.breaks, t.outliers, t.pair_flags), (0, 0, 0));
    }

    #[test]
    fn callbacks_of_uneven_size_are_not_breaks() {
        // WASAPI hands over whatever has accumulated: 441, 480, 528 frames and so on.
        let sizes = [441u64, 480, 528, 480, 432, 480, 480, 528];
        let steps: Vec<(i64, u64)> = (0..800).map(|i| (0, sizes[i % sizes.len()])).collect();
        let t = run(&steps);
        assert_eq!((t.breaks, t.outliers, t.pair_flags), (0, 0, 0));
    }

    #[test]
    fn two_different_excursions_in_a_row_are_two_outliers() {
        let mut steps = steady(300);
        steps[100].0 = 5 * MS;
        steps[101].0 = -5 * MS;
        let t = run(&steps);
        assert_eq!(t.breaks, 0);
        assert_eq!(t.outliers, 2);
    }

    #[test]
    fn a_new_stream_starts_from_nothing() {
        // A restart builds a new tracker: its first callback is on the line by definition.
        let mut tracker = PositionTracker::new(48_000);
        let step = tracker.check(123 * MS, 480);
        assert_eq!(step.verdict, PositionVerdict::Ok);
        let _ = PERIOD;
    }
}
