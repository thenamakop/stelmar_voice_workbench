//! Which language a chunk is decoded in (PLAN section 5, "Code-switching").
//!
//! Speakers switch between English and Hindi at any point. Detection runs per sentence-sized
//! chunk, restricted to the session's languages, and follows the operator's locks.
//!
//! The rules, in the order they apply:
//! 1. A language lock (Alt+1/2/3) in force at the chunk's audio time wins; Alt+4 returns to detection.
//! 2. A session with one language decodes everything in it.
//! 3. A chunk shorter than 2 seconds takes its neighbours' language unless detection is at least
//!    0.8 sure.
//! 4. When the two best languages are within 0.2 of each other the chunk is decoded both ways and
//!    the result with the higher average log-probability is kept.
//! 5. A chunk is never decoded as English when another expected language scores 0.5 or more, and
//!    an output whose script contradicts its language is re-decoded.
//!
//! Probabilities are those of the session's languages only, renormalised to sum to 1: whisper
//! spreads probability over about 100 languages, and only the choice among the ones this session
//! expects matters.

use wb_protocol::LangLogEntry;

/// Probabilities of the session's languages for one chunk, best first, summing to 1.
#[derive(Debug, Clone, PartialEq)]
pub struct Detection {
    pub probs: Vec<(String, f32)>,
}

impl Detection {
    /// Restricts raw probabilities to `languages` and renormalises. A language whisper has no
    /// probability for counts as 0. If everything is 0 the languages share the probability equally.
    pub fn restrict(raw: &[(String, f32)], languages: &[String]) -> Self {
        let mut probs: Vec<(String, f32)> = languages
            .iter()
            .map(|l| {
                let p = raw.iter().find(|(r, _)| r == l).map_or(0.0, |(_, p)| *p);
                (l.clone(), p.max(0.0))
            })
            .collect();
        let sum: f32 = probs.iter().map(|(_, p)| p).sum();
        if sum > 0.0 {
            for (_, p) in &mut probs {
                *p /= sum;
            }
        } else if !probs.is_empty() {
            let share = 1.0 / probs.len() as f32;
            for (_, p) in &mut probs {
                *p = share;
            }
        }
        // Stable: ties keep the session's order (English, Hindi, Urdu by default).
        probs.sort_by(|a, b| b.1.total_cmp(&a.1));
        Self { probs }
    }

    pub fn best(&self) -> Option<&(String, f32)> {
        self.probs.first()
    }

    pub fn second(&self) -> Option<&(String, f32)> {
        self.probs.get(1)
    }

    pub fn prob_of(&self, lang: &str) -> f32 {
        self.probs
            .iter()
            .find(|(l, _)| l == lang)
            .map_or(0.0, |(_, p)| *p)
    }
}

