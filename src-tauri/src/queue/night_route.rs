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

/// What becomes of a session whose units were decoded on the processor without the Hinglish model after a graphics
/// failure (owner, 27 Nov 2026): the night pass redoes it with the Hinglish model when the night routing says that
/// finishes before 08:00, else it is marked "best finished on a faster machine".
#[derive(Debug, Clone, PartialEq)]
pub struct Redo {
    /// The night routing runs the session with the Hinglish model.
    pub redone: bool,
    /// Why not, when it does not (the text of the faster-machine marker).
    pub why_not: Option<String>,
    /// The line for the Queue log and the notice.
    pub text: String,
}

/// Projects the night for one session that fell back: the same routing the night pass uses, with the audio known now,
/// started at `night_start_ms`, on the profile of a redo (`ProfileView::for_redo`). The night pass decides again with the
/// audio really recorded.
pub fn project_redo(
    profile: &ProfileView,
    graphics_abandoned: bool,
    night_start_ms: u64,
    session: &NightSession,
    factor: f64,
) -> Redo {
    let redo_profile = profile.for_redo(graphics_abandoned);
    let d = decide(
        &redo_profile,
        night_start_ms,
        std::slice::from_ref(session),
        factor,
    );
    let head = format!(
        "{}: decoded without the Hinglish model after a graphics failure",
        session.name
    );
    if d.hinglish.contains(&session.name) {
        return Redo {
            redone: true,
            why_not: None,
            text: format!("{head}; will be redone at night"),
        };
    }
    let why = d
        .marks
        .iter()
        .find(|(n, _)| n == &session.name)
        .map(|(_, w)| w.clone())
        .unwrap_or_else(|| "the Hinglish night pass was not planned".to_owned());
    Redo {
        redone: false,
        text: format!("{head}; needs a faster machine ({why})"),
        why_not: Some(why),
    }
}

/// A night job decoding an English + Hindi session with the Hinglish model, as the re-check sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct HinglishNightJob {
    pub name: String,
    pub languages: Vec<String>,
    /// The audio the job has left to decode, per language.
    pub remaining_ms_by_lang: BTreeMap<String, u64>,
    /// A unit of the job has started or committed: the job stays as it is (a running unit is never stopped, and a
    /// session is not decoded with two models).
    pub started: bool,
}

/// What the night safety switch found.
#[derive(Debug, Clone, PartialEq)]
pub struct Recheck {
    /// Seconds the Hinglish night jobs still need at the speed the night's own units showed.
    pub needed_s: f64,
    pub finish_ms: u64,
    pub deadline_ms: u64,
    /// The sessions whose night job has not started: they go back to today's models and are marked.
    pub switch: Vec<String>,
    /// Seconds the work still needs once those sessions use today's models (the sessions that keep Hinglish at
    /// the Hinglish speed, the others at today's night speed, all at the measured factor).
    pub needed_after_s: f64,
}

impl Recheck {
    /// The reason written in the Queue log and in `faster-machine.json`.
    pub fn reason(&self) -> String {
        format!(
            "the night ran slower than planned: the Hinglish night pass needs about {:.1} h more at the speed the first night units showed and would end at {} IST, after 08:00 IST",
            self.needed_s / 3600.0,
            wb_queue::estimate::format_ist_hhmm(self.finish_ms)
        )
    }
}

