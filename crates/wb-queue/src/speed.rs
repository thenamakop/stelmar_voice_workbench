//! The speed this computer actually reaches, learned from the units it has finished (owner, 17 Nov 2026).
//!
//! The profile's real-time factors come from a bench of one model on one language with no language detection; a
//! unit runs the whole batch path (detection on every chunk of a mixed session, sentence splitting, previous-text
//! context, retries). The second smoke measured 1.45 to 1.79 where the profile said 0.696. Every estimate starts
//! from the profile and, once units have finished, moves to the ratio between what they took and what the profile
//! promised for them: the median of the last few units, so one odd unit does not move it and a slow stretch does
//! not stay for the whole day.

use std::collections::VecDeque;

/// Units with less audio than this say little about speed (a unit of mostly silence ends fast).
pub const MIN_AUDIO_MS: u64 = 60_000;
/// How many finished units the median covers.
pub const KEEP: usize = 5;
/// A ratio outside this range is a broken measurement (a clock jump, a unit that never ran), not a speed.
const FACTOR_RANGE: (f64, f64) = (0.25, 8.0);

/// One finished unit: how much audio it covered, how long it took and what the profile promised for it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UnitSpeed {
    pub audio_ms: u64,
    /// Wall time from the engine's start to the unit's commit, in seconds (load, VAD and every retry included).
    pub took_s: f64,
    /// The profile's real-time factor for the unit's models on its device, weighted by the unit's language mix.
    pub expected_rtf: f64,
}

impl UnitSpeed {
    fn usable(&self) -> bool {
        self.audio_ms >= MIN_AUDIO_MS
            && self.took_s.is_finite()
            && self.took_s > 0.0
            && self.expected_rtf.is_finite()
            && self.expected_rtf > 0.0
    }

    /// Seconds the unit took per second of audio.
    pub fn observed_rtf(&self) -> f64 {
        self.took_s / (self.audio_ms as f64 / 1000.0)
    }
}

/// The last finished units of the day, rolling and night alike (the profile's promise for each is in the
/// unit, so the ratio is comparable across models and devices).
#[derive(Debug, Clone, Default)]
pub struct SpeedLog {
    recent: VecDeque<UnitSpeed>,
}

impl SpeedLog {
    /// Adds a finished unit; a unit that cannot say anything about speed is not kept.
    pub fn record(&mut self, unit: UnitSpeed) {
        if !unit.usable() {
            return;
        }
        self.recent.push_back(unit);
        while self.recent.len() > KEEP {
            self.recent.pop_front();
        }
    }

    /// How many units the factor rests on.
    pub fn units(&self) -> usize {
        self.recent.len()
    }

    /// Measured over promised: the median, over the kept units, of observed RTF divided by the profile's RTF for
    /// the unit. 1.0 means the profile was right; `None` until a unit has finished.
    pub fn factor(&self) -> Option<f64> {
        if self.recent.is_empty() {
            return None;
        }
        let mut ratios: Vec<f64> = self
            .recent
            .iter()
            .map(|u| u.observed_rtf() / u.expected_rtf)
            .collect();
        ratios.sort_by(f64::total_cmp);
        let mid = ratios.len() / 2;
        let median = if ratios.len() % 2 == 1 {
            ratios[mid]
        } else {
            (ratios[mid - 1] + ratios[mid]) / 2.0
        };
        Some(median.clamp(FACTOR_RANGE.0, FACTOR_RANGE.1))
    }
}

