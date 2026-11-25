//! Which models the night pass uses for each session (Phase 5, 26 Nov 2026).
//!
//! An English + Hindi session that was not decoded with the Hinglish model while recording gets it at night when the
//! system check measured it on this computer, it fits the night window and the memory, and the plan made from the
//! audio the sessions really recorded finishes before 08:00 IST. Otherwise the night pass keeps today's models (the
//! plan is made for those alone), and the session is marked "best finished on a faster machine" with the reason.
//! Pure functions: the app's `night_step` supplies the sessions and carries out the decision.

use std::collections::{BTreeMap, BTreeSet};

use wb_queue::night::{self, Plan, Route, Routed, SessionAudio, Speeds};
use wb_queue::speed::scaled;

use super::profile::ProfileView;

/// One session that waits for its night pass.
#[derive(Debug, Clone)]
pub struct NightSession {
    pub name: String,
    pub languages: Vec<String>,
    pub audio: SessionAudio,
}

/// The prefix of the languages of a session planned with the Hinglish model (so that one plan can hold both kinds of
/// session on a day that has both).
const H: &str = "hinglish:";

#[derive(Debug, Clone, PartialEq)]
pub struct NightDecision {
    pub routed: Routed,
    /// The sessions whose night job decodes with the Hinglish model.
    pub hinglish: BTreeSet<String>,
    /// English + Hindi sessions left without the Hinglish model, with the reason: "best finished on a faster machine".
    pub marks: Vec<(String, String)>,
}

fn prefixed(a: &SessionAudio) -> SessionAudio {
    SessionAudio {
        id: a.id.clone(),
        audio_ms_by_lang: a
            .audio_ms_by_lang
            .iter()
            .map(|(l, ms)| (format!("{H}{l}"), *ms))
            .collect(),
    }
}

