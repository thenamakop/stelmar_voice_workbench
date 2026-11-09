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
    /// Callbacks whose device capture time did not advance by the frames delivered before them
    /// (a jump or repeat on the capture side, even when the driver reports no xrun).
    pub position_breaks: AtomicU64,
}

/// Checks that each callback's capture timestamp advances by the frames of the one before.
/// Arithmetic only: safe for the audio callback.
struct PositionCheck {
    rate: u64,
    last: Option<(cpal::StreamInstant, u64)>,
}

impl PositionCheck {
    fn new(rate: u32) -> Self {
        Self {
            rate: u64::from(rate.max(1)),
            last: None,
        }
    }

    #[inline]
    fn check(&mut self, info: &cpal::InputCallbackInfo, frames: u64, counters: &StreamCounters) {
        let ts = info.timestamp().capture;
        if let Some((prev, prev_frames)) = self.last {
            let expected = u128::from(prev_frames) * 1_000_000_000 / u128::from(self.rate);
            let actual = ts.saturating_duration_since(prev).as_nanos();
            let diff = actual.abs_diff(expected);
            // Timestamps jitter a little; a break is more than 2 ms and a quarter of a period.
            if diff > 2_000_000 && diff > expected / 4 {
                counters.position_breaks.fetch_add(1, Ordering::Relaxed);
            }
        }
        self.last = Some((ts, frames));
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
            },
        )
    }
}

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

    let err_counters = Arc::clone(&counters);
    let on_error = move |err: cpal::Error| {
        let kind = match err.kind() {
            ErrorKind::Xrun => {
                err_counters.xruns.fetch_add(1, Ordering::Relaxed);
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
    let mut position = PositionCheck::new(config.sample_rate);
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
