//! Compares a recorded session with the known test signal: where the signal starts, whether every
//! block of the recording lines up with the source at one constant offset (no dropped, repeated
//! or silent stretches), and how long the recorded content is against the source.

use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Serialize;
use wb_recorder::parts;

use crate::signal;

/// 100 ms blocks at 48 kHz.
const BLOCK: usize = 4_800;
const PROBE: usize = 512;
const WINDOW: usize = 64;
const MATCH: f64 = 0.9;
/// RMS (16-bit units) below which a block counts as silent. The signal's RMS is about 4,700.
const SILENT_RMS: f64 = 200.0;

#[derive(Debug, Clone, Serialize)]
pub struct UnmatchedRange {
    pub source_ms: f64,
    pub ms: f64,
}

/// Adds a stretch, merging it with the previous one when they touch.
fn add_range(ranges: &mut Vec<UnmatchedRange>, source_ms: f64, len_ms: f64) {
    if let Some(last) = ranges.last_mut()
        && (source_ms - (last.source_ms + last.ms)).abs() < 1.0
    {
        last.ms += len_ms;
        return;
    }
    ranges.push(UnmatchedRange {
        source_ms,
        ms: len_ms,
    });
}

#[derive(Debug, Clone, Serialize)]
pub struct Discontinuity {
    /// Position in the recording (ms on the session's audio clock).
    pub at_ms: f64,
    /// Position in the source where the recording broke (ms), for comparing two captures.
    pub source_ms: f64,
    pub part: u32,
    /// Source frames skipped (positive: audio missing from the recording) or repeated (negative).
    pub source_jump_frames: i64,
    pub source_jump_ms: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Analysis {
    pub sample_rate: u32,
    pub recorded_channels: u16,
    pub parts: usize,
    pub recorded_ms: f64,
    pub source_ms: f64,
    /// Loud audio before the signal that is not the signal (leftovers in the cable's buffer),
    /// from the first loud sample to where the signal starts. Reported, not a failure: it is
    /// before the gate's own playback.
    pub leading_junk_ms: f64,
    /// Where the signal starts in the recording.
    pub content_start_ms: f64,
    /// First source frame found there (0 when the start of the file was recorded).
    pub content_first_source_frame: i64,
    /// Recorded content from the signal's start to its end.
    pub content_ms: f64,
    /// content_ms − source_ms (the gate: within ±1000 ms).
    pub duration_diff_ms: f64,
    pub discontinuities: Vec<Discontinuity>,
    /// Silent stretches inside the content.
    pub silent_gap_ms: f64,
    /// Blocks that matched no part of the source.
    pub unmatched_ms: f64,
    /// Where the unmatched stretches are, by source position (adjacent ones merged).
    pub unmatched_ranges: Vec<UnmatchedRange>,
    /// Lowest block correlation seen where the alignment held.
    /// Channel 1 blocks that did not match at channel 0's offset (None for mono recordings).
    pub channel1_unmatched_ms: Option<f64>,
    pub min_block_correlation: f64,
    pub passed: bool,
    /// The reference capture's xruns (callback count, position and wall clock), from the
    /// `xruns.json` it writes next to its recording. Empty for any other recording.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub xruns: Vec<crate::reference::XrunRecord>,
}

/// Channel 0 of every part in `parts.json` order.
struct Recorded {
    rate: u32,
    channels: u16,
    samples: Vec<i16>,
    /// Channel 1, when the recording has a second channel.
    samples1: Option<Vec<i16>>,
    /// (part number, first frame of the part in `samples`).
    starts: Vec<(u32, usize)>,
}

fn load_channel0(session: &Path) -> Result<Recorded> {
    let index = parts::load(session).context("parts.json")?;
    if index.parts.is_empty() {
        bail!("{} has no parts", session.display());
    }
    let mut out = Vec::new();
    let mut out1 = Vec::new();
    let mut starts = Vec::new();
    let mut rate = 0;
    let mut ch = 0;
    for p in &index.parts {
        let mut r =
            hound::WavReader::open(session.join(&p.file)).with_context(|| p.file.clone())?;
        let spec = r.spec();
        if rate != 0 && (spec.sample_rate != rate || spec.channels != ch) {
            bail!(
                "part {} changes format ({} Hz, {} ch)",
                p.part,
                spec.sample_rate,
                spec.channels
            );
        }
        rate = spec.sample_rate;
        ch = spec.channels;
        starts.push((p.part, out.len()));
        let step = usize::from(ch);
        for (i, s) in r.samples::<i16>().enumerate() {
            let s = s?;
            if i % step == 0 {
                out.push(s);
            } else if i % step == 1 {
                out1.push(s);
            }
        }
    }
    Ok(Recorded {
        rate,
        channels: ch,
        samples: out,
        samples1: if ch >= 2 { Some(out1) } else { None },
        starts,
    })
}

struct Source {
    seed: u64,
    frames: u64,
    channel: u16,
}

impl Source {
    fn at(&self, k: i64) -> f64 {
        if k < 0 || k as u64 >= self.frames {
            0.0
        } else {
            f64::from(signal::sample(self.seed, k as u64, self.channel))
        }
    }
}

fn ncorr(r: &[i16], src: &Source, k: i64) -> f64 {
    let (mut xy, mut xx, mut yy) = (0.0, 0.0, 0.0);
    for (i, &x) in r.iter().enumerate() {
        let x = f64::from(x);
        let y = src.at(k + i as i64);
        xy += x * y;
        xx += x * x;
        yy += y * y;
    }
    if xx == 0.0 || yy == 0.0 {
        0.0
    } else {
        xy / (xx * yy).sqrt()
    }
}

fn rms(r: &[i16]) -> f64 {
    if r.is_empty() {
        return 0.0;
    }
    (r.iter().map(|&x| f64::from(x) * f64::from(x)).sum::<f64>() / r.len() as f64).sqrt()
}

/// Best source offset for the probe at `n`, searching source frames `lo..hi`.
fn search(r: &[i16], n: usize, src: &Source, lo: i64, hi: i64) -> Option<(i64, f64)> {
    let probe = &r[n..(n + PROBE).min(r.len())];
    if probe.len() < PROBE / 2 || rms(probe) < SILENT_RMS {
        return None;
    }
    let mut best: Option<(i64, f64)> = None;
    for k in lo.max(0)..hi.min(src.frames as i64) {
        let c = ncorr(probe, src, k);
        if c > MATCH && best.is_none_or(|(_, b)| c > b) {
            best = Some((k, c));
            if c > 0.999 {
                break;
            }
        }
    }
    best
}

fn part_at(starts: &[(u32, usize)], n: usize) -> u32 {
    starts
        .iter()
        .rev()
        .find(|(_, s)| *s <= n)
        .map_or(0, |(p, _)| *p)
}

pub fn analyse(
    session: &Path,
    seed: u64,
    source_frames: u64,
    source_rate: u32,
) -> Result<Analysis> {
    let Recorded {
        rate,
        channels: ch,
        samples: r,
        samples1: r1,
        starts,
    } = load_channel0(session)?;
    if rate != source_rate {
        bail!("recorded at {rate} Hz, source is {source_rate} Hz");
    }
    let src = Source {
        seed,
        frames: source_frames,
        channel: 0,
    };
    let src1 = Source {
        seed,
        frames: source_frames,
        channel: 1,
    };
    let mut ch1_unmatched = 0usize;
    let ms = |frames: f64| frames * 1000.0 / f64::from(rate);

    // 1. Where the signal starts: the first stretch that matches the source's first 5 seconds.
    // A few loud samples left in the cable's buffer by an earlier run (then silence) fail the
    // probe, so later candidates are tried, up to the first 30 seconds of the recording. What
    // came before the match is reported as `leading_junk_ms`.
    let limit = r.len().min(30 * rate as usize);
    let mut first_loud: Option<usize> = None;
    let mut cand = 0usize;
    let mut found: Option<(usize, i64)> = None;
    let mut attempts = 0;
    while cand + PROBE <= limit && attempts < 300 {
        while cand + 480 <= limit && rms(&r[cand..cand + 480]) < SILENT_RMS {
            cand += 480;
        }
        if cand + PROBE > limit {
            break;
        }
        // Refine to the first loud sample in that block.
        let mut at = cand;
        while at < limit && i32::from(r[at]).abs() < 64 {
            at += 1;
        }
        first_loud.get_or_insert(at);
        attempts += 1;
        if let Some((k, _)) = search(&r, at, &src, 0, i64::from(rate) * 5) {
            found = Some((at, k));
            break;
        }
        cand = at + 4_800; // skip 100 ms and look again
    }
    let Some((mut n, k0)) = found else {
        bail!("the test signal was not found in the recording");
    };
    let content_start = n;
    let leading_junk_frames = first_loud.map_or(0, |f| content_start.saturating_sub(f));
    let mut d = k0 - n as i64; // source frame = recorded frame + d
    let mut discontinuities = Vec::new();
    let mut silent = 0usize;
    let mut unmatched = 0usize;
    let mut ranges: Vec<UnmatchedRange> = Vec::new();
    let mut min_corr = 1.0f64;
    let mut end_n = n;

    while n < r.len() {
        let k = n as i64 + d;
        if k >= src.frames as i64 {
            end_n = n;
            break;
        }
        let len = BLOCK.min(r.len() - n).min((src.frames as i64 - k) as usize);
        let block = &r[n..n + len];
        let c = ncorr(block, &src, k);
        if c >= MATCH {
            min_corr = min_corr.min(c);
            // Channel 1 must line up at the same offset (a dropped or swapped channel fails).
            if let Some(r1) = &r1
                && let Some(b1) = r1.get(n..n + len)
                && ncorr(b1, &src1, k) < MATCH
            {
                ch1_unmatched += len;
            }
            n += len;
            end_n = n;
            continue;
        }
        if rms(block) < SILENT_RMS {
            // Silence inside the content: either a gap, or the end of the recording.
            let rest_loud = r[n..].chunks(BLOCK).any(|b| rms(b) >= SILENT_RMS);
            if !rest_loud {
                end_n = n;
                break;
            }
            silent += len;
            n += len;
            continue;
        }
        // Find where in this block the alignment breaks (64-sample windows).
        let mut j = 0;
        while j + WINDOW <= len && ncorr(&block[j..j + WINDOW], &src, k + j as i64) >= MATCH {
            j += WINDOW;
        }
        let at = n + j;
        let search_lo = (at as i64 + d) - i64::from(rate);
        let search_hi = (at as i64 + d) + i64::from(rate) * 120;
        match search(&r, at, &src, search_lo, search_hi) {
            Some((k2, _)) => {
                let jump = k2 - (at as i64 + d);
                if jump != 0 {
                    discontinuities.push(Discontinuity {
                        at_ms: ms(at as f64),
                        source_ms: ms((at as i64 + d) as f64),
                        part: part_at(&starts, at),
                        source_jump_frames: jump,
                        source_jump_ms: ms(jump as f64),
                    });
                    d = k2 - at as i64;
                    n = at;
                } else {
                    // Same offset, just a weak window: count it and move on.
                    unmatched += WINDOW;
                    add_range(&mut ranges, ms((at as i64 + d) as f64), ms(WINDOW as f64));
                    n = at + WINDOW;
                }
            }
            None => {
                unmatched += len - j;
                add_range(
                    &mut ranges,
                    ms((at as i64 + d) as f64),
                    ms((len - j) as f64),
                );
                n += len;
            }
        }
    }
    let content_frames = (end_n - content_start) as f64;
    let source_ms = ms(source_frames as f64);
    let content_ms = ms(content_frames);
    let duration_diff_ms = content_ms - source_ms;
    let passed = duration_diff_ms.abs() <= 1000.0
        && discontinuities.is_empty()
        && silent == 0
        && unmatched == 0
        && ch1_unmatched == 0
        && k0 == 0;
    Ok(Analysis {
        sample_rate: rate,
        recorded_channels: ch,
        parts: starts.len(),
        recorded_ms: ms(r.len() as f64),
        source_ms,
        content_start_ms: ms(content_start as f64),
        leading_junk_ms: ms(leading_junk_frames as f64),
        content_first_source_frame: k0,
        content_ms,
        duration_diff_ms,
        discontinuities,
        silent_gap_ms: ms(silent as f64),
        unmatched_ms: ms(unmatched as f64),
        unmatched_ranges: ranges,
        channel1_unmatched_ms: r1.as_ref().map(|_| ms(ch1_unmatched as f64)),
        min_block_correlation: min_corr,
        passed,
        xruns: crate::reference::read_xruns(session),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wb_recorder::parts::{AUDIO_DIR, PartsIndex};
    use wb_recorder::testutil::TempDir;
    use wb_recorder::wav::PartWriter;

    const SEED: u64 = 7;

    /// Writes source frames `ranges` (with optional silence) as consecutive 1-second parts.
    fn session(dir: &Path, pieces: &[(i64, usize)]) -> Result<()> {
        let mut frames: Vec<f32> = Vec::new();
        for &(start, len) in pieces {
            for i in 0..len {
                let s = match start {
                    -1 => 0,
                    -2 => signal::sample(SEED + 99, i as u64, 0), // unrelated noise
                    _ => signal::sample(SEED, start as u64 + i as u64, 0),
                };
                frames.push(f32::from(s) / 32768.0);
            }
        }
        std::fs::create_dir_all(dir.join(AUDIO_DIR))?;
        let mut index = PartsIndex::new();
        for (i, chunk) in frames.chunks(48_000).enumerate() {
            let n = i as u32 + 1;
            let mut w = PartWriter::create(
                &dir.join(AUDIO_DIR).join(parts::part_file_name(n)),
                48_000,
                1,
            )?;
            w.write_f32(chunk)?;
            w.finish()?;
            index
                .parts
                .push(parts::record_for_file(dir, n, index.end_ms())?);
        }
        parts::save(dir, &index)?;
        Ok(())
    }

    /// Stereo: channel 0 the source, channel 1 the source's channel 1 or silence.
    fn stereo_session(dir: &Path, frames: usize, ch1_silent: bool) -> Result<()> {
        let mut samples: Vec<f32> = Vec::with_capacity(frames * 2);
        for i in 0..frames as u64 {
            samples.push(f32::from(signal::sample(SEED, i, 0)) / 32768.0);
            let c1 = if ch1_silent {
                0
            } else {
                signal::sample(SEED, i, 1)
            };
            samples.push(f32::from(c1) / 32768.0);
        }
        std::fs::create_dir_all(dir.join(AUDIO_DIR))?;
        let mut w = PartWriter::create(
            &dir.join(AUDIO_DIR).join(parts::part_file_name(1)),
            48_000,
            2,
        )?;
        w.write_f32(&samples)?;
        w.finish()?;
        let mut index = PartsIndex::new();
        index.parts.push(parts::record_for_file(dir, 1, 0)?);
        parts::save(dir, &index)?;
        Ok(())
    }

    #[test]
    fn both_channels_are_checked() -> Result<()> {
        let good = TempDir::new("analyse-stereo-good")?;
        stereo_session(good.path(), 96_000, false)?;
        let a = analyse(good.path(), SEED, 96_000, 48_000)?;
        assert!(a.passed, "{a:?}");
        assert_eq!(a.channel1_unmatched_ms, Some(0.0));

        let bad = TempDir::new("analyse-stereo-bad")?;
        stereo_session(bad.path(), 96_000, true)?;
        let a = analyse(bad.path(), SEED, 96_000, 48_000)?;
        assert!(!a.passed);
        assert!(
            a.channel1_unmatched_ms.is_some_and(|ms| ms > 1_900.0),
            "{a:?}"
        );
        Ok(())
    }

    /// The shape of the overnight load run: a 29 ms burst of leftover audio, then silence, then the
    /// signal at exactly 2 s. The first probe lands on the burst and must not end the search.
    #[test]
    fn leading_junk_before_the_signal_is_skipped_and_reported() -> Result<()> {
        let t = TempDir::new("analyse-junk")?;
        session(
            t.path(),
            &[(-2, 1_400), (-1, 96_000 - 1_400), (0, 120_000), (-1, 5_000)],
        )?;
        let a = analyse(t.path(), SEED, 120_000, 48_000)?;
        assert!(a.passed, "{a:?}");
        assert!((a.content_start_ms - 2_000.0).abs() < 1.0, "{a:?}");
        assert!((a.leading_junk_ms - 2_000.0).abs() < 5.0, "{a:?}");
        assert_eq!(a.content_first_source_frame, 0);
        Ok(())
    }

    #[test]
    fn no_signal_anywhere_is_still_an_error() -> Result<()> {
        let t = TempDir::new("analyse-nosignal")?;
        session(t.path(), &[(-2, 96_000)])?;
        assert!(analyse(t.path(), SEED, 120_000, 48_000).is_err());
        Ok(())
    }

    #[test]
    fn clean_recording_passes() -> Result<()> {
        let t = TempDir::new("analyse-clean")?;
        session(t.path(), &[(-1, 7_000), (0, 120_000), (-1, 9_000)])?;
        let a = analyse(t.path(), SEED, 120_000, 48_000)?;
        assert!(a.passed, "{a:?}");
        assert!((a.content_start_ms - 145.8).abs() < 1.0);
        assert!(a.duration_diff_ms.abs() < 1.0);
        Ok(())
    }

    #[test]
    fn dropped_samples_are_found_where_they_happen() -> Result<()> {
        let t = TempDir::new("analyse-drop")?;
        // 300 frames missing at recorded frame 70,000 (inside part 2).
        session(t.path(), &[(0, 70_000), (70_300, 49_700)])?;
        let a = analyse(t.path(), SEED, 120_000, 48_000)?;
        assert!(!a.passed);
        assert_eq!(a.discontinuities.len(), 1, "{a:?}");
        let d = &a.discontinuities[0];
        assert_eq!(d.source_jump_frames, 300);
        assert_eq!(d.part, 2);
        assert!((d.at_ms - 70_000.0 / 48.0).abs() < 2.0, "{d:?}");
        Ok(())
    }

    #[test]
    fn silent_gap_is_found() -> Result<()> {
        let t = TempDir::new("analyse-silence")?;
        // 0.2 s of silence replaces source frames 50,000..59,600: a gap and a jump.
        session(t.path(), &[(0, 50_000), (-1, 9_600), (59_600, 60_400)])?;
        let a = analyse(t.path(), SEED, 120_000, 48_000)?;
        assert!(!a.passed);
        assert!(
            a.silent_gap_ms > 0.0 || !a.discontinuities.is_empty(),
            "{a:?}"
        );
        Ok(())
    }

    #[test]
    fn short_recording_fails_the_duration_check() -> Result<()> {
        let t = TempDir::new("analyse-short")?;
        session(t.path(), &[(0, 60_000)])?;
        let a = analyse(t.path(), SEED, 120_000, 48_000)?;
        assert!(!a.passed);
        assert!((a.duration_diff_ms + 1250.0).abs() < 1.0, "{a:?}");
        Ok(())
    }
}
