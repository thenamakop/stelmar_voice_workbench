//! The operator's language locks (Alt+1, Alt+2, Alt+3 lock English, Hindi or Urdu; Alt+4 returns to detection).
//! Each change is appended to `<session>/language-log.jsonl` with its audio time (milliseconds on the session's
//! audio clock) and the IST wall-clock time, and fsynced. The recorder does not carry it, the same way as
//! `start-options.json`. Post-processing reads it: each unit gets the entries inside its range and the last one
//! before it (`wb_runner::exec::lang_log_for_unit`), so the lock is applied at the right chunk.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use wb_protocol::LangLogEntry;

pub fn path_in(session_dir: &Path) -> PathBuf {
    session_dir.join("language-log.jsonl")
}

#[derive(Debug, Serialize, Deserialize)]
struct Line {
    audio_ms: u64,
    /// `en`, `hi`, `ur`; absent (null) returns to detection among the session's languages.
    lang: Option<String>,
    /// IST, for the UI and the log only.
    #[serde(default)]
    at: String,
}

/// Appends one change. `lang` is None for Alt+4.
pub fn append(
    session_dir: &Path,
    audio_ms: u64,
    lang: Option<&str>,
    at_ist: &str,
) -> std::io::Result<()> {
    let line = Line {
        audio_ms,
        lang: lang.map(str::to_owned),
        at: at_ist.to_owned(),
    };
    let mut text = serde_json::to_string(&line).map_err(std::io::Error::other)?;
    text.push('\n');
    let path = path_in(session_dir);
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    file.write_all(text.as_bytes())?;
    file.sync_data()
}

/// Every change of the session, in file order (audio time is non-decreasing because the app writes them as they
/// happen). A torn last line (a crash while writing) is ignored.
pub fn read(session_dir: &Path) -> Vec<LangLogEntry> {
    let Ok(text) = std::fs::read_to_string(path_in(session_dir)) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|l| serde_json::from_str::<Line>(l).ok())
        .map(|l| LangLogEntry {
            audio_ms: l.audio_ms,
            lang: l.lang,
        })
        .collect()
}

/// The locks as the job decodes them. Hindi and Urdu are one spoken language with an output script the organiser
/// chose (13 Nov 2026), and the job carries only that one of the two: a lock on the other (Alt+3 in a Devanagari
/// session, Alt+2 in a Nastaliq one) is a lock on the job's Hindi/Urdu language, never a switch of the script (ASR
/// audit of 20 Nov 2026: the engine would otherwise fall back to the catch-all model and write the other script).
pub fn for_job(entries: Vec<LangLogEntry>, job_languages: &[String]) -> Vec<LangLogEntry> {
    let own = ["hi", "ur"]
        .into_iter()
        .find(|l| job_languages.iter().any(|j| j == l));
    entries
        .into_iter()
        .map(|mut e| {
            if let (Some(own), Some("hi" | "ur")) = (own, e.lang.as_deref()) {
                e.lang = Some(own.to_owned());
            }
            e
        })
        .collect()
}

/// A lock language the session accepts: one of its expected languages.
pub fn accepted(lang: &str, session_languages: &[String]) -> bool {
    session_languages.iter().any(|l| l == lang)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wb-langlock-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn locks_and_returns_to_detection_are_logged_with_their_audio_times() {
        let d = dir("log");
        append(&d, 12_000, Some("hi"), "10:02:11").unwrap();
        append(&d, 95_500, None, "10:03:34").unwrap();
        append(&d, 130_000, Some("en"), "10:04:09").unwrap();
        let got = read(&d);
        assert_eq!(
            got,
            vec![
                LangLogEntry {
                    audio_ms: 12_000,
                    lang: Some("hi".into())
                },
                LangLogEntry {
                    audio_ms: 95_500,
                    lang: None
                },
                LangLogEntry {
                    audio_ms: 130_000,
                    lang: Some("en".into())
                },
            ]
        );
    }

    #[test]
    fn a_lock_on_hindi_or_urdu_means_the_language_the_organiser_chose_the_script_for() {
        let e = |ms, l: Option<&str>| LangLogEntry {
            audio_ms: ms,
            lang: l.map(str::to_owned),
        };
        let log = vec![
            e(1, Some("ur")),
            e(2, Some("en")),
            e(3, Some("hi")),
            e(4, None),
        ];
        // A Devanagari session carries `hi`: Alt+3 (Urdu) locks Hindi, in Devanagari.
        let devanagari = vec!["en".to_owned(), "hi".to_owned()];
        let got = for_job(log.clone(), &devanagari);
        assert_eq!(
            got.iter().map(|x| x.lang.as_deref()).collect::<Vec<_>>(),
            vec![Some("hi"), Some("en"), Some("hi"), None]
        );
        // A Nastaliq session carries `ur`: Alt+2 (Hindi) locks Urdu, in Nastaliq.
        let nastaliq = vec!["en".to_owned(), "ur".to_owned()];
        let got = for_job(log.clone(), &nastaliq);
        assert_eq!(
            got.iter().map(|x| x.lang.as_deref()).collect::<Vec<_>>(),
            vec![Some("ur"), Some("en"), Some("ur"), None]
        );
        // A session with neither (English only) leaves the log as it is: the engine drops what no model serves.
        let english = vec!["en".to_owned()];
        assert_eq!(for_job(log.clone(), &english), log);
    }

    #[test]
    fn a_torn_last_line_and_a_missing_file_are_not_errors() {
        let d = dir("torn");
        assert!(read(&d).is_empty());
        append(&d, 1_000, Some("en"), "t").unwrap();
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(path_in(&d))
            .unwrap();
        f.write_all(b"{\"audio_ms\":2000,\"la").unwrap();
        assert_eq!(read(&d).len(), 1);
    }

    #[test]
    fn only_a_language_of_the_session_can_be_locked() {
        let langs = vec!["en".to_string(), "hi".to_string()];
        assert!(accepted("hi", &langs));
        assert!(!accepted("ur", &langs));
    }
}
