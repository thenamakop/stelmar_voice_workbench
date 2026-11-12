//! `wb-testbench scan-repeats`: runs the recorder's own repeated-block detector over the parts of a
//! recorded session, so the detector can be checked against recordings whose shared upstream
//! breaks are known (the 10 Nov 2026 gate runs) without recording again.
//!
//! The detector is `wb_recorder::repeat::RepeatDetector`, the same code the recorder thread runs.
//! It is continuous across the parts of one run of the app and starts afresh at each later run
//! (a restart after a kill), as the recorder starts a new detector for each stream.

use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};
use serde::Serialize;
use wb_recorder::dropout::{DropoutDetector, ZeroGap};
use wb_recorder::parts;
use wb_recorder::repeat::{RepeatDetector, RepeatedBlock};

#[derive(Debug, Serialize)]
pub struct FoundBlock {
    /// Audio-clock time of the first frame of the repeated copy, in ms (the recorder's clock:
    /// the part's `start_ms` plus the frames into the part).
    pub audio_ms: f64,
    pub part: u32,
    pub len_frames: u32,
    pub lag_frames: u32,
    pub level_dbfs: f32,
}

#[derive(Debug, Serialize)]
pub struct ScanReport {
    pub session: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub parts: usize,
    /// Indices (into the parts list) where a new run of the app starts.
    pub run_starts: Vec<usize>,
    pub frames_scanned: u64,
    pub audio_seconds: f64,
    pub elapsed_s: f64,
    pub blocks: Vec<FoundBlock>,
    /// Zero gaps: runs of exact digital silence inside live audio (what the shared upstream breaks
    /// of the 10 Nov 2026 runs are at the sample level).
    pub zero_gaps: Vec<FoundGap>,
}

#[derive(Debug, Serialize)]
pub struct FoundGap {
    pub audio_ms: f64,
    pub part: u32,
    pub len_frames: u32,
    pub len_ms: f64,
}

/// Parts at which a later run of the app starts (a restart after a kill): the first part whose
/// start is at or after the audio time of each `audio_started` event after the first.
fn run_start_parts(session: &Path, index: &parts::PartsIndex) -> Result<Vec<usize>> {
    let meta = wb_recorder::session::load_meta(session)?;
    let mut starts = vec![0usize];
    for e in meta
        .events
        .iter()
        .filter(|e| e.kind == "audio_started")
        .skip(1)
    {
        if let Some(i) = index
            .parts
            .iter()
            .position(|p| p.start_ms >= e.audio_ms.saturating_sub(2))
        {
            starts.push(i);
        }
    }
    starts.sort_unstable();
    starts.dedup();
    Ok(starts)
}

pub fn scan(session: &Path) -> Result<ScanReport> {
    let started = Instant::now();
    let index = parts::load(session)?;
    let runs = run_start_parts(session, &index)?;
    let mut detectors: Option<(RepeatDetector, DropoutDetector)> = None;
    let mut blocks = Vec::new();
    let mut gaps = Vec::new();
    // The detector frame count at the start of the part being read, and that part.
    let mut cur: Option<(u64, &parts::PartRecord)> = None;
    let mut frames_total = 0u64;
    let (mut rate, mut channels) = (0u32, 0u16);
    for (i, part) in index.parts.iter().enumerate() {
        let path = session.join(&part.file);
        let mut reader =
            hound::WavReader::open(&path).with_context(|| format!("open {}", path.display()))?;
        let spec = reader.spec();
        if spec.bits_per_sample != 16 || spec.sample_format != hound::SampleFormat::Int {
            anyhow::bail!("{}: only 16-bit PCM parts are supported", path.display());
        }
        rate = spec.sample_rate;
        channels = spec.channels;
        if runs.contains(&i) || detectors.is_none() {
            if let (Some((mut rep, mut drop)), Some((first, prev))) = (detectors.take(), cur) {
                // End of the earlier run.
                rep.flush(|b| blocks.push(to_found(b, first, prev, rate)));
                drop.flush();
            }
            detectors = Some((
                RepeatDetector::new(channels),
                DropoutDetector::new(channels, rate),
            ));
        }
        let Some((rep, drop)) = detectors.as_mut() else {
            continue;
        };
        let ch = usize::from(channels.max(1));
        let chunk = ch * 4_800;
        let first_frame = rep.frames_seen();
        cur = Some((first_frame, part));
        let mut buf: Vec<f32> = Vec::with_capacity(chunk);
        let mut samples = reader.samples::<i16>();
        loop {
            buf.clear();
            for s in samples.by_ref().take(chunk) {
                buf.push(f32::from(s?) / 32_768.0);
            }
            if buf.is_empty() {
                break;
            }
            rep.push(&buf, |b| blocks.push(to_found(b, first_frame, part, rate)));
            drop.push(&buf, |g| gaps.push(gap_found(g, first_frame, part, rate)));
            frames_total += (buf.len() / ch) as u64;
        }
    }
    if let (Some((mut rep, mut drop)), Some((first, last))) = (detectors, cur) {
        rep.flush(|b| blocks.push(to_found(b, first, last, rate)));
        drop.flush();
    }
    // The audio time of a block comes from its part: sort in audio order.
    blocks.sort_by(|a, b| a.audio_ms.total_cmp(&b.audio_ms));
    gaps.sort_by(|a, b| a.audio_ms.total_cmp(&b.audio_ms));
    Ok(ScanReport {
        session: session.display().to_string(),
        sample_rate: rate,
        channels,
        parts: index.parts.len(),
        run_starts: runs,
        frames_scanned: frames_total,
        audio_seconds: frames_total as f64 / f64::from(rate.max(1)),
        elapsed_s: started.elapsed().as_secs_f64(),
        blocks,
        zero_gaps: gaps,
    })
}

