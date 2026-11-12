//! Reading audio and bringing it to what whisper.cpp takes: 16 kHz mono f32.

use std::path::Path;

use anyhow::{Context, Result, bail};
use rubato::{FftFixedIn, Resampler};

pub const WHISPER_RATE: u32 = 16_000;

/// Downmixes interleaved samples to mono by averaging the channels.
pub fn to_mono(interleaved: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return interleaved.to_vec();
    }
    let inv = 1.0 / channels as f32;
    interleaved
        .chunks_exact(channels)
        .map(|f| f.iter().sum::<f32>() * inv)
        .collect()
}

/// Resamples mono audio from `from` Hz to 16 kHz. A rate of 16 kHz is returned unchanged.
pub fn resample_to_16k(mono: &[f32], from: u32) -> Result<Vec<f32>> {
    if from == WHISPER_RATE || mono.is_empty() {
        return Ok(mono.to_vec());
    }
    const CHUNK: usize = 1_024;
    let mut rs = FftFixedIn::<f32>::new(from as usize, WHISPER_RATE as usize, CHUNK, 2, 1)
        .context("could not build the resampler")?;
    let delay = rs.output_delay();
    let expected = (mono.len() as u64 * u64::from(WHISPER_RATE) / u64::from(from)) as usize;
    let mut out: Vec<f32> = Vec::with_capacity(expected + delay + CHUNK);
    let mut pos = 0usize;
    while pos + CHUNK <= mono.len() {
        let block = rs
            .process(&[&mono[pos..pos + CHUNK]], None)
            .context("resampling failed")?;
        out.extend_from_slice(&block[0]);
        pos += CHUNK;
    }
    if pos < mono.len() {
        let tail = rs
            .process_partial(Some(&[&mono[pos..]]), None)
            .context("resampling the tail failed")?;
        out.extend_from_slice(&tail[0]);
    }
    // Flush what the filter still holds, so the output is as long as the input implies.
    let mut guard = 0;
    while out.len() < delay + expected && guard < 8 {
        guard += 1;
        let more = rs
            .process_partial::<&[f32]>(None, None)
            .context("resampling flush failed")?;
        if more[0].is_empty() {
            break;
        }
        out.extend_from_slice(&more[0]);
    }
    let start = delay.min(out.len());
    let end = (delay + expected).min(out.len());
    Ok(out[start..end].to_vec())
}

/// Reads a WAV file (16, 24 or 32-bit integer or 32-bit float PCM) as 16 kHz mono f32.
pub fn read_wav_16k(path: &Path) -> Result<Vec<f32>> {
    let mut reader =
        hound::WavReader::open(path).with_context(|| format!("open {}", path.display()))?;
    let spec = reader.spec();
    let channels = usize::from(spec.channels.max(1));
    let interleaved: Vec<f32> = match (spec.sample_format, spec.bits_per_sample) {
        (hound::SampleFormat::Float, 32) => reader
            .samples::<f32>()
            .collect::<std::result::Result<_, _>>()?,
        (hound::SampleFormat::Int, 16) => reader
            .samples::<i16>()
            .map(|s| s.map(|v| f32::from(v) / 32_768.0))
            .collect::<std::result::Result<_, _>>()?,
        (hound::SampleFormat::Int, bits @ (24 | 32)) => {
            let scale = 1.0 / (1u64 << (bits - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 * scale))
                .collect::<std::result::Result<_, _>>()?
        }
        (fmt, bits) => bail!(
            "{}: unsupported WAV format {fmt:?} with {bits} bits",
            path.display()
        ),
    };
    resample_to_16k(&to_mono(&interleaved, channels), spec.sample_rate)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(hz: f32, rate: u32, seconds: f32) -> Vec<f32> {
        (0..(rate as f32 * seconds) as usize)
            .map(|i| (2.0 * std::f32::consts::PI * hz * i as f32 / rate as f32).sin() * 0.5)
            .collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    #[test]
    fn stereo_is_averaged_to_mono() {
        assert_eq!(to_mono(&[1.0, 0.0, 0.5, 0.5], 2), vec![0.5, 0.5]);
        assert_eq!(to_mono(&[0.25, 0.75], 1), vec![0.25, 0.75]);
    }

    #[test]
    fn a_48_khz_clip_becomes_16_khz_with_the_same_length_and_level() -> Result<()> {
        let x = sine(440.0, 48_000, 2.0);
        let y = resample_to_16k(&x, 48_000)?;
        assert_eq!(y.len(), 32_000);
        assert!(
            (rms(&y[2_000..30_000]) - rms(&x)).abs() < 0.01,
            "{} vs {}",
            rms(&y),
            rms(&x)
        );
        Ok(())
    }

    #[test]
    fn other_rates_work_and_16_khz_is_untouched() -> Result<()> {
        let x = sine(300.0, 44_100, 1.5);
        let y = resample_to_16k(&x, 44_100)?;
        assert_eq!(y.len(), 24_000);
        let z = sine(300.0, 16_000, 1.0);
        assert_eq!(resample_to_16k(&z, 16_000)?, z);
        assert!(resample_to_16k(&[], 48_000)?.is_empty());
        Ok(())
    }

    #[test]
    fn a_tone_stays_a_tone() -> Result<()> {
        // 1 kHz at 48 kHz, resampled: the period is still 16 samples at 16 kHz.
        let y = resample_to_16k(&sine(1_000.0, 48_000, 1.0), 48_000)?;
        let mid = &y[4_000..12_000];
        let crossings = mid.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        assert!(
            (crossings as i64 - 500).abs() <= 2,
            "{crossings} rising crossings"
        );
        Ok(())
    }
}
