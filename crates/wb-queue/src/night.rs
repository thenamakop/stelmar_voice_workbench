//! The night pass (decision D6, Phase 4 step 5). After the day's last session, each language whose night
//! model ranks above its rolling model is transcribed again into `post-2`, if the estimate fits before
//! 08:00 IST; `post-2` becomes the default revision when it is complete.
//!
//! Which languages have a night model is already decided by the system check (`Decision::languages[l].night`
//! exists only when the model differs from the rolling one and fits the time), so this module takes the
//! night models' speeds as input and plans the time.

use std::collections::BTreeMap;

use crate::{DAY_MS, IST_OFFSET_MS};

/// The night pass must be finished by 08:00 IST.
pub const DEADLINE_HOUR_IST: u64 = 8;
/// The night pass starts this long after the day's last session ends (and never while recording).
pub const START_AFTER_LAST_SESSION_MS: u64 = 30 * 60_000;

/// One session of the day with its audio by language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionAudio {
    pub id: String,
    pub audio_ms_by_lang: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SkipReason {
    /// No language of the day has a night model that beats its rolling model.
    NoNightModel,
    /// There is no recorded audio in a language that has a night model.
    NothingToDo,
    /// The estimate runs past 08:00.
    DoesNotFit { needed_s: f64, available_s: f64 },
}

