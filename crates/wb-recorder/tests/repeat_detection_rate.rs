//! Measures how many spliced-in repeats the `repeated_block` detector finds in real speech
//! (R3, owner's decision of 12 Nov 2026).
//!
//! Method: FLEURS test clips from `eval/data` (16 kHz mono, git-ignored, never committed; English,
//! Hindi and Urdu, 40 clips each, spread evenly over the sorted file names). Each clip gets ten
//! separate splices, each in its own copy of the clip. A splice is a repeat at a random place:
//! `lag` is drawn from 400 to 1,200 frames and `len` from 256 to 1,200 frames, and the `len`
//! frames after the splice point are made equal to the frames `lag` before them, sample for
//! sample, the way a driver that hands over the same period again does (so with `len > lag` the
//! period repeats more than once). The audio after the splice point follows unchanged. The same
//! clips without a splice are the control for false alarms.
//!
//! A splice counts as found when a reported block overlaps the spliced stretch by at least half
//! of the stretch. The detector is the recorder's own `RepeatDetector`, fed 480 frames at a time
//! (its result does not depend on the chunking). Misses are split by what the detector's design
//! already excludes (see `repeat.rs`): a stretch whose RMS is under -60 dBFS, and a run longer
//! than twice its lag (taken for a periodic signal).
//!
//! Needs the clips, so it is ignored by default. Run it with
//! `cargo test -p wb-recorder --release --test repeat_detection_rate -- --ignored --nocapture`
//! (`scripts/gates/repeat-detection-rate.ps1` does that and writes the evidence). The report goes
//! to standard output, one line each, starting with `RATE `.

use std::path::{Path, PathBuf};

use wb_recorder::repeat::{MIN_RMS, RepeatDetector, RepeatedBlock};
use wb_recorder::wav;

const CLIPS_PER_LANGUAGE: usize = 40;
const SPLICES_PER_CLIP: usize = 10;
const MARGIN: usize = 4_000;
const CHUNK: usize = 480;

fn splitmix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A number in `lo..=hi` drawn from the stream `seed`.
fn draw(seed: u64, lo: usize, hi: usize) -> usize {
    lo + (splitmix(seed) % ((hi - lo + 1) as u64)) as usize
}

fn eval_data() -> PathBuf {
    std::env::var_os("WB_EVAL_DATA").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../../eval/data"),
        PathBuf::from,
    )
}

fn clip_paths(lang: &str) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let dir = eval_data().join(format!("fleurs-{lang}")).join("audio");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map_err(|e| {
            format!(
                "{}: {e} (the FLEURS clips are not on this machine)",
                dir.display()
            )
        })?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "wav"))
        .collect();
    files.sort();
    let step = (files.len() / CLIPS_PER_LANGUAGE).max(1);
    Ok(files
        .into_iter()
        .step_by(step)
        .take(CLIPS_PER_LANGUAGE)
        .collect())
}

fn scan(audio: &[i16]) -> Vec<RepeatedBlock> {
    let f: Vec<f32> = audio.iter().map(|&s| f32::from(s) / 32_768.0).collect();
    let mut det = RepeatDetector::new(1);
    let mut found = Vec::new();
    for c in f.chunks(CHUNK) {
        det.push(c, |b| found.push(b));
    }
    det.flush(|b| found.push(b));
    found
}

fn rms(audio: &[i16]) -> f32 {
    if audio.is_empty() {
        return 0.0;
    }
    let sum: f64 = audio
        .iter()
        .map(|&s| {
            let v = f64::from(s) / 32_768.0;
            v * v
        })
        .sum();
    (sum / audio.len() as f64).sqrt() as f32
}

/// `base` with `len` frames after `at` made equal to the frames `lag` before them.
fn splice(base: &[i16], at: usize, lag: usize, len: usize) -> Vec<i16> {
    let mut out = base[..at].to_vec();
    for _ in 0..len {
        out.push(out[out.len() - lag]);
    }
    out.extend_from_slice(&base[at..]);
    out
}

#[derive(Default, Clone, Copy)]
struct Tally {
    n: u32,
    found: u32,
}

impl Tally {
    fn add(&mut self, found: bool) {
        self.n += 1;
        self.found += u32::from(found);
    }

    fn text(&self) -> String {
        if self.n == 0 {
            "none".into()
        } else {
            format!(
                "{} of {} found ({:.1}%)",
                self.found,
                self.n,
                100.0 * f64::from(self.found) / f64::from(self.n)
            )
        }
    }
}