fn gap_found(g: ZeroGap, part_first_frame: u64, part: &parts::PartRecord, rate: u32) -> FoundGap {
    let rate_f = f64::from(rate.max(1));
    let into = g.start_frame as f64 - part_first_frame as f64;
    FoundGap {
        audio_ms: part.start_ms as f64 + into * 1000.0 / rate_f,
        part: part.part,
        len_frames: g.len,
        len_ms: f64::from(g.len) * 1000.0 / rate_f,
    }
}

/// A found block as a report entry. `part_first_frame` is the detector frame count at the start
/// of `part`; the block's audio time is the part's start plus its distance into the part. A block
/// that began in an earlier part is placed in that part's time base, so its distance can be
/// negative and is added to the part's start (the audio clock is continuous across parts).
fn to_found(
    b: RepeatedBlock,
    part_first_frame: u64,
    part: &parts::PartRecord,
    rate: u32,
) -> FoundBlock {
    let rate_f = f64::from(rate.max(1));
    let (start_ms, part_no) = (part.start_ms as f64, part.part);
    let into = b.start_frame as f64 - part_first_frame as f64;
    FoundBlock {
        audio_ms: start_ms + into * 1000.0 / rate_f,
        part: part_no,
        len_frames: b.len,
        lag_frames: b.lag,
        level_dbfs: b.level_dbfs,
    }
}

/// One lag's longest exact self-match inside a window (diagnostic).
#[derive(Debug, Serialize)]
pub struct LagMatch {
    pub lag_frames: usize,
    pub longest_run_frames: usize,
    /// Window-relative frame where that run starts.
    pub run_start_frame: usize,
    /// Of channel 0 and of both channels equal.
    pub both_channels: bool,
}

/// For the audio-clock window `[from_ms, to_ms)` of a session, the lags (1..=`max_lag` frames) with
/// the longest runs where a frame equals the one `lag` frames before, best first. Reads channel 0
/// and checks channel 1 over the winning run.
pub fn lag_scan(
    session: &Path,
    from_ms: f64,
    to_ms: f64,
    min_lag: usize,
    max_lag: usize,
) -> Result<Vec<LagMatch>> {
    let index = parts::load(session)?;
    let mut frames: Vec<[i16; 2]> = Vec::new();
    let mut rate = 48_000u32;
    for part in &index.parts {
        let end_ms = part.start_ms as f64 + part.duration_ms as f64;
        if end_ms < from_ms || (part.start_ms as f64) > to_ms {
            continue;
        }
        let path = session.join(&part.file);
        let mut reader = hound::WavReader::open(&path)?;
        let spec = reader.spec();
        rate = spec.sample_rate;
        let ch = usize::from(spec.channels.max(1));
        let first = ((from_ms - part.start_ms as f64).max(0.0) * f64::from(rate) / 1000.0) as usize;
        let last = ((to_ms - part.start_ms as f64) * f64::from(rate) / 1000.0) as usize;
        let all: Vec<i16> = reader
            .samples::<i16>()
            .collect::<std::result::Result<_, _>>()?;
        for f in first..last.min(all.len() / ch) {
            frames.push([all[f * ch], if ch > 1 { all[f * ch + 1] } else { 0 }]);
        }
    }
    let mut out = Vec::new();
    for lag in min_lag.max(1)..=max_lag.min(frames.len().saturating_sub(1)) {
        // A stretch of one repeated value matches itself at every lag: only runs in which the
        // signal varies (at least half the frames differ from the one before) count.
        let (mut best, mut best_at, mut run, mut varying) = (0usize, 0usize, 0usize, 0usize);
        for t in lag..frames.len() {
            if frames[t][0] == frames[t - lag][0] {
                run += 1;
                if t > 0 && frames[t][0] != frames[t - 1][0] {
                    varying += 1;
                }
                if run > best && varying * 2 >= run {
                    best = run;
                    best_at = t + 1 - run;
                }
            } else {
                run = 0;
                varying = 0;
            }
        }
        if best >= 32 {
            let both = (best_at..best_at + best).all(|t| frames[t][1] == frames[t - lag][1]);
            out.push(LagMatch {
                lag_frames: lag,
                longest_run_frames: best,
                run_start_frame: best_at,
                both_channels: both,
            });
        }
    }
    out.sort_by_key(|m| std::cmp::Reverse(m.longest_run_frames));
    out.truncate(10);
    let _ = rate;
    Ok(out)
}

/// Channel 0 and 1 samples for `count` frames from audio-clock time `at_ms` (diagnostic).
pub fn dump_frames(session: &Path, at_ms: f64, count: usize) -> Result<Vec<[i16; 2]>> {
    let index = parts::load(session)?;
    let mut out = Vec::new();
    for part in &index.parts {
        if at_ms < part.start_ms as f64 || at_ms >= part.end_ms() as f64 {
            continue;
        }
        let mut reader = hound::WavReader::open(session.join(&part.file))?;
        let spec = reader.spec();
        let ch = usize::from(spec.channels.max(1));
        let first =
            ((at_ms - part.start_ms as f64) * f64::from(spec.sample_rate) / 1000.0) as usize;
        let all: Vec<i16> = reader
            .samples::<i16>()
            .collect::<std::result::Result<_, _>>()?;
        for f in first..(first + count).min(all.len() / ch) {
            out.push([all[f * ch], if ch > 1 { all[f * ch + 1] } else { 0 }]);
        }
    }
    Ok(out)
}