/// Plans the night: the Hinglish model for the sessions that may use it when that finishes before 08:00, today's
/// models otherwise. `factor` is the planning factor of the speeds (measured speed of this computer, or the
/// provisional allowance for sessions with several languages).
pub fn decide(
    profile: &ProfileView,
    now_ms: u64,
    sessions: &[NightSession],
    factor: f64,
) -> NightDecision {
    let langs: Vec<String> = sessions
        .iter()
        .flat_map(|s| s.languages.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let scale = |m: BTreeMap<String, f64>| -> BTreeMap<String, f64> {
        m.into_iter()
            .map(|(l, r)| (l, scaled(r, Some(factor))))
            .collect()
    };
    let today = Speeds {
        night_rtf: scale(profile.night_rtf(&langs)),
        decode_rtf: scale(profile.night_decode_rtf(&langs)),
    };
    let qualifying: BTreeSet<String> = sessions
        .iter()
        .filter(|s| profile.may_use_hinglish_at_night(&s.languages))
        .map(|s| s.name.clone())
        .collect();
    let plain: Vec<SessionAudio> = sessions.iter().map(|s| s.audio.clone()).collect();
    let hinglish_input = if qualifying.is_empty() {
        None
    } else {
        let mut speeds = today.clone();
        let mut audio = Vec::new();
        for s in sessions {
            if qualifying.contains(&s.name) {
                for (l, r) in scale(profile.hinglish_night_rtf(&s.languages)) {
                    speeds.night_rtf.insert(format!("{H}{l}"), r);
                    speeds.decode_rtf.insert(format!("{H}{l}"), r);
                }
                audio.push(prefixed(&s.audio));
            } else {
                audio.push(s.audio.clone());
            }
        }
        Some((audio, speeds))
    };
    let routed = night::route(
        now_ms,
        &plain,
        hinglish_input.as_ref().map(|(a, s)| (a.as_slice(), s)),
        &today,
    );
    let planned: BTreeSet<String> = match &routed.plan {
        Plan::Run { sessions, .. } => sessions.iter().cloned().collect(),
        Plan::Skip(_) => BTreeSet::new(),
    };
    let hinglish: BTreeSet<String> = if routed.route == Route::Hinglish {
        qualifying.intersection(&planned).cloned().collect()
    } else {
        BTreeSet::new()
    };
    let why_not = match (&routed.hinglish_skip, hinglish_input.is_some()) {
        (Some(reason), _) => format!("the Hinglish night pass was not started: {}", reason.text()),
        (None, false) => "the Hinglish model was not measured on this computer, or it is too slow or too large for it".to_owned(),
        (None, true) => "the Hinglish night pass was not planned".to_owned(),
    };
    let marks = sessions
        .iter()
        .filter(|s| profile.lacks_hinglish(&s.languages) && !hinglish.contains(&s.name))
        .map(|s| (s.name.clone(), why_not.clone()))
        .collect();
    NightDecision {
        routed,
        hinglish,
        marks,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use wb_capability::Hinglish;

    const DAY_MS: u64 = 86_400_000;
    const IST_OFFSET_MS: u64 = 19_800_000;

    /// 18:00 IST on 2027-02-08 in epoch ms.
    fn six_pm() -> u64 {
        let utc_midnight = 1_802_174_400_000u64 - 1_802_174_400_000 % DAY_MS;
        utc_midnight - DAY_MS - IST_OFFSET_MS + 18 * 3_600_000
    }

    fn profile(night_ok: bool, rtf_hi: f64, rolling_ok: bool) -> ProfileView {
        let mut cap = wb_capability::parse_capability(include_str!(
            "../../../crates/wb-capability/fixtures/tier-g.json"
        ))
        .unwrap();
        cap.decision.hinglish = Hinglish {
            ok: rolling_ok,
            rtf: Some(rtf_hi),
            source: "measured".into(),
            model: Some("hinglish-f16".into()),
            night_ok,
            night_hours: Some(rtf_hi * 3.0),
        };
        let mut v = ProfileView::from_capability(&cap, vec![]);
        v.set_cell_for_test("hinglish-f16", "hi", rtf_hi);
        v.set_cell_for_test("hinglish-f16", "en", rtf_hi / 2.5);
        v
    }

    fn session(name: &str, en_min: u64, hi_min: u64) -> NightSession {
        NightSession {
            name: name.into(),
            languages: vec!["en".into(), "hi".into()],
            audio: SessionAudio {
                id: name.into(),
                audio_ms_by_lang: BTreeMap::from([
                    ("en".to_owned(), en_min * 60_000),
                    ("hi".to_owned(), hi_min * 60_000),
                ]),
            },
        }
    }

    #[test]
    fn a_measured_hinglish_model_that_fits_the_night_decodes_the_english_and_hindi_sessions() {
        // The UHD floor: Hindi 1.69, English 0.68 (the measured ratio is about 2.5): 3 h of audio takes about 3.5 h.
        let p = profile(true, 1.69, false);
        let d = decide(&p, six_pm(), &[session("hall-a", 90, 90)], 1.25);
        assert_eq!(d.routed.route, Route::Hinglish, "{d:?}");
        assert!(d.hinglish.contains("hall-a"));
        assert!(d.marks.is_empty(), "{:?}", d.marks);
        assert!(matches!(d.routed.plan, Plan::Run { .. }));
    }

    #[test]
    fn a_night_the_hinglish_model_cannot_finish_keeps_todays_models_and_marks_the_session() {
        // Hindi at 9.0 (a CPU-only low-baseline laptop): 3 h of audio needs about 20 h, only 14 h are left.
        let p = profile(true, 9.0, false);
        let d = decide(&p, six_pm(), &[session("hall-a", 90, 90)], 1.25);
        assert_eq!(d.routed.route, Route::Today);
        assert!(d.hinglish.is_empty());
        assert_eq!(d.marks.len(), 1, "{:?}", d.marks);
        assert!(d.marks[0].1.contains("was not started"), "{}", d.marks[0].1);
        assert!(d.routed.hinglish_skip.is_some());
    }

    #[test]
    fn a_model_that_was_not_measured_or_does_not_fit_the_night_marks_the_session_without_a_hinglish_plan()
     {
        let p = profile(false, 9.0, false);
        let d = decide(&p, six_pm(), &[session("hall-a", 90, 90)], 1.25);
        assert_eq!(d.routed.route, Route::Today);
        assert_eq!(d.routed.hinglish_skip, None);
        assert_eq!(d.marks.len(), 1);
        assert!(d.marks[0].1.contains("not measured"), "{}", d.marks[0].1);
    }

    #[test]
    fn a_session_that_the_recording_pass_already_decoded_with_hinglish_is_never_marked_or_planned_again()
     {
        let p = profile(true, 0.19, true);
        let d = decide(&p, six_pm(), &[session("main", 90, 90)], 1.25);
        assert!(d.marks.is_empty(), "{:?}", d.marks);
        assert!(d.hinglish.is_empty());
    }

    #[test]
    fn a_day_with_an_english_only_session_and_an_english_hindi_session_plans_each_with_its_own_models()
     {
        let p = profile(true, 1.69, false);
        let mut en_only = session("en-only", 60, 0);
        en_only.languages = vec!["en".into()];
        let d = decide(&p, six_pm(), &[session("hall-a", 90, 90), en_only], 1.25);
        assert!(d.hinglish.contains("hall-a"));
        assert!(!d.hinglish.contains("en-only"));
        assert!(
            d.marks.is_empty(),
            "an English-only session is not an English + Hindi session: {:?}",
            d.marks
        );
    }
}
