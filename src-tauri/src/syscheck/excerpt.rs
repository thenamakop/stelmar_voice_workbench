//! Cuts a piece out of a bundled bench clip. The quick check times 30 seconds of each clip, the full
//! check 60; the hybrid test gives the two engines different halves of one clip.

use std::path::Path;

use hound::{SampleFormat, WavReader, WavSpec, WavWriter};

#[derive(Debug, thiserror::Error)]
pub enum ExcerptError {
    #[error("{}: {source}", path.display())]
    Wav {
        path: std::path::PathBuf,
        source: hound::Error,
    },
    #[error("{} has only {have:.1} s of audio after {start:.1} s", path.display())]
    TooShort {
        path: std::path::PathBuf,
        have: f64,
        start: f64,
    },
}

fn copy<S: hound::Sample + Copy>(
    reader: &mut WavReader<std::io::BufReader<std::fs::File>>,
    writer: &mut WavWriter<std::io::BufWriter<std::fs::File>>,
    skip: usize,
    take: usize,
) -> Result<usize, hound::Error> {
    let mut written = 0usize;
    for (i, s) in reader.samples::<S>().enumerate() {
        if i < skip {
            continue;
        }
        if written >= take {
            break;
        }
        writer.write_sample(s?)?;
        written += 1;
    }
    Ok(written)
}

/// Writes `len_s` seconds of `src` starting at `start_s` to `dst` (same format), and returns the
/// seconds written (less than `len_s` when the clip ends first).
pub fn write_excerpt(
    src: &Path,
    dst: &Path,
    start_s: f64,
    len_s: f64,
) -> Result<f64, ExcerptError> {
    let wav_err = |p: &Path| {
        let p = p.to_path_buf();
        move |source| ExcerptError::Wav {
            path: p.clone(),
            source,
        }
    };
    let mut reader = WavReader::open(src).map_err(wav_err(src))?;
    let spec: WavSpec = reader.spec();
    let frames = reader.duration() as f64;
    let rate = f64::from(spec.sample_rate);
    let channels = usize::from(spec.channels);
    if frames / rate <= start_s {
        return Err(ExcerptError::TooShort {
            path: src.to_path_buf(),
            have: frames / rate,
            start: start_s,
        });
    }
    let skip = (start_s * rate) as usize * channels;
    let take = (len_s * rate) as usize * channels;
    let mut writer = WavWriter::create(dst, spec).map_err(wav_err(dst))?;
    let written = match (spec.sample_format, spec.bits_per_sample) {
        (SampleFormat::Float, _) => copy::<f32>(&mut reader, &mut writer, skip, take),
        (SampleFormat::Int, 16) => copy::<i16>(&mut reader, &mut writer, skip, take),
        (SampleFormat::Int, _) => copy::<i32>(&mut reader, &mut writer, skip, take),
    }
    .map_err(wav_err(src))?;
    writer.finalize().map_err(wav_err(dst))?;
    Ok(written as f64 / channels as f64 / rate)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn make(path: &Path, seconds: u32) {
        let spec = WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let mut w = WavWriter::create(path, spec).unwrap();
        for i in 0..seconds * 16_000 {
            w.write_sample((i / 16_000) as i16).unwrap();
        }
        w.finalize().unwrap();
    }

    #[test]
    fn an_excerpt_starts_where_asked_and_is_as_long_as_asked() {
        let dir = std::env::temp_dir().join(format!("wb-excerpt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (src, dst) = (dir.join("a.wav"), dir.join("b.wav"));
        make(&src, 10);
        let got = write_excerpt(&src, &dst, 3.0, 4.0).unwrap();
        assert!((got - 4.0).abs() < 1e-6);
        let mut r = WavReader::open(&dst).unwrap();
        assert_eq!(r.duration(), 64_000);
        assert_eq!(r.samples::<i16>().next().unwrap().unwrap(), 3);
        // Asking for more than is left gives what is left; asking past the end is an error.
        assert!((write_excerpt(&src, &dst, 8.0, 5.0).unwrap() - 2.0).abs() < 1e-6);
        assert!(write_excerpt(&src, &dst, 12.0, 1.0).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