impl SkipReason {
    pub fn text(&self) -> String {
        match self {
            SkipReason::NoNightModel => {
                "no night model is more accurate than the rolling model".to_owned()
            }
            SkipReason::NothingToDo => "no audio in a language that has a night model".to_owned(),
            SkipReason::DoesNotFit {
                needed_s,
                available_s,
            } => format!(
                "it would need about {:.1} h and only {:.1} h are left before 08:00",
                needed_s / 3600.0,
                available_s / 3600.0
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Plan {
    Run {
        /// The sessions that get a `post-2` (those with audio in a night-model language), in day order.
        sessions: Vec<String>,
        estimate_s: f64,
        finish_ms: u64,
    },
    Skip(SkipReason),
}

/// The next 08:00 IST after `now_ms` (an epoch time in ms).
pub fn deadline_ms(now_ms: u64) -> u64 {
    let local = now_ms + IST_OFFSET_MS;
    let day_start = local - local % DAY_MS;
    let mut deadline = day_start + DEADLINE_HOUR_IST * 3_600_000;
    if local >= deadline {
        deadline += DAY_MS;
    }
    deadline - IST_OFFSET_MS
}

/// Plans the night pass. `night_rtf` holds, per language that has a night model, its RTF in the chosen
/// configuration. Only the audio of those languages counts toward the time; a session's other languages
/// keep their rolling text in `post-2`.
pub fn plan(start_ms: u64, sessions: &[SessionAudio], night_rtf: &BTreeMap<String, f64>) -> Plan {
    if night_rtf.is_empty() {
        return Plan::Skip(SkipReason::NoNightModel);
    }
    let mut planned = Vec::new();
    let mut needed_s = 0.0;
    for s in sessions {
        let secs: f64 = s
            .audio_ms_by_lang
            .iter()
            .filter_map(|(lang, ms)| night_rtf.get(lang).map(|rtf| *ms as f64 / 1000.0 * rtf))
            .sum();
        if secs > 0.0 {
            planned.push(s.id.clone());
            needed_s += secs;
        }
    }
    if planned.is_empty() {
        return Plan::Skip(SkipReason::NothingToDo);
    }
    let deadline = deadline_ms(start_ms);
    let available_s = deadline.saturating_sub(start_ms) as f64 / 1000.0;
    if needed_s > available_s {
        return Plan::Skip(SkipReason::DoesNotFit {
            needed_s,
            available_s,
        });
    }
    Plan::Run {
        sessions: planned,
        estimate_s: needed_s,
        finish_ms: start_ms + (needed_s * 1000.0).round() as u64,
    }
}

/// Days from 1970-01-01 to a civil date (proleptic Gregorian; Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The epoch time (ms) of an IST date `YYYY-MM-DD` and time `HH:MM`; None when either does not parse.
fn ist_epoch_ms(date: &str, time: &str) -> Option<u64> {
    let mut d = date.split('-').map(|x| x.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    if d.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&day) {
        return None;
    }
    let mut t = time.split(':').map(|x| x.parse::<u64>().ok());
    let (h, min) = (t.next()??, t.next()??);
    if t.next().is_some() || h > 23 || min > 59 {
        return None;
    }
    let local = u64::try_from(days_from_civil(y, m, day)).ok()? * DAY_MS + (h * 60 + min) * 60_000;
    local.checked_sub(IST_OFFSET_MS)
}

#[derive(serde::Deserialize)]
struct Programme {
    sessions: Vec<ProgrammeSession>,
}

#[derive(serde::Deserialize)]
struct ProgrammeSession {
    date: String,
    end: String,
    #[serde(default)]
    transcribe: bool,
}

/// The end of the programme's last session of the IST day that contains `reference_ms`, counting only the
/// sessions the programme marks for transcription (docs/programme.json). `reference_ms` is the end of a
/// session that was recorded, so a night pass after midnight still belongs to the day that just ended.
/// None when the programme does not parse or has no such session that day (a rehearsal, another date):
/// the caller then goes by the recorded sessions alone.
pub fn programme_day_end_ms(programme_json: &str, reference_ms: u64) -> Option<u64> {
    let programme: Programme = serde_json::from_str(programme_json).ok()?;
    let local = reference_ms.checked_add(IST_OFFSET_MS)?;
    let day_start = local - local % DAY_MS - IST_OFFSET_MS;
    programme
        .sessions
        .iter()
        .filter(|s| s.transcribe)
        .filter_map(|s| ist_epoch_ms(&s.date, &s.end))
        .filter(|end| *end > day_start && *end <= day_start + DAY_MS)
        .max()
}

/// May the night pass start now? Nothing is recording, every rolling job of the day is done, and the last
/// session ended at least 30 minutes ago.
pub fn may_start(
    now_ms: u64,
    recording: bool,
    rolling_pending: bool,
    last_session_end_ms: u64,
) -> bool {
    !recording && !rolling_pending && now_ms >= last_session_end_ms + START_AFTER_LAST_SESSION_MS
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2027-02-09 00:00 IST in epoch ms.
    fn day0() -> u64 {
        let utc_midnight = 1_802_174_400_000u64 - 1_802_174_400_000 % DAY_MS;
        utc_midnight - IST_OFFSET_MS
    }

    fn at(h: u64, m: u64) -> u64 {
        day0() + h * 3_600_000 + m * 60_000
    }

    fn session(id: &str, en_min: u64, hi_min: u64) -> SessionAudio {
        SessionAudio {
            id: id.into(),
            audio_ms_by_lang: BTreeMap::from([
                ("en".to_owned(), en_min * 60_000),
                ("hi".to_owned(), hi_min * 60_000),
            ]),
        }
    }

    const PROGRAMME: &str = r#"{"sessions":[
        {"date":"2027-02-09","end":"10:00","transcribe":false},
        {"date":"2027-02-09","end":"11:30","transcribe":true},
        {"date":"2027-02-09","end":"18:15","transcribe":true},
        {"date":"2027-02-09","end":"19:30","transcribe":false},
        {"date":"2027-02-10","end":"13:00","transcribe":true}]}"#;

    #[test]
    fn the_night_waits_for_the_programmes_last_transcribed_session_of_the_day() {
        // The day's last session to transcribe ends 18:15; a networking item after it does not count.
        let end = programme_day_end_ms(PROGRAMME, at(11, 30)).unwrap();
        assert_eq!(end, at(18, 15));
        // The pass may not start before 18:45 even though the recorded sessions ended at 11:30.
        assert!(!may_start(at(12, 5), false, false, end.max(at(11, 30))));
        assert!(!may_start(at(18, 44), false, false, end));
        assert!(may_start(at(18, 45), false, false, end));
    }

    #[test]
    fn the_reference_is_the_end_of_the_last_recorded_session_so_its_own_day_counts() {
        // A reference at 09:00 the next day is that next IST day: its own programme is used.
        assert_eq!(programme_day_end_ms(PROGRAMME, at(24 + 9, 0)), Some(at(24 + 13, 0)));
        // The reference is the end of the last recorded session, so a night pass at 01:00 still uses its day.
        assert_eq!(programme_day_end_ms(PROGRAMME, at(18, 15)), Some(at(18, 15)));
    }

    #[test]
    fn a_day_the_programme_does_not_know_falls_back_to_the_recorded_sessions() {
        assert_eq!(programme_day_end_ms(PROGRAMME, at(24 * 40, 10)), None);
        assert_eq!(programme_day_end_ms("not json", at(11, 30)), None);
        assert_eq!(programme_day_end_ms(r#"{"sessions":[]}"#, at(11, 30)), None);
        // A date or time that does not parse is ignored, not guessed.
        let odd = r#"{"sessions":[{"date":"2027-13-40","end":"25:99","transcribe":true}]}"#;
        assert_eq!(programme_day_end_ms(odd, at(11, 30)), None);
    }

    #[test]
    fn the_shipped_programme_parses_and_has_a_last_session_on_each_day() {
        let shipped = include_str!("../../../docs/programme.json");
        let end = programme_day_end_ms(shipped, at(11, 30)).expect("day 1 has sessions to transcribe");
        assert!(end > at(11, 30) && end <= at(24, 0));
    }

    #[test]
    fn the_deadline_is_the_next_8_am_in_ist() {
        assert_eq!(deadline_ms(at(18, 0)), at(32, 0));
        assert_eq!(deadline_ms(at(3, 0)), at(8, 0));
        assert_eq!(
            deadline_ms(at(8, 0)),
            at(32, 0),
            "at 08:00 exactly the next one counts"
        );
    }

    #[test]
    fn a_night_model_for_hindi_only_plans_only_the_hindi_audio() {
        let sessions = vec![
            session("a", 90, 0),
            session("b", 45, 45),
            session("c", 0, 90),
        ];
        let rtf = BTreeMap::from([("hi".to_owned(), 2.0)]);
        match plan(at(18, 0), &sessions, &rtf) {
            Plan::Run {
                sessions,
                estimate_s,
                finish_ms,
            } => {
                // Session a has no Hindi audio and gets no post-2.
                assert_eq!(sessions, vec!["b", "c"]);
                assert!((estimate_s - 135.0 * 60.0 * 2.0).abs() < 1e-6);
                assert_eq!(finish_ms, at(18, 0) + (estimate_s * 1000.0) as u64);
            }
            other => panic!("expected a run, got {other:?}"),
        }
    }

    #[test]
    fn it_is_skipped_when_it_cannot_finish_before_8_am() {
        // 14 hours left at 18:00; 4 h of Hindi at RTF 4 needs 16 h.
        let sessions = vec![session("a", 0, 240)];
        let rtf = BTreeMap::from([("hi".to_owned(), 4.0)]);
        let Plan::Skip(SkipReason::DoesNotFit {
            needed_s,
            available_s,
        }) = plan(at(18, 0), &sessions, &rtf)
        else {
            panic!("expected DoesNotFit");
        };
        assert!(needed_s > available_s);
        assert!((available_s - 14.0 * 3600.0).abs() < 1e-6);
    }

    #[test]
    fn no_night_model_and_no_matching_audio_are_skipped_with_a_reason() {
        let sessions = vec![session("a", 60, 0)];
        assert_eq!(
            plan(at(18, 0), &sessions, &BTreeMap::new()),
            Plan::Skip(SkipReason::NoNightModel)
        );
        let rtf = BTreeMap::from([("hi".to_owned(), 2.0)]);
        assert_eq!(
            plan(at(18, 0), &sessions, &rtf),
            Plan::Skip(SkipReason::NothingToDo)
        );
        assert!(SkipReason::NoNightModel.text().contains("rolling"));
    }

    #[test]
    fn it_starts_only_after_recording_and_rolling_are_done_and_the_last_session_is_30_minutes_old()
    {
        let end = at(17, 30);
        assert!(!may_start(at(17, 59), false, false, end));
        assert!(may_start(at(18, 0), false, false, end));
        assert!(!may_start(at(18, 0), true, false, end));
        assert!(!may_start(at(18, 0), false, true, end));
    }
}
