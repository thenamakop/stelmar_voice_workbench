//! Plays a WAV file to an output device (for example "CABLE Input" of VB-CABLE) in real time.
//! A reader thread feeds a ring buffer; the callback only copies.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ringbuf::HeapRb;
use ringbuf::traits::{Consumer, Observer, Producer, Split};
use serde::Serialize;
use wb_recorder::util::{now_ist, rfc3339};

/// A render callback that arrived later than the buffer it follows allows.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LateCallback {
    /// Position in the source of the audio this callback delivers, in milliseconds.
    pub source_ms: f64,
    /// Milliseconds since the player started its clock.
    pub at_ms: f64,
    /// Time since the previous callback.
    pub interval_ms: f64,
    /// The output buffer it exceeded.
    pub buffer_ms: f64,
}

/// Counts render callbacks that come later than the output buffer allows. The stream holds
/// `buffer_ms` of audio ahead (the fixed 4800-frame buffer is 100 ms): every callback tops it up,
/// so the next one can be as much as `buffer_ms` late before the engine runs dry. A callback is
/// LATE when the gap since the previous one is longer than that. Ordinary gaps (the engine's 10 ms
/// timer sometimes fires every 20 ms) are not late and cannot starve the buffer; they are
/// counted in a histogram so a report shows how irregular the callbacks were. The first callback
/// has no predecessor. Keeps the first `MAX_LATE_EVENTS` late callbacks and counts the rest.
#[derive(Debug)]
pub struct LateDetector {
    rate: u32,
    buffer_ms: f64,
    last_us: Option<u64>,
    pub late: u64,
    pub max_interval_ms: f64,
    pub gaps_ge_20ms: u64,
    pub gaps_ge_30ms: u64,
    pub gaps_ge_50ms: u64,
    pub events: Vec<LateCallback>,
}

pub const MAX_LATE_EVENTS: usize = 64;
/// The output buffer the player asks for, in frames (100 ms at 48 kHz).
pub const OUTPUT_BUFFER_FRAMES: u32 = 4_800;

impl LateDetector {
    pub fn new(rate: u32) -> Self {
        let rate = rate.max(1);
        Self {
            rate,
            buffer_ms: f64::from(OUTPUT_BUFFER_FRAMES) * 1000.0 / f64::from(rate),
            last_us: None,
            late: 0,
            max_interval_ms: 0.0,
            gaps_ge_20ms: 0,
            gaps_ge_30ms: 0,
            gaps_ge_50ms: 0,
            // Reserved up front: the render callback must not allocate.
            events: Vec::with_capacity(MAX_LATE_EVENTS),
        }
    }

    /// `now_us`: microseconds on the player's clock. `frames_played_before`: frames delivered by
    /// earlier callbacks (the position in the source of the audio this callback delivers).
    pub fn on_callback(&mut self, now_us: u64, frames_played_before: u64) {
        if let Some(last) = self.last_us {
            let interval_ms = now_us.saturating_sub(last) as f64 / 1000.0;
            if interval_ms > self.max_interval_ms {
                self.max_interval_ms = interval_ms;
            }
            if interval_ms >= 20.0 {
                self.gaps_ge_20ms += 1;
            }
            if interval_ms >= 30.0 {
                self.gaps_ge_30ms += 1;
            }
            if interval_ms >= 50.0 {
                self.gaps_ge_50ms += 1;
            }
            if interval_ms > self.buffer_ms {
                self.late += 1;
                if self.events.len() < MAX_LATE_EVENTS {
                    self.events.push(LateCallback {
                        source_ms: frames_played_before as f64 * 1000.0 / f64::from(self.rate),
                        at_ms: now_us as f64 / 1000.0,
                        interval_ms,
                        buffer_ms: self.buffer_ms,
                    });
                }
            }
        }
        self.last_us = Some(now_us);
    }
}

