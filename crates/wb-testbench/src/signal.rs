//! The known test signal: white noise at about -12 dBFS peak, a pure function of (seed, frame,
//! channel), so the analyser can regenerate any stretch of it without reading the source file
//! and can find exactly where a recorded block came from.

/// Default seed for gate runs.
pub const DEFAULT_SEED: u64 = 0x5745_4c4d_4152_2027; // "STELMAR 2027"-ish
/// Peak amplitude in 16-bit units (about -12 dBFS).
pub const AMPLITUDE: i32 = 8_192;

fn splitmix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Sample `frame` of channel `channel`.
pub fn sample(seed: u64, frame: u64, channel: u16) -> i16 {
    let r = splitmix64(
        seed ^ frame
            .wrapping_mul(0x100_0000_01B3)
            .wrapping_add(u64::from(channel) << 56),
    );
    // Uniform in [-AMPLITUDE, AMPLITUDE).
    let v = (r >> 32) as u32 % (2 * AMPLITUDE as u32);
    (v as i32 - AMPLITUDE) as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_and_bounded() {
        for f in 0..10_000u64 {
            let a = sample(DEFAULT_SEED, f, 0);
            assert_eq!(a, sample(DEFAULT_SEED, f, 0));
            assert!((-8_192..8_192).contains(&i32::from(a)));
        }
        assert_ne!(sample(DEFAULT_SEED, 5, 0), sample(DEFAULT_SEED, 5, 1));
        assert_ne!(sample(DEFAULT_SEED, 5, 0), sample(DEFAULT_SEED + 1, 5, 0));
    }
}
