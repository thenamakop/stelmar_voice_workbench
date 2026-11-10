//! An independent reference capture of the same input, for telling a glitch in the source chain
//! (player, virtual cable, audio engine) from one in the recorder under test.
//!
//! It deliberately does not use `wb-recorder`'s capture or writer: cpal callback → ring buffer →
//! one thread writing a single WAV with hound. Its output is laid out like a session
//! (`audio\part-0001.wav` and `parts.json`) so `analyse` reads it the same way.

use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ringbuf::HeapRb;
use ringbuf::traits::{Consumer, Observer, Producer, Split};
use serde::{Deserialize, Serialize};
use wb_recorder::parts::{self, AUDIO_DIR, PartsIndex};
use wb_recorder::util::{now_ist, rfc3339};

/// One driver glitch (cpal `ErrorKind::Xrun`, WASAPI's DATA_DISCONTINUITY flag) seen by the
/// reference capture. cpal raises it just before it hands over the flagged packet, so the counts
/// are of what came before it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct XrunRecord {
    /// 1 for the first xrun of the capture.
    pub n: u64,
    /// Data callbacks delivered before the flagged packet.
    pub callbacks_before: u64,
    /// Frames delivered before it: the position of the flagged packet in the reference recording.
    pub frames_before: u64,
    /// `frames_before` in milliseconds from the start of the reference recording (the same
    /// timeline as the reference analysis).
    pub audio_ms: f64,
    /// IST wall clock when cpal raised it, to the millisecond.
    pub wall_clock: String,
}

/// The reference capture's xrun list.
#[derive(Debug, Default)]
pub struct XrunLog {
    rate: u32,
    pub events: Vec<XrunRecord>,
}

impl XrunLog {
    pub fn new(rate: u32) -> Self {
        Self {
            rate: rate.max(1),
            events: Vec::new(),
        }
    }

    /// Records an xrun seen after `callbacks` data callbacks that delivered `frames` frames.
    pub fn push(&mut self, callbacks: u64, frames: u64, wall_clock: String) {
        self.events.push(XrunRecord {
            n: self.events.len() as u64 + 1,
            callbacks_before: callbacks,
            frames_before: frames,
            audio_ms: frames as f64 * 1000.0 / f64::from(self.rate),
            wall_clock,
        });
    }
}

/// `2026-11-09T20:34:09.912+05:30`: IST, to the millisecond.
pub fn wall_clock_ms(dt: time::OffsetDateTime) -> String {
    let dt = dt.to_offset(wb_recorder::util::IST);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}+05:30",
        dt.year(),
        u8::from(dt.month()),
        dt.day(),
        dt.hour(),
        dt.minute(),
        dt.second(),
        dt.millisecond()
    )
}

/// Name of the xrun list written next to the reference recording; `analyse` copies it into its
/// report.
pub const XRUNS_FILE: &str = "xruns.json";