#[derive(Debug, Serialize)]
pub struct PlayReport {
    pub device: String,
    pub file: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub frames_in_file: u64,
    pub frames_played: u64,
    /// Callbacks that found the ring empty before the file ended (the player's own glitches).
    pub underrun_callbacks: u64,
    /// IST time of the first output callback.
    pub started_at: String,
    /// How the render thread's priority was raised (mmcss, highest or normal).
    pub render_priority: String,
    /// Output buffer asked for: "fixed 4800 frames" (100 ms) or the device default.
    pub buffer: String,
    pub ended_at: String,
    pub wall_seconds: f64,
    /// Render callbacks that came later than the output buffer allows (see `LateDetector`); the
    /// empty-ring `underrun_callbacks` above does not see these.
    pub late_callbacks: u64,
    /// Longest gap between two render callbacks.
    pub max_callback_interval_ms: f64,
    /// Gaps between callbacks of at least 20, 30 and 50 ms (ordinary jitter is not late).
    pub callback_gaps_ge_20ms: u64,
    pub callback_gaps_ge_30ms: u64,
    pub callback_gaps_ge_50ms: u64,
    /// The first late callbacks, each with its position in the source.
    pub late_callback_events: Vec<LateCallback>,
    /// This process's affinity mask when the stream started (hex), as the rig pinned it.
    pub affinity_mask: Option<String>,
}

pub fn find_output(name_part: &str) -> Result<cpal::Device> {
    let needle = name_part.to_lowercase();
    let host = cpal::default_host();
    host.output_devices()?
        .find(|d| d.to_string().to_lowercase().contains(&needle))
        .ok_or_else(|| anyhow!("no output device whose name contains {name_part:?}"))
}

