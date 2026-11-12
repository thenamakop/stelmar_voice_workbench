//! Fidelity: does the app's recording equal what Windows delivered?
//!
//! The reference capture (a separate cpal + hound recorder reading the same cable) is the record
//! of what Windows delivered. Both recordings come from the same endpoint, so they carry the same
//! samples. This compares them sample for sample over the whole run after ONE alignment offset per
//! run of the app, found at its start and never adjusted later: a recorder that drops or repeats
//! samples shifts everything after it and shows up as a difference from there on.
//!
//! A run of the app is one continuous recording; a force-killed and restarted session has one run
//! per start (`audio_started` events in `session.json`, each at the first part of its run), and
//! each run gets its own offset from its own start.
//!
//! Rule dated 9 Nov 2026 (docs/PLAN.md, section 12): fidelity decides PASS or FAIL; breaks
//! between the source and the reference are reported separately and judged for validity.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use serde::Serialize;
use wb_recorder::parts::{self, PartRecord};
use wb_recorder::wav;

/// Frames in the snippet that is searched for to find the offset.
const SNIPPET_FRAMES: usize = 2_048;
/// The snippet starts this long after the first loud frame (the first packet of a stream can be
/// flagged as discontinuous and carry stale data).
const SNIPPET_AFTER_FIRST_LOUD_MS: u64 = 250;
/// How far into a recording the alignment looks (minutes).
const SEARCH_MINUTES: u64 = 12;
/// A sample above this (in 16-bit units) is audio, not the cable's idle floor.
const LOUD: u16 = 2;
/// A window with a sample above this is part of the signal.
const SIGNAL: u16 = 256;
/// Frames read at a time.
const CHUNK_FRAMES: u64 = 1 << 20;
/// Differing stretches listed in the report (all are counted).
const MAX_RANGES: usize = 32;

/// One recording: the parts of a session in `parts.json` order.
pub struct Recording {
    files: Vec<(PathBuf, u64)>,
    /// First frame of each part.
    starts: Vec<u64>,
    pub channels: usize,
    pub rate: u32,
    pub total: u64,
}

impl Recording {
    pub fn load(session: &Path) -> Result<Self> {
        let index = parts::load(session)?;
        if index.parts.is_empty() {
            return Err(anyhow!("{} has no parts", session.display()));
        }
        let mut files = Vec::new();
        let mut starts = Vec::new();
        let mut total = 0u64;
        let (mut channels, mut rate) = (0usize, 0u32);
        for p in &index.parts {
            let path = session.join(&p.file);
            let info =
                wav::inspect(&path).with_context(|| format!("inspect {}", path.display()))?;
            if channels == 0 {
                channels = usize::from(info.channels);
                rate = info.sample_rate;
            } else if usize::from(info.channels) != channels || info.sample_rate != rate {
                return Err(anyhow!("{} has a different format", path.display()));
            }
            starts.push(total);
            files.push((path, info.frames_on_disk));
            total += info.frames_on_disk;
        }
        Ok(Self {
            files,
            starts,
            channels,
            rate,
            total,
        })
    }

    /// First frame of the part `parts.json` calls `part_no`'s successor boundary, by index.
    pub fn part_start(&self, index: usize) -> Option<u64> {
        self.starts.get(index).copied()
    }

    /// Interleaved samples of frames `[start, start + count)`, clamped to the recording.
    pub fn read(&self, start: u64, count: u64) -> Result<Vec<i16>> {
        let end = (start + count).min(self.total);
        let mut out = Vec::with_capacity(((end.saturating_sub(start)) as usize) * self.channels);
        let mut pos = start;
        while pos < end {
            let i = match self.starts.binary_search(&pos) {
                Ok(i) => i,
                Err(i) => i - 1,
            };
            let (path, frames) = &self.files[i];
            let in_file = pos - self.starts[i];
            let take = (frames - in_file).min(end - pos);
            let mut reader =
                hound::WavReader::open(path).with_context(|| format!("open {}", path.display()))?;
            reader
                .seek(in_file as u32)
                .with_context(|| format!("seek {}", path.display()))?;
            let want = take as usize * self.channels;
            for s in reader.samples::<i16>().take(want) {
                out.push(s?);
            }
            pos += take;
        }
        Ok(out)
    }
}

