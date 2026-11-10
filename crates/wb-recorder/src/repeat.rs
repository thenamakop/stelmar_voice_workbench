//! Repeated-block detection (Phase 2, GATES row "1 -> 2").
//!
//! A cable driver, an audio interface or the Windows audio engine can hand a capture client the
//! same block of samples twice (on the Legion: 490 to 580 frames, one WASAPI period). The driver
//! reports no xrun and the timestamps look normal, so nothing else in the app sees it. This
//! detector finds the repeat in the recorded audio itself, in the recorder thread, after the
//! samples are in the ring and before they are written.
//!
//! A repeated block is a stretch of at least [`MIN_RUN`] frames that equals the stretch exactly
//! `lag` frames before it, sample for sample, with `lag` between [`MIN_LAG`] and [`MAX_LAG`]. Real
//! recordings carry noise, so an exact match of that length does not happen by chance. What is
//! not flagged, and why:
//!
//! * digital silence and very quiet passages: the RMS of the matched stretch must be at least
//!   [`MIN_RMS`] (-60 dBFS);
//! * a signal that is periodic with a period under [`MIN_LAG`] (a pure tone, a DC offset): the
//!   most recent equal window is one period back, below the lag range, so it is never a candidate;
//! * a signal periodic with a longer period: the matched run keeps going past `2 * lag` instead of
//!   ending after about `lag` frames, so it is ignored.
//!
//! Cost: one rolling-hash update, one table lookup and, while a candidate run is active, one
//! comparison per frame. All memory is allocated in [`RepeatDetector::new`]; [`push`] never
//! allocates, locks or does I/O.
//!
//! [`push`]: RepeatDetector::push

/// Frames in the window that the hash covers and that must match before a run starts.
const WINDOW: usize = 32;
/// Shortest and longest repeat distance, in frames (2.7 ms to 100 ms at 48 kHz).
pub const MIN_LAG: usize = 128;
pub const MAX_LAG: usize = 4_800;
/// Shortest matched stretch that is reported, in frames.
pub const MIN_RUN: usize = 256;
/// Quietest RMS (full scale = 1.0) of a reported stretch: -60 dBFS.
pub const MIN_RMS: f32 = 0.001;
/// History kept, in frames: room for a lag of [`MAX_LAG`] plus a backward scan of `2 * lag + 1`.
const HISTORY: usize = 16_384;
const TABLE_BITS: u32 = 15;
const TABLE: usize = 1 << TABLE_BITS;
const BASE: u64 = 0x100_0000_01B3;

/// One repeated block found in the audio.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RepeatedBlock {
    /// Frame index (since the detector was created or reset) of the first frame of the repeated copy.
    pub start_frame: u64,
    /// Frames in the repeated copy.
    pub len: u32,
    /// Distance in frames to the copy it repeats. For a whole block repeated once, `len == lag`.
    pub lag: u32,
    /// RMS of the repeated copy in dBFS.
    pub level_dbfs: f32,
}

#[derive(Debug, Clone, Copy)]
struct Run {
    lag: usize,
    len: usize,
    sumsq: f64,
}

pub struct RepeatDetector {
    channels: usize,
    history: Box<[f32]>,
    table: Box<[u32]>,
    /// Frames seen so far.
    pos: u64,
    hash: u64,
    /// `BASE` to the power `WINDOW`, to drop the sample that leaves the window.
    pow: u64,
    run: Option<Run>,
}

impl RepeatDetector {
    pub fn new(channels: u16) -> Self {
        let mut pow = 1u64;
        for _ in 0..WINDOW {
            pow = pow.wrapping_mul(BASE);
        }
        Self {
            channels: usize::from(channels.max(1)),
            history: vec![0.0; HISTORY].into_boxed_slice(),
            table: vec![0; TABLE].into_boxed_slice(),
            pos: 0,
            hash: 0,
            pow,
            run: None,
        }
    }