/// What to decode a chunk as.
#[derive(Debug, Clone, PartialEq)]
pub enum Plan {
    /// One language.
    Single { lang: String, reason: Reason },
    /// Too close to call: decode in both and keep the better one.
    Both { first: String, second: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    Locked,
    OnlyLanguage,
    Detected,
    /// Short and uncertain: took its neighbours' language.
    Neighbours,
    /// English was the best guess but another expected language scores 0.5 or more.
    NotEnglish,
}

/// The language in force from the operator's log at `audio_ms`: the last entry at or before it.
/// `None` means detection (no lock, or Alt+4 was pressed).
pub fn locked_language(log: &[LangLogEntry], audio_ms: u64) -> Option<&str> {
    log.iter()
        .filter(|e| e.audio_ms <= audio_ms)
        .max_by_key(|e| e.audio_ms)
        .and_then(|e| e.lang.as_deref())
}

/// Thresholds of the rules above.
#[derive(Debug, Clone, Copy)]
pub struct Rules {
    pub short_chunk_s: f32,
    pub short_chunk_sure: f32,
    pub close_margin: f32,
    pub not_english_at: f32,
}

impl Default for Rules {
    fn default() -> Self {
        Self {
            short_chunk_s: 2.0,
            short_chunk_sure: 0.8,
            close_margin: 0.2,
            not_english_at: 0.5,
        }
    }
}

/// Decides how to decode one chunk.
///
/// `locked` is the operator's lock at the chunk's start, `prev` and `next` the languages chosen
/// for the neighbouring chunks (`next` is known when detection ran over the whole unit first).
pub fn decide(
    det: &Detection,
    duration_s: f32,
    locked: Option<&str>,
    prev: Option<&str>,
    next: Option<&str>,
    languages: &[String],
    rules: &Rules,
) -> Plan {
    if let Some(l) = locked {
        return Plan::Single {
            lang: l.to_string(),
            reason: Reason::Locked,
        };
    }
    if languages.len() <= 1 {
        let lang = languages.first().cloned().unwrap_or_else(|| "en".into());
        return Plan::Single {
            lang,
            reason: Reason::OnlyLanguage,
        };
    }
    let Some((best, best_p)) = det.best().cloned() else {
        return Plan::Single {
            lang: languages[0].clone(),
            reason: Reason::OnlyLanguage,
        };
    };
    // Rule 3: short and unsure.
    if duration_s < rules.short_chunk_s && best_p < rules.short_chunk_sure {
        let pick = match (prev, next) {
            (Some(a), Some(b)) if a == b => Some(a),
            (Some(a), _) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        };
        if let Some(l) = pick
            && languages.iter().any(|x| x == l)
        {
            return Plan::Single {
                lang: l.to_string(),
                reason: Reason::Neighbours,
            };
        }
    }
    let second = det.second().cloned();
    // Rule 5: never English when another expected language is at 0.5 or more.
    let other_strong = det
        .probs
        .iter()
        .find(|(l, p)| l != "en" && *p >= rules.not_english_at)
        .map(|(l, _)| l.clone());
    if best == "en"
        && let Some(other) = other_strong
    {
        return Plan::Single {
            lang: other,
            reason: Reason::NotEnglish,
        };
    }
    // Rule 4: too close to call.
    if let Some((second_lang, second_p)) = second
        && best_p - second_p < rules.close_margin
    {
        return Plan::Both {
            first: best,
            second: second_lang,
        };
    }
    Plan::Single {
        lang: best,
        reason: Reason::Detected,
    }
}

/// One candidate of a both-ways decode.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub lang: String,
    pub avg_logprob: f32,
    /// The text is mostly in a script that does not belong to `lang`.
    pub wrong_script: bool,
    /// Nothing usable was decoded.
    pub empty: bool,
}

