//! Zero-gap detection: a short run of exact digital silence in the middle of audio.
//!
//! The shared upstream breaks in the 10 Nov 2026 gate evidence are not repeated blocks. At the
//! sample level each is a run of exact zeros of 490 to 551 frames (about one WASAPI period,
//! 10.2 to 11.5 ms) inserted into the noise signal, which the player, the app and the reference
//! capture all record identically (`docs/evidence/phase-2/repeated-block-validation.txt`). Neither
//! the driver's discontinuity flag nor the capture timestamps see it, so the recorder looks for it
//! in the audio.
//!
//! A zero gap is a run of [`MIN_ZERO`] to [`MAX_ZERO`] frames in which every channel is exactly
//! zero, with live audio on both sides: at least half the frames in the 10 ms before it and in the
//! 10 ms after it are not exactly zero. What is not flagged:
//!
//! * digital silence longer than [`MAX_ZERO`] (a muted channel, silence between sessions);
//! * silence at the start or end of a recording, or beside other silence (no live audio on one
//!   side), such as the gap between two words in a recording whose noise floor is exactly zero;
//! * runs shorter than [`MIN_ZERO`].
//!
//! A microphone or an analogue line always has a noise floor above one 16-bit step, so exact zeros
//! in the middle of speech mean the digital path dropped samples. A digitally generated or
//! hard-gated source can produce them legitimately; the event is a mark for review, not a verdict.
//!
//! Allocation happens only in [`DropoutDetector::new`].

/// Shortest and longest run of zeros that is reported, in milliseconds.
const MIN_ZERO_MS: u64 = 2;
const MAX_ZERO_MS: u64 = 50;
/// Live audio is required in this many milliseconds on each side.
const FLANK_MS: u64 = 10;

/// Shortest run reported at 48 kHz, for documentation and tests.
pub const MIN_ZERO: usize = 96;
pub const MAX_ZERO: usize = 2_400;

/// One zero gap found in the audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZeroGap {
    /// Frame index (since the detector was created or reset) of the first zero frame.
    pub start_frame: u64,
    /// Frames of zeros.
    pub len: u32,
}

#[derive(Debug, Clone, Copy)]
struct Pending {
    start: u64,
    len: u64,
    seen: u64,
    live: u64,
}

pub struct DropoutDetector {
    channels: usize,
    min_zero: u64,
    max_zero: u64,
    flank: u64,
    /// 1 for a frame that is not exactly zero, over the last `flank` frames.
    ring: Box<[u8]>,
    ring_pos: usize,
    live_in_ring: u64,
    pos: u64,
    run_len: u64,
    run_start: u64,
    /// The flank before the current zero run was live.
    before_ok: bool,
    pending: Option<Pending>,
}

impl DropoutDetector {
    pub fn new(channels: u16, sample_rate: u32) -> Self {
        let per_ms = u64::from(sample_rate.max(1000)) / 1000;
        let flank = (FLANK_MS * per_ms).max(8);
        Self {
            channels: usize::from(channels.max(1)),
            min_zero: (MIN_ZERO_MS * per_ms).max(1),
            max_zero: MAX_ZERO_MS * per_ms,
            flank,
            ring: vec![0; flank as usize].into_boxed_slice(),
            ring_pos: 0,
            live_in_ring: 0,
            pos: 0,
            run_len: 0,
            run_start: 0,
            before_ok: false,
            pending: None,
        }
    }

    pub fn reset(&mut self) {
        self.ring.fill(0);
        self.ring_pos = 0;
        self.live_in_ring = 0;
        self.pos = 0;
        self.run_len = 0;
        self.before_ok = false;
        self.pending = None;
    }

    /// Frames seen so far.
    pub fn frames_seen(&self) -> u64 {
        self.pos
    }

    /// Feeds interleaved samples and calls `found` for every zero gap whose trailing flank is
    /// complete. A gap is reported up to 10 ms after it ends.
    pub fn push(&mut self, samples: &[f32], mut found: impl FnMut(ZeroGap)) {
        for frame in samples.chunks_exact(self.channels) {
            let zero = frame.iter().all(|&s| s == 0.0);
            self.step(zero, &mut found);
        }
    }

    /// Drops a gap still waiting for its trailing flank: one at the very end is not flanked.
    pub fn flush(&mut self) {
        self.pending = None;
        self.run_len = 0;
    }

