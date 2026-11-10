//! Input level (RMS and peak in dBFS) over fixed windows, computed on the recorder thread from the
//! samples it drains, never in the audio callback.

use serde::Serialize;

/// Floor for silence and digital zero.
pub const FLOOR_DBFS: f32 = -100.0;

/// One meter reading.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct LevelReading {
    pub rms_dbfs: f32,
    pub peak_dbfs: f32,
    /// A sample reached full scale in this window.
    pub clipped: bool,
}

/// Converts a linear amplitude (0 to 1) to dBFS.
pub fn dbfs(x: f32) -> f32 {
    if x <= 1e-5 {
        FLOOR_DBFS
    } else {
        (20.0 * x.log10()).max(FLOOR_DBFS)
    }
}

/// Accumulates interleaved samples and yields a reading every window.
pub struct LevelMeter {
    window_samples: usize,
    seen: usize,
    sum_sq: f64,
    peak: f32,
}

impl LevelMeter {
    /// A meter giving `per_second` readings for audio at `rate` with `channels`.
    pub fn new(rate: u32, channels: u16, per_second: u32) -> Self {
        let frames = (rate / per_second.max(1)).max(1) as usize;
        Self {
            window_samples: frames * usize::from(channels.max(1)),
            seen: 0,
            sum_sq: 0.0,
            peak: 0.0,
        }
    }

    /// Adds samples; calls `emit` for each completed window.
    pub fn push(&mut self, samples: &[f32], mut emit: impl FnMut(LevelReading)) {
        for &s in samples {
            let a = s.abs();
            if a > self.peak {
                self.peak = a;
            }
            self.sum_sq += f64::from(s) * f64::from(s);
            self.seen += 1;
            if self.seen == self.window_samples {
                let rms = (self.sum_sq / self.seen as f64).sqrt() as f32;
                emit(LevelReading {
                    rms_dbfs: dbfs(rms),
                    peak_dbfs: dbfs(self.peak),
                    clipped: self.peak >= 0.999,
                });
                self.seen = 0;
                self.sum_sq = 0.0;
                self.peak = 0.0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_scale_sine_reads_minus_three_rms() {
        let mut m = LevelMeter::new(48_000, 1, 20);
        let samples: Vec<f32> = (0..2_400)
            .map(|i| (i as f32 / 48_000.0 * 1_000.0 * std::f32::consts::TAU).sin())
            .collect();
        let mut readings = Vec::new();
        m.push(&samples, |r| readings.push(r));
        assert_eq!(readings.len(), 1);
        assert!(
            (readings[0].rms_dbfs + 3.01).abs() < 0.05,
            "{:?}",
            readings[0]
        );
        assert!(readings[0].peak_dbfs.abs() < 0.01);
        assert!(readings[0].clipped);
    }

    #[test]
    fn windows_span_pushes_and_silence_is_floor() {
        let mut m = LevelMeter::new(48_000, 2, 20); // 4800 samples per window
        let mut n = 0;
        for _ in 0..10 {
            m.push(&[0.0; 1000], |r| {
                n += 1;
                assert_eq!(r.rms_dbfs, FLOOR_DBFS);
                assert!(!r.clipped);
            });
        }
        assert_eq!(n, 2);
        assert!((dbfs(0.25) + 12.04).abs() < 0.01);
    }
}
