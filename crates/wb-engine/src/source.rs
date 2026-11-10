//! Where `batch` reads audio from: one WAV file (evaluation, bench) or the parts of a session
//! folder (`parts.json` lists the closed 10-minute parts with their audio-clock start times).
//! Everything is returned as 16 kHz mono f32 with the audio-clock time of its first sample.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use wb_protocol::{Source, Unit};

use crate::audio::{self, WHISPER_RATE};

/// The part of `parts.json` the engine needs (the app writes more).
#[derive(Debug, Clone, Deserialize)]
pub struct PartMeta {
    file: String,
    start_ms: u64,
    duration_ms: u64,
    channels: u16,
}

#[derive(Debug, Deserialize)]
struct PartsFile {
    #[serde(default)]
    parts: Vec<PartMeta>,
}

/// Audio for one work unit.
#[derive(Debug)]
pub struct UnitAudio {
    /// 16 kHz mono samples.
    pub pcm: Vec<f32>,
    /// Audio-clock time of `pcm[0]`, in ms.
    pub start_ms: u64,
}

impl UnitAudio {
    pub fn duration_ms(&self) -> u64 {
        self.pcm.len() as u64 * 1000 / u64::from(WHISPER_RATE)
    }
}

pub enum Reader {
    File { pcm: Vec<f32> },
    Session { dir: PathBuf, parts: Vec<PartMeta> },
}

impl Reader {
    pub fn open(source: &Source) -> Result<Self> {
        match source {
            Source::File { path } => Ok(Self::File {
                pcm: audio::read_wav_16k(Path::new(path))?,
            }),
            Source::Session { dir } => {
                let dir = PathBuf::from(dir);
                let text = std::fs::read_to_string(dir.join("parts.json"))
                    .with_context(|| format!("read {}", dir.join("parts.json").display()))?;
                let file: PartsFile =
                    serde_json::from_str(&text).context("parts.json is not valid")?;
                if file.parts.is_empty() {
                    bail!(
                        "{} lists no closed parts yet",
                        dir.join("parts.json").display()
                    );
                }
                Ok(Self::Session {
                    dir,
                    parts: file.parts,
                })
            }
        }
    }

    /// Total audio available, in ms.
    pub fn total_ms(&self) -> u64 {
        match self {
            Self::File { pcm } => pcm.len() as u64 * 1000 / u64::from(WHISPER_RATE),
            Self::Session { parts, .. } => parts
                .iter()
                .map(|p| p.start_ms + p.duration_ms)
                .max()
                .unwrap_or(0),
        }
    }

    /// The units to process: those asked for, or the whole source as one unit.
    pub fn units(&self, asked: &[Unit]) -> Vec<Unit> {
        if asked.is_empty() {
            vec![Unit {
                index: 0,
                start_ms: 0,
                end_ms: self.total_ms(),
            }]
        } else {
            asked.to_vec()
        }
    }