pub fn read_xruns(session: &Path) -> Vec<XrunRecord> {
    std::fs::read(session.join(XRUNS_FILE))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

#[derive(Debug, Serialize)]
pub struct ReferenceReport {
    pub device: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: u64,
    /// Samples the reference's own ring could not hold (its result is suspect if above 0).
    pub overrun_samples: u64,
    pub xruns: u64,
    /// Each xrun with its callback count, position in the recording and wall-clock time.
    pub xrun_events: Vec<XrunRecord>,
    pub started_at: String,
    pub ended_at: String,
    /// This process's affinity mask when the stream started (hex), as the rig pinned it.
    pub affinity_mask: Option<String>,
}

/// Records `device_part` into `dir` until `stop_file` exists.
pub fn capture(
    device_part: &str,
    dir: &Path,
    stop_file: &Path,
    go_file: Option<&Path>,
) -> Result<ReferenceReport> {
    crate::play::wait_for_go(go_file)?;
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
    let callbacks = Arc::new(AtomicU64::new(0));
    let frames_in = Arc::new(AtomicU64::new(0));
    let xrun_log = Arc::new(Mutex::new(XrunLog::new(rate)));
    let (cb_n, cb_frames) = (Arc::clone(&callbacks), Arc::clone(&frames_in));
    let (err_n, err_frames, err_log) = (
        Arc::clone(&callbacks),
        Arc::clone(&frames_in),
        Arc::clone(&xrun_log),
    );
    let raised = Arc::new(AtomicBool::new(false));
    let stream = device.build_input_stream::<f32, _, _>(
        config,
        move |data: &[f32], _| {
            if !raised.swap(true, Ordering::Relaxed) {
                std::mem::forget(wb_recorder::sys::raise_recorder_priority());
            }
            cb_n.fetch_add(1, Ordering::Relaxed);
            cb_frames.fetch_add(
                (data.len() / usize::from(channels.max(1))) as u64,
                Ordering::Relaxed,
            );
            let n = prod.push_slice(data);
            if n < data.len() {
                o1.fetch_add((data.len() - n) as u64, Ordering::Relaxed);
            }
        },
        move |e| {
            if e.kind() == cpal::ErrorKind::Xrun {
                x1.fetch_add(1, Ordering::Relaxed);
                if let Ok(mut log) = err_log.lock() {
                    log.push(
                        err_n.load(Ordering::Relaxed),
                        err_frames.load(Ordering::Relaxed),
                        wall_clock_ms(now_ist()),
                    );
                }
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
    let affinity_mask = crate::play::affinity_hex();
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
    let xrun_events = xrun_log
        .lock()
        .map(|l| l.events.clone())
        .unwrap_or_default();
    std::fs::write(
        dir.join(XRUNS_FILE),
        serde_json::to_vec_pretty(&xrun_events)?,
    )?;
    Ok(ReferenceReport {
        device: device.to_string(),
        sample_rate: rate,
        channels,
        frames: samples / u64::from(channels.max(1)),
        overrun_samples: overruns.load(Ordering::Relaxed),
        xruns: xruns.load(Ordering::Relaxed),
        xrun_events,
        started_at,
        ended_at: rfc3339(now_ist()),
        affinity_mask,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_xrun_is_logged_with_its_callback_count_audio_time_and_wall_clock() {
        let mut log = XrunLog::new(48_000);
        log.push(0, 0, "2026-11-09T20:34:08.912+05:30".into());
        log.push(412, 197_760, "2026-11-09T20:34:13.034+05:30".into());
        assert_eq!(log.events.len(), 2);
        assert_eq!(log.events[0].n, 1);
        assert_eq!(log.events[1].n, 2);
        assert_eq!(log.events[1].callbacks_before, 412);
        assert_eq!(log.events[1].frames_before, 197_760);
        // 197,760 frames at 48 kHz = 4,120 ms into the reference recording.
        assert!((log.events[1].audio_ms - 4_120.0).abs() < 1e-9);
        assert_eq!(log.events[1].wall_clock, "2026-11-09T20:34:13.034+05:30");
    }

    #[test]
    fn the_wall_clock_has_milliseconds_and_the_ist_offset() {
        let s = wall_clock_ms(now_ist());
        // 2026-11-09T20:34:13.034+05:30
        assert_eq!(s.len(), 29, "{s}");
        assert_eq!(&s[10..11], "T");
        assert_eq!(&s[19..20], ".");
        assert!(s.ends_with("+05:30"));
    }

    #[test]
    fn the_list_round_trips_through_xruns_json_for_the_analysis() -> anyhow::Result<()> {
        let dir = std::env::temp_dir().join(format!("wb-xruns-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let mut log = XrunLog::new(48_000);
        log.push(3, 1_440, "2026-11-09T20:34:09.000+05:30".into());
        std::fs::write(dir.join(XRUNS_FILE), serde_json::to_vec(&log.events)?)?;
        assert_eq!(read_xruns(&dir), log.events);
        assert!(read_xruns(&dir.join("missing")).is_empty());
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }
}
