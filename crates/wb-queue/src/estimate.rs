//! Estimates (Phase 4 step 8): per job and per day, from the measured throughput of the configuration and the
//! remaining audio ("All transcripts ready by 19:40 IST"). Times shown to people are IST.

use std::collections::BTreeMap;

use crate::{DAY_MS, IST_OFFSET_MS};

/// Observed audio needed before the job's own speed counts fully (10 minutes of audio).
const FULL_WEIGHT_AUDIO_MS: u64 = 600_000;

/// The real-time factor to plan with: the profile's measured one at first, moving to what this job has
/// actually done as it accumulates audio. A job that has decoded nothing keeps the profile's number.
pub fn blended_rtf(profile_rtf: f64, observed_audio_ms: u64, observed_decode_s: f64) -> f64 {
    if observed_audio_ms == 0 || observed_decode_s <= 0.0 {
        return profile_rtf;
    }
    let observed = observed_decode_s / (observed_audio_ms as f64 / 1000.0);
    let w = (observed_audio_ms as f64 / FULL_WEIGHT_AUDIO_MS as f64).min(1.0);
    w * observed + (1.0 - w) * profile_rtf
}

/// Splits remaining audio over the session's languages by the mix committed so far (milliseconds of audio
/// per language), or evenly when nothing is committed yet.
pub fn split_by_mix(
    remaining_audio_ms: u64,
    languages: &[String],
    committed_mix_ms: &BTreeMap<String, u64>,
) -> Vec<(String, u64)> {
    if languages.is_empty() {
        return Vec::new();
    }
    let total: u64 = languages
        .iter()
        .map(|l| committed_mix_ms.get(l).copied().unwrap_or(0))
        .sum();
    languages
        .iter()
        .map(|l| {
            let share = if total == 0 {
                remaining_audio_ms / languages.len() as u64
            } else {
                (u128::from(remaining_audio_ms)
                    * u128::from(committed_mix_ms.get(l).copied().unwrap_or(0))
                    / u128::from(total)) as u64
            };
            (l.clone(), share)
        })
        .collect()
}

/// Seconds a job still needs: each language's remaining audio times its RTF (`default_rtf` where a language
/// has none), plus the model load time of each unit still to start.
pub fn job_seconds(
    remaining: &[(String, u64)],
    rtf_by_lang: &BTreeMap<String, f64>,
    default_rtf: f64,
    units_remaining: u32,
    load_s_per_unit: f64,
) -> f64 {
    let decode: f64 = remaining
        .iter()
        .map(|(lang, ms)| {
            *ms as f64 / 1000.0 * rtf_by_lang.get(lang).copied().unwrap_or(default_rtf)
        })
        .sum();
    decode + f64::from(units_remaining) * load_s_per_unit
}

/// When everything is ready: the jobs run one after another on the configuration (its RTF already counts
/// both devices of a hybrid), so the seconds add up.
pub fn day_ready_ms(now_ms: u64, job_seconds: &[f64]) -> u64 {
    let total: f64 = job_seconds
        .iter()
        .filter(|s| s.is_finite() && **s > 0.0)
        .sum();
    now_ms + (total * 1000.0).round() as u64
}

/// `19:40` in IST for an epoch time in milliseconds.
pub fn format_ist_hhmm(epoch_ms: u64) -> String {
    let local = (epoch_ms + IST_OFFSET_MS) % DAY_MS;
    format!("{:02}:{:02}", local / 3_600_000, local / 60_000 % 60)
}

/// `2026-11-28 19:40:05 IST` for an epoch time in milliseconds: the stamp of a line in `app.log`.
pub fn format_ist_stamp(epoch_ms: u64) -> String {
    let local = (epoch_ms + IST_OFFSET_MS) % DAY_MS;
    format!(
        "{} {:02}:{:02}:{:02} IST",
        crate::night::ist_date(epoch_ms),
        local / 3_600_000,
        local / 60_000 % 60,
        local / 1000 % 60
    )
}

/// "All transcripts ready by 19:40 IST", or the words for a queue with nothing to do.
pub fn ready_text(now_ms: u64, job_seconds: &[f64]) -> String {
    if job_seconds.iter().all(|s| !(s.is_finite() && *s > 0.0)) {
        return "Nothing is waiting to be transcribed".to_owned();
    }
    format!(
        "All transcripts ready by {} IST",
        format_ist_hhmm(day_ready_ms(now_ms, job_seconds))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> String {
        v.to_owned()
    }

    #[test]
    fn a_new_job_plans_with_the_profile_and_an_old_one_with_its_own_speed() {
        assert_eq!(blended_rtf(0.7, 0, 0.0), 0.7);
        // 10 minutes of audio decoded in 480 s: RTF 0.8, fully trusted.
        assert!((blended_rtf(0.7, 600_000, 480.0) - 0.8).abs() < 1e-9);
        // 5 minutes: half and half.
        assert!((blended_rtf(0.6, 300_000, 240.0) - 0.7).abs() < 1e-9);
    }

    #[test]
    fn remaining_audio_is_split_by_the_language_mix_so_far() {
        let langs = vec![s("en"), s("hi")];
        let mix = BTreeMap::from([(s("en"), 100_000), (s("hi"), 300_000)]);
        assert_eq!(
            split_by_mix(400_000, &langs, &mix),
            vec![(s("en"), 100_000), (s("hi"), 300_000)]
        );
        assert_eq!(
            split_by_mix(400_000, &langs, &BTreeMap::new()),
            vec![(s("en"), 200_000), (s("hi"), 200_000)]
        );
        assert!(split_by_mix(1, &[], &mix).is_empty());
    }

    #[test]
    fn job_seconds_weights_each_language_by_its_rtf_and_adds_the_loads() {
        let rtf = BTreeMap::from([(s("en"), 0.5), (s("hi"), 1.0)]);
        let rem = vec![(s("en"), 600_000), (s("hi"), 600_000), (s("ur"), 100_000)];
        // 600 x 0.5 + 600 x 1.0 + 100 x 0.8 (default) + 3 x 10.
        let t = job_seconds(&rem, &rtf, 0.8, 3, 10.0);
        assert!((t - (300.0 + 600.0 + 80.0 + 30.0)).abs() < 1e-9);
    }

    #[test]
    fn a_log_stamp_is_the_ist_date_and_time_to_the_second() {
        // 2026-11-28 14:35:13 UTC = 2026-11-28 20:05:13 IST; 18:30 UTC is already the next day in IST.
        assert_eq!(
            format_ist_stamp(1_795_876_513_000),
            "2026-11-28 20:05:13 IST"
        );
        assert_eq!(
            format_ist_stamp(1_795_876_513_000 + 4 * 3_600_000 - 5 * 60_000 - 13_000),
            "2026-11-29 00:00:00 IST"
        );
    }

    #[test]
    fn the_day_estimate_is_in_ist() {
        // 2027-02-08 12:00 UTC = 17:30 IST. 133 minutes later is 19:43 IST.
        let noon_utc = 1_802_174_400_000u64 - 1_802_174_400_000 % DAY_MS + 12 * 3_600_000;
        assert_eq!(format_ist_hhmm(noon_utc), "17:30");
        assert_eq!(
            format_ist_hhmm(day_ready_ms(noon_utc, &[3_600.0, 4_380.0])),
            "19:43"
        );
        assert_eq!(
            ready_text(noon_utc, &[3_600.0]),
            "All transcripts ready by 18:30 IST"
        );
        assert_eq!(
            ready_text(noon_utc, &[0.0]),
            "Nothing is waiting to be transcribed"
        );
    }
}
