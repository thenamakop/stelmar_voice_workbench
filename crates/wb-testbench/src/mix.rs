//! Builds the speech source of the microphone run: public test clips (16-bit PCM WAV, any rate)
//! joined with pauses, repeated to the length asked for, written as 48 kHz stereo 16-bit with one
//! stretch of true silence (the room-silence window). No transcripts, no private audio: the
//! clips are the ones the app ships in `resources/bench`.

use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;

/// Linear-interpolation resampling of mono samples. Good enough for a playback source: the
/// recorder is compared against references that heard the same loudspeaker.
pub fn resample_linear(src: &[i16], from: u32, to: u32) -> Vec<i16> {
    if src.is_empty() || from == 0 || to == 0 {
        return Vec::new();
    }
    if from == to {
        return src.to_vec();
    }
    let out_len = (src.len() as u64 * u64::from(to) / u64::from(from)) as usize;
    let mut out = Vec::with_capacity(out_len);
    let ratio = f64::from(from) / f64::from(to);
    for i in 0..out_len {
        let pos = i as f64 * ratio;
        let i0 = pos.floor() as usize;
        let frac = pos - i0 as f64;
        let a = f64::from(src[i0.min(src.len() - 1)]);
        let b = f64::from(src[(i0 + 1).min(src.len() - 1)]);
        out.push((a + (b - a) * frac).round() as i16);
    }
    out
}

/// One cycle of the source: each clip followed by `pause_s` of silence, at `rate`.
pub fn build_cycle(clips: &[Vec<i16>], pause_s: f64, rate: u32) -> Vec<i16> {
    let pause = (pause_s * f64::from(rate)).round() as usize;
    let mut cycle = Vec::new();
    for c in clips {
        cycle.extend_from_slice(c);
        cycle.extend(std::iter::repeat_n(0i16, pause));
    }
    cycle
}

/// The sample at frame `f` of the endless repetition of `cycle`, zero inside the silent window
/// `[silent_from, silent_to)` (frames).
pub fn sample_at(cycle: &[i16], f: u64, silent_from: u64, silent_to: u64) -> i16 {
    if f >= silent_from && f < silent_to {
        return 0;
    }
    if cycle.is_empty() {
        return 0;
    }
    cycle[(f % cycle.len() as u64) as usize]
}

/// The mono samples and rate of a 16-bit PCM WAV (the first channel when it has more).
pub fn read_clip(path: &Path) -> Result<(Vec<i16>, u32)> {
    let mut r =
        hound::WavReader::open(path).with_context(|| format!("opening {}", path.display()))?;
    let spec = r.spec();
    if spec.bits_per_sample != 16 || spec.sample_format != hound::SampleFormat::Int {
        bail!("{}: only 16-bit PCM clips are supported", path.display());
    }
    let ch = usize::from(spec.channels.max(1));
    let all: Vec<i16> = r.samples::<i16>().collect::<std::result::Result<_, _>>()?;
    Ok((all.chunks(ch).map(|f| f[0]).collect(), spec.sample_rate))
}

#[derive(Serialize)]
pub struct MixInfo {
    pub frames: u64,
    pub rate: u32,
    pub channels: u16,
    pub cycle_seconds: f64,
    pub silent_from_s: f64,
    pub silent_for_s: f64,
}

/// Writes the source. `clips`: paths; `seconds`: length; `silent`: (from_s, for_s), `for_s` 0 = none.
pub fn write_mix(
    out: &Path,
    clips: &[&Path],
    seconds: u64,
    rate: u32,
    pause_s: f64,
    silent: (f64, f64),
    gain: f64,
) -> Result<MixInfo> {
    if clips.is_empty() {
        return Err(anyhow!("--clips needs at least one file"));
    }
    let mut resampled = Vec::new();
    for p in clips {
        let (s, r) = read_clip(p)?;
        let mut v = resample_linear(&s, r, rate);
        if (gain - 1.0).abs() > f64::EPSILON {
            for x in &mut v {
                *x = (f64::from(*x) * gain).clamp(-32768.0, 32767.0) as i16;
            }
        }
        resampled.push(v);
    }
    let cycle = build_cycle(&resampled, pause_s, rate);
    if cycle.is_empty() {
        bail!("the clips are empty");
    }
    let frames = seconds * u64::from(rate);
    let from = (silent.0 * f64::from(rate)).round() as u64;
    let to = from + (silent.1 * f64::from(rate)).round() as u64;
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(out, spec)?;
    for f in 0..frames {
        let s = sample_at(&cycle, f, from, to);
        w.write_sample(s)?;
        w.write_sample(s)?;
    }
    w.finalize()?;
    Ok(MixInfo {
        frames,
        rate,
        channels: 2,
        cycle_seconds: cycle.len() as f64 / f64::from(rate),
        silent_from_s: silent.0,
        silent_for_s: silent.1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampling_triples_the_length_and_keeps_the_ends() {
        let src = vec![0i16, 300, 600, 900];
        let out = resample_linear(&src, 16_000, 48_000);
        assert_eq!(out.len(), 12);
        assert_eq!(out[0], 0);
        assert_eq!(out[3], 300);
        assert_eq!(out[1], 100);
    }

    #[test]
    fn same_rate_is_a_copy_and_empty_stays_empty() {
        assert_eq!(resample_linear(&[1, 2, 3], 48_000, 48_000), vec![1, 2, 3]);
        assert!(resample_linear(&[], 16_000, 48_000).is_empty());
    }

    #[test]
    fn the_cycle_has_a_pause_after_every_clip() {
        let c = build_cycle(&[vec![5; 4], vec![7; 2]], 0.5, 8);
        assert_eq!(c.len(), 4 + 4 + 2 + 4);
        assert_eq!(c[4..8], [0, 0, 0, 0]);
        assert_eq!(c[8], 7);
    }

    #[test]
    fn the_source_repeats_and_is_silent_inside_the_window() {
        let cycle = vec![1i16, 2, 3];
        assert_eq!(sample_at(&cycle, 0, 100, 200), 1);
        assert_eq!(sample_at(&cycle, 4, 100, 200), 2);
        assert_eq!(sample_at(&cycle, 100, 100, 200), 0);
        assert_eq!(sample_at(&cycle, 199, 100, 200), 0);
        assert_eq!(sample_at(&cycle, 200, 100, 200), (200 % 3) as i16 + 1);
        assert_eq!(sample_at(&[], 5, 0, 0), 0);
    }

    #[test]
    fn a_written_mix_has_the_length_and_the_silence() {
        let dir = std::env::temp_dir().join(format!("wb-mix-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let clip = dir.join("clip.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&clip, spec).expect("clip");
        for i in 0..16_000 {
            w.write_sample(((i % 100) as i16 + 1) * 10).expect("sample");
        }
        w.finalize().expect("finalize");
        let out = dir.join("mix.wav");
        let info =
            write_mix(&out, &[clip.as_path()], 4, 48_000, 0.5, (2.0, 1.0), 1.0).expect("mix");
        assert_eq!(info.frames, 4 * 48_000);
        let mut r = hound::WavReader::open(&out).expect("open");
        assert_eq!(r.spec().channels, 2);
        let all: Vec<i16> = r.samples::<i16>().map(|s| s.expect("s")).collect();
        assert_eq!(all.len() as u64, info.frames * 2);
        let inside = &all[(2 * 48_000 * 2) as usize..(3 * 48_000 * 2) as usize];
        assert!(inside.iter().all(|s| *s == 0));
        assert!(all[..2 * 48_000].iter().any(|s| *s != 0));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
