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
    /// The unit's session has more than one language, so the engine detected the language of every chunk (which
    /// the profile's bench never does). Units of the two kinds are not compared with each other.
    pub multi_language: bool,
    /// The unit belongs to a night job. The night safety switch judges the night by night units alone: the day's
    /// rolling units ran other models, often on another mix of devices.
    pub night: bool,
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

    /// How many kept units the factor over all of them rests on.
    pub fn units(&self) -> usize {
        self.recent.len()
    }

    /// How many kept units are of one kind (sessions with more than one language, or with one).
    pub fn units_for(&self, multi_language: bool) -> usize {
        self.recent
            .iter()
            .filter(|u| u.multi_language == multi_language)
            .count()
    }

    /// Measured over promised for one kind of session: the median over the kept units of that kind, `None` until
    /// a unit of that kind has finished. A session with several languages is judged by units of such sessions
    /// only: a unit of a one-language session ran without language detection and says nothing about it.
    pub fn factor_for(&self, multi_language: bool) -> Option<f64> {
        median_ratio(
            self.recent
                .iter()
                .filter(|u| u.multi_language == multi_language),
        )
    }

    /// How many kept units are night units of one kind of session.
    pub fn night_units_for(&self, multi_language: bool) -> usize {
        self.recent
            .iter()
            .filter(|u| u.night && u.multi_language == multi_language)
            .count()
    }

    /// Measured over promised for the night units of one kind of session, `None` until one has finished. The night
    /// safety switch uses it: after the first night units it knows what the night really runs at.
    pub fn night_factor_for(&self, multi_language: bool) -> Option<f64> {
        median_ratio(
            self.recent
                .iter()
                .filter(|u| u.night && u.multi_language == multi_language),
        )
    }

    /// Measured over promised: the median, over all the kept units, of observed RTF divided by the profile's RTF
    /// for the unit. 1.0 means the profile was right; `None` until a unit has finished.
    pub fn factor(&self) -> Option<f64> {
        median_ratio(self.recent.iter())
    }
}

fn median_ratio<'a>(units: impl Iterator<Item = &'a UnitSpeed>) -> Option<f64> {
    let mut ratios: Vec<f64> = units.map(|u| u.observed_rtf() / u.expected_rtf).collect();
    if ratios.is_empty() {
        return None;
    }
    ratios.sort_by(f64::total_cmp);
    let mid = ratios.len() / 2;
    let median = if ratios.len() % 2 == 1 {
        ratios[mid]
    } else {
        (ratios[mid - 1] + ratios[mid]) / 2.0
    };
    Some(median.clamp(FACTOR_RANGE.0, FACTOR_RANGE.1))
}

/// A profile real-time factor moved to this computer's measured speed (unchanged until a unit has finished).
pub fn scaled(profile_rtf: f64, factor: Option<f64>) -> f64 {
    profile_rtf * factor.unwrap_or(1.0)
}

/// The kind of device a night pass decodes on. A hybrid configuration counts as a processor: its processor engine
/// works on a share of the units and the plan must not be more hopeful than that engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceClass {
    Cpu,
    Gpu,
}

/// Which pass an estimate is for: the day's rolling pass or the night pass on a class of device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pass {
    Rolling,
    Night(DeviceClass),
}

// PROVISIONAL emu-low figures (owner, 17 Nov 2026): replace them with the real laptops' measurements (the gate
// pack) before the venue. The profile's bench decodes one language with no language detection, so it
// understates a session with several languages, whose engine detects the language of every chunk. Measured on the
// Legion under `wb-emulate --profile emu-low` (Vulkan UHD, 2 threads, Hindi turbo q5_0 for the rolling pass,
// large-v3 q5_0 for the night pass), the same 288 s unit and the same 295 s night unit, quiet machine:
//   rolling: queue's request (English + Hindi, detection on) 1.011, the profile cell's conditions 0.753;
//   night:   queue's request 1.868, night model alone 1.498.
// Evidence: docs/evidence/phase-4/speed-diagnosis-emu-low.txt.
/// How much slower than the profile a rolling pass of a session with several languages runs (emu-low, provisional).
pub const ROLLING_DETECTION_ALLOWANCE: f64 = 1.011 / 0.753;
/// The same for the night pass on a graphics adapter (emu-low UHD, provisional: the queue's request over the night
/// model alone). The Hinglish model's batch runs measured 1.13 to 1.17 times its bench there
/// (docs/evidence/phase-5/fit-and-fallback.txt against hinglish-speed-A/B.txt), so this figure covers both.
pub const NIGHT_ALLOWANCE_GPU: f64 = 1.868 / 1.498;
/// The same on a processor (provisional): the Hinglish f16 batch over its bench, emu-low 12.82 / 8.98 = 1.43 and emu-mid
/// 4.49 / 3.30 = 1.36. Both come from one 74 s English + Hindi clip, whose first-chunk and detection overhead weighs more than
/// in a night of hours, so they are an upper bound; the real laptops' measurements replace them (gate pack).
pub const NIGHT_ALLOWANCE_CPU: f64 = 12.82 / 8.983;

