//! The languages of a session: English plus at most one of Hindi (Devanagari) or Urdu (Nastaliq).
//!
//! Hindi and Urdu are one spoken language and the organiser chooses the script of the transcript (owner's decisions of
//! 13 Nov and 24 Nov 2026). A session therefore never lists both, and detection never chooses between them. This
//! module is the one place that turns whatever a session or the programme lists into that rule. It never refuses:
//! a list that breaks the rule (an old session folder, the day plan, a hand-edited programme) is loaded, Hindi in
//! Devanagari is used, and a warning that names the session comes back for the app to show. Recording is never
//! blocked by it.

use serde::Deserialize;

/// The script of Hindi.
pub const DEVANAGARI: &str = "devanagari";
/// The script of Urdu.
pub const NASTALIQ: &str = "nastaliq";

/// The languages a session is processed in, the script of its Hindi or Urdu, and what to tell the organiser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionLanguages {
    /// `en`, and at most one of `hi` or `ur`, in the order listed.
    pub languages: Vec<String>,
    /// `devanagari` for `hi`, `nastaliq` for `ur`, `None` when the session has neither.
    pub script: Option<&'static str>,
    /// A message naming the session, when what it listed broke the rule.
    pub warning: Option<String>,
}

/// Turns the languages a session lists (and the script it carries, if any) into the one-language rule.
///
/// * only `hi`: Hindi, Devanagari; only `ur`: Urdu, Nastaliq. The listed language wins over a script that disagrees
///   (a warning says so).
/// * both: Hindi in Devanagari, with a warning.
/// * neither: no Hindi or Urdu, no script.
pub fn session_languages(title: &str, listed: &[String], script: Option<&str>) -> SessionLanguages {
    let has_hi = listed.iter().any(|l| l == "hi");
    let has_ur = listed.iter().any(|l| l == "ur");
    let (code, resolved, warning): (Option<&str>, Option<&'static str>, Option<String>) = match (
        has_hi, has_ur,
    ) {
        (true, true) => (
            Some("hi"),
            Some(DEVANAGARI),
            Some(format!(
                "{title} lists both Hindi and Urdu. A session uses English plus one of them; \
                     Hindi (Devanagari) is used. Change it in the programme or in the Start Session dialog."
            )),
        ),
        (true, false) => (
            Some("hi"),
            Some(DEVANAGARI),
            (script == Some(NASTALIQ)).then(|| {
                format!(
                    "{title} lists Hindi but its script is Nastaliq. Hindi (Devanagari) is used; \
                         to write Urdu list Urdu instead."
                )
            }),
        ),
        (false, true) => (
            Some("ur"),
            Some(NASTALIQ),
            (script == Some(DEVANAGARI)).then(|| {
                format!(
                    "{title} lists Urdu but its script is Devanagari. Urdu (Nastaliq) is used; \
                         to write Hindi list Hindi instead."
                )
            }),
        ),
        (false, false) => (None, None, None),
    };
    let mut languages: Vec<String> = Vec::new();
    for l in listed {
        let mapped = if l == "hi" || l == "ur" {
            match code {
                Some(c) => c.to_owned(),
                None => continue,
            }
        } else {
            l.clone()
        };
        if !languages.contains(&mapped) {
            languages.push(mapped);
        }
    }
    SessionLanguages {
        languages,
        script: resolved,
        warning,
    }
}

#[derive(Deserialize)]
struct Programme {
    sessions: Vec<Entry>,
}

#[derive(Deserialize)]
struct Entry {
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    languages: Option<Vec<String>>,
}