    #[inline]
    fn step(&mut self, zero: bool, found: &mut impl FnMut(ZeroGap)) {
        let t = self.pos;
        if let Some(p) = self.pending.as_mut() {
            p.seen += 1;
            if !zero {
                p.live += 1;
            }
            if p.seen >= self.flank {
                let p = *p;
                self.pending = None;
                if p.live * 2 >= self.flank {
                    found(ZeroGap {
                        start_frame: p.start,
                        len: p.len as u32,
                    });
                }
            }
        }
        if zero {
            if self.run_len == 0 {
                self.run_start = t;
                // The flank before the run: at least half of the last `flank` frames were live,
                // once there are that many frames to look at.
                self.before_ok = t >= self.flank && self.live_in_ring * 2 >= self.flank;
            }
            self.run_len += 1;
        } else {
            if self.run_len > 0 {
                if self.before_ok
                    && (self.min_zero..=self.max_zero).contains(&self.run_len)
                    && self.pending.is_none()
                {
                    self.pending = Some(Pending {
                        start: self.run_start,
                        len: self.run_len,
                        seen: 0,
                        live: 0,
                    });
                }
                self.run_len = 0;
            }
        }
        // Slide the flank window.
        let out = u64::from(self.ring[self.ring_pos]);
        let live = u8::from(!zero);
        self.ring[self.ring_pos] = live;
        self.live_in_ring = self.live_in_ring + u64::from(live) - out;
        self.ring_pos = (self.ring_pos + 1) % self.ring.len();
        self.pos = t + 1;
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

    fn noise(frames: usize, ch: usize, seed: u64) -> Vec<f32> {
        let mut v = Vec::with_capacity(frames * ch);
        for f in 0..frames {
            for c in 0..ch {
                let r = splitmix(seed ^ (f as u64).wrapping_mul(0x100_0000_01B3) ^ ((c as u64) << 56));
                let q = ((r >> 32) as u32 % 16_384) as i32 - 8_192;
                // Never exactly zero: the noise floor of a real input.
                let q = if q == 0 { 1 } else { q };
                v.push(q as f32 / 32_768.0);
            }
        }
        v
    }

    fn zero_out(audio: &mut [f32], ch: usize, start: usize, len: usize) {
        for s in &mut audio[start * ch..(start + len) * ch] {
            *s = 0.0;
        }
    }

    fn scan(audio: &[f32], ch: u16, chunk: usize) -> Vec<ZeroGap> {
        let mut d = DropoutDetector::new(ch, 48_000);
        let mut out = Vec::new();
        for c in audio.chunks(chunk * usize::from(ch)) {
            d.push(c, |g| out.push(g));
        }
        d.flush();
        out
    }

    #[test]
    fn a_490_frame_hole_in_noise_is_found_where_it_starts() {
        let mut audio = noise(96_000, 2, 1);
        zero_out(&mut audio, 2, 40_000, 490);
        let found = scan(&audio, 2, 480);
        assert_eq!(found, vec![ZeroGap { start_frame: 40_000, len: 490 }]);
    }

    #[test]
    fn holes_of_537_and_551_frames_are_found_and_chunking_does_not_matter() {
        let mut audio = noise(120_000, 2, 2);
        zero_out(&mut audio, 2, 30_000, 537);
        zero_out(&mut audio, 2, 90_000, 551);
        let a = scan(&audio, 2, 480);
        assert_eq!(a.len(), 2, "{a:?}");
        assert_eq!((a[0].start_frame, a[0].len), (30_000, 537));
        assert_eq!((a[1].start_frame, a[1].len), (90_000, 551));
        assert_eq!(a, scan(&audio, 2, 1));
        assert_eq!(a, scan(&audio, 2, 7_001));
    }

    #[test]
    fn a_hole_in_one_channel_only_is_not_a_gap() {
        let mut audio = noise(60_000, 2, 3);
        for f in 20_000..20_500 {
            audio[f * 2] = 0.0;
        }
        assert!(scan(&audio, 2, 480).is_empty());
    }

    #[test]
    fn silence_that_is_too_short_or_too_long_is_not_reported() {
        let mut audio = noise(200_000, 1, 4);
        zero_out(&mut audio, 1, 30_000, 40); // under 2 ms
        zero_out(&mut audio, 1, 80_000, 3_000); // over 50 ms: a mute, not a dropout
        assert!(scan(&audio, 1, 480).is_empty());
    }

    #[test]
    fn silence_at_the_start_or_the_end_has_no_audio_on_one_side() {
        let mut audio = noise(60_000, 1, 5);
        zero_out(&mut audio, 1, 0, 500);
        zero_out(&mut audio, 1, 59_500, 500);
        assert!(scan(&audio, 1, 480).is_empty());
    }

    #[test]
    fn a_pause_in_digital_silence_around_the_gap_is_not_flagged() {
        // Two seconds of exact silence with a short burst of noise inside: the silence on either
        // side of the burst has no live flank on its far side.
        let mut audio = vec![0.0f32; 96_000];
        let burst = noise(2_000, 1, 6);
        audio[48_000..50_000].copy_from_slice(&burst);
        assert!(scan(&audio, 1, 480).is_empty());
    }

    #[test]
    fn two_holes_closer_than_the_flank_are_not_both_reported() {
        let mut audio = noise(60_000, 1, 7);
        zero_out(&mut audio, 1, 30_000, 500);
        zero_out(&mut audio, 1, 30_700, 500);
        let found = scan(&audio, 1, 480);
        assert!(found.len() <= 1, "{found:?}");
    }

    #[test]
    fn reset_forgets_everything() {
        let mut d = DropoutDetector::new(1, 48_000);
        let mut audio = noise(10_000, 1, 8);
        zero_out(&mut audio, 1, 9_800, 100);
        d.push(&audio[..9_900], |_| {});
        d.reset();
        assert_eq!(d.frames_seen(), 0);
        let mut out = Vec::new();
        d.push(&audio[9_900..], |g| out.push(g));
        assert!(out.is_empty());
    }

    #[test]
    fn documented_limits_match_48_khz() {
        let d = DropoutDetector::new(1, 48_000);
        assert_eq!((d.min_zero as usize, d.max_zero as usize), (MIN_ZERO, MAX_ZERO));
    }
}