    /// Audio from `start_ms` to `end_ms` of the audio clock.
    pub fn unit_audio(&self, start_ms: u64, end_ms: u64) -> Result<UnitAudio> {
        match self {
            Self::File { pcm } => {
                let a = (start_ms as usize * 16).min(pcm.len());
                let b = (end_ms as usize * 16).min(pcm.len());
                Ok(UnitAudio {
                    pcm: pcm[a..b.max(a)].to_vec(),
                    start_ms,
                })
            }
            Self::Session { dir, parts } => {
                let mut mono_48k: Vec<f32> = Vec::new();
                let mut rate = 0u32;
                let mut first_ms = None;
                for p in parts
                    .iter()
                    .filter(|p| p.start_ms < end_ms && p.start_ms + p.duration_ms > start_ms)
                {
                    let path = dir.join(&p.file);
                    let mut r = hound::WavReader::open(&path)
                        .with_context(|| format!("open {}", path.display()))?;
                    let spec = r.spec();
                    if spec.bits_per_sample != 16 || spec.sample_format != hound::SampleFormat::Int
                    {
                        bail!("{}: only 16-bit PCM parts are supported", path.display());
                    }
                    if rate != 0 && rate != spec.sample_rate {
                        bail!(
                            "{}: the sample rate changed inside the unit ({} then {})",
                            path.display(),
                            rate,
                            spec.sample_rate
                        );
                    }
                    rate = spec.sample_rate;
                    let ch = usize::from(p.channels.max(1));
                    let from_ms = start_ms.max(p.start_ms);
                    let to_ms = end_ms.min(p.start_ms + p.duration_ms);
                    let first_frame = (from_ms - p.start_ms) * u64::from(rate) / 1000;
                    let frames = (to_ms - from_ms) * u64::from(rate) / 1000;
                    r.seek(u32::try_from(first_frame).unwrap_or(u32::MAX))
                        .with_context(|| format!("seek in {}", path.display()))?;
                    let mut inter = Vec::with_capacity(frames as usize * ch);
                    for s in r.samples::<i16>().take(frames as usize * ch) {
                        inter.push(f32::from(s?) / 32_768.0);
                    }
                    mono_48k.extend(audio::to_mono(&inter, ch));
                    first_ms.get_or_insert(from_ms);
                }
                if rate == 0 {
                    return Ok(UnitAudio {
                        pcm: Vec::new(),
                        start_ms,
                    });
                }
                Ok(UnitAudio {
                    pcm: audio::resample_to_16k(&mono_48k, rate)?,
                    start_ms: first_ms.unwrap_or(start_ms),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("wb-engine-source-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("audio")).expect("temp dir");
        d
    }

    fn write_part(dir: &Path, name: &str, seconds: u32, rate: u32, value: i16) {
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(dir.join("audio").join(name), spec).expect("wav");
        for _ in 0..seconds * rate {
            w.write_sample(value).expect("sample");
            w.write_sample(value).expect("sample");
        }
        w.finalize().expect("finalize");
    }

    #[test]
    fn a_unit_spanning_two_parts_is_joined_in_audio_order_at_16_khz() {
        let d = temp("two-parts");
        write_part(&d, "part-0001.wav", 4, 48_000, 8_192);
        write_part(&d, "part-0002.wav", 4, 48_000, -8_192);
        std::fs::write(
            d.join("parts.json"),
            r#"{"parts":[
              {"file":"audio/part-0001.wav","start_ms":0,"duration_ms":4000,"sample_rate":48000,"channels":2},
              {"file":"audio/part-0002.wav","start_ms":4000,"duration_ms":4000,"sample_rate":48000,"channels":2}]}"#,
        )
        .expect("parts.json");
        let r = Reader::open(&Source::Session {
            dir: d.to_string_lossy().into_owned(),
        })
        .expect("open");
        assert_eq!(r.total_ms(), 8_000);
        // Last 2 s of part 1 and first 2 s of part 2.
        let u = r.unit_audio(2_000, 6_000).expect("unit");
        assert_eq!(u.start_ms, 2_000);
        assert_eq!(u.pcm.len(), 64_000);
        assert!(u.pcm[8_000] > 0.2, "{}", u.pcm[8_000]);
        assert!(u.pcm[56_000] < -0.2, "{}", u.pcm[56_000]);
        // Units: none asked means the whole source.
        assert_eq!(
            r.units(&[]),
            vec![Unit {
                index: 0,
                start_ms: 0,
                end_ms: 8_000
            }]
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_session_without_parts_is_an_error_and_a_file_source_slices_by_time() {
        let d = temp("empty");
        std::fs::write(d.join("parts.json"), r#"{"parts":[]}"#).expect("parts.json");
        assert!(
            Reader::open(&Source::Session {
                dir: d.to_string_lossy().into_owned()
            })
            .is_err()
        );
        let r = Reader::File {
            pcm: vec![0.5; 32_000],
        };
        assert_eq!(r.total_ms(), 2_000);
        assert_eq!(r.unit_audio(500, 1_500).expect("slice").pcm.len(), 16_000);
        assert_eq!(r.unit_audio(1_900, 5_000).expect("clip").pcm.len(), 1_600);
        let _ = std::fs::remove_dir_all(&d);
    }
}