/// The languages the programme gives to the session `programme_id`, or `None` when the programme does not parse or
/// has no such session. An entry without a `languages` field means English and Hindi (the default everywhere).
/// An entry that lists both Hindi and Urdu loads like any other, with the warning of [`session_languages`].
pub fn programme_languages(programme_json: &str, programme_id: &str) -> Option<SessionLanguages> {
    let programme: Programme = serde_json::from_str(programme_json).ok()?;
    let entry = programme.sessions.iter().find(|s| s.id == programme_id)?;
    let listed = entry
        .languages
        .clone()
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| vec!["en".into(), "hi".into()]);
    Some(session_languages(&entry.title, &listed, None))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn l(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn english_and_hindi_is_devanagari_with_no_warning() {
        let r = session_languages("Keynote", &l(&["en", "hi"]), None);
        assert_eq!(r.languages, l(&["en", "hi"]));
        assert_eq!(r.script, Some(DEVANAGARI));
        assert_eq!(r.warning, None);
    }

    #[test]
    fn english_and_urdu_is_nastaliq_with_no_warning() {
        let r = session_languages("Keynote", &l(&["en", "ur"]), Some(NASTALIQ));
        assert_eq!(r.languages, l(&["en", "ur"]));
        assert_eq!(r.script, Some(NASTALIQ));
        assert_eq!(r.warning, None);
    }

    /// The Panel of the 21 Nov day simulation listed all three languages with script nastaliq. The old rule let the
    /// script pick Urdu; a session that lists both now gets Devanagari and a warning that names it.
    #[test]
    fn a_session_that_lists_both_gets_hindi_in_devanagari_and_a_warning_that_names_it() {
        for script in [None, Some(DEVANAGARI), Some(NASTALIQ)] {
            let r = session_languages("Panel", &l(&["en", "hi", "ur"]), script);
            assert_eq!(r.languages, l(&["en", "hi"]), "script {script:?}");
            assert_eq!(r.script, Some(DEVANAGARI));
            let w = r.warning.expect("a warning");
            assert!(
                w.starts_with("Panel lists both Hindi and Urdu"),
                "names the session: {w}"
            );
        }
        let reversed = session_languages("Panel", &l(&["ur", "en", "hi"]), None);
        assert_eq!(
            reversed.languages,
            l(&["hi", "en"]),
            "the pair keeps its place"
        );
    }

    #[test]
    fn a_listed_language_wins_over_a_script_that_disagrees() {
        let r = session_languages("Talk", &l(&["en", "ur"]), Some(DEVANAGARI));
        assert_eq!((r.languages, r.script), (l(&["en", "ur"]), Some(NASTALIQ)));
        assert!(r.warning.unwrap().starts_with("Talk lists Urdu"));
        let r = session_languages("Talk", &l(&["en", "hi"]), Some(NASTALIQ));
        assert_eq!(
            (r.languages, r.script),
            (l(&["en", "hi"]), Some(DEVANAGARI))
        );
        assert!(r.warning.unwrap().starts_with("Talk lists Hindi"));
    }

    #[test]
    fn english_only_has_no_hindi_urdu_script() {
        let r = session_languages("Talk", &l(&["en"]), Some(NASTALIQ));
        assert_eq!((r.languages, r.script, r.warning), (l(&["en"]), None, None));
        let r = session_languages("Talk", &l(&[]), None);
        assert!(r.languages.is_empty() && r.script.is_none());
    }

    #[test]
    fn the_programme_gives_each_session_its_languages_and_defaults_to_english_and_hindi() {
        let json = r#"{"sessions":[
            {"id":"a","title":"Hindi talk","languages":["en","hi"]},
            {"id":"b","title":"Urdu talk","languages":["en","ur"]},
            {"id":"c","title":"No field"},
            {"id":"d","title":"Both","languages":["en","hi","ur"]}]}"#;
        assert_eq!(
            programme_languages(json, "a").unwrap().languages,
            l(&["en", "hi"])
        );
        let b = programme_languages(json, "b").unwrap();
        assert_eq!((b.languages, b.script), (l(&["en", "ur"]), Some(NASTALIQ)));
        assert_eq!(
            programme_languages(json, "c").unwrap().languages,
            l(&["en", "hi"])
        );
        assert_eq!(programme_languages(json, "zzz"), None);
        assert_eq!(programme_languages("not json", "a"), None);
    }

    /// A programme that lists both is loaded, never refused: the session still records.
    #[test]
    fn a_programme_entry_that_lists_both_loads_with_a_warning() {
        let json = r#"{"sessions":[{"id":"d","title":"Both","languages":["en","hi","ur"]}]}"#;
        let r = programme_languages(json, "d").expect("loads");
        assert_eq!(r.languages, l(&["en", "hi"]));
        assert!(
            r.warning
                .unwrap()
                .starts_with("Both lists both Hindi and Urdu")
        );
    }

    #[test]
    fn the_shipped_programme_loads_and_every_session_is_english_plus_one() {
        let shipped = include_str!("../../../docs/programme.json");
        let all: serde_json::Value = serde_json::from_str(shipped).unwrap();
        for s in all["sessions"].as_array().unwrap() {
            let id = s["id"].as_str().unwrap();
            let r = programme_languages(shipped, id).expect(id);
            assert!(r.warning.is_none(), "{id}: {:?}", r.warning);
            assert!(
                r.languages
                    .iter()
                    .filter(|x| *x == "hi" || *x == "ur")
                    .count()
                    <= 1,
                "{id}"
            );
        }
    }
}