    /// Forgets everything (a new stream or a new part of the audio clock).
    pub fn reset(&mut self) {
        self.history.fill(0.0);
        self.table.fill(0);
        self.pos = 0;
        self.hash = 0;
        self.run = None;
    }

    /// Frames seen so far.
    pub fn frames_seen(&self) -> u64 {
        self.pos
    }

    /// Feeds interleaved samples (whole frames; a trailing partial frame is ignored) and calls
    /// `found` for every repeated block that ended inside them.
    pub fn push(&mut self, samples: &[f32], mut found: impl FnMut(RepeatedBlock)) {
        let ch = self.channels;
        let inv = 1.0 / ch as f32;
        for frame in samples.chunks_exact(ch) {
            let mut sum = 0.0f32;
            for &s in frame {
                sum += s;
            }
            self.push_mono(sum * inv, &mut found);
        }
    }

    /// Ends a run that is still going (at Stop or before a reset).
    pub fn flush(&mut self, mut found: impl FnMut(RepeatedBlock)) {
        self.finish_run(&mut found);
    }

    #[inline]
    fn push_mono(&mut self, x: f32, found: &mut impl FnMut(RepeatedBlock)) {
        let mask = HISTORY - 1;
        let t = self.pos;
        let slot_now = (t as usize) & mask;
        let old = self.history[(t as usize).wrapping_sub(WINDOW) & mask];
        self.history[slot_now] = x;
        self.hash = self
            .hash
            .wrapping_mul(BASE)
            .wrapping_add(u64::from(x.to_bits()))
            .wrapping_sub(u64::from(old.to_bits()).wrapping_mul(self.pow));

        if let Some(run) = self.run.as_mut() {
            let before = self.history[(t as usize).wrapping_sub(run.lag) & mask];
            if x == before {
                run.len += 1;
                run.sumsq += f64::from(x) * f64::from(x);
            } else {
                self.finish_run(found);
            }
        }

        let slot = (self.hash.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> (64 - TABLE_BITS)) as usize;
        if self.run.is_none() && t >= (WINDOW + MIN_LAG) as u64 {
            let cand = self.table[slot];
            if cand != 0 {
                // `cand` is the frame count when the window was inserted: its last frame is cand - 1.
                let p = u64::from(cand.wrapping_sub(1));
                let lag = (t as u32).wrapping_sub(p as u32) as usize;
                if (MIN_LAG..=MAX_LAG).contains(&lag) {
                    self.try_start_run(t, lag);
                }
            }
        }
        self.table[slot] = (t as u32).wrapping_add(1);
        self.pos = t + 1;
    }

    /// Verifies the window at `t` against `t - lag` and, if it is equal, measures how far back
    /// the match goes and starts tracking the run.
    fn try_start_run(&mut self, t: u64, lag: usize) {
        let mask = HISTORY - 1;
        let at = |k: usize| -> f32 { self.history[(t as usize).wrapping_sub(k) & mask] };
        for j in 0..WINDOW {
            if at(j) != at(j + lag) {
                return;
            }
        }
        // History holds HISTORY frames; the scan needs k + lag < HISTORY.
        let cap = (2 * lag + 1).min(HISTORY - lag - 1);
        let mut n = 0usize;
        let mut sumsq = 0.0f64;
        while n < cap && at(n) == at(n + lag) {
            let v = f64::from(at(n));
            sumsq += v * v;
            n += 1;
        }
        if n >= WINDOW {
            self.run = Some(Run {
                lag,
                len: n,
                sumsq,
            });
        }
    }

