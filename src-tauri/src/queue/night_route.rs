//! Which models the night pass uses for each session (Phase 5, 26 Nov 2026).
//!
//! An English + Hindi session that was not decoded with the Hinglish model while recording gets it at night when the
//! system check measured it on this computer, it fits the night window and the memory, and the plan made from the
//! audio the sessions really recorded finishes before 08:00 IST. Otherwise the night pass keeps today's models (the
//! plan is made for those alone), and the session is marked "best finished on a faster machine" with the reason.
//! Pure functions: the app's `night_step` supplies the sessions and carries out the decision.

use std::collections::{BTreeMap, BTreeSet};

use wb_queue::night::{self, Route, Routed, SessionAudio, SkipReason, Speeds, Tier};
use wb_queue::speed::scaled;

use super::profile::ProfileView;

/// One session that waits for its night pass.
#[derive(Debug, Clone)]
pub struct NightSession {
    pub name: String,
    pub languages: Vec<String>,
    pub audio: SessionAudio,
}

/// Why a session is marked "best finished on a faster machine" when the check never measured the Hinglish model here (the
/// quick check does not measure it without a discrete card: the model cannot keep up while recording on shared graphics or the
/// processor). The Full Check measures it on every configuration; the night pass only uses a measured model (owner, 28 Nov 2026).
pub const UNMEASURED_WHY: &str = wb_capability::HINGLISH_UNMEASURED_WHY;

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
    /// Sessions whose night job with the large model was left out because the sessions of higher priority need the time,
    /// with the reason (written to the Queue log; the session keeps its rolling transcript).
    pub dropped: Vec<(String, String)>,
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
    // Priority 1: the Hinglish redos of English + Hindi sessions (they fix translation), all or none of them.
    let hinglish_tier = Tier {
        sessions: sessions
            .iter()
            .filter(|s| qualifying.contains(&s.name))
            .map(|s| prefixed(&s.audio))
            .collect(),
        speeds: {
            let mut speeds = Speeds::default();
            for s in sessions.iter().filter(|s| qualifying.contains(&s.name)) {
                for (l, r) in scale(profile.hinglish_night_rtf(&s.languages)) {
                    speeds.night_rtf.insert(format!("{H}{l}"), r);
                    speeds.decode_rtf.insert(format!("{H}{l}"), r);
                }
            }
            speeds
        },
        all_or_nothing: true,
    };
    // Priority 2: Urdu sessions with the large model; priority 3: English sessions with it (the large model is no night
    // model for Hindi). A session that took the Hinglish night is not planned again; one that did not keeps today's models
    // and competes here.
    let class_of = |s: &NightSession| usize::from(!has_urdu(&s.languages));
    let by_class = |class: usize| Tier {
        sessions: sessions
            .iter()
            .filter(|s| class_of(s) == class)
            .map(|s| s.audio.clone())
            .collect(),
        speeds: today.clone(),
        all_or_nothing: false,
    };
    let tiers = [hinglish_tier, by_class(0), by_class(1)];
    let tiered = night::plan_tiered(now_ms, &tiers);
    let hinglish: BTreeSet<String> = tiered.planned[0].iter().cloned().collect();
    let hinglish_skip = if qualifying.is_empty() || !hinglish.is_empty() {
        None
    } else {
        let hinglish_dropped: Vec<_> = tiered.dropped.iter().filter(|d| d.tier == 0).collect();
        hinglish_dropped
            .first()
            .map(|first| SkipReason::DoesNotFit {
                needed_s: hinglish_dropped.iter().map(|d| d.needed_s).sum(),
                available_s: first.left_s,
            })
    };
    let routed = Routed {
        route: if hinglish.is_empty() {
            Route::Today
        } else {
            Route::Hinglish
        },
        plan: tiered.plan.clone(),
        hinglish_skip,
    };
    let why_not = match (&routed.hinglish_skip, !qualifying.is_empty()) {
        (Some(reason), _) => format!("the Hinglish night pass was not started: {}", reason.text()),
        // The mark says what happened: the quick check ran out of time, the measurement began and did not complete, or it was
        // never started. The wording is the verdict's (`wb_capability::HinglishGap`).
        (None, false) => match profile.hinglish_gap() {
            Some(gap) => gap.night_why(),
            None if !profile.hinglish_measured() => UNMEASURED_WHY.to_owned(),
            None => "the Hinglish model is too slow or too large for this computer".to_owned(),
        },
        (None, true) => "the Hinglish night pass was not planned".to_owned(),
    };
    let marks = sessions
        .iter()
        .filter(|s| profile.lacks_hinglish(&s.languages) && !hinglish.contains(&s.name))
        .map(|s| (s.name.clone(), why_not.clone()))
        .collect();
    // What the priorities left out (the large model's night for Urdu or English sessions): no night job, the
    // rolling transcript stands, and the reason is written down.
    let dropped = tiered
        .dropped
        .iter()
        .filter(|d| d.tier > 0)
        .filter_map(|d| {
            let s = sessions.iter().find(|s| s.name == d.session)?;
            let night: BTreeSet<String> = profile
                .night_models(&s.languages)
                .into_iter()
                .filter(|(l, _)| l != "*")
                .map(|(_, m)| m)
                .collect();
            let rolling: BTreeSet<String> = profile
                .rolling_models(&s.languages)
                .into_iter()
                .filter(|(l, _)| l != "*")
                .map(|(_, m)| m)
                .collect();
            Some((
                s.name.clone(),
                format!(
                    "no night pass with {}: it would need about {:.1} h and only {:.1} h are left before 08:00 IST after the work with higher priority (Hinglish redos of English + Hindi and Hindi sessions first, then Urdu sessions, then English); the rolling transcript made with {} stands",
                    night.into_iter().collect::<Vec<_>>().join(" and "),
                    d.needed_s / 3600.0,
                    d.left_s / 3600.0,
                    rolling.into_iter().collect::<Vec<_>>().join(" and ")
                ),
            ))
        })
        .collect();
    NightDecision {
        routed,
        hinglish,
        marks,
        dropped,
    }
}