/// Plays `path` to the end, or until `stop_file` exists (the kill gate plays a file longer than
/// it needs and then asks the player to stop, so the report is still written).
pub fn play(
    device_part: &str,
    path: &std::path::Path,
    stop_file: Option<&std::path::Path>,
    go_file: Option<&std::path::Path>,
) -> Result<PlayReport> {
    wait_for_go(go_file)?;
    let device = find_output(device_part)?;
    let reader =
        hound::WavReader::open(path).with_context(|| format!("open {}", path.display()))?;
    let spec = reader.spec();
    let frames_in_file = u64::from(reader.duration());
    let channels = usize::from(spec.channels);
    let cap = spec.sample_rate as usize * channels * 10;
    let (mut prod, mut cons) = HeapRb::<f32>::new(cap).split();

    let reader_done = Arc::new(AtomicBool::new(false));
    let done_flag = Arc::clone(&reader_done);
    let abort = Arc::new(AtomicBool::new(false));
    let abort_feeder = Arc::clone(&abort);
    let feeder = std::thread::spawn(move || -> Result<()> {
        let mut samples = reader.into_samples::<i16>();
        let mut batch: Vec<f32> = Vec::with_capacity(4_800 * channels);
        loop {
            batch.clear();
            for s in samples.by_ref().take(4_800 * channels) {
                batch.push(f32::from(s?) / 32_768.0);
            }
            if batch.is_empty() {
                break;
            }
            let mut off = 0;
            while off < batch.len() {
                off += prod.push_slice(&batch[off..]);
                if off < batch.len() {
                    if abort_feeder.load(Ordering::Relaxed) {
                        return Ok(());
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        }
        done_flag.store(true, Ordering::Release);
        Ok(())
    });
    // Prefill about 5 seconds before starting.
    let prefill_start = Instant::now();
    while cons.occupied_len() < cap / 2
        && !reader_done.load(Ordering::Acquire)
        && prefill_start.elapsed() < Duration::from_secs(20)
    {
        std::thread::sleep(Duration::from_millis(10));
    }

    let base = Instant::now();
    let base_wall = now_ist();
    let first_cb_us = Arc::new(AtomicU64::new(u64::MAX));
    let played = Arc::new(AtomicU64::new(0));
    let underruns = Arc::new(AtomicU64::new(0));
    let finished = Arc::new(AtomicBool::new(false));
    let late = Arc::new(std::sync::Mutex::new(LateDetector::new(spec.sample_rate)));
    let late_cb = Arc::clone(&late);
    let (f1, p1, u1, fin1, rd1) = (
        Arc::clone(&first_cb_us),
        Arc::clone(&played),
        Arc::clone(&underruns),
        Arc::clone(&finished),
        Arc::clone(&reader_done),
    );
    // The player is the gate's source: a late render callback here would show up as a glitch in
    // the recording and be blamed on the recorder. So its render thread gets the recorder's own
    // priority (MMCSS "Pro Audio") and a 100 ms buffer.
    let priority = Arc::new(std::sync::Mutex::new(String::from("not set")));
    let prio = Arc::clone(&priority);
    let raised = Arc::new(AtomicBool::new(false));
    let callback = move |out: &mut [f32], _: &cpal::OutputCallbackInfo| {
        if !raised.swap(true, Ordering::Relaxed) {
            let guard = wb_recorder::sys::raise_recorder_priority();
            if let Ok(mut p) = prio.lock() {
                *p = format!("{:?}", guard.mode).to_lowercase();
            }
            // Keep the registration for the life of the render thread.
            std::mem::forget(guard);
        }
        let _ = f1.compare_exchange(
            u64::MAX,
            base.elapsed().as_micros() as u64,
            Ordering::Relaxed,
            Ordering::Relaxed,
        );
        if !fin1.load(Ordering::Acquire)
            && let Ok(mut d) = late_cb.lock()
        {
            d.on_callback(
                base.elapsed().as_micros() as u64,
                p1.load(Ordering::Relaxed),
            );
        }
        let n = cons.pop_slice(out);
        for s in &mut out[n..] {
            *s = 0.0;
        }
        p1.fetch_add((n / channels) as u64, Ordering::Relaxed);
        if n < out.len() {
            if rd1.load(Ordering::Acquire) {
                fin1.store(true, Ordering::Release);
            } else {
                u1.fetch_add(1, Ordering::Relaxed);
            }
        }
    };
    let fixed = cpal::StreamConfig {
        channels: spec.channels,
        sample_rate: spec.sample_rate,
        buffer_size: cpal::BufferSize::Fixed(OUTPUT_BUFFER_FRAMES),
    };
    let shared = std::sync::Arc::new(std::sync::Mutex::new(callback));
    let cb_a = Arc::clone(&shared);
    let (stream, buffer) = match device.build_output_stream::<f32, _, _>(
        fixed,
        move |out: &mut [f32], info: &cpal::OutputCallbackInfo| {
            if let Ok(mut f) = cb_a.lock() {
                f(out, info);
            }
        },
        |e| eprintln!("output stream error: {e}"),
        Some(Duration::from_secs(5)),
    ) {
        Ok(s) => (s, "fixed 4800 frames (100 ms)".to_string()),
        Err(e) => {
            eprintln!("100 ms output buffer refused ({e}); using the device default");
            let cb_b = Arc::clone(&shared);
            let config = cpal::StreamConfig {
                channels: spec.channels,
                sample_rate: spec.sample_rate,
                buffer_size: cpal::BufferSize::Default,
            };
            let s = device.build_output_stream::<f32, _, _>(
                config,
                move |out: &mut [f32], info: &cpal::OutputCallbackInfo| {
                    if let Ok(mut f) = cb_b.lock() {
                        f(out, info);
                    }
                },
                |e| eprintln!("output stream error: {e}"),
                Some(Duration::from_secs(5)),
            )?;
            (s, "device default".to_string())
        }
    };
    let affinity_mask = affinity_hex();
    stream.play()?;
    while !finished.load(Ordering::Acquire) {
        if stop_file.is_some_and(std::path::Path::exists) {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    abort.store(true, Ordering::Relaxed);
    std::thread::sleep(Duration::from_millis(300));
    drop(stream);
    feeder
        .join()
        .map_err(|_| anyhow!("reader thread panicked"))??;
    let first = first_cb_us.load(Ordering::Relaxed);
    let (late_callbacks, max_callback_interval_ms, gaps, late_callback_events) = late
        .lock()
        .map(|d| {
            (
                d.late,
                d.max_interval_ms,
                (d.gaps_ge_20ms, d.gaps_ge_30ms, d.gaps_ge_50ms),
                d.events.clone(),
            )
        })
        .unwrap_or_default();
    let started = base_wall + Duration::from_micros(if first == u64::MAX { 0 } else { first });
    Ok(PlayReport {
        device: device.to_string(),
        file: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        sample_rate: spec.sample_rate,
        channels: spec.channels,
        frames_in_file,
        frames_played: played.load(Ordering::Relaxed),
        underrun_callbacks: underruns.load(Ordering::Relaxed),
        started_at: rfc3339(started),
        render_priority: priority.lock().map(|p| p.clone()).unwrap_or_default(),
        buffer,
        ended_at: rfc3339(now_ist()),
        wall_seconds: base.elapsed().as_secs_f64(),
        late_callbacks,
        max_callback_interval_ms,
        callback_gaps_ge_20ms: gaps.0,
        callback_gaps_ge_30ms: gaps.1,
        callback_gaps_ge_50ms: gaps.2,
        late_callback_events,
        affinity_mask,
    })
}

/// Waits (up to a minute) until `go_file` exists. The gate pins the player and the reference
/// capture to their cores and only then creates it, so neither opens its stream before it is
/// pinned.
pub fn wait_for_go(go_file: Option<&std::path::Path>) -> Result<()> {
    let Some(go) = go_file else { return Ok(()) };
    let start = Instant::now();
    while !go.exists() {
        if start.elapsed() > Duration::from_secs(60) {
            return Err(anyhow!(
                "--go-file {} did not appear within 60 s",
                go.display()
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

/// This process's affinity mask as a hex string, for the reports.
pub fn affinity_hex() -> Option<String> {
    #[cfg(windows)]
    {
        crate::win::affinity_mask().map(|m| format!("0x{m:x}"))
    }
    #[cfg(not(windows))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Callbacks of 480 frames at 48 kHz, one every 10 ms, with the given extra delays (in
    /// microseconds) before chosen callbacks.
    fn run(delays: &[(usize, u64)]) -> LateDetector {
        let mut d = LateDetector::new(48_000);
        let mut now = 1_000_000u64;
        let mut played = 0u64;
        for i in 0..200usize {
            if let Some((_, extra)) = delays.iter().find(|(at, _)| *at == i) {
                now += extra;
            }
            d.on_callback(now, played);
            played += 480;
            now += 10_000;
        }
        d
    }

    #[test]
    fn a_steady_stream_has_no_late_callbacks() {
        let d = run(&[]);
        assert_eq!(d.late, 0);
        assert!(d.events.is_empty());
        assert!((d.max_interval_ms - 10.0).abs() < 0.001);
        assert_eq!(d.gaps_ge_20ms, 0);
    }

    #[test]
    fn a_callback_later_than_the_buffer_is_late_with_its_source_position() {
        // Callback 100 comes 150 ms late: a 160 ms gap against a 100 ms buffer.
        let d = run(&[(100, 150_000)]);
        assert_eq!(d.late, 1);
        let e = &d.events[0];
        assert!((e.interval_ms - 160.0).abs() < 0.001, "{e:?}");
        assert!((e.buffer_ms - 100.0).abs() < 0.001);
        // 100 callbacks of 480 frames played before it: 1000 ms into the source.
        assert!((e.source_ms - 1000.0).abs() < 0.001, "{e:?}");
        assert!((d.max_interval_ms - 160.0).abs() < 0.001);
    }

    #[test]
    fn ordinary_irregular_gaps_cannot_starve_the_buffer_and_are_not_late() {
        // 20, 35 and 60 ms gaps are common timer jitter against a 100 ms buffer.
        let d = run(&[(40, 10_000), (80, 25_000), (120, 50_000)]);
        assert_eq!(d.late, 0);
        assert_eq!(d.gaps_ge_20ms, 3);
        assert_eq!(d.gaps_ge_30ms, 2);
        assert_eq!(d.gaps_ge_50ms, 1);
    }

    #[test]
    fn a_gap_of_exactly_the_buffer_is_not_late_but_one_more_millisecond_is() {
        let d = run(&[(50, 90_000)]); // 100 ms gap
        assert_eq!(d.late, 0);
        let d = run(&[(50, 91_000)]); // 101 ms gap
        assert_eq!(d.late, 1);
    }

    #[test]
    fn events_are_capped_but_still_counted() {
        let delays: Vec<(usize, u64)> = (1..150).map(|i| (i, 200_000)).collect();
        let d = run(&delays);
        assert_eq!(d.late, 149);
        assert_eq!(d.events.len(), MAX_LATE_EVENTS);
    }
}
