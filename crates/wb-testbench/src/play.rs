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
}

pub fn find_output(name_part: &str) -> Result<cpal::Device> {
    let needle = name_part.to_lowercase();
    let host = cpal::default_host();
    host.output_devices()?
        .find(|d| d.to_string().to_lowercase().contains(&needle))
        .ok_or_else(|| anyhow!("no output device whose name contains {name_part:?}"))
}

pub fn play(device_part: &str, path: &std::path::Path) -> Result<PlayReport> {
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
        buffer_size: cpal::BufferSize::Fixed(4_800),
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
    stream.play()?;
    while !finished.load(Ordering::Acquire) {
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(300));
    drop(stream);
    feeder
        .join()
        .map_err(|_| anyhow!("reader thread panicked"))??;
    let first = first_cb_us.load(Ordering::Relaxed);
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
    })
}