/// The night allowance of a class of device.
pub fn night_allowance(class: DeviceClass) -> f64 {
    match class {
        DeviceClass::Gpu => NIGHT_ALLOWANCE_GPU,
        DeviceClass::Cpu => NIGHT_ALLOWANCE_CPU,
    }
}

/// The detection allowance for a pass over a session with `languages` languages: 1.0 for one language, the
/// provisional figure of the pass (and, at night, of the device class) for more.
pub fn detection_allowance(pass: Pass, languages: usize) -> f64 {
    if languages <= 1 {
        return 1.0;
    }
    match pass {
        Pass::Rolling => ROLLING_DETECTION_ALLOWANCE,
        Pass::Night(class) => night_allowance(class),
    }
}

/// The factor an estimate multiplies the profile's real-time factors by: what this computer's finished units of
/// the same kind showed (`measured`), and until one has finished the detection allowance for a session with
/// several languages (and 1.0, the profile's word, for one language).
pub fn planning_factor(measured: Option<f64>, pass: Pass, languages: usize) -> f64 {
    measured.unwrap_or_else(|| detection_allowance(pass, languages))
}

/// What the estimates rest on, for the Queue screen's report and the smoke's evidence.
pub fn basis_text(
    multi: Option<f64>,
    multi_units: usize,
    single: Option<f64>,
    single_units: usize,
) -> String {
    let part = |what: &str, factor: Option<f64>, units: usize, allowance: String| match factor {
        Some(f) => {
            format!("{what}: measured, {f:.2} times the profile's, from {units} finished unit(s)")
        }
        None => format!("{what}: {allowance}"),
    };
    format!(
        "{}; {}",
        part(
            "sessions with several languages",
            multi,
            multi_units,
            format!(
                "provisional emu-low detection allowance (x{ROLLING_DETECTION_ALLOWANCE:.2} rolling, x{NIGHT_ALLOWANCE_GPU:.2} night on a graphics adapter, x{NIGHT_ALLOWANCE_CPU:.2} on a processor) until a unit of such a session finishes"
            ),
        ),
        part(
            "sessions with one language",
            single,
            single_units,
            "the profile's speed until a unit finishes".to_owned(),
        )
    )
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
            multi_language: false,
            night: false,
        }
    }

    fn unit_multi(audio_s: u64, took_s: f64, expected: f64) -> UnitSpeed {
        UnitSpeed {
            multi_language: true,
            ..unit(audio_s, took_s, expected)
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

    /// Owner, 17 Nov 2026: until a session's first unit finishes, a session with several languages is planned with
    /// the provisional emu-low detection allowance, measured as the queue's request over the profile cell's
    /// conditions on the same audio.
    #[test]
    fn the_detection_allowance_is_the_measured_emu_low_ratio_and_only_for_several_languages() {
        assert!((ROLLING_DETECTION_ALLOWANCE - 1.011 / 0.753).abs() < 1e-12);
        assert!((NIGHT_ALLOWANCE_GPU - 1.868 / 1.498).abs() < 1e-12);
        assert!((ROLLING_DETECTION_ALLOWANCE - 1.3426).abs() < 1e-3);
        assert!((NIGHT_ALLOWANCE_GPU - 1.2470).abs() < 1e-3);
        for pass in [
            Pass::Rolling,
            Pass::Night(DeviceClass::Gpu),
            Pass::Night(DeviceClass::Cpu),
        ] {
            assert_eq!(detection_allowance(pass, 0), 1.0);
            assert_eq!(detection_allowance(pass, 1), 1.0);
        }
        assert_eq!(
            detection_allowance(Pass::Rolling, 2),
            ROLLING_DETECTION_ALLOWANCE
        );
        assert_eq!(
            detection_allowance(Pass::Night(DeviceClass::Gpu), 3),
            NIGHT_ALLOWANCE_GPU
        );
        assert_eq!(
            detection_allowance(Pass::Night(DeviceClass::Cpu), 3),
            NIGHT_ALLOWANCE_CPU
        );
        // Applied to the profile's cells it lands near what the quiet emu-low runs measured for the queue's
        // request: rolling 0.696 -> 0.93 (measured 1.011), night 1.479 -> 1.84 (measured 1.868).
        let rolling = scaled(0.696, Some(planning_factor(None, Pass::Rolling, 2)));
        let night = scaled(
            1.479,
            Some(planning_factor(None, Pass::Night(DeviceClass::Gpu), 2)),
        );
        assert!((rolling - 1.011).abs() / 1.011 < 0.10, "{rolling}");
        assert!((night - 1.868).abs() / 1.868 < 0.03, "{night}");
        // Without the allowance the profile was 31% and 21% under.
        assert!((0.696f64 - 1.011).abs() / 1.011 > 0.30);
        assert!((1.479f64 - 1.868).abs() / 1.868 > 0.20);
    }

    #[test]
    fn the_first_finished_unit_of_a_session_with_several_languages_replaces_the_allowance() {
        // Nothing finished: the allowance for several languages, the profile's word for one.
        assert_eq!(
            planning_factor(None, Pass::Rolling, 2),
            ROLLING_DETECTION_ALLOWANCE
        );
        assert_eq!(planning_factor(None, Pass::Rolling, 1), 1.0);
        let mut log = SpeedLog::default();
        // A unit of a one-language session says nothing about a session with detection.
        log.record(unit(300, 150.0, 0.5));
        assert_eq!(log.factor_for(true), None);
        assert_eq!(log.units_for(true), 0);
        assert_eq!(log.units_for(false), 1);
        assert_eq!(
            planning_factor(log.factor_for(true), Pass::Rolling, 2),
            ROLLING_DETECTION_ALLOWANCE
        );
        // The first unit of a two-language session takes over, whatever the allowance said.
        log.record(unit_multi(288, 291.1, 0.696));
        let f = log.factor_for(true).unwrap();
        assert!((f - 291.1 / 288.0 / 0.696).abs() < 1e-9, "{f}");
        assert_eq!(planning_factor(Some(f), Pass::Rolling, 2), f);
        // And the one-language factor is its own.
        assert!((log.factor_for(false).unwrap() - 1.0).abs() < 1e-9);
        assert_eq!(log.factor_for(true).map(|_| log.units_for(true)), Some(1));
    }

    #[test]
    fn the_night_plan_is_judged_with_the_allowance_before_any_unit_has_finished() {
        const H: u64 = 3_600_000;
        // 22:30 IST: 9.5 h to 08:00. 5.3 h of Hindi at 1.479 is 7.8 h and fits on the profile's word; with the
        // allowance (1.847) it is 9.8 h and does not.
        let start = 1_800_000_000_000u64 - 1_800_000_000_000 % (24 * H) + 17 * H;
        let audio = vec![SessionAudio {
            id: "a".into(),
            audio_ms_by_lang: BTreeMap::from([("hi".to_string(), 5 * H + 18 * 60_000)]),
        }];
        let profile = BTreeMap::from([("hi".to_string(), 1.479)]);
        assert!(matches!(
            plan(start, &audio, &profile, &profile),
            Plan::Run { .. }
        ));
        let with_allowance: BTreeMap<String, f64> = profile
            .iter()
            .map(|(l, r)| {
                (
                    l.clone(),
                    scaled(
                        *r,
                        Some(planning_factor(None, Pass::Night(DeviceClass::Gpu), 2)),
                    ),
                )
            })
            .collect();
        assert!(matches!(
            plan(start, &audio, &with_allowance, &with_allowance),
            Plan::Skip(SkipReason::DoesNotFit { .. })
        ));
    }

    /// 26 Nov 2026: the night estimate was too optimistic on a processor. The Hinglish f16 batch ran at 1.36 to 1.43 times
    /// its bench on the emulated processors and 1.13 to 1.17 on the integrated graphics; the night allowance follows the
    /// device class, for the ready-by estimate and for the choice of the Hinglish model alike.
    #[test]
    fn the_night_allowance_follows_the_device_class_and_a_processor_night_is_judged_longer() {
        const H: u64 = 3_600_000;
        const { assert!(NIGHT_ALLOWANCE_CPU > NIGHT_ALLOWANCE_GPU) };
        assert!((NIGHT_ALLOWANCE_CPU - 1.427).abs() < 1e-3);
        // 22:30 IST: 9.5 h to 08:00. 2.9 h of Hindi at a bench RTF of 2.5 is 7.25 h: it fits on the bench's word, and
        // on a graphics adapter (x1.25 is 9.04 h); on a processor (x1.43 is 10.35 h) it does not.
        let start = 1_800_000_000_000u64 - 1_800_000_000_000 % (24 * H) + 17 * H;
        let audio = vec![SessionAudio {
            id: "a".into(),
            audio_ms_by_lang: BTreeMap::from([("hi".to_string(), 2 * H + 54 * 60_000)]),
        }];
        let bench = BTreeMap::from([("hi".to_string(), 2.5)]);
        let at = |class| -> BTreeMap<String, f64> {
            bench
                .iter()
                .map(|(l, r)| {
                    (
                        l.clone(),
                        scaled(*r, Some(planning_factor(None, Pass::Night(class), 2))),
                    )
                })
                .collect()
        };
        let gpu = at(DeviceClass::Gpu);
        let cpu = at(DeviceClass::Cpu);
        assert!(matches!(
            plan(start, &audio, &bench, &bench),
            Plan::Run { .. }
        ));
        assert!(matches!(plan(start, &audio, &gpu, &gpu), Plan::Run { .. }));
        assert!(matches!(
            plan(start, &audio, &cpu, &cpu),
            Plan::Skip(SkipReason::DoesNotFit { .. })
        ));
    }

    #[test]
    fn night_units_are_judged_apart_from_the_day_units_of_the_same_log() {
        let mut log = SpeedLog::default();
        // Three day units ran at the profile's word; the first night unit ran 1.6 times slower.
        for _ in 0..3 {
            log.record(UnitSpeed {
                multi_language: true,
                ..unit(300, 300.0 * 0.7, 0.7)
            });
        }
        assert_eq!(log.night_factor_for(true), None);
        log.record(UnitSpeed {
            multi_language: true,
            night: true,
            ..unit(300, 300.0 * 1.6 * 2.0, 2.0)
        });
        assert_eq!(log.night_units_for(true), 1);
        let night = log.night_factor_for(true).unwrap();
        assert!((night - 1.6).abs() < 1e-9, "{night}");
        // The all-units median is still the day's 1.0: it would have hidden the slow night.
        assert!((log.factor_for(true).unwrap() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn the_basis_text_says_whether_the_allowance_or_a_measurement_is_in_use() {
        let first = basis_text(None, 0, None, 0);
        assert!(
            first.contains("provisional emu-low detection allowance"),
            "{first}"
        );
        assert!(
            first
                .contains("x1.34 rolling, x1.25 night on a graphics adapter, x1.43 on a processor"),
            "{first}"
        );
        assert!(
            first.contains("the profile's speed until a unit finishes"),
            "{first}"
        );
        let later = basis_text(Some(1.46), 3, Some(1.0), 2);
        assert!(
            later.contains("measured, 1.46 times the profile's, from 3 finished unit(s)"),
            "{later}"
        );
        assert!(!later.contains("provisional"), "{later}");
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
        assert!(matches!(
            plan(start, &audio, &profile, &profile),
            Plan::Run { .. }
        ));
        let mut log = SpeedLog::default();
        log.record(unit(300, 300.0 * 0.696 * 2.0, 0.696)); // twice as slow as promised
        let f = log.factor();
        let measured: BTreeMap<String, f64> = profile
            .iter()
            .map(|(l, r)| (l.clone(), scaled(*r, f)))
            .collect();
        assert!(matches!(
            plan(start, &audio, &measured, &measured),
            Plan::Skip(SkipReason::DoesNotFit { .. })
        ));
    }
}