/// Picks between the two decodes of an uncertain chunk: a usable result beats an unusable one, a
/// right script beats a wrong one, English loses when the other language scored 0.5 or more, and
/// otherwise the higher average log-probability wins.
pub fn pick_better<'a>(
    a: &'a Candidate,
    b: &'a Candidate,
    det: &Detection,
    rules: &Rules,
) -> &'a Candidate {
    let en_barred = |c: &Candidate, other: &Candidate| {
        c.lang == "en" && det.prob_of(&other.lang) >= rules.not_english_at
    };
    let rank = |c: &Candidate, other: &Candidate| -> (u8, u8, u8) {
        (
            u8::from(!c.empty),
            u8::from(!c.wrong_script),
            u8::from(!en_barred(c, other)),
        )
    };
    let (ra, rb) = (rank(a, b), rank(b, a));
    if ra != rb {
        return if ra > rb { a } else { b };
    }
    if b.avg_logprob > a.avg_logprob { b } else { a }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn langs(l: &[&str]) -> Vec<String> {
        l.iter().map(|s| s.to_string()).collect()
    }

    fn det(en: f32, hi: f32, ur: f32) -> Detection {
        let raw = vec![
            ("en".to_string(), en),
            ("hi".to_string(), hi),
            ("ur".to_string(), ur),
        ];
        Detection::restrict(&raw, &langs(&["en", "hi", "ur"]))
    }

    fn det2(en: f32, hi: f32) -> Detection {
        let raw = vec![
            ("en".to_string(), en),
            ("hi".to_string(), hi),
            ("ur".to_string(), 0.3),
        ];
        Detection::restrict(&raw, &langs(&["en", "hi"]))
    }

    fn single(plan: &Plan) -> Option<(&str, Reason)> {
        match plan {
            Plan::Single { lang, reason } => Some((lang.as_str(), *reason)),
            Plan::Both { .. } => None,
        }
    }

    #[test]
    fn probabilities_are_restricted_to_the_sessions_languages_and_renormalised() {
        // Whisper gave Urdu 0.3; the session expects English and Hindi only.
        let d = det2(0.1, 0.2);
        assert_eq!(d.probs.len(), 2);
        assert_eq!(d.best().map(|b| b.0.as_str()), Some("hi"));
        assert!((d.prob_of("hi") - 2.0 / 3.0).abs() < 1e-6);
        assert!((d.probs.iter().map(|p| p.1).sum::<f32>() - 1.0).abs() < 1e-6);
        // Nothing known: equal shares, session order.
        let none = Detection::restrict(&[], &langs(&["en", "hi"]));
        assert_eq!(
            none.probs,
            vec![("en".to_string(), 0.5), ("hi".to_string(), 0.5)]
        );
    }

    #[test]
    fn the_operators_lock_wins_until_detection_is_restored() {
        let log = vec![
            LangLogEntry {
                audio_ms: 10_000,
                lang: Some("hi".into()),
            },
            LangLogEntry {
                audio_ms: 50_000,
                lang: None,
            },
            LangLogEntry {
                audio_ms: 70_000,
                lang: Some("ur".into()),
            },
        ];
        assert_eq!(locked_language(&log, 5_000), None);
        assert_eq!(locked_language(&log, 10_000), Some("hi"));
        assert_eq!(locked_language(&log, 49_999), Some("hi"));
        assert_eq!(
            locked_language(&log, 50_000),
            None,
            "Alt+4 returns to detection"
        );
        assert_eq!(locked_language(&log, 90_000), Some("ur"));
        // The log may arrive unsorted.
        let shuffled = vec![log[2].clone(), log[0].clone(), log[1].clone()];
        assert_eq!(locked_language(&shuffled, 60_000), None);
        let d = det(0.9, 0.05, 0.05);
        let plan = decide(
            &d,
            8.0,
            Some("hi"),
            None,
            None,
            &langs(&["en", "hi", "ur"]),
            &Rules::default(),
        );
        assert_eq!(single(&plan), Some(("hi", Reason::Locked)));
    }

    #[test]
    fn a_session_with_one_language_decodes_in_it() {
        let d = det2(0.1, 0.9);
        let plan = decide(
            &d,
            8.0,
            None,
            None,
            None,
            &langs(&["en"]),
            &Rules::default(),
        );
        assert_eq!(single(&plan), Some(("en", Reason::OnlyLanguage)));
    }

    #[test]
    fn a_clear_chunk_is_decoded_in_its_language() {
        let en = decide(
            &det2(0.9, 0.05),
            6.0,
            None,
            Some("hi"),
            None,
            &langs(&["en", "hi"]),
            &Rules::default(),
        );
        assert_eq!(single(&en), Some(("en", Reason::Detected)));
        let hi = decide(
            &det2(0.05, 0.9),
            6.0,
            None,
            Some("en"),
            None,
            &langs(&["en", "hi"]),
            &Rules::default(),
        );
        assert_eq!(single(&hi), Some(("hi", Reason::Detected)));
    }

    #[test]
    fn a_short_unsure_chunk_takes_its_neighbours_language() {
        let l = langs(&["en", "hi"]);
        // 1.2 s, best 0.7 (< 0.8): takes the language both neighbours agree on, then the previous.
        let d = det2(0.7, 0.3);
        let r = Rules::default();
        assert_eq!(
            single(&decide(&d, 1.2, None, Some("hi"), Some("hi"), &l, &r)),
            Some(("hi", Reason::Neighbours))
        );
        assert_eq!(
            single(&decide(&d, 1.2, None, Some("hi"), Some("en"), &l, &r)),
            Some(("hi", Reason::Neighbours))
        );
        assert_eq!(
            single(&decide(&d, 1.2, None, None, Some("hi"), &l, &r)),
            Some(("hi", Reason::Neighbours))
        );
        // No neighbours: its own best guess (0.7 vs 0.3 is a clear margin).
        assert_eq!(
            single(&decide(&d, 1.2, None, None, None, &l, &r)),
            Some(("en", Reason::Detected))
        );
        // Sure enough (>= 0.8): keeps its own language even when short.
        let sure = det2(0.85, 0.15);
        assert_eq!(
            single(&decide(&sure, 1.2, None, Some("hi"), Some("hi"), &l, &r)),
            Some(("en", Reason::Detected))
        );
        // Long enough: detection stands.
        assert_eq!(
            single(&decide(&d, 2.5, None, Some("hi"), Some("hi"), &l, &r)),
            Some(("en", Reason::Detected))
        );
    }

    #[test]
    fn chunks_within_02_are_decoded_both_ways() {
        let l = langs(&["en", "hi"]);
        let plan = decide(
            &det2(0.55, 0.45),
            6.0,
            None,
            None,
            None,
            &l,
            &Rules::default(),
        );
        assert_eq!(
            plan,
            Plan::Both {
                first: "en".into(),
                second: "hi".into()
            }
        );
        let plan = decide(
            &det2(0.42, 0.58),
            6.0,
            None,
            None,
            None,
            &l,
            &Rules::default(),
        );
        assert_eq!(
            plan,
            Plan::Both {
                first: "hi".into(),
                second: "en".into()
            }
        );
        // 0.65 vs 0.35: a margin of 0.3 is clear.
        assert_eq!(
            single(&decide(
                &det2(0.65, 0.35),
                6.0,
                None,
                None,
                None,
                &l,
                &Rules::default()
            )),
            Some(("en", Reason::Detected))
        );
        // A margin just over 0.2 (0.61 vs 0.39) is not within 0.2.
        assert_eq!(
            single(&decide(
                &det2(0.61, 0.39),
                6.0,
                None,
                None,
                None,
                &l,
                &Rules::default()
            )),
            Some(("en", Reason::Detected))
        );
    }

    #[test]
    fn three_languages_compare_the_two_best() {
        let l = langs(&["en", "hi", "ur"]);
        let plan = decide(
            &det(0.1, 0.45, 0.4),
            6.0,
            None,
            None,
            None,
            &l,
            &Rules::default(),
        );
        assert_eq!(
            plan,
            Plan::Both {
                first: "hi".into(),
                second: "ur".into()
            }
        );
    }

    #[test]
    fn english_is_never_chosen_while_another_language_scores_half_or_more() {
        // With ties the session's order puts English first; Hindi holds half the probability.
        let l = langs(&["en", "hi"]);
        let d = det2(0.5, 0.5);
        assert_eq!(d.best().map(|b| b.0.as_str()), Some("en"));
        let plan = decide(&d, 6.0, None, None, None, &l, &Rules::default());
        assert_eq!(single(&plan), Some(("hi", Reason::NotEnglish)));
    }

    #[test]
    fn the_better_decode_wins_unless_it_is_unusable_or_in_the_wrong_script() {
        let d = det2(0.55, 0.45);
        let r = Rules::default();
        let en = Candidate {
            lang: "en".into(),
            avg_logprob: -0.4,
            wrong_script: false,
            empty: false,
        };
        let hi = Candidate {
            lang: "hi".into(),
            avg_logprob: -0.6,
            wrong_script: false,
            empty: false,
        };
        assert_eq!(pick_better(&en, &hi, &d, &r).lang, "en");
        let hi_better = Candidate {
            avg_logprob: -0.2,
            ..hi.clone()
        };
        assert_eq!(pick_better(&en, &hi_better, &d, &r).lang, "hi");
        // Order does not matter.
        assert_eq!(pick_better(&hi_better, &en, &d, &r).lang, "hi");
        // A better score in the wrong script loses: Hindi speech "decoded" as Latin letters.
        let hi_wrong = Candidate {
            avg_logprob: -0.1,
            wrong_script: true,
            ..hi.clone()
        };
        assert_eq!(pick_better(&en, &hi_wrong, &d, &r).lang, "en");
        // An empty decode loses.
        let hi_empty = Candidate {
            avg_logprob: -0.01,
            empty: true,
            ..hi.clone()
        };
        assert_eq!(pick_better(&en, &hi_empty, &d, &r).lang, "en");
        // English barred (Hindi at 0.6) loses even with the better score.
        let strong_hi = det2(0.4, 0.6);
        let en_good = Candidate {
            avg_logprob: -0.1,
            ..en.clone()
        };
        assert_eq!(pick_better(&en_good, &hi, &strong_hi, &r).lang, "hi");
        // A tie goes to the first.
        let same = Candidate {
            lang: "hi".into(),
            avg_logprob: -0.4,
            wrong_script: false,
            empty: false,
        };
        assert_eq!(pick_better(&en, &same, &d, &r).lang, "en");
    }
}