#[test]
#[ignore = "needs the FLEURS clips in eval/data; run by scripts/gates/repeat-detection-rate.ps1"]
fn measured_detection_rate_of_spliced_repeats_in_real_speech()
-> Result<(), Box<dyn std::error::Error>> {
    let mut all = Tally::default();
    let mut in_scope = Tally::default(); // len <= 2 * lag and RMS at least -60 dBFS
    let mut by_shape = [Tally::default(); 3]; // len <= lag, lag < len <= 2*lag, len > 2*lag
    let mut quiet = Tally::default();
    let mut lag_right = 0u32;
    let mut controls = 0u32;
    let mut false_alarms = 0u32;
    let mut clips = 0u32;
    let mut total_minutes = 0.0f64;
    for lang in ["en", "hi", "ur"] {
        let mut lang_tally = Tally::default();
        let mut lang_clips = 0u32;
        for (ci, path) in clip_paths(lang)?.iter().enumerate() {
            let (_, base) = wav::read_i16(path)?;
            if base.len() < 2 * MARGIN + 3_000 {
                continue;
            }
            clips += 1;
            lang_clips += 1;
            total_minutes += base.len() as f64 / 16_000.0 / 60.0;
            controls += 1;
            false_alarms += scan(&base).len() as u32;
            for k in 0..SPLICES_PER_CLIP {
                let seed = splitmix(
                    ((ci as u64) << 32) ^ ((k as u64) << 8) ^ u64::from(lang.as_bytes()[0]),
                );
                let lag = draw(seed ^ 0xA1, 400, 1_200);
                let len = draw(seed ^ 0xB2, 256, 1_200);
                let hi = base.len() - len - MARGIN;
                let lo = lag + MARGIN;
                if hi <= lo {
                    continue;
                }
                let at = draw(seed ^ 0xC3, lo, hi);
                let out = splice(&base, at, lag, len);
                let level = rms(&out[at..at + len]);
                let found: Vec<RepeatedBlock> = scan(&out);
                let hit = found.iter().find(|b| {
                    let s = b.start_frame as usize;
                    let e = s + b.len as usize;
                    let overlap = e.min(at + len).saturating_sub(s.max(at));
                    overlap * 2 >= len
                });
                let is_hit = hit.is_some();
                all.add(is_hit);
                lang_tally.add(is_hit);
                if let Some(b) = hit
                    && b.lag as usize == lag
                {
                    lag_right += 1;
                }
                let shape = if len <= lag {
                    0
                } else if len <= 2 * lag {
                    1
                } else {
                    2
                };
                by_shape[shape].add(is_hit);
                let loud = level >= MIN_RMS;
                if !loud {
                    quiet.add(is_hit);
                }
                if shape < 2 && loud {
                    in_scope.add(is_hit);
                }
            }
        }
        println!(
            "RATE language {lang}: {lang_clips} clips, splices {}",
            lang_tally.text()
        );
    }
    println!(
        "RATE method: FLEURS test clips from eval/data (git-ignored, not committed), 16 kHz mono, {CLIPS_PER_LANGUAGE} per language spread over the sorted file names; {SPLICES_PER_CLIP} separate splices per clip; lag 400 to 1,200 frames and length 256 to 1,200 frames drawn uniformly, splice point anywhere {MARGIN} frames or more from the ends; the length frames after the point equal the frames lag before them; found = a reported block overlaps at least half of the spliced stretch"
    );
    println!(
        "RATE detector: wb_recorder::repeat::RepeatDetector (the recorder's own code), fed {CHUNK} frames at a time, minimum run 256 frames, lag range 128 to 4,800 frames, minimum RMS -60 dBFS, runs over 2 x lag ignored as periodic"
    );
    println!("RATE clips: {clips} ({total_minutes:.1} minutes of speech before splicing)");
    println!("RATE all splices: {}", all.text());
    println!(
        "RATE within the detector's design (length at most 2 x lag, stretch RMS at least -60 dBFS): {}",
        in_scope.text()
    );
    println!(
        "RATE length at most lag (a whole block repeated once or part of one): {}",
        by_shape[0].text()
    );
    println!(
        "RATE length between lag and 2 x lag: {}",
        by_shape[1].text()
    );
    println!(
        "RATE length over 2 x lag (excluded by design as a periodic signal): {}",
        by_shape[2].text()
    );
    println!(
        "RATE stretch RMS under -60 dBFS (excluded by design as near-silence): {}",
        quiet.text()
    );
    println!(
        "RATE found with the right lag: {lag_right} of {}",
        all.found
    );
    println!(
        "RATE control (the same {controls} clips without a splice): {false_alarms} reported blocks"
    );
    assert!(all.n > 0, "no splices were made");
    Ok(())
}