/// A profile real-time factor moved to this computer's measured speed (unchanged until a unit has finished).
pub fn scaled(profile_rtf: f64, factor: Option<f64>) -> f64 {
    profile_rtf * factor.unwrap_or(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::night::{Plan, SessionAudio, SkipReason, plan};
    use std::collections::BTreeMap;

    fn unit(audio_s: u64, took_s: f64, expected: f64) -> UnitSpeed {
        UnitSpeed {
            audio_ms: audio_s * 1000,
            took_s,
            expected_rtf: expected,
        }
    }

    #[test]
    fn nothing_is_known_before_a_unit_has_finished_and_the_profile_stands() {
        let log = SpeedLog::default();
        assert_eq!(log.factor(), None);
        assert_eq!(scaled(0.7, log.factor()), 0.7);
    }

    #[test]
    fn the_factor_is_the_median_of_measured_over_promised() {
        let mut log = SpeedLog::default();
        // The second smoke's three rolling units (Hindi turbo, profile 0.696): 1.79, 1.45 and 1.48.
        log.record(unit(295, 529.7, 0.696));
        log.record(unit(288, 417.8, 0.696));
        log.record(unit(285, 421.3, 0.696));
        let f = log.factor().unwrap();
        assert!((f - 1.4782 / 0.696).abs() < 0.01, "{f}");
        // The night profile (1.479) moved by it, before any night unit has run, is within 20% of the night unit's
        // measured 2.73 (the profile alone was off by 46%).
        let night = scaled(1.479, Some(f));
        assert!((night - 2.73).abs() / 2.73 < 0.20, "{night}");
        assert!((1.479f64 - 2.73).abs() / 2.73 > 0.45);
    }

    #[test]
    fn units_with_little_audio_or_a_broken_clock_say_nothing() {
        let mut log = SpeedLog::default();
        log.record(unit(30, 90.0, 0.7)); // under a minute of audio
        log.record(unit(300, 0.0, 0.7)); // no time taken
        log.record(unit(300, f64::NAN, 0.7));
        log.record(unit(300, 200.0, 0.0)); // nothing promised
        assert_eq!(log.units(), 0);
        assert_eq!(log.factor(), None);
    }

    #[test]
    fn only_the_last_five_units_count_so_a_slow_morning_does_not_last_all_day() {
        let mut log = SpeedLog::default();
        for _ in 0..5 {
            log.record(unit(300, 600.0, 0.5)); // ratio 4.0
        }
        assert!((log.factor().unwrap() - 4.0).abs() < 1e-9);
        for _ in 0..5 {
            log.record(unit(300, 150.0, 0.5)); // ratio 1.0
        }
        assert_eq!(log.units(), KEEP);
        assert!((log.factor().unwrap() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn one_odd_unit_does_not_move_the_median_and_an_even_count_takes_the_middle_pair() {
        let mut log = SpeedLog::default();
        for took in [150.0, 150.0, 1500.0] {
            log.record(unit(300, took, 0.5));
        }
        assert!((log.factor().unwrap() - 1.0).abs() < 1e-9);
        log.record(unit(300, 300.0, 0.5)); // ratios 1, 1, 2, 10 -> middle pair 1 and 2
        assert!((log.factor().unwrap() - 1.5).abs() < 1e-9);
    }

    #[test]
    fn a_machine_faster_than_its_profile_pulls_the_estimate_down() {
        let mut log = SpeedLog::default();
        log.record(unit(300, 100.0, 0.5)); // observed 0.333: ratio 0.667
        assert!((scaled(0.9, log.factor()) - 0.6).abs() < 1e-9);
    }

    #[test]
    fn an_impossible_ratio_is_held_to_the_range() {
        let mut log = SpeedLog::default();
        log.record(unit(300, 30_000.0, 0.5));
        assert_eq!(log.factor(), Some(8.0));
    }

    /// The night planner skips a night pass that does not fit before 08:00 IST. With the profile's speed a night
    /// that is really too long looks short; with the measured speed it is skipped, with the reason shown.
    #[test]
    fn the_night_plan_uses_the_measured_speed_and_skips_a_pass_that_would_not_fit() {
        const H: u64 = 3_600_000;
        // 22:30 IST on some day: 9.5 h to 08:00.
        let start = 1_800_000_000_000u64 - 1_800_000_000_000 % (24 * H) + 17 * H;
        let audio = vec![SessionAudio {
            id: "a".into(),
            audio_ms_by_lang: BTreeMap::from([("hi".to_string(), 6 * H)]),
        }];
        let profile = BTreeMap::from([("hi".to_string(), 1.479)]);
        // 6 h of Hindi at 1.479 is 8.9 h: it fits in 9.5 h on the profile's word.
        assert!(matches!(plan(start, &audio, &profile), Plan::Run { .. }));
        let mut log = SpeedLog::default();
        log.record(unit(300, 300.0 * 0.696 * 2.0, 0.696)); // twice as slow as promised
        let f = log.factor();
        let measured: BTreeMap<String, f64> = profile
            .iter()
            .map(|(l, r)| (l.clone(), scaled(*r, f)))
            .collect();
        assert!(matches!(
            plan(start, &audio, &measured),
            Plan::Skip(SkipReason::DoesNotFit { .. })
        ));
    }
}