/// The night safety switch. After the first night units of the night have finished, `night_factor` is how much slower
/// (above 1) or faster than the profile this computer really ran them. If the Hinglish night jobs that are left will
/// not finish before 08:00 IST at that speed, the sessions that have not started go back to today's models.
/// None when the Hinglish night still finishes in time, or when no Hinglish job is left.
pub fn recheck(
    profile: &ProfileView,
    now_ms: u64,
    jobs: &[HinglishNightJob],
    night_factor: f64,
) -> Option<Recheck> {
    if jobs.is_empty() {
        return None;
    }
    let seconds = |rtf: &BTreeMap<String, f64>, j: &HinglishNightJob| -> f64 {
        let slowest = rtf.values().copied().fold(1.0, f64::max);
        j.remaining_ms_by_lang
            .iter()
            .map(|(l, ms)| {
                *ms as f64 / 1000.0
                    * scaled(rtf.get(l).copied().unwrap_or(slowest), Some(night_factor))
            })
            .sum()
    };
    let hinglish_s = |j: &HinglishNightJob| seconds(&profile.hinglish_night_rtf(&j.languages), j);
    let today_s = |j: &HinglishNightJob| seconds(&profile.night_decode_rtf(&j.languages), j);
    let needed_s: f64 = jobs.iter().map(hinglish_s).sum();
    let deadline = night::deadline_ms(now_ms);
    let finish_ms = now_ms.saturating_add((needed_s * 1000.0).round() as u64);
    if finish_ms <= deadline {
        return None;
    }
    let switch: Vec<String> = jobs
        .iter()
        .filter(|j| !j.started)
        .map(|j| j.name.clone())
        .collect();
    let needed_after_s: f64 = jobs
        .iter()
        .map(|j| if j.started { hinglish_s(j) } else { today_s(j) })
        .sum();
    Some(Recheck {
        needed_s,
        finish_ms,
        deadline_ms: deadline,
        switch,
        needed_after_s,
    })
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
            detector: None,
            night_ok,
            night_hours: Some(rtf_hi * 3.0),
        };
        let mut v = ProfileView::from_capability(&cap, vec![]);
        v.set_cell_for_test("hinglish-f16", "hi", rtf_hi);
        v.set_cell_for_test("hinglish-f16", "en", rtf_hi / 2.5);
        v
    }

    /// A profile whose check chose the Hinglish model on the card (graphics-only configuration), with the card's
    /// cells and, when `processor_cells` is set, the same model measured on the processor too.
    fn card_profile(processor_cells: Option<(f64, f64)>) -> ProfileView {
        let mut p = profile(false, 0.3, true);
        p.config = Some(wb_capability::DeviceConfig::Gpu { index: 0 });
        p.set_cell_on_for_test("hinglish-f16", "hi", 0.3, true);
        p.set_cell_on_for_test("hinglish-f16", "en", 0.12, true);
        if let Some((hi, en)) = processor_cells {
            p.set_cell_on_for_test("hinglish-f16", "hi", hi, false);
            p.set_cell_on_for_test("hinglish-f16", "en", en, false);
        }
        p
    }

    /// Owner, 27 Nov 2026: after a graphics failure the units decoded without the Hinglish model are redone at night
    /// where the night routing runs the Hinglish model before 08:00 (the processor's measured cell for it), and the
    /// session is marked for a faster machine where it does not (no processor cell, or a night that does not fit).
    #[test]
    fn a_session_that_fell_back_is_redone_with_the_hinglish_model_at_night_when_the_night_fits() {
        let p = card_profile(Some((1.2, 0.5)));
        let r = project_redo(&p, true, six_pm(), &session("hall-a", 60, 60), 1.25);
        assert!(r.redone, "{r:?}");
        assert_eq!(r.why_not, None);
        assert_eq!(
            r.text,
            "hall-a: decoded without the Hinglish model after a graphics failure; will be redone at night"
        );
        // After a restart the card may work again: the setup the check chose (on the card) is planned.
        let on_card = card_profile(None);
        let r = project_redo(&on_card, false, six_pm(), &session("hall-a", 60, 60), 1.25);
        assert!(r.redone, "{r:?}");
    }

    #[test]
    fn a_session_that_fell_back_is_marked_for_a_faster_machine_when_the_night_does_not_fit() {
        // No processor cell for the Hinglish model: nothing says it can run there, so no redo is planned.
        let p = card_profile(None);
        let r = project_redo(&p, true, six_pm(), &session("hall-a", 60, 60), 1.25);
        assert!(!r.redone, "{r:?}");
        assert!(r.why_not.is_some());
        assert!(
            r.text.starts_with(
                "hall-a: decoded without the Hinglish model after a graphics failure; needs a faster machine ("
            ),
            "{}",
            r.text
        );
        // A processor cell that is too slow for the audio: Hindi at 9.0 needs about 20 h, 14 h are left.
        let slow = card_profile(Some((9.0, 3.6)));
        let r = project_redo(&slow, true, six_pm(), &session("hall-a", 90, 90), 1.25);
        assert!(!r.redone, "{r:?}");
        assert!(r.text.contains("needs a faster machine"), "{}", r.text);
    }

    /// The job's models decide whether the processor loses the Hinglish model, and a job whose models were all measured
    /// on the processor (a redo planned for it) runs as it is.
    #[test]
    fn the_job_models_tell_whether_the_processor_loses_the_hinglish_model() {
        let p = card_profile(None);
        let pair =
            serde_json::json!({"en": "small-q5_0", "hi": "hinglish-f16", "*": "hinglish-f16"});
        assert!(p.job_decodes_with_hinglish(pair.as_object().unwrap()));
        let other = serde_json::json!({"en": "small-q5_0", "hi": "large-v3-turbo-q5_0"});
        assert!(!p.job_decodes_with_hinglish(other.as_object().unwrap()));
        assert!(!p.models_measured_on_processor(pair.as_object().unwrap(), false));
        let both = card_profile(Some((1.2, 0.5)));
        let alone = serde_json::json!({"en": "hinglish-f16", "hi": "hinglish-f16"});
        assert!(both.models_measured_on_processor(alone.as_object().unwrap(), false));
        assert!(
            !both.models_measured_on_processor(alone.as_object().unwrap(), true),
            "a day job must also keep up (Hindi at 1.2)"
        );
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

    fn hinglish_job(name: &str, started: bool, en_min: u64, hi_min: u64) -> HinglishNightJob {
        HinglishNightJob {
            name: name.into(),
            languages: vec!["en".into(), "hi".into()],
            remaining_ms_by_lang: BTreeMap::from([
                ("en".to_owned(), en_min * 60_000),
                ("hi".to_owned(), hi_min * 60_000),
            ]),
            started,
        }
    }

    /// 26 Nov 2026: the night safety switch. Three English + Hindi sessions of 3 h each at Hindi 1.69 / English 0.68 need
    /// about 3.55 h each: 10.7 h at the profile's speed (fits the 14 h before 08:00), 17 h at 1.6 times that (does not).
    #[test]
    fn a_night_that_runs_slower_than_planned_sends_the_sessions_not_yet_started_back_to_todays_models()
     {
        let p = profile(true, 1.69, false);
        let jobs = [
            hinglish_job("first", true, 45, 45),
            hinglish_job("second", false, 90, 90),
            hinglish_job("third", false, 90, 90),
        ];
        // At the planned speed the Hinglish night still finishes: nothing changes.
        assert_eq!(recheck(&p, six_pm(), &jobs, 1.0), None);
        // The first units showed 1.6 times the profile's speed: the second and third go back; the running first stays.
        let r =
            recheck(&p, six_pm(), &jobs, 1.6).expect("the Hinglish night would end after 08:00");
        assert_eq!(r.switch, vec!["second".to_string(), "third".to_string()]);
        assert!(r.finish_ms > r.deadline_ms);
        assert!(r.needed_s / 3600.0 > 14.0, "{}", r.needed_s);
        assert!(r.needed_after_s < r.needed_s);
        assert!(r.reason().contains("after 08:00 IST"), "{}", r.reason());
    }

    #[test]
    fn a_hinglish_night_that_misses_08_00_with_every_session_already_started_changes_no_model() {
        let p = profile(true, 1.69, false);
        let jobs = [
            hinglish_job("only", true, 90, 90),
            hinglish_job("other", true, 90, 90),
        ];
        let r = recheck(&p, six_pm(), &jobs, 4.0).expect("slower than the night allows");
        assert!(r.switch.is_empty(), "{:?}", r.switch);
        assert_eq!(recheck(&p, six_pm(), &[], 4.0), None);
    }

    /// The night allowance reaches the Hinglish decision: three sessions that the earlier graphics-adapter allowance (1.25:
    /// 10.8 h at the bench's speed, 13.5 h, 14 h before 08:00) let through are sent to a faster machine at the allowance of
    /// both device classes (1.5: 16.2 h); two sessions (7.2 h at the bench, 10.8 h) still run Hinglish.
    #[test]
    fn the_hinglish_night_decision_uses_the_night_allowance_of_both_device_classes() {
        // Hindi 4.0, English 1.6 (the measured ratio is about 2.5); 90 minutes of audio per session.
        let p = profile(true, 4.0, false);
        let three = [
            session("a", 60, 30),
            session("b", 60, 30),
            session("c", 60, 30),
        ];
        let gpu = decide(&p, six_pm(), &three, wb_queue::speed::NIGHT_ALLOWANCE_GPU);
        let cpu = decide(&p, six_pm(), &three, wb_queue::speed::NIGHT_ALLOWANCE_CPU);
        let earlier = decide(&p, six_pm(), &three, 1.25);
        assert_eq!(earlier.routed.route, Route::Hinglish, "{earlier:?}");
        for (what, d) in [("graphics adapter", &gpu), ("processor", &cpu)] {
            assert_eq!(d.routed.route, Route::Today, "{what}: {d:?}");
            assert_eq!(d.marks.len(), 3, "{what}: {:?}", d.marks);
        }
        let two = [session("a", 60, 30), session("b", 60, 30)];
        let ok = decide(&p, six_pm(), &two, wb_queue::speed::NIGHT_ALLOWANCE_GPU);
        assert_eq!(ok.routed.route, Route::Hinglish, "{ok:?}");
    }
}