/// A stretch where the two recordings differ.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DiffRange {
    /// First differing frame in the reference recording.
    pub reference_frame: u64,
    /// The same frame in the app's recording (`reference_frame + offset`).
    pub app_frame: u64,
    pub frames: u64,
}

/// The comparison of one run of the app with the reference.
#[derive(Debug, Clone, Serialize)]
pub struct SegmentCompare {
    pub index: usize,
    /// The run's frames in the app's recording.
    pub app_start_frame: u64,
    pub app_end_frame: u64,
    pub aligned: bool,
    /// App frame = reference frame + offset.
    pub offset_frames: Option<i64>,
    /// Which snippet found it: "reference" (a stretch of the reference was found in the app) or
    /// "app" (the other way round, when the reference starts later).
    pub aligned_by: Option<String>,
    pub compared_frames: u64,
    pub differing_frames: u64,
    pub max_abs_diff: u32,
    /// The first differing stretches (at most 32; `differing_frames` counts all).
    pub differing_ranges: Vec<DiffRange>,
    /// Frames of the run with no counterpart in the reference, and how many of them hold audio.
    pub app_only_frames: u64,
    pub app_only_loud_frames: u64,
    /// The reference frames this run covers.
    pub reference_start_frame: u64,
    pub reference_end_frame: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Compare {
    pub sample_rate: u32,
    pub channels: usize,
    pub app_frames: u64,
    pub reference_frames: u64,
    pub segments: Vec<SegmentCompare>,
    /// Every run of the app was aligned with one fixed offset.
    pub aligned: bool,
    pub compared_frames: u64,
    pub differing_frames: u64,
    /// Frames of the reference after the span the app covers that hold audio: audio Windows
    /// delivered after the app's last frame that the app has no recording of (a FAIL). Audio
    /// before the app's first frame and the gaps between its runs are not counted.
    pub reference_only_loud_frames: u64,
    /// The same count, reported even when the tail is not judged (`ignore_reference_tail`).
    pub reference_tail_loud_frames: u64,
    /// True when reference audio after the app's last frame was not judged.
    pub reference_tail_ignored: bool,
    /// Frames of the app with audio that the reference does not cover (the reference is
    /// incomplete: the run is INVALID).
    pub app_only_loud_frames: u64,
    /// Reference frames between two runs of the app (a restart's gap), not judged here.
    pub restart_gap_frames: u64,
    /// True when every run aligned and nothing differs and no audio is missing from the app.
    pub equal: bool,
    pub notes: Vec<String>,
}

fn is_loud(s: i16) -> bool {
    s.unsigned_abs() > LOUD
}

fn frames_with_audio(samples: &[i16], channels: usize) -> u64 {
    samples
        .chunks_exact(channels)
        .filter(|f| f.iter().any(|&s| is_loud(s)))
        .count() as u64
}

/// Counts the frames of `rec` in `[start, end)` that hold audio.
fn loud_frames_in(rec: &Recording, start: u64, end: u64) -> Result<u64> {
    let mut n = 0;
    let mut pos = start;
    while pos < end {
        let take = CHUNK_FRAMES.min(end - pos);
        let buf = rec.read(pos, take)?;
        if buf.is_empty() {
            break;
        }
        n += frames_with_audio(&buf, rec.channels);
        pos += take;
    }
    Ok(n)
}

/// First frame at or after `from` (before `to`) from which a window of the signal starts, or None.
fn first_signal_frame(rec: &Recording, from: u64, to: u64) -> Result<Option<u64>> {
    let mut pos = from;
    while pos < to {
        let take = CHUNK_FRAMES.min(to - pos);
        let buf = rec.read(pos, take)?;
        if buf.is_empty() {
            break;
        }
        if let Some(i) = buf
            .chunks_exact(rec.channels)
            .position(|f| f.iter().any(|&s| s.unsigned_abs() > SIGNAL))
        {
            return Ok(Some(pos + i as u64));
        }
        pos += take;
    }
    Ok(None)
}

/// Where `snippet` (interleaved, whole frames) occurs in `rec` within `[from, to)`.
fn find_snippet(rec: &Recording, snippet: &[i16], from: u64, to: u64) -> Result<Option<u64>> {
    let ch = rec.channels;
    let w = snippet.len() / ch;
    let first = &snippet[..ch];
    let mut pos = from;
    let to = to.min(rec.total);
    while pos + w as u64 <= to {
        let want = (CHUNK_FRAMES + w as u64 - 1).min(to - pos);
        let buf = rec.read(pos, want)?;
        let frames = buf.len() / ch;
        if frames < w {
            break;
        }
        for i in 0..=(frames - w) {
            if buf[i * ch..(i + 1) * ch] == *first && buf[i * ch..(i + w) * ch] == *snippet {
                return Ok(Some(pos + i as u64));
            }
        }
        if frames as u64 >= to - pos {
            break;
        }
        pos += frames as u64 - (w as u64 - 1);
    }
    Ok(None)
}

/// Where the signal starts in each recording. The cable can hold stale audio from an earlier run
/// before the signal (a short buffer replayed in a loop, which matches itself at many offsets), so
/// the alignment snippet is taken from the signal, not from the first loud frame. The gates pass
/// the signal's start from the source analysis; without it the first loud frame is used.
#[derive(Debug, Clone, Copy, Default)]
pub struct Hints {
    /// First frame of the signal in the reference recording.
    pub reference_signal_start: Option<u64>,
    /// First frame of the signal in the first run of the app.
    pub app_signal_start: Option<u64>,
    /// Do not judge reference audio after the app's last frame. The force-kill test stops the
    /// app first and the player a moment later by design, so the signal outlives the session.
    pub ignore_reference_tail: bool,
}

/// Finds the one offset for the app's run `[s, e)`: app frame = reference frame + offset.
fn align(
    app: &Recording,
    reference: &Recording,
    s: u64,
    e: u64,
    hints: &Hints,
) -> Result<Option<(i64, &'static str)>> {
    let limit = SEARCH_MINUTES * 60 * u64::from(app.rate);
    let after = SNIPPET_AFTER_FIRST_LOUD_MS * u64::from(app.rate) / 1000;
    let w = SNIPPET_FRAMES as u64;
    // A stretch of the reference, found in the app's run.
    let ref_from = hints
        .reference_signal_start
        .unwrap_or(0)
        .min(reference.total);
    if let Some(r0) =
        first_signal_frame(reference, ref_from, reference.total.min(ref_from + limit))?
    {
        let at = r0 + after;
        if at + w <= reference.total {
            let snippet = reference.read(at, w)?;
            if let Some(a) = find_snippet(app, &snippet, s, e.min(s + limit))? {
                return Ok(Some((a as i64 - at as i64, "reference")));
            }
        }
    }
    // The other way round: a stretch of the app's run, found in the reference.
    let app_from = match (s, hints.app_signal_start) {
        (0, Some(f)) => f.clamp(s, e),
        _ => s,
    };
    if let Some(a0) = first_signal_frame(app, app_from, e.min(app_from + limit))? {
        let at = a0 + after;
        if at + w <= e {
            let snippet = app.read(at, w)?;
            if let Some(r) = find_snippet(reference, &snippet, 0, reference.total.min(limit * 3))? {
                return Ok(Some((at as i64 - r as i64, "app")));
            }
        }
    }
    Ok(None)
}

fn compare_segment(
    index: usize,
    app: &Recording,
    reference: &Recording,
    s: u64,
    e: u64,
    hints: &Hints,
) -> Result<SegmentCompare> {
    let mut seg = SegmentCompare {
        index,
        app_start_frame: s,
        app_end_frame: e,
        aligned: false,
        offset_frames: None,
        aligned_by: None,
        compared_frames: 0,
        differing_frames: 0,
        max_abs_diff: 0,
        differing_ranges: Vec::new(),
        app_only_frames: 0,
        app_only_loud_frames: 0,
        reference_start_frame: 0,
        reference_end_frame: 0,
    };
    let Some((offset, by)) = align(app, reference, s, e, hints)? else {
        return Ok(seg);
    };
    seg.aligned = true;
    seg.offset_frames = Some(offset);
    seg.aligned_by = Some(by.to_string());
    // Reference frames this run covers: those whose app counterpart lies in [s, e).
    let r_start = (s as i64 - offset).max(0) as u64;
    let r_end = ((e as i64 - offset).max(0) as u64).min(reference.total);
    seg.reference_start_frame = r_start;
    seg.reference_end_frame = r_end.max(r_start);
    let ch = app.channels;
    let mut pos = r_start;
    let mut open: Option<DiffRange> = None;
    while pos < r_end {
        let take = CHUNK_FRAMES.min(r_end - pos);
        let rb = reference.read(pos, take)?;
        let ab = app.read((pos as i64 + offset) as u64, take)?;
        let frames = (rb.len().min(ab.len()) / ch) as u64;
        if frames == 0 {
            break;
        }
        for f in 0..frames as usize {
            let (r, a) = (&rb[f * ch..(f + 1) * ch], &ab[f * ch..(f + 1) * ch]);
            if r == a {
                if let Some(d) = open.take() {
                    seg.differing_ranges_push(d);
                }
            } else {
                let diff = r
                    .iter()
                    .zip(a)
                    .map(|(x, y)| u32::from(x.abs_diff(*y)))
                    .max()
                    .unwrap_or(0);
                seg.max_abs_diff = seg.max_abs_diff.max(diff);
                seg.differing_frames += 1;
                match open.as_mut() {
                    Some(d) => d.frames += 1,
                    None => {
                        open = Some(DiffRange {
                            reference_frame: pos + f as u64,
                            app_frame: (pos as i64 + f as i64 + offset) as u64,
                            frames: 1,
                        });
                    }
                }
            }
        }
        seg.compared_frames += frames;
        pos += frames;
    }
    if let Some(d) = open.take() {
        seg.differing_ranges_push(d);
    }
    // The run's frames that have no counterpart in the reference.
    let covered_start = (r_start as i64 + offset).max(s as i64) as u64;
    let covered_end = ((r_start + seg.compared_frames) as i64 + offset).min(e as i64) as u64;
    let (head, tail) = (
        covered_start.saturating_sub(s),
        e.saturating_sub(covered_end.max(covered_start)),
    );
    seg.app_only_frames = head + tail;
    // Stale audio before the signal (the cable replaying an earlier run) is not the signal: the
    // reference starting a little later than the app may miss some of it. From the signal on,
    // everything the app recorded must be in the reference.
    let head_from = match (s, hints.app_signal_start) {
        (0, Some(f)) => f.clamp(s, s + head),
        _ => s,
    };
    seg.app_only_loud_frames =
        loud_frames_in(app, head_from, s + head)? + loud_frames_in(app, e.saturating_sub(tail), e)?;
    Ok(seg)
}

impl SegmentCompare {
    fn differing_ranges_push(&mut self, d: DiffRange) {
        if self.differing_ranges.len() < MAX_RANGES {
            self.differing_ranges.push(d);
        }
    }
}

/// Compares the app's recording with the reference. `run_starts` are the first frames of the
/// app's runs (the first is 0); a run lasts until the next one starts.
pub fn compare(
    app: &Recording,
    reference: &Recording,
    run_starts: &[u64],
    hints: &Hints,
) -> Result<Compare> {
    let mut notes = Vec::new();
    if app.rate != reference.rate || app.channels != reference.channels {
        return Err(anyhow!(
            "the recordings differ in format: app {} Hz {} ch, reference {} Hz {} ch",
            app.rate,
            app.channels,
            reference.rate,
            reference.channels
        ));
    }
    let mut starts: Vec<u64> = run_starts
        .iter()
        .copied()
        .filter(|&s| s < app.total)
        .collect();
    if starts.first() != Some(&0) {
        starts.insert(0, 0);
    }
    starts.dedup();
    let mut segments = Vec::new();
    for (i, &s) in starts.iter().enumerate() {
        let e = starts.get(i + 1).copied().unwrap_or(app.total);
        segments.push(compare_segment(i, app, reference, s, e, hints)?);
    }
    let aligned = segments.iter().all(|s| s.aligned);
    if !aligned {
        notes.push("a run of the app could not be aligned with the reference".into());
    }
    // Reference frames outside the span the app covers hold audio the app has no recording of.
    let mut reference_only_loud = 0;
    let mut reference_tail_loud = 0;
    let mut restart_gap = 0;
    if aligned {
        let span_start = segments
            .iter()
            .map(|s| s.reference_start_frame)
            .min()
            .unwrap_or(0);
        let span_end = segments
            .iter()
            .map(|s| s.reference_end_frame)
            .max()
            .unwrap_or(0);
        // Audio before the app's first frame is not the app's to record (the kill test starts
        // the app after the signal began); audio after its last frame is.
        let _ = span_start;
        reference_tail_loud = loud_frames_in(reference, span_end, reference.total)?;
        reference_only_loud = if hints.ignore_reference_tail {
            0
        } else {
            reference_tail_loud
        };
        let mut ordered: Vec<(u64, u64)> = segments
            .iter()
            .map(|s| (s.reference_start_frame, s.reference_end_frame))
            .collect();
        ordered.sort_unstable();
        for w in ordered.windows(2) {
            restart_gap += w[1].0.saturating_sub(w[0].1);
        }
    }
    let compared_frames = segments.iter().map(|s| s.compared_frames).sum();
    let differing_frames = segments.iter().map(|s| s.differing_frames).sum();
    let app_only_loud_frames = segments.iter().map(|s| s.app_only_loud_frames).sum();
    let equal = aligned && differing_frames == 0 && reference_only_loud == 0;
    Ok(Compare {
        sample_rate: app.rate,
        channels: app.channels,
        app_frames: app.total,
        reference_frames: reference.total,
        segments,
        aligned,
        compared_frames,
        differing_frames,
        reference_only_loud_frames: reference_only_loud,
        reference_tail_loud_frames: reference_tail_loud,
        reference_tail_ignored: hints.ignore_reference_tail,
        app_only_loud_frames,
        restart_gap_frames: restart_gap,
        equal,
        notes,
    })
}

/// First frame of each run of the app: 0, then the first frame of the part that starts at each
/// later `audio_started` event of `session.json` (a restart after a kill).
pub fn run_starts(session: &Path, app: &Recording) -> Result<Vec<u64>> {
    let index = parts::load(session)?;
    let meta = wb_recorder::session::load_meta(session)?;
    let mut starts = vec![0u64];
    for e in meta
        .events
        .iter()
        .filter(|e| e.kind == "audio_started")
        .skip(1)
    {
        // The run's first part starts at the audio time of the event (the audio clock counts
        // recorded audio only), or a millisecond or two later.
        let part = index
            .parts
            .iter()
            .position(|p: &PartRecord| p.start_ms >= e.audio_ms.saturating_sub(2));
        if let Some(i) = part
            && let Some(f) = app.part_start(i)
        {
            starts.push(f);
        }
    }
    starts.sort_unstable();
    starts.dedup();
    Ok(starts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wb_recorder::parts::PartsIndex;
    use wb_recorder::testutil::TempDir;
    use wb_recorder::wav::PartWriter;

    const RATE: u32 = 8_000;

    /// Reproducible noise, stereo: frame k is (noise(2k), noise(2k+1)).
    fn noise(frames: usize, seed: u64) -> Vec<i16> {
        let mut x = seed;
        (0..frames * 2)
            .map(|_| {
                x = x
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                ((x >> 40) as i16) / 2
            })
            .collect()
    }

    fn silence(frames: usize) -> Vec<i16> {
        vec![0; frames * 2]
    }

    /// Writes `samples` as a session of one part per `split` frames.
    fn session(root: &Path, name: &str, samples: &[i16], split: usize) -> Result<PathBuf> {
        let dir = root.join(name);
        std::fs::create_dir_all(dir.join("audio"))?;
        let mut index = PartsIndex::new();
        let mut start = 0u64;
        for (n, chunk) in samples.chunks(split * 2).enumerate() {
            let path = dir.join("audio").join(parts::part_file_name(n as u32 + 1));
            let mut w = PartWriter::create(&path, RATE, 2)?;
            let f: Vec<f32> = chunk.iter().map(|&s| f32::from(s) / 32_768.0).collect();
            w.write_f32(&f)?;
            w.finish()?;
            let rec = parts::record_for_file(&dir, n as u32 + 1, start)?;
            start = rec.end_ms();
            index.parts.push(rec);
        }
        parts::save(&dir, &index)?;
        Ok(dir)
    }

    fn cmp(app: &Path, reference: &Path, starts: &[u64]) -> Result<Compare> {
        compare(
            &Recording::load(app)?,
            &Recording::load(reference)?,
            starts,
            &Hints::default(),
        )
    }

    /// The reference starts 300 frames after the app; both then hold the same signal.
    fn pair(root: &Path) -> Result<(PathBuf, PathBuf, Vec<i16>)> {
        let signal = noise(40_000, 7);
        let mut app = silence(1_000);
        app.extend(&signal);
        app.extend(silence(500));
        let mut reference = silence(700);
        reference.extend(&signal);
        reference.extend(silence(200));
        Ok((
            session(root, "app", &app, 16_000)?,
            session(root, "ref", &reference, 100_000)?,
            signal,
        ))
    }

    #[test]
    fn identical_recordings_with_different_starts_are_equal_after_one_offset() -> Result<()> {
        let t = TempDir::new("cmp-equal")?;
        let (app, reference, _) = pair(t.path())?;
        let c = cmp(&app, &reference, &[0])?;
        assert!(c.aligned && c.equal, "{c:?}");
        assert_eq!(c.differing_frames, 0);
        assert_eq!(c.segments[0].offset_frames, Some(300));
        assert_eq!(c.segments[0].aligned_by.as_deref(), Some("reference"));
        assert_eq!(c.reference_only_loud_frames, 0);
        assert_eq!(c.app_only_loud_frames, 0);
        Ok(())
    }

    #[test]
    fn one_changed_sample_in_the_app_is_a_difference_at_that_frame() -> Result<()> {
        let t = TempDir::new("cmp-one")?;
        let signal = noise(40_000, 7);
        let mut app = silence(1_000);
        app.extend(&signal);
        let mut reference = silence(700);
        reference.extend(&signal);
        // Change one sample (frame 20,000 of the signal, channel 1) in the app only.
        let at = (1_000 + 20_000) * 2 + 1;
        app[at] = app[at].wrapping_add(1);
        let a = session(t.path(), "app", &app, 16_000)?;
        let r = session(t.path(), "ref", &reference, 100_000)?;
        let c = cmp(&a, &r, &[0])?;
        assert!(c.aligned && !c.equal, "{c:?}");
        assert_eq!(c.differing_frames, 1);
        let range = &c.segments[0].differing_ranges[0];
        assert_eq!(
            (range.reference_frame, range.app_frame, range.frames),
            (20_700, 21_000, 1)
        );
        assert_eq!(c.segments[0].max_abs_diff, 1);
        Ok(())
    }

    #[test]
    fn dropped_frames_shift_everything_after_them_and_are_not_realigned() -> Result<()> {
        let t = TempDir::new("cmp-drop")?;
        let signal = noise(40_000, 7);
        let mut app = silence(1_000);
        app.extend(&signal[..20_000]);
        app.extend(&signal[20_000 + 2 * 100..]); // 100 frames lost
        let mut reference = silence(700);
        reference.extend(&signal);
        let a = session(t.path(), "app", &app, 16_000)?;
        let r = session(t.path(), "ref", &reference, 100_000)?;
        let c = cmp(&a, &r, &[0])?;
        assert!(c.aligned && !c.equal, "{c:?}");
        // Everything after the loss differs (the noise never matches at the wrong offset).
        assert!(c.differing_frames > 9_000, "{}", c.differing_frames);
        assert_eq!(
            c.segments[0].differing_ranges[0].reference_frame,
            700 + 10_000
        );
        Ok(())
    }

    #[test]
    fn audio_the_app_has_no_recording_of_is_counted() -> Result<()> {
        let t = TempDir::new("cmp-missing")?;
        let signal = noise(40_000, 7);
        let mut app = silence(1_000);
        app.extend(&signal[..2 * 30_000]); // the last 10,000 frames were never recorded
        let mut reference = silence(700);
        reference.extend(&signal);
        let a = session(t.path(), "app", &app, 16_000)?;
        let r = session(t.path(), "ref", &reference, 100_000)?;
        let c = cmp(&a, &r, &[0])?;
        assert_eq!(c.differing_frames, 0);
        assert_eq!(c.reference_only_loud_frames, 10_000);
        assert!(!c.equal);
        Ok(())
    }

    #[test]
    fn audio_in_the_app_that_the_reference_lacks_is_counted() -> Result<()> {
        let t = TempDir::new("cmp-app-only")?;
        let signal = noise(40_000, 7);
        let mut app = silence(500);
        app.extend(&signal);
        // The reference started late: it misses the first 5,000 frames of the signal.
        let reference = signal[2 * 5_000..].to_vec();
        let a = session(t.path(), "app", &app, 16_000)?;
        let r = session(t.path(), "ref", &reference, 100_000)?;
        let c = cmp(&a, &r, &[0])?;
        assert!(c.aligned);
        assert_eq!(c.differing_frames, 0);
        assert_eq!(c.app_only_loud_frames, 5_000);
        Ok(())
    }

    #[test]
    fn a_recording_that_shares_nothing_with_the_reference_is_not_aligned() -> Result<()> {
        let t = TempDir::new("cmp-none")?;
        let a = session(t.path(), "app", &noise(30_000, 1), 16_000)?;
        let r = session(t.path(), "ref", &noise(30_000, 2), 100_000)?;
        let c = cmp(&a, &r, &[0])?;
        assert!(!c.aligned && !c.equal, "{c:?}");
        Ok(())
    }

    #[test]
    fn each_run_of_the_app_has_its_own_offset_and_a_restart_gap_is_not_a_difference() -> Result<()>
    {
        let t = TempDir::new("cmp-runs")?;
        let signal = noise(60_000, 9);
        // The app's first run records signal[0..20,000]; it is killed; the second run records
        // signal[26,000..60,000] (6,000 frames were not recorded during the restart).
        let mut app = silence(300);
        app.extend(&signal[..2 * 20_000]);
        let second_start = (app.len() / 2) as u64;
        app.extend(&signal[2 * 26_000..]);
        let mut reference = silence(900);
        reference.extend(&signal);
        let a = session(t.path(), "app", &app, 16_000)?;
        let r = session(t.path(), "ref", &reference, 200_000)?;
        let c = cmp(&a, &r, &[0, second_start])?;
        assert!(c.aligned && c.equal, "{c:?}");
        assert_eq!(c.segments.len(), 2);
        assert_ne!(c.segments[0].offset_frames, c.segments[1].offset_frames);
        assert_eq!(c.restart_gap_frames, 6_000);
        Ok(())
    }

    #[test]
    fn stale_looped_audio_before_the_signal_does_not_fool_the_alignment_when_the_signal_start_is_given()
    -> Result<()> {
        let t = TempDir::new("cmp-loop")?;
        let signal = noise(40_000, 5);
        // A short buffer replayed in a loop before the signal: it matches itself every 1,000
        // frames, so a snippet taken from it is found at the wrong place.
        let looped = |frames: usize| -> Vec<i16> {
            let unit = noise(1_000, 3);
            unit.iter().copied().cycle().take(frames * 2).collect()
        };
        let mut app = looped(9_000);
        app.extend(&signal);
        // The reference started 4,500 frames later and sees the same stale stream from there.
        let mut reference = looped(9_000)[2 * 4_500..].to_vec();
        reference.extend(&signal);
        let a = session(t.path(), "app", &app, 16_000)?;
        let r = session(t.path(), "ref", &reference, 100_000)?;
        // The app started 4,500 frames earlier: app frame = reference frame + 4,500.
        let hints = Hints {
            reference_signal_start: Some(4_500),
            app_signal_start: Some(9_000),
            ..Hints::default()
        };
        let c = compare(&Recording::load(&a)?, &Recording::load(&r)?, &[0], &hints)?;
        assert!(c.aligned && c.equal, "{c:?}");
        assert_eq!(c.segments[0].offset_frames, Some(4_500));
        Ok(())
    }

    #[test]
    fn reference_audio_from_before_the_app_started_is_not_a_difference() -> Result<()> {
        let t = TempDir::new("cmp-late-app")?;
        let signal = noise(40_000, 11);
        // The reference started with the signal; the app started 6,000 frames into it.
        let a = session(t.path(), "app", &signal[2 * 6_000..], 16_000)?;
        let r = session(t.path(), "ref", &signal, 100_000)?;
        let c = cmp(&a, &r, &[0])?;
        assert!(c.aligned && c.equal, "{c:?}");
        assert_eq!(c.reference_only_loud_frames, 0);
        assert_eq!(c.segments[0].offset_frames, Some(-6_000));
        Ok(())
    }

    #[test]
    fn stale_audio_only_the_app_recorded_before_the_signal_is_not_counted_with_the_signal_start()
    -> Result<()> {
        let t = TempDir::new("cmp-stale-head")?;
        let signal = noise(40_000, 12);
        let stale = noise(3_000, 13);
        let mut app = stale.clone();
        app.extend(&signal);
        // The reference started 2,000 frames later and misses the first 2,000 stale frames.
        let mut reference = stale[2 * 2_000..].to_vec();
        reference.extend(&signal);
        let a = session(t.path(), "app", &app, 16_000)?;
        let r = session(t.path(), "ref", &reference, 100_000)?;
        let hints = Hints {
            reference_signal_start: Some(1_000),
            app_signal_start: Some(3_000),
            ..Hints::default()
        };
        let c = compare(&Recording::load(&a)?, &Recording::load(&r)?, &[0], &hints)?;
        assert!(c.aligned && c.equal, "{c:?}");
        assert_eq!(c.app_only_loud_frames, 0);
        assert_eq!(c.segments[0].app_only_frames, 2_000);
        Ok(())
    }

    #[test]
    fn reference_audio_after_the_app_stopped_can_be_left_unjudged_but_is_still_reported()
    -> Result<()> {
        let t = TempDir::new("cmp-tail")?;
        let signal = noise(40_000, 14);
        // The app was stopped 10,000 frames before the signal ended.
        let a = session(t.path(), "app", &signal[..2 * 30_000], 16_000)?;
        let r = session(t.path(), "ref", &signal, 100_000)?;
        let hints = Hints {
            ignore_reference_tail: true,
            ..Hints::default()
        };
        let c = compare(&Recording::load(&a)?, &Recording::load(&r)?, &[0], &hints)?;
        assert!(c.aligned && c.equal, "{c:?}");
        assert_eq!(c.reference_only_loud_frames, 0);
        assert_eq!(c.reference_tail_loud_frames, 10_000);
        assert!(c.reference_tail_ignored);
        Ok(())
    }
}
