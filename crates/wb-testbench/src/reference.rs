//! An independent reference capture of the same input, for telling a glitch in the source chain
//! (player, virtual cable, audio engine) from one in the recorder under test.
//!
//! It deliberately does not use `wb-recorder`'s capture or writer: cpal callback → ring buffer →
//! one thread writing a single WAV with hound. Its output is laid out like a session
//! (`audio\part-0001.wav` and `parts.json`) so `analyse` reads it the same way.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ringbuf::HeapRb;
use ringbuf::traits::{Consumer, Observer, Producer, Split};
use serde::Serialize;
use wb_recorder::parts::{self, AUDIO_DIR, PartsIndex};
use wb_recorder::util::{now_ist, rfc3339};

#[derive(Debug, Serialize)]
pub struct ReferenceReport {
    pub device: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: u64,
    /// Samples the reference's own ring could not hold (its result is suspect if above 0).
    pub overrun_samples: u64,
    pub xruns: u64,
    pub started_at: String,
    pub ended_at: String,
}

/// Records `device_part` into `dir` until `stop_file` exists.
pub fn capture(device_part: &str, dir: &Path, stop_file: &Path) -> Result<ReferenceReport> {
    let needle = device_part.to_lowercase();
    let device = cpal::default_host()
        .input_devices()?
        .find(|d| d.to_string().to_lowercase().contains(&needle))
        .ok_or_else(|| anyhow!("no input device whose name contains {device_part:?}"))?;
    let cfg = device.default_input_config()?;
    let rate = cfg.sample_rate();
    let channels = cfg.channels();
    if cfg.sample_format() != cpal::SampleFormat::F32 {
        return Err(anyhow!(
            "reference capture needs a float input, got {:?}",
            cfg.sample_format()
        ));
    }
    let config = cpal::StreamConfig {
        channels,
        sample_rate: rate,
        buffer_size: cpal::BufferSize::Fixed(rate / 10),
    };
    let cap = rate as usize * usize::from(channels) * 30;
    let (mut prod, mut cons) = HeapRb::<f32>::new(cap).split();
    let overruns = Arc::new(AtomicU64::new(0));
    let xruns = Arc::new(AtomicU64::new(0));
    let (o1, x1) = (Arc::clone(&overruns), Arc::clone(&xruns));
    let raised = Arc::new(AtomicBool::new(false));
    let stream = device.build_input_stream::<f32, _, _>(
        config,
        move |data: &[f32], _| {
            if !raised.swap(true, Ordering::Relaxed) {
                std::mem::forget(wb_recorder::sys::raise_recorder_priority());
            }
            let n = prod.push_slice(data);
            if n < data.len() {
                o1.fetch_add((data.len() - n) as u64, Ordering::Relaxed);
            }
        },
        move |e| {
            if e.kind() == cpal::ErrorKind::Xrun {
                x1.fetch_add(1, Ordering::Relaxed);
            }
        },
        Some(Duration::from_secs(5)),
    )?;
    std::fs::create_dir_all(dir.join(AUDIO_DIR))?;
    let path = dir.join(AUDIO_DIR).join(parts::part_file_name(1));
    let spec = hound::WavSpec {
        channels,
        sample_rate: rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w =
        hound::WavWriter::create(&path, spec).with_context(|| path.display().to_string())?;
    let _guard = wb_recorder::sys::raise_recorder_priority();
    let started_at = rfc3339(now_ist());
    stream.play()?;
    let mut buf = vec![0.0f32; rate as usize * usize::from(channels) / 2];
    let mut samples = 0u64;
    loop {
        let stop = stop_file.exists();
        loop {
            let n = cons.pop_slice(&mut buf);
            if n == 0 {
                break;
            }
            for &s in &buf[..n] {
                w.write_sample(wb_recorder::wav::f32_to_i16(s))?;
            }
            samples += n as u64;
        }
        if stop && cons.occupied_len() == 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(stream);
    w.finalize()?;
    let mut index = PartsIndex::new();
    index.parts.push(parts::record_for_file(dir, 1, 0)?);
    parts::save(dir, &index)?;
    Ok(ReferenceReport {
        device: device.to_string(),
        sample_rate: rate,
        channels,
        frames: samples / u64::from(channels.max(1)),
        overrun_samples: overruns.load(Ordering::Relaxed),
        xruns: xruns.load(Ordering::Relaxed),
        started_at,
        ended_at: rfc3339(now_ist()),
    })
}