    fn finish_run(&mut self, found: &mut impl FnMut(RepeatedBlock)) {
        let Some(run) = self.run.take() else { return };
        // The run ended on the frame before the current one (`finish_run` is called while the
        // current frame is being pushed) or at flush; either way it ends at `pos` exclusive,
        // except while a frame is being pushed, where `pos` has not advanced yet and so is the
        // first frame that did not match: the run spans [pos - len, pos).
        if run.len < MIN_RUN || run.len > 2 * run.lag {
            return;
        }
        let rms = (run.sumsq / run.len as f64).sqrt() as f32;
        if rms < MIN_RMS {
            return;
        }
        found(RepeatedBlock {
            start_frame: self.pos - run.len as u64,
            len: run.len as u32,
            lag: run.lag as u32,
            level_dbfs: 20.0 * rms.log10(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn splitmix(mut z: u64) -> u64 {
        z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// White noise in 16-bit steps at `peak` (full scale = 1.0), `frames` frames of `ch` channels.
    fn noise(frames: usize, ch: usize, seed: u64, peak: f32) -> Vec<f32> {
        let mut v = Vec::with_capacity(frames * ch);
        for f in 0..frames {
            for c in 0..ch {
                let r = splitmix(seed ^ (f as u64).wrapping_mul(0x100_0000_01B3) ^ ((c as u64) << 56));
                let q = ((r >> 32) as u32 % 65_536) as i32 - 32_768;
                v.push((q as f32 / 32_768.0) * peak);
            }
        }
        v
    }

    /// Repeats `len` frames that end at frame `at` (so the copy starts at `at`).
    fn splice_repeat(audio: &[f32], ch: usize, at: usize, len: usize) -> Vec<f32> {
        let mut out = audio[..at * ch].to_vec();
        out.extend_from_slice(&audio[(at - len) * ch..at * ch]);
        out.extend_from_slice(&audio[at * ch..]);
        out
    }

    fn scan(audio: &[f32], ch: u16, chunk: usize) -> Vec<RepeatedBlock> {
        let mut d = RepeatDetector::new(ch);
        let mut out = Vec::new();
        for c in audio.chunks(chunk * usize::from(ch)) {
            d.push(c, |b| out.push(b));
        }
        d.flush(|b| out.push(b));
        out
    }

    #[test]
    fn a_526_frame_repeat_in_noise_is_found_once_at_the_right_place() {
        let audio = splice_repeat(&noise(96_000, 2, 1, 0.25), 2, 40_000, 526);
        let found = scan(&audio, 2, 480);
        assert_eq!(found.len(), 1, "{found:?}");
        let b = found[0];
        assert_eq!((b.start_frame, b.len, b.lag), (40_000, 526, 526));
        assert!((-21.0..-18.0).contains(&b.level_dbfs), "{}", b.level_dbfs);
    }

    #[test]
    fn a_580_frame_repeat_is_found() {
        let audio = splice_repeat(&noise(96_000, 2, 2, 0.25), 2, 70_000, 580);
        let found = scan(&audio, 2, 512);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!((found[0].start_frame, found[0].len), (70_000, 580));
    }

    #[test]
    fn the_result_does_not_depend_on_how_the_audio_is_chunked() {
        let audio = splice_repeat(&noise(60_000, 2, 3, 0.25), 2, 30_000, 500);
        let a = scan(&audio, 2, 480);
        assert_eq!(a, scan(&audio, 2, 1));
        assert_eq!(a, scan(&audio, 2, 7_919));
        assert_eq!(a.len(), 1);
    }

    #[test]
    fn mono_audio_works_too() {
        let audio = splice_repeat(&noise(60_000, 1, 4, 0.25), 1, 20_000, 490);
        let found = scan(&audio, 1, 480);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!((found[0].start_frame, found[0].len), (20_000, 490));
    }

    #[test]
    fn clean_noise_has_no_repeats() {
        assert!(scan(&noise(240_000, 2, 5, 0.25), 2, 480).is_empty());
    }

    #[test]
    fn digital_silence_is_not_a_repeat() {
        assert!(scan(&vec![0.0; 96_000], 1, 480).is_empty());
    }

    #[test]
    fn a_dc_offset_is_not_a_repeat() {
        assert!(scan(&vec![0.2; 96_000], 1, 480).is_empty());
    }

    #[test]
    fn a_repeat_below_the_level_threshold_is_not_reported() {
        // Peak 0.0015 full scale: RMS about -62 dBFS, under the -60 dBFS limit.
        let audio = splice_repeat(&noise(60_000, 1, 6, 0.0015), 1, 30_000, 526);
        assert!(scan(&audio, 1, 480).is_empty());
        // The same repeat 6 dB louder is reported.
        let audio = splice_repeat(&noise(60_000, 1, 6, 0.003), 1, 30_000, 526);
        assert_eq!(scan(&audio, 1, 480).len(), 1);
    }

    #[test]
    fn a_repeat_shorter_than_the_minimum_is_not_reported() {
        let audio = splice_repeat(&noise(60_000, 1, 7, 0.25), 1, 30_000, 200);
        assert!(scan(&audio, 1, 480).is_empty());
    }

    fn periodic(period: usize, frames: usize) -> Vec<f32> {
        let cycle = noise(period, 1, 8, 0.25);
        (0..frames).map(|i| cycle[i % period]).collect()
    }

    #[test]
    fn periodic_signals_are_not_repeats() {
        // A 1 kHz tone at 48 kHz (period 48, under the lag range), a 100 Hz one (480) and a
        // multi-tone with a long period (1200): none is a repeated block.
        for period in [48usize, 480, 1_200, 4_000] {
            let found = scan(&periodic(period, 96_000), 1, 480);
            assert!(found.is_empty(), "period {period}: {found:?}");
        }
    }

    #[test]
    fn a_periodic_signal_with_a_repeat_spliced_in_is_still_quiet() {
        // The repeat is of a periodic signal, so it looks like more of the same: not reported.
        let audio = splice_repeat(&periodic(480, 96_000), 1, 40_000, 480);
        assert!(scan(&audio, 1, 480).is_empty());
    }

    #[test]
    fn two_repeats_in_one_stream_are_both_found_in_order() {
        let a = splice_repeat(&noise(120_000, 2, 9, 0.25), 2, 30_000, 526);
        let b = splice_repeat(&a, 2, 90_000, 548);
        let found = scan(&b, 2, 480);
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!((found[0].start_frame, found[0].len), (30_000, 526));
        assert_eq!((found[1].start_frame, found[1].len), (90_000, 548));
    }

    #[test]
    fn a_repeat_split_across_pushes_and_a_flush_at_the_end() {
        // The repeat is the last thing in the stream: only flush ends the run.
        let base = noise(50_000, 1, 10, 0.25);
        let mut audio = base.clone();
        audio.extend_from_slice(&base[50_000 - 500..]);
        let found = scan(&audio, 1, 333);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!((found[0].start_frame, found[0].len), (50_000, 500));
    }

    #[test]
    fn reset_forgets_the_history() {
        let mut d = RepeatDetector::new(1);
        let base = noise(10_000, 1, 11, 0.25);
        d.push(&base, |_| {});
        d.reset();
        assert_eq!(d.frames_seen(), 0);
        // The same noise again after the reset is new audio, not a repeat of the old.
        let mut out = Vec::new();
        d.push(&base, |b| out.push(b));
        d.flush(|b| out.push(b));
        assert!(out.is_empty());
    }

    #[test]
    fn cost_is_small_per_frame() {
        let audio = noise(480_000, 2, 12, 0.25);
        let start = std::time::Instant::now();
        let found = scan(&audio, 2, 480);
        let per_frame_ns = start.elapsed().as_nanos() as f64 / 480_000.0;
        assert!(found.is_empty());
        // 5% of one core at 48 kHz is about 1,000 ns per frame. Debug builds are slow, so the
        // check is only strict in release; the gate runs measure the real recorder thread.
        if !cfg!(debug_assertions) {
            assert!(per_frame_ns < 1_000.0, "{per_frame_ns} ns per frame");
        }
    }
}