/// Sessions with Urdu have the second priority (the large model's night gains 1.0 to 1.6 WER points for Urdu); the rest
/// the third (English: 0.5 to 0.6 points, at 1.6 to 1.9 times the time). Hindi is not here: the Hinglish redo is its night.
fn has_urdu(languages: &[String]) -> bool {
    languages.iter().any(|l| l == "ur")
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

/// A night job as the re-check sees it: any night job, with the Hinglish model or with the large model.
#[derive(Debug, Clone, PartialEq)]
pub struct NightJob {
    pub name: String,
    pub languages: Vec<String>,
    /// The audio the job has left to decode, per language.
    pub remaining_ms_by_lang: BTreeMap<String, u64>,
    /// A unit of the job has started or committed: the job stays as it is (a running unit is never stopped, and a
    /// session is not decoded with two models).
    pub started: bool,
    /// The job decodes with the Hinglish model (priority 1); the others use the large model of their language.
    pub hinglish: bool,
    /// The job's place in the plan's priority order (0 first): the night runs in this order and the re-check drops from
    /// the end.
    pub rank: usize,
}

/// What the night safety switch found.
#[derive(Debug, Clone, PartialEq)]
pub struct Recheck {
    /// Seconds the night jobs still need at the speed the night's own units showed.
    pub needed_s: f64,
    pub finish_ms: u64,
    pub deadline_ms: u64,
    /// The sessions whose night job has not started and is dropped, lowest priority first: no night pass, the rolling
    /// transcript stands. Hinglish jobs come last, and only when dropping the others was not enough and today's models are
    /// no faster (or the session is Hindi only): they are marked for a faster machine.
    pub drop: Vec<String>,
    /// The English + Hindi sessions whose Hinglish night job has not started and goes back to today's faster models (only
    /// when dropping the other jobs was not enough): they are marked.
    pub switch: Vec<String>,
    /// Seconds the work still needs once the dropped jobs are gone and the switched ones use today's models (the sessions
    /// that keep their model at its own speed, all at the measured factor).
    pub needed_after_s: f64,
}

impl Recheck {
    /// The reason written in the Queue log and in `faster-machine.json`.
    pub fn reason(&self) -> String {
        format!(
            "the night ran slower than planned: the night pass needs about {:.1} h more at the speed the first night units showed and would end at {} IST, after 08:00 IST",
            self.needed_s / 3600.0,
            wb_queue::estimate::format_ist_hhmm(self.finish_ms)
        )
    }
}

/// The night safety switch (owner, 26 Nov 2026, extended to every night job on 27 Nov 2026). After the first night units of
/// the night have finished, `night_factor` is how much slower (above 1) or faster than the profile this computer really ran
/// them. If the night jobs that are left will not finish before 08:00 IST at that speed:
/// 1. the jobs that have not started are dropped from the lowest priority upwards (English, then Urdu; within a priority the
///    later in the plan first) until the rest finishes, and the rolling transcripts stand;
/// 2. if that is not enough, the Hinglish jobs that have not started go back to today's models when those are faster
///    (English + Hindi sessions), and are dropped otherwise (Hindi-only sessions, or models that are no faster).
///
/// A job that has started is never touched. None when the night still finishes in time, or when no job is left.
pub fn recheck(
    profile: &ProfileView,
    now_ms: u64,
    jobs: &[NightJob],
    night_factor: f64,
) -> Option<Recheck> {
    if jobs.is_empty() {
        return None;
    }
    let seconds = |rtf: &BTreeMap<String, f64>, j: &NightJob| -> f64 {
        let slowest = rtf.values().copied().fold(1.0, f64::max);
        j.remaining_ms_by_lang
            .iter()
            .map(|(l, ms)| {
                *ms as f64 / 1000.0
                    * scaled(rtf.get(l).copied().unwrap_or(slowest), Some(night_factor))
            })
            .sum()
    };
    let hinglish_s = |j: &NightJob| seconds(&profile.hinglish_night_rtf(&j.languages), j);
    let today_s = |j: &NightJob| seconds(&profile.night_decode_rtf(&j.languages), j);
    let own_s = |j: &NightJob| {
        if j.hinglish {
            hinglish_s(j)
        } else {
            today_s(j)
        }
    };
    let needed_s: f64 = jobs.iter().map(own_s).sum();
    let deadline = night::deadline_ms(now_ms);
    let finish_of = |s: f64| now_ms.saturating_add((s * 1000.0).round() as u64);
    let finish_ms = finish_of(needed_s);
    if finish_ms <= deadline {
        return None;
    }
    let mut left = needed_s;
    // Step 1: drop the jobs that have not started and are not Hinglish jobs, the lowest priority first.
    let mut droppable: Vec<&NightJob> = jobs.iter().filter(|j| !j.started && !j.hinglish).collect();
    droppable.sort_by_key(|j| std::cmp::Reverse(j.rank));
    let mut drop = Vec::new();
    for j in droppable {
        if finish_of(left) <= deadline {
            break;
        }
        left -= own_s(j);
        drop.push(j.name.clone());
    }
    // Step 2: when that is still not enough, the Hinglish jobs that have not started go back to today's models if those are
    // faster, and are dropped otherwise. A Hindi-only job is dropped, not switched: today's model for Hindi is far less
    // accurate (owner, 27 Nov 2026: large-v3 q5_0 27.9 against Hinglish q8_0 11.4 on Kathbath), and the session keeps its
    // rolling transcript, marked for a faster machine.
    let mut switch = Vec::new();
    // The lowest priority first, and only as many as the deadline needs (a dropped job loses its whole night pass).
    let mut hinglish_left: Vec<&NightJob> =
        jobs.iter().filter(|j| !j.started && j.hinglish).collect();
    hinglish_left.sort_by_key(|j| std::cmp::Reverse(j.rank));
    {
        for j in hinglish_left {
            if finish_of(left) <= deadline {
                break;
            }
            let english_too = j.languages.iter().any(|l| l == "en");
            if english_too && today_s(j) < hinglish_s(j) {
                left += today_s(j) - hinglish_s(j);
                switch.push(j.name.clone());
            } else {
                left -= hinglish_s(j);
                drop.push(j.name.clone());
            }
        }
    }
    Some(Recheck {
        needed_s,
        finish_ms,
        deadline_ms: deadline,
        drop,
        switch,
        needed_after_s: left,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use wb_capability::Hinglish;
    use wb_queue::night::Plan;

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
            model: Some("hinglish-q8_0".into()),
            detector: None,
            night_ok,
            night_hours: Some(rtf_hi * 3.0),
        };
        let mut v = ProfileView::from_capability(&cap, vec![]);
        v.set_cell_for_test("hinglish-q8_0", "hi", rtf_hi);
        v.set_cell_for_test("hinglish-q8_0", "en", rtf_hi / 2.5);
        v
    }

    /// A profile whose check chose the Hinglish model on the card (graphics-only configuration), with the card's
    /// cells and, when `processor_cells` is set, the same model measured on the processor too.
    fn card_profile(processor_cells: Option<(f64, f64)>) -> ProfileView {
        let mut p = profile(false, 0.3, true);
        p.config = Some(wb_capability::DeviceConfig::Gpu { index: 0 });
        p.set_cell_on_for_test("hinglish-q8_0", "hi", 0.3, true);
        p.set_cell_on_for_test("hinglish-q8_0", "en", 0.12, true);
        if let Some((hi, en)) = processor_cells {
            p.set_cell_on_for_test("hinglish-q8_0", "hi", hi, false);
            p.set_cell_on_for_test("hinglish-q8_0", "en", en, false);
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
            serde_json::json!({"en": "small-q5_0", "hi": "hinglish-q8_0", "*": "hinglish-q8_0"});
        assert!(p.job_decodes_with_hinglish(pair.as_object().unwrap()));
        let other = serde_json::json!({"en": "small-q5_0", "hi": "large-v3-turbo-q5_0"});
        assert!(!p.job_decodes_with_hinglish(other.as_object().unwrap()));
        assert!(!p.models_measured_on_processor(pair.as_object().unwrap(), false));
        let both = card_profile(Some((1.2, 0.5)));
        let alone = serde_json::json!({"en": "hinglish-q8_0", "hi": "hinglish-q8_0"});
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
    fn a_model_that_does_not_fit_the_night_marks_the_session_without_a_hinglish_plan() {
        let p = profile(false, 9.0, false);
        let d = decide(&p, six_pm(), &[session("hall-a", 90, 90)], 1.25);
        assert_eq!(d.routed.route, Route::Today);
        assert_eq!(d.routed.hinglish_skip, None);
        assert_eq!(d.marks.len(), 1);
        assert!(p.hinglish_measured());
        assert!(
            d.marks[0].1.contains("too slow or too large"),
            "{}",
            d.marks[0].1
        );
    }

    /// A profile of a computer whose quick check never measured the Hinglish model (no discrete card, or the step was left
    /// for the Full Check).
    fn unmeasured_profile() -> ProfileView {
        unmeasured_profile_with(Vec::new(), None)
    }

    /// A profile whose Hinglish model has no measurement (`model` None) or only the model alone (`Some`), with what the check
    /// recorded as skipped.
    fn unmeasured_profile_with(
        skipped: Vec<wb_capability::Skipped>,
        model_alone: Option<&str>,
    ) -> ProfileView {
        let mut cap = wb_capability::parse_capability(include_str!(
            "../../../crates/wb-capability/fixtures/tier-g.json"
        ))
        .unwrap();
        cap.profile.skipped = skipped;
        cap.decision.hinglish = Hinglish {
            ok: false,
            rtf: None,
            source: "unmeasured".into(),
            model: model_alone.map(str::to_owned),
            detector: None,
            night_ok: false,
            night_hours: None,
        };
        ProfileView::from_capability(&cap, vec![])
    }

    /// Owner, 28 Nov 2026: where the Hinglish model was not measured (a quick check on a computer without a discrete card),
    /// English + Hindi and Hindi-only sessions keep today's models at night, get no Hinglish plan, and are marked "best
    /// finished on a faster machine" with the words that tell the organiser to run the Full Check once.
    #[test]
    fn where_the_hinglish_model_was_not_measured_english_hindi_and_hindi_only_sessions_keep_todays_models_and_are_marked()
     {
        let p = unmeasured_profile();
        assert!(!p.hinglish_measured());
        let mut hindi_only = session("hall-b", 0, 90);
        hindi_only.languages = vec!["hi".into()];
        let d = decide(&p, six_pm(), &[session("hall-a", 90, 90), hindi_only], 1.25);
        assert_eq!(d.routed.route, Route::Today);
        assert!(d.hinglish.is_empty());
        assert_eq!(d.routed.hinglish_skip, None);
        let marked: Vec<&str> = d.marks.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(marked, ["hall-a", "hall-b"], "{:?}", d.marks);
        for (_, why) in &d.marks {
            assert_eq!(why, UNMEASURED_WHY);
            assert!(
                why.contains(
                    "Run the Full Check once before the event on this machine (about 95 minutes)"
                ),
                "{why}"
            );
        }
        // An English-only session is not an English + Hindi session: no mark.
        let mut en_only = session("hall-c", 60, 0);
        en_only.languages = vec!["en".into()];
        let d = decide(&p, six_pm(), &[en_only], 1.25);
        assert!(d.marks.is_empty(), "{:?}", d.marks);
    }

    /// The mark says what happened (owner, 28 Nov 2026): the quick check ran out of time, the measurement began and did not
    /// complete, the model was never started. A pair cut by the cap on a slower card is told the same way.
    #[test]
    fn the_night_mark_says_what_happened_to_the_hinglish_measurement() {
        use wb_capability::{
            SKIPPED_HINGLISH, SKIPPED_HINGLISH_PAIR, SKIPPED_NOT_COMPLETED, Skipped,
        };
        let marks_of = |p: &ProfileView| -> Vec<String> {
            decide(p, six_pm(), &[session("hall-a", 90, 90)], 1.25)
                .marks
                .into_iter()
                .map(|(_, why)| why)
                .collect()
        };
        let skip = |step: &str, reason: &str| Skipped {
            step: step.to_owned(),
            reason: reason.to_owned(),
        };
        // Time ran out before the model was measured.
        let p = unmeasured_profile_with(
            vec![skip(
                SKIPPED_HINGLISH,
                "left for the full check: the quick check aims to end by 360 s and 327 s had passed",
            )],
            None,
        );
        assert_eq!(
            marks_of(&p),
            [
                "the Hinglish model could not be measured in the Quick Check (time ran out). Run the Full Check once on this machine (about 95 minutes) to enable it"
            ]
        );
        // The measurement began and did not complete: no advice to run the Full Check.
        let p = unmeasured_profile_with(
            vec![skip(
                SKIPPED_HINGLISH,
                &format!("{SKIPPED_NOT_COMPLETED}NVIDIA GeForce (Vulkan)"),
            )],
            None,
        );
        let marks = marks_of(&p);
        assert_eq!(marks.len(), 1);
        assert!(
            marks[0].contains("did not complete") && !marks[0].contains("Run the Full Check"),
            "{marks:?}"
        );
        // Never started (the ratio rule): the advice before the event.
        let p = unmeasured_profile_with(
            vec![skip(
                SKIPPED_HINGLISH,
                "not measured: Hindi already runs at 0.69x real time with the rolling model, and the Hinglish model is at least 1.5 times slower, over the 0.75 bar",
            )],
            None,
        );
        assert_eq!(marks_of(&p), [UNMEASURED_WHY]);
        // The model alone was measured and is too slow; the pair was cut by the cap on a slower card.
        let p = unmeasured_profile_with(
            vec![skip(
                SKIPPED_HINGLISH_PAIR,
                "left for the full check: the quick check's time ran out while this step was measured on NVIDIA GeForce (Vulkan)",
            )],
            Some("hinglish-q8_0"),
        );
        assert!(p.hinglish_measured());
        assert_eq!(
            marks_of(&p),
            [
                "the Hinglish model with the small model in front of it could not be measured in the Quick Check (time ran out). Run the Full Check once on this machine (about 95 minutes) to enable it"
            ]
        );
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

    fn hinglish_job(name: &str, started: bool, en_min: u64, hi_min: u64) -> NightJob {
        NightJob {
            name: name.into(),
            languages: vec!["en".into(), "hi".into()],
            remaining_ms_by_lang: BTreeMap::from([
                ("en".to_owned(), en_min * 60_000),
                ("hi".to_owned(), hi_min * 60_000),
            ]),
            started,
            hinglish: true,
            rank: 0,
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
        // The first units showed 1.6 times the profile's speed: sessions that have not started go back to today's models, one
        // at a time (the lowest priority first; equal ranks in the order listed) until the rest fits; the running first
        // stays. One goes back here, which is enough (auditor, third run: not every unstarted job at once).
        let r =
            recheck(&p, six_pm(), &jobs, 1.6).expect("the Hinglish night would end after 08:00");
        assert_eq!(r.switch, vec!["second".to_string()], "{r:?}");
        assert!(r.needed_after_s / 3600.0 < 14.0, "{}", r.needed_after_s);
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

    fn large_job(name: &str, lang: &str, started: bool, minutes: u64, rank: usize) -> NightJob {
        NightJob {
            name: name.into(),
            languages: vec![lang.into()],
            remaining_ms_by_lang: BTreeMap::from([(lang.to_owned(), minutes * 60_000)]),
            started,
            hinglish: false,
            rank,
        }
    }

    /// Owner, 27 Nov 2026: the safety switch covers every night job. A night that runs slower than planned drops the jobs that
    /// have not started from the lowest priority upwards (English before Urdu) and only as many as the 08:00 deadline needs;
    /// the Hinglish jobs and every job that has started stay. On the earlier code only the Hinglish jobs were looked at, so a
    /// night of large-model jobs was never checked.
    #[test]
    fn a_slow_night_drops_the_lowest_priority_jobs_that_have_not_started_and_only_as_many_as_it_needs()
     {
        let p = with_large_night(profile(true, 1.69, false));
        // At the measured factor 1.5 the large model runs 0.9 times real time: Urdu 4 h (3.6 h), English 4 h (3.6 h), second
        // English 4 h (3.6 h) and a Hinglish job (1.5 h of each language: about 3.9 h at that factor), 14.7 h > 14 h.
        let jobs = [
            hinglish_job("hall-a", false, 90, 90),
            large_job("urdu-hall", "ur", false, 240, 1),
            large_job("english-one", "en", false, 240, 2),
            large_job("english-two", "en", false, 240, 3),
        ];
        let r = recheck(&p, six_pm(), &jobs, 1.5).expect("the night does not finish by 08:00");
        assert_eq!(r.drop, vec!["english-two".to_string()], "{r:?}");
        assert!(r.switch.is_empty(), "{r:?}");
        assert!(r.needed_after_s < r.needed_s);
        assert!(r.reason().contains("after 08:00 IST"), "{}", r.reason());
        // Slower still: English goes, then Urdu, in that order, and the Hinglish redo stays while that is enough.
        let r = recheck(&p, six_pm(), &jobs, 3.0).expect("slower still");
        assert_eq!(
            r.drop,
            vec![
                "english-two".to_string(),
                "english-one".to_string(),
                "urdu-hall".to_string()
            ],
            "{r:?}"
        );
        assert!(r.switch.is_empty(), "{r:?}");
        // On time: nothing is dropped.
        assert_eq!(recheck(&p, six_pm(), &jobs, 1.0), None);
    }

    #[test]
    fn a_job_that_has_started_is_never_dropped_and_the_hinglish_jobs_go_back_only_when_dropping_is_not_enough()
     {
        let p = with_large_night(profile(true, 1.69, false));
        let jobs = [
            hinglish_job("hall-a", false, 120, 120),
            large_job("english-started", "en", true, 240, 1),
            large_job("english-two", "en", false, 240, 2),
        ];
        let r = recheck(&p, six_pm(), &jobs, 2.4).expect("far too slow");
        assert_eq!(r.drop, vec!["english-two".to_string()], "{r:?}");
        assert!(!r.drop.contains(&"english-started".to_string()));
        assert_eq!(r.switch, vec!["hall-a".to_string()], "{r:?}");
        // Every job has started: nothing to drop or switch.
        let started = [
            hinglish_job("hall-a", true, 120, 120),
            large_job("english-started", "en", true, 240, 1),
        ];
        let r = recheck(&p, six_pm(), &started, 4.0).expect("far too slow");
        assert!(r.drop.is_empty() && r.switch.is_empty(), "{r:?}");
    }

    /// Auditor, second run: a Hindi-only Hinglish job that the night cannot finish is dropped (marked for a faster machine),
    /// never switched to the large model whose Hindi the owner rejected; a job with English whose today's models are no faster
    /// is dropped too, and the larger the night the fewer jobs it takes.
    #[test]
    fn a_hindi_only_hinglish_job_is_dropped_not_switched_when_the_night_cannot_finish() {
        let mut p = with_large_night(profile(true, 1.69, false));
        p.set_night_for_test("hi", "large-v3-q5_0");
        p.set_cell_for_test("large-v3-q5_0", "hi", 0.6);
        let hindi = |name: &str, started: bool, rank: usize| NightJob {
            name: name.into(),
            languages: vec!["hi".into()],
            remaining_ms_by_lang: BTreeMap::from([("hi".to_owned(), 240 * 60_000)]),
            started,
            hinglish: true,
            rank,
        };
        // Four hours of Hindi at 1.69 x 3 = 5.07: 20 h for one job; two jobs cannot finish whatever is done.
        let jobs = [hindi("hindi-a", false, 0), hindi("hindi-b", false, 1)];
        let r = recheck(&p, six_pm(), &jobs, 3.0).expect("far too slow");
        assert!(r.switch.is_empty(), "never switched to large-v3: {r:?}");
        assert_eq!(
            r.drop,
            vec!["hindi-b".to_string(), "hindi-a".to_string()],
            "the lowest priority first: {r:?}"
        );
        // A started Hindi job stays, and nothing else is touched.
        let jobs = [hindi("hindi-a", true, 0), hindi("hindi-b", false, 1)];
        let r = recheck(&p, six_pm(), &jobs, 3.0).expect("far too slow");
        assert_eq!(r.drop, vec!["hindi-b".to_string()], "{r:?}");
    }

    /// Auditor, third run: only as many Hinglish jobs are dropped as the deadline needs, the lowest priority first. Three
    /// Hindi jobs of 4 h at 1.69 times real time need 20.3 h at the profile's speed; dropping the last one leaves 13.5 h.
    #[test]
    fn only_as_many_hinglish_jobs_are_dropped_as_the_deadline_needs_and_the_lowest_priority_goes_first()
     {
        let p = with_large_night(profile(true, 1.69, false));
        let hindi = |name: &str, rank: usize| NightJob {
            name: name.into(),
            languages: vec!["hi".into()],
            remaining_ms_by_lang: BTreeMap::from([("hi".to_owned(), 240 * 60_000)]),
            started: false,
            hinglish: true,
            rank,
        };
        // Listed in an order that is not the rank order: the drop still goes by rank.
        let jobs = [
            hindi("hindi-b", 1),
            hindi("hindi-c", 2),
            hindi("hindi-a", 0),
        ];
        let r = recheck(&p, six_pm(), &jobs, 1.0).expect("20 h do not fit before 08:00");
        assert_eq!(r.drop, vec!["hindi-c".to_string()], "{r:?}");
        assert!(r.switch.is_empty());
        assert!(r.needed_after_s / 3600.0 < 14.0, "{}", r.needed_after_s);
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
    /// The profile of the Hinglish tests plus the large model as the night pick for English and Urdu (owner, 27 Nov 2026),
    /// at 0.6 times real time here.
    fn with_large_night(mut p: ProfileView) -> ProfileView {
        for lang in ["en", "ur"] {
            p.set_night_for_test(lang, "large-v3-q5_0");
            p.set_cell_for_test("large-v3-q5_0", lang, 0.6);
        }
        p
    }

    fn one_language(name: &str, lang: &str, minutes: u64) -> NightSession {
        NightSession {
            name: name.into(),
            languages: vec![lang.into()],
            audio: SessionAudio {
                id: name.into(),
                audio_ms_by_lang: BTreeMap::from([(lang.to_owned(), minutes * 60_000)]),
            },
        }
    }

    /// Owner, 27 Nov 2026: night time is the scarcest resource. The Hinglish redo of an English + Hindi session comes first,
    /// then Urdu with the large model, then English with it, and the English night is dropped when the Hinglish redo needs
    /// the time (it keeps the rolling transcript; the reason is written down). Before this the Hinglish plan counted the
    /// English session too, did not fit, and sent the Hinglish session to a faster machine.
    #[test]
    fn the_english_large_model_night_is_dropped_when_a_hinglish_redo_needs_the_time() {
        // The Hinglish session: 4 h of English and 4 h of Hindi at 0.68 and 1.69 times real time, x1.25: about 11.8 h of
        // the 14 h before 08:00. English-only, 4 h at 0.6 x 1.25: 3 h more, which does not fit.
        let p = with_large_night(profile(true, 1.69, false));
        let hall = session("hall-a", 240, 240);
        let english = one_language("english-hall", "en", 240);
        let d = decide(&p, six_pm(), &[hall, english], 1.25);
        assert_eq!(d.routed.route, Route::Hinglish, "{d:?}");
        assert!(d.hinglish.contains("hall-a"));
        let Plan::Run { sessions, .. } = &d.routed.plan else {
            panic!("{d:?}")
        };
        assert_eq!(sessions, &["hall-a"], "only the Hinglish redo runs");
        assert!(
            d.marks.is_empty(),
            "the Hinglish session is not marked: {:?}",
            d.marks
        );
        assert_eq!(d.dropped.len(), 1, "{:?}", d.dropped);
        assert_eq!(d.dropped[0].0, "english-hall");
        assert!(
            d.dropped[0].1.contains("higher priority"),
            "{}",
            d.dropped[0].1
        );
        assert!(
            d.dropped[0].1.contains("large-v3-q5_0"),
            "{}",
            d.dropped[0].1
        );
    }

    #[test]
    fn urdu_with_the_large_model_comes_before_english_and_both_run_when_the_night_has_room() {
        let p = with_large_night(profile(true, 1.69, false));
        let sessions = [
            one_language("english-hall", "en", 90),
            one_language("urdu-hall", "ur", 90),
            session("hall-a", 90, 90),
        ];
        let d = decide(&p, six_pm(), &sessions, 1.25);
        let Plan::Run { sessions, .. } = &d.routed.plan else {
            panic!("{d:?}")
        };
        assert_eq!(
            sessions,
            &["hall-a", "urdu-hall", "english-hall"],
            "Hinglish redo, then Urdu, then English"
        );
        assert!(d.dropped.is_empty(), "{:?}", d.dropped);
        // Short of time, Urdu stays and English goes: 4 h of Hinglish content (11.8 h) leaves 2.2 h; Urdu 90 min x 0.6 x 1.25
        // is 1.1 h, English 150 min is 1.9 h.
        let tight = [
            session("hall-a", 240, 240),
            one_language("english-hall", "en", 150),
            one_language("urdu-hall", "ur", 90),
        ];
        let d = decide(&p, six_pm(), &tight, 1.25);
        let Plan::Run { sessions, .. } = &d.routed.plan else {
            panic!("{d:?}")
        };
        assert_eq!(sessions, &["hall-a", "urdu-hall"]);
        assert_eq!(d.dropped.len(), 1);
        assert_eq!(d.dropped[0].0, "english-hall");
    }

    /// A session in Hindi alone takes the Hinglish night with the English + Hindi ones (owner, 27 Nov 2026): priority 1, the
    /// Hinglish model's speed, and never the large model. On the earlier wiring it competed in the Urdu and Hindi tier with
    /// large-v3 q5_0 and was not a Hinglish session at all.
    #[test]
    fn a_hindi_only_session_takes_the_hinglish_night_in_the_first_priority() {
        let p = with_large_night(profile(true, 1.69, false));
        let sessions = [
            one_language("urdu-hall", "ur", 90),
            one_language("hindi-hall", "hi", 90),
        ];
        let d = decide(&p, six_pm(), &sessions, 1.25);
        assert_eq!(d.routed.route, Route::Hinglish, "{d:?}");
        assert!(d.hinglish.contains("hindi-hall"), "{:?}", d.hinglish);
        let Plan::Run {
            sessions: order, ..
        } = &d.routed.plan
        else {
            panic!("{d:?}")
        };
        assert_eq!(
            order,
            &["hindi-hall", "urdu-hall"],
            "Hindi first, then Urdu"
        );
        assert!(d.marks.is_empty(), "{:?}", d.marks);
        // Too slow for the night: the session is marked for a faster machine instead of being given the large model.
        let slow = with_large_night(profile(true, 30.0, false));
        let d = decide(&slow, six_pm(), &sessions, 1.25);
        assert!(d.hinglish.is_empty());
        assert!(
            d.marks.iter().any(|(n, _)| n == "hindi-hall"),
            "{:?}",
            d.marks
        );
    }

    /// A Hinglish class that cannot finish is dropped whole (as before) and the large-model sessions then use the night.
    #[test]
    fn when_the_hinglish_redo_cannot_finish_the_urdu_and_english_nights_still_run() {
        let p = with_large_night(profile(true, 9.0, false));
        let sessions = [
            session("hall-a", 90, 90),
            one_language("urdu-hall", "ur", 90),
        ];
        let d = decide(&p, six_pm(), &sessions, 1.25);
        assert_eq!(d.routed.route, Route::Today);
        assert!(d.hinglish.is_empty());
        assert_eq!(d.marks.len(), 1, "{:?}", d.marks);
        assert!(d.marks[0].1.contains("was not started"), "{}", d.marks[0].1);
        let Plan::Run { sessions, .. } = &d.routed.plan else {
            panic!("{d:?}")
        };
        assert!(sessions.contains(&"urdu-hall".to_string()), "{sessions:?}");
    }
}
