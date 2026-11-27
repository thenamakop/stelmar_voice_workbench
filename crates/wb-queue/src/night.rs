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
/// When the programme has no entry for the day, the night pass does not start before this hour (IST) either: a
/// session that ends at midday does not mean the day is over (owner, 19 Nov 2026).
pub const EVENING_HOUR_IST: u64 = 20;

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

/// A language the job decodes but neither map knows counts at the slowest speed known, not for free.
fn slowest_speed(night_rtf: &BTreeMap<String, f64>, decode_rtf: &BTreeMap<String, f64>) -> f64 {
    decode_rtf
        .values()
        .chain(night_rtf.values())
        .copied()
        .fold(1.0, f64::max)
}

/// A session gets a night job when it has audio in a language that has a night model.
fn has_night_audio(s: &SessionAudio, night_rtf: &BTreeMap<String, f64>) -> bool {
    s.audio_ms_by_lang
        .iter()
        .any(|(lang, ms)| *ms > 0 && night_rtf.contains_key(lang))
}

/// The seconds the night job of a session needs: the whole audio at the speed of the model it decodes each language with.
fn session_seconds(
    s: &SessionAudio,
    night_rtf: &BTreeMap<String, f64>,
    decode_rtf: &BTreeMap<String, f64>,
    slowest: f64,
) -> f64 {
    s.audio_ms_by_lang
        .iter()
        .map(|(lang, ms)| {
            let rtf = decode_rtf
                .get(lang)
                .or_else(|| night_rtf.get(lang))
                .copied()
                .unwrap_or(slowest);
            *ms as f64 / 1000.0 * rtf
        })
        .sum()
}

/// Plans the night pass. `night_rtf` holds, per language that has a night model, its RTF in the chosen
/// configuration: a session gets a night job when it has audio in one of those languages. `decode_rtf` holds, per
/// language, the RTF of the model the night job uses for it (the night model, or the rolling model for a language
/// without one). The job decodes the WHOLE audio of the session, so the time counts all of it at those speeds,
/// not only the languages with a night model (decision of 23 Nov 2026: the plan counts what the job decodes; the
/// 21 Nov plan counted only the Hindi minutes of Inaugural, about 0.5 of its 90.5, while its job decoded all of them).
/// Whether a language without a night model should be decoded again at night is a Phase 5 question.
pub fn plan(
    start_ms: u64,
    sessions: &[SessionAudio],
    night_rtf: &BTreeMap<String, f64>,
    decode_rtf: &BTreeMap<String, f64>,
) -> Plan {
    if night_rtf.is_empty() {
        return Plan::Skip(SkipReason::NoNightModel);
    }
    let slowest = slowest_speed(night_rtf, decode_rtf);
    let mut planned = Vec::new();
    let mut needed_s = 0.0;
    for s in sessions {
        if !has_night_audio(s, night_rtf) {
            continue;
        }
        planned.push(s.id.clone());
        needed_s += session_seconds(s, night_rtf, decode_rtf, slowest);
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

/// The speeds a night plan is made with: the RTF of each language's night model, and of what the job decodes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Speeds {
    pub night_rtf: BTreeMap<String, f64>,
    pub decode_rtf: BTreeMap<String, f64>,
}

/// Which models the night job uses (Phase 5, 26 Nov 2026).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// The Hinglish model decodes the night pass of the English + Hindi sessions: its plan finishes before 08:00.
    Hinglish,
    /// Today's night models (or none): the Hinglish plan does not finish before 08:00, or there is no measured model.
    Today,
}

/// A night plan and the models it uses.
#[derive(Debug, Clone, PartialEq)]
pub struct Routed {
    pub route: Route,
    pub plan: Plan,
    /// Why the Hinglish plan was not used, when one was tried (shown in the log and on the session).
    pub hinglish_skip: Option<SkipReason>,
}

/// The file in a session folder that marks it "best finished on a faster machine" (written by the app, never by the
/// recorder): why the Hinglish model could not finish it here, so the export screen can offer it (Phase 6).
pub const FASTER_MACHINE_FILE: &str = "faster-machine.json";

/// The text of [`FASTER_MACHINE_FILE`].
pub fn faster_machine_json(reason: &str, hinglish_model: Option<&str>, now_ms: u64) -> String {
    serde_json::json!({
        "version": 1,
        "best_finished_on": "a faster machine",
        "reason": reason,
        "model": hinglish_model,
        "marked_at_ms": now_ms,
    })
    .to_string()
}

/// Plans the night with the Hinglish model when that finishes before 08:00 and with today's models otherwise.
/// `hinglish` is the sessions that would use it (English + Hindi) with their speeds, `None` where the system check
/// did not measure the model or it does not fit the night here; the sessions of `today` are the same ones, with the
/// languages that Hinglish sessions use under their own names when a day mixes both kinds. The Hinglish plan is
/// tried first and used only when it is a `Run`: a night that cannot finish must not be started, and a session left
/// with today's models (or none) is marked "best finished on a faster machine" by the caller.
pub fn route(
    start_ms: u64,
    today_sessions: &[SessionAudio],
    hinglish: Option<(&[SessionAudio], &Speeds)>,
    today: &Speeds,
) -> Routed {
    let mut hinglish_skip = None;
    if let Some((sessions, h)) = hinglish {
        match plan(start_ms, sessions, &h.night_rtf, &h.decode_rtf) {
            run @ Plan::Run { .. } => {
                return Routed {
                    route: Route::Hinglish,
                    plan: run,
                    hinglish_skip: None,
                };
            }
            Plan::Skip(reason) => hinglish_skip = Some(reason),
        }
    }
    Routed {
        route: Route::Today,
        plan: plan(
            start_ms,
            today_sessions,
            &today.night_rtf,
            &today.decode_rtf,
        ),
        hinglish_skip,
    }
}

/// One priority class of the night's work: its sessions and the speeds they decode at.
#[derive(Debug, Clone)]
pub struct Tier {
    pub sessions: Vec<SessionAudio>,
    pub speeds: Speeds,
    /// The whole class goes or none of it (the Hinglish redos: a night that cannot finish them must not start them).
    /// Otherwise each session is taken in order if it still fits.
    pub all_or_nothing: bool,
}

/// A session that did not get a night job in a tiered plan.
#[derive(Debug, Clone, PartialEq)]
pub struct Dropped {
    pub session: String,
    /// The index of its tier.
    pub tier: usize,
    pub needed_s: f64,
    /// The seconds that were left when it was tried (after the sessions of higher priority).
    pub left_s: f64,
}

/// A night plan made tier by tier.
#[derive(Debug, Clone, PartialEq)]
pub struct Tiered {
    pub plan: Plan,
    /// The sessions planned from each tier.
    pub planned: Vec<Vec<String>>,
    pub dropped: Vec<Dropped>,
}

/// Plans the night by priority (owner, 27 Nov 2026: night time is the scarcest resource): the sessions of the first tier
/// are planned first, then those of the second with the time that is left, and so on. A session that does not fit keeps
/// its rolling transcript (it gets no night job) and is listed in `dropped`; a session planned in an earlier tier is not
/// planned again. The order of `Plan::Run::sessions` is the priority order, so the night job queue works through it in that
/// order.
pub fn plan_tiered(start_ms: u64, tiers: &[Tier]) -> Tiered {
    let deadline = deadline_ms(start_ms);
    let available_s = deadline.saturating_sub(start_ms) as f64 / 1000.0;
    let mut used_s = 0.0;
    let mut planned: Vec<Vec<String>> = vec![Vec::new(); tiers.len()];
    let mut dropped = Vec::new();
    let mut taken: Vec<String> = Vec::new();
    let mut any_model = false;
    let mut any_candidate = false;
    for (t, tier) in tiers.iter().enumerate() {
        let night_rtf = &tier.speeds.night_rtf;
        let decode_rtf = &tier.speeds.decode_rtf;
        any_model |= !night_rtf.is_empty();
        let slowest = slowest_speed(night_rtf, decode_rtf);
        let candidates: Vec<(&SessionAudio, f64)> = tier
            .sessions
            .iter()
            .filter(|s| !taken.contains(&s.id) && has_night_audio(s, night_rtf))
            .map(|s| (s, session_seconds(s, night_rtf, decode_rtf, slowest)))
            .collect();
        any_candidate |= !candidates.is_empty();
        let total: f64 = candidates.iter().map(|(_, secs)| secs).sum();
        let whole_fits = used_s + total <= available_s;
        for (s, secs) in &candidates {
            let fits = if tier.all_or_nothing {
                whole_fits
            } else {
                used_s + secs <= available_s
            };
            if fits {
                // An all-or-nothing tier adds its total once, below; a per-session tier adds as it goes.
                if !tier.all_or_nothing {
                    used_s += secs;
                }
                planned[t].push(s.id.clone());
                taken.push(s.id.clone());
            } else {
                dropped.push(Dropped {
                    session: s.id.clone(),
                    tier: t,
                    needed_s: *secs,
                    left_s: (available_s - used_s).max(0.0),
                });
            }
        }
        if tier.all_or_nothing && whole_fits {
            used_s += total;
        }
    }
    let sessions: Vec<String> = planned.iter().flatten().cloned().collect();
    let plan = if !sessions.is_empty() {
        Plan::Run {
            sessions,
            estimate_s: used_s,
            finish_ms: start_ms + (used_s * 1000.0).round() as u64,
        }
    } else if !any_model {
        Plan::Skip(SkipReason::NoNightModel)
    } else if !any_candidate {
        Plan::Skip(SkipReason::NothingToDo)
    } else {
        Plan::Skip(SkipReason::DoesNotFit {
            needed_s: dropped.iter().map(|d| d.needed_s).sum(),
            available_s,
        })
    };
    Tiered {
        plan,
        planned,
        dropped,
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
    title: String,
    #[serde(default)]
    transcribe: bool,
}

/// The end of the programme's last session of the event day that contains `reference_ms` (an event day runs from
/// 08:00 to 08:00 IST, like [`evening_ms`]), counting only the sessions the programme marks for transcription
/// (docs/programme.json). `reference_ms` is the end of a session that was recorded, so a night pass after midnight
/// still belongs to the day that just ended: a last session ending at 00:30 must not pick up the next day's entry.
/// None when the programme does not parse or has no such session that day (a rehearsal, another date):
/// the caller then goes by the recorded sessions alone.
pub fn programme_day_end_ms(programme_json: &str, reference_ms: u64) -> Option<u64> {
    programme_day(programme_json, reference_ms).map(|d| d.last_end_ms)
}

/// What the programme lists, for transcription, on one IST day.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgrammeDay {
    /// The titles of the sessions marked for transcription, in the order the programme lists them.
    pub titles: Vec<String>,
    /// The end of the last of them (epoch ms).
    pub last_end_ms: u64,
}

/// The programme's entry for the event day (08:00 to 08:00 IST) that contains `reference_ms` (the sessions marked for transcription), or
/// None when the programme does not parse or lists none that day. Gate script: the pre-flight check of the day
/// simulation reads this through the app (`WB_GATE_CHECK=programme`), so the script cannot disagree with the app.
pub fn programme_day(programme_json: &str, reference_ms: u64) -> Option<ProgrammeDay> {
    let programme: Programme = serde_json::from_str(programme_json).ok()?;
    // Shift the reference back by the day's 08:00 start so that the event day, not the calendar day, is found.
    let day_hour_ms = DEADLINE_HOUR_IST * 3_600_000;
    let shifted = reference_ms
        .checked_add(IST_OFFSET_MS)?
        .checked_sub(day_hour_ms)?;
    let day_start = shifted - shifted % DAY_MS + day_hour_ms - IST_OFFSET_MS;
    let mut titles = Vec::new();
    let mut last_end_ms = 0u64;
    for s in programme.sessions.iter().filter(|s| s.transcribe) {
        let Some(end) = ist_epoch_ms(&s.date, &s.end) else {
            continue;
        };
        if end > day_start && end <= day_start + DAY_MS {
            titles.push(s.title.clone());
            last_end_ms = last_end_ms.max(end);
        }
    }
    (!titles.is_empty()).then_some(ProgrammeDay {
        titles,
        last_end_ms,
    })
}

/// The IST calendar date (`YYYY-MM-DD`) of an epoch time in ms (Howard Hinnant's civil-from-days).
pub fn ist_date(ms: u64) -> String {
    let days = i64::try_from(ms.saturating_add(IST_OFFSET_MS) / DAY_MS).unwrap_or(0);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    format!(
        "{:04}-{:02}-{:02}",
        if month <= 2 { year + 1 } else { year },
        month,
        day
    )
}

/// 20:00 IST of the event day that contains `end_ms`. An event day runs from 08:00 to 08:00 (the night pass's own
/// deadline is 08:00), so a session that ends at 01:00 belongs to the day before and its 20:00 has long passed.
pub fn evening_ms(end_ms: u64) -> u64 {
    let local = end_ms.saturating_add(IST_OFFSET_MS);
    let shifted = local.saturating_sub(DEADLINE_HOUR_IST * 3_600_000);
    let day_start = shifted - shifted % DAY_MS;
    (day_start + EVENING_HOUR_IST * 3_600_000).saturating_sub(IST_OFFSET_MS)
}

/// The moment the day counts as ended, for the 30-minute rule of [`may_start`]: the end of the last session
/// recorded or, when the programme lists the day, of its last session to transcribe if that is later. When the
/// programme has no entry for the day the pass also waits for 20:00 IST (`evening_floor`; the gate builds'
/// shortened wait turns it off). A programme entry overrides the evening rule: it is the better knowledge of
/// when the day ends.
pub fn day_end_reference_ms(
    programme_json: &str,
    last_recorded_end_ms: u64,
    evening_floor: bool,
) -> u64 {
    match programme_day_end_ms(programme_json, last_recorded_end_ms) {
        Some(day_end) => last_recorded_end_ms.max(day_end),
        None if evening_floor => last_recorded_end_ms
            .max(evening_ms(last_recorded_end_ms).saturating_sub(START_AFTER_LAST_SESSION_MS)),
        None => last_recorded_end_ms,
    }
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
        // A reference at 09:00 the next day is that next event day: its own programme is used.
        assert_eq!(
            programme_day_end_ms(PROGRAMME, at(24 + 9, 0)),
            Some(at(24 + 13, 0))
        );
        // The event day runs 08:00 to 08:00 (auditor item 10): a last recorded session that ends at 00:20, or at
        // 07:59, still belongs to the day before, so the night pass waits for that day's last session (18:15), not
        // for the next day's (13:00 on the 10th). The day changes at 08:00.
        assert_eq!(
            programme_day_end_ms(PROGRAMME, at(24, 20)),
            Some(at(18, 15))
        );
        assert_eq!(
            programme_day_end_ms(PROGRAMME, at(24 + 7, 59)),
            Some(at(18, 15))
        );
        assert_eq!(
            programme_day_end_ms(PROGRAMME, at(24 + 8, 0)),
            Some(at(24 + 13, 0))
        );
        // The reference is the end of the last recorded session, so a night pass at 01:00 still uses its day.
        assert_eq!(
            programme_day_end_ms(PROGRAMME, at(18, 15)),
            Some(at(18, 15))
        );
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
        let end =
            programme_day_end_ms(shipped, at(11, 30)).expect("day 1 has sessions to transcribe");
        assert!(end > at(11, 30) && end <= at(24, 0));
    }

    /// A programme without an entry for the day (a rehearsal, another date).
    const NO_ENTRY_PROGRAMME: &str = r#"{"sessions":[
        {"date":"2027-03-01","end":"11:30","transcribe":true}]}"#;

    #[test]
    fn with_no_programme_entry_a_midday_session_does_not_start_the_night_before_20_00() {
        // The last recorded session ended at 12:00; the day is not over: 20:00 is the earliest start.
        let reference = day_end_reference_ms(NO_ENTRY_PROGRAMME, at(12, 0), true);
        assert!(
            !may_start(at(12, 30), false, false, reference),
            "30 minutes after the session is too early"
        );
        assert!(!may_start(at(19, 59), false, false, reference));
        assert!(may_start(at(20, 0), false, false, reference));
        // Without the evening floor (the gate builds' shortened wait) the 30-minute rule alone applies.
        let plain = day_end_reference_ms(NO_ENTRY_PROGRAMME, at(12, 0), false);
        assert!(may_start(at(12, 30), false, false, plain));
    }

    #[test]
    fn with_no_programme_entry_an_evening_session_waits_only_30_minutes_after_it() {
        // Ended at 21:00, past 20:00: the later of the two is 30 minutes after the session.
        let reference = day_end_reference_ms(NO_ENTRY_PROGRAMME, at(21, 0), true);
        assert!(!may_start(at(21, 29), false, false, reference));
        assert!(may_start(at(21, 30), false, false, reference));
        // A session that ended at 01:00 belongs to the day before: its 20:00 passed long ago.
        let late = day_end_reference_ms(NO_ENTRY_PROGRAMME, at(24 + 1, 0), true);
        assert!(!may_start(at(24 + 1, 29), false, false, late));
        assert!(may_start(at(24 + 1, 30), false, false, late));
        // The event day runs 08:00 to 08:00: 07:59 is still the day before, 08:01 the new one.
        assert_eq!(evening_ms(at(7, 59)), at(20, 0) - DAY_MS);
        assert_eq!(evening_ms(at(8, 1)), at(20, 0));
    }

    #[test]
    fn a_programme_entry_overrides_the_evening_rule_as_before() {
        // The programme's last session to transcribe ends 18:15: the pass may start 18:45, not 20:00.
        let reference = day_end_reference_ms(PROGRAMME, at(11, 30), true);
        assert!(!may_start(at(18, 44), false, false, reference));
        assert!(may_start(at(18, 45), false, false, reference));
        // Sessions recorded past the programme's end push the reference later, never earlier.
        let late = day_end_reference_ms(PROGRAMME, at(19, 0), true);
        assert!(!may_start(at(19, 29), false, false, late));
        assert!(may_start(at(19, 30), false, false, late));
    }

    #[test]
    fn the_ist_date_of_an_epoch_time() {
        assert_eq!(ist_date(at(0, 0)), "2027-02-09");
        assert_eq!(ist_date(at(0, 0) - 1), "2027-02-08");
        assert_eq!(ist_date(at(23, 59)), "2027-02-09");
        assert_eq!(ist_date(0), "1970-01-01");
        // 2028 is a leap year.
        assert_eq!(
            ist_date(ist_epoch_ms("2028-02-29", "12:00").unwrap()),
            "2028-02-29"
        );
    }

    #[test]
    fn the_programme_day_lists_the_sessions_to_transcribe_for_the_pre_flight_check() {
        let day = programme_day(PROGRAMME, at(11, 30)).expect("an entry for the day");
        assert_eq!(day.titles.len(), 2);
        assert_eq!(day.last_end_ms, at(18, 15));
        assert_eq!(programme_day(NO_ENTRY_PROGRAMME, at(11, 30)), None);
        let shipped = include_str!("../../../docs/programme.json");
        // 2027-02-08: the four sessions of the day simulation (Inaugural, Parallel I, Parallel II, Panel).
        let day1 = programme_day(shipped, at(11, 30) - DAY_MS).expect("day 1");
        assert_eq!(day1.titles.len(), 4);
        assert_eq!(day1.last_end_ms, at(17, 30) - DAY_MS);
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

    /// English at the rolling model's speed 1.0 and Hindi at the night model's 2.0, as the night job decodes them.
    fn decode() -> BTreeMap<String, f64> {
        BTreeMap::from([("en".to_owned(), 1.0), ("hi".to_owned(), 2.0)])
    }

    #[test]
    fn a_night_model_for_hindi_only_gives_post_2_to_sessions_with_hindi_and_counts_all_their_audio()
    {
        let sessions = vec![
            session("a", 90, 0),
            session("b", 45, 45),
            session("c", 0, 90),
        ];
        let rtf = BTreeMap::from([("hi".to_owned(), 2.0)]);
        match plan(at(18, 0), &sessions, &rtf, &decode()) {
            Plan::Run {
                sessions,
                estimate_s,
                finish_ms,
            } => {
                // Session a has no Hindi audio and gets no post-2. Sessions b and c are decoded whole: b is 45 min
                // of English at 1.0 and 45 min of Hindi at 2.0, c is 90 min of Hindi at 2.0.
                assert_eq!(sessions, vec!["b", "c"]);
                assert!((estimate_s - (45.0 * 1.0 + 45.0 * 2.0 + 90.0 * 2.0) * 60.0).abs() < 1e-6);
                assert_eq!(finish_ms, at(18, 0) + (estimate_s * 1000.0) as u64);
            }
            other => panic!("expected a run, got {other:?}"),
        }
    }

    #[test]
    fn the_plan_counts_the_whole_session_the_job_decodes_not_only_the_languages_with_a_night_model()
    {
        // The 21 Nov 2026 shape: a session that is almost all English (no night model) with a little Hindi (one).
        // The job decodes all of it; the old plan counted only the Hindi minutes (10 min at 3.0 = 30 min) and the
        // new one counts the 100 min of English at the rolling model's 1.0 as well (130 min).
        let sessions = vec![session("a", 100, 10)];
        let night = BTreeMap::from([("hi".to_owned(), 3.0)]);
        let decode = BTreeMap::from([("en".to_owned(), 1.0), ("hi".to_owned(), 3.0)]);
        let Plan::Run { estimate_s, .. } = plan(at(18, 0), &sessions, &night, &decode) else {
            panic!("expected a run");
        };
        assert!(
            (estimate_s - 130.0 * 60.0).abs() < 1e-6,
            "plan minutes {} against the job's 130",
            estimate_s / 60.0
        );
        // And it decides on that size: 14 hours are left at 18:00, 820 min of English at 1.0 and 10 of Hindi
        // at 3.0 need 14 h 10 min; the old plan counted 30 min and would have run it.
        let sessions = vec![session("a", 820, 10)];
        let Plan::Skip(SkipReason::DoesNotFit { needed_s, .. }) =
            plan(at(18, 0), &sessions, &night, &decode)
        else {
            panic!("expected DoesNotFit");
        };
        assert!((needed_s - (820.0 + 30.0) * 60.0).abs() < 1e-6);
    }

    #[test]
    fn a_language_the_job_decodes_that_no_map_knows_counts_at_the_slowest_speed() {
        let sessions = vec![SessionAudio {
            id: "a".into(),
            audio_ms_by_lang: BTreeMap::from([
                ("hi".to_owned(), 60_000),
                ("ur".to_owned(), 60_000),
            ]),
        }];
        let night = BTreeMap::from([("hi".to_owned(), 2.0)]);
        let Plan::Run { estimate_s, .. } = plan(at(18, 0), &sessions, &night, &night) else {
            panic!("expected a run");
        };
        assert!((estimate_s - (60.0 * 2.0 + 60.0 * 2.0)).abs() < 1e-6);
    }

    #[test]
    fn it_is_skipped_when_it_cannot_finish_before_8_am() {
        // 14 hours left at 18:00; 4 h of Hindi at RTF 4 needs 16 h.
        let sessions = vec![session("a", 0, 240)];
        let rtf = BTreeMap::from([("hi".to_owned(), 4.0)]);
        let Plan::Skip(SkipReason::DoesNotFit {
            needed_s,
            available_s,
        }) = plan(at(18, 0), &sessions, &rtf, &rtf)
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
            plan(at(18, 0), &sessions, &BTreeMap::new(), &decode()),
            Plan::Skip(SkipReason::NoNightModel)
        );
        let rtf = BTreeMap::from([("hi".to_owned(), 2.0)]);
        assert_eq!(
            plan(at(18, 0), &sessions, &rtf, &decode()),
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

    fn speeds(en: f64, hi: f64) -> Speeds {
        let m = BTreeMap::from([("en".to_owned(), en), ("hi".to_owned(), hi)]);
        Speeds {
            night_rtf: m.clone(),
            decode_rtf: m,
        }
    }

    /// Day 1's seminar hall: 3 h of English + Hindi audio, the night starts at 18:00 and has 14 h.
    fn three_hours() -> Vec<SessionAudio> {
        vec![session("a", 90, 90)]
    }

    #[test]
    fn the_night_uses_the_hinglish_model_when_its_plan_finishes_before_08_00() {
        // The UHD floor: Hinglish f16 at 0.67 (en) and 1.69 (hi) takes about 3.5 h for 3 h of audio.
        let hinglish = speeds(0.67, 1.69);
        let today = speeds(0.5, 1.0);
        let r = route(
            at(18, 0),
            &three_hours(),
            Some((&three_hours(), &hinglish)),
            &today,
        );
        assert_eq!(r.route, Route::Hinglish);
        assert!(matches!(r.plan, Plan::Run { .. }), "{r:?}");
        assert_eq!(r.hinglish_skip, None);
    }

    #[test]
    fn a_hinglish_night_that_would_run_past_08_00_is_not_started_and_today_s_models_run_instead() {
        // Hinglish on a slow processor: 9 h per hour of audio, 27 h for 3 h; today's model needs 2.5 h.
        let hinglish = speeds(5.0, 9.0);
        let today = speeds(0.5, 1.0);
        let r = route(
            at(18, 0),
            &three_hours(),
            Some((&three_hours(), &hinglish)),
            &today,
        );
        assert_eq!(r.route, Route::Today);
        assert!(matches!(r.plan, Plan::Run { .. }), "{r:?}");
        assert!(matches!(
            r.hinglish_skip,
            Some(SkipReason::DoesNotFit { needed_s, .. }) if needed_s > 14.0 * 3600.0
        ));
    }

    #[test]
    fn neither_model_fitting_skips_the_night_and_keeps_the_reason_for_the_marker() {
        let r = route(
            at(18, 0),
            &three_hours(),
            Some((&three_hours(), &speeds(5.0, 9.0))),
            &speeds(5.0, 9.0),
        );
        assert_eq!(r.route, Route::Today);
        assert!(
            matches!(r.plan, Plan::Skip(SkipReason::DoesNotFit { .. })),
            "{r:?}"
        );
        assert!(r.hinglish_skip.is_some());
    }

    #[test]
    fn without_a_measured_hinglish_model_the_plan_is_today_s_alone() {
        let today = speeds(0.5, 1.0);
        let r = route(at(18, 0), &three_hours(), None, &today);
        assert_eq!(r.route, Route::Today);
        assert_eq!(
            r.plan,
            plan(
                at(18, 0),
                &three_hours(),
                &today.night_rtf,
                &today.decode_rtf
            )
        );
        assert_eq!(r.hinglish_skip, None);
    }

    #[test]
    fn the_hinglish_plan_counts_the_hours_left_from_the_real_start_time() {
        // 6 h of audio at 1.69 (all Hindi) is 10.1 h: fits the 14 h of an 18:00 start, not the 6.5 h left at 01:30.
        let all_hindi = vec![session("a", 0, 360)];
        let hinglish = speeds(0.67, 1.69);
        let today = speeds(0.5, 1.0);
        assert_eq!(
            route(at(18, 0), &all_hindi, Some((&all_hindi, &hinglish)), &today).route,
            Route::Hinglish
        );
        let late = route(
            at(24 + 1, 30),
            &all_hindi,
            Some((&all_hindi, &hinglish)),
            &today,
        );
        assert_eq!(late.route, Route::Today);
        assert!(late.hinglish_skip.is_some());
    }

    #[test]
    fn the_marker_file_says_why_and_which_model_was_tried() {
        let text = faster_machine_json(
            "it would need about 27.0 h and only 14.0 h are left",
            Some("hinglish-q8_0"),
            42,
        );
        let v: serde_json::Value = serde_json::from_str(&text).expect("json");
        assert_eq!(v["version"], 1);
        assert_eq!(v["model"], "hinglish-q8_0");
        assert!(v["reason"].as_str().is_some_and(|r| r.contains("27.0 h")));
        assert_eq!(FASTER_MACHINE_FILE, "faster-machine.json");
    }
    fn speeds_of(rtf: &[(&str, f64)]) -> Speeds {
        let m: BTreeMap<String, f64> = rtf.iter().map(|(l, r)| ((*l).to_owned(), *r)).collect();
        Speeds {
            night_rtf: m.clone(),
            decode_rtf: m,
        }
    }

    fn one_lang(id: &str, lang: &str, minutes: u64) -> SessionAudio {
        SessionAudio {
            id: id.into(),
            audio_ms_by_lang: BTreeMap::from([(lang.to_owned(), minutes * 60_000)]),
        }
    }

    #[test]
    fn the_night_is_planned_by_priority_and_what_does_not_fit_is_dropped_not_started() {
        // 14 h are left at 18:00. Hinglish redos: 2 h of audio at 4.0 = 8 h. Urdu: 1 h at 3.0 = 3 h. English: 1 h at 3.0
        // = 3 h (14 h in all), and a second English hour would make 17 h: dropped.
        let tiers = [
            Tier {
                sessions: vec![one_lang("hall-a", "hi", 120)],
                speeds: speeds_of(&[("hi", 4.0)]),
                all_or_nothing: true,
            },
            Tier {
                sessions: vec![one_lang("urdu", "ur", 60)],
                speeds: speeds_of(&[("ur", 3.0)]),
                all_or_nothing: false,
            },
            Tier {
                sessions: vec![one_lang("en-1", "en", 60), one_lang("en-2", "en", 60)],
                speeds: speeds_of(&[("en", 3.0)]),
                all_or_nothing: false,
            },
        ];
        let t = plan_tiered(at(18, 0), &tiers);
        let Plan::Run {
            sessions,
            estimate_s,
            ..
        } = &t.plan
        else {
            panic!("{t:?}")
        };
        assert_eq!(sessions, &["hall-a", "urdu", "en-1"], "priority order");
        assert!((estimate_s / 3600.0 - 14.0).abs() < 1e-9, "{estimate_s}");
        assert_eq!(t.dropped.len(), 1);
        assert_eq!(t.dropped[0].session, "en-2");
        assert_eq!(t.dropped[0].tier, 2);
        assert!(t.dropped[0].left_s < t.dropped[0].needed_s);
    }

    #[test]
    fn a_hinglish_class_that_does_not_fit_is_dropped_whole_and_the_lower_priorities_use_the_time() {
        // Two Hinglish sessions of 8 h each do not fit in 14 h together: neither starts; the English one runs.
        let tiers = [
            Tier {
                sessions: vec![one_lang("a", "hi", 120), one_lang("b", "hi", 120)],
                speeds: speeds_of(&[("hi", 4.0)]),
                all_or_nothing: true,
            },
            Tier {
                sessions: vec![one_lang("en-1", "en", 60)],
                speeds: speeds_of(&[("en", 3.0)]),
                all_or_nothing: false,
            },
        ];
        let t = plan_tiered(at(18, 0), &tiers);
        assert_eq!(t.planned[0], Vec::<String>::new());
        assert_eq!(t.planned[1], ["en-1"]);
        assert_eq!(t.dropped.len(), 2);
        assert!(t.dropped.iter().all(|d| d.tier == 0));
    }

    #[test]
    fn a_session_planned_in_an_earlier_tier_is_not_planned_again_and_an_empty_night_says_why() {
        let x = one_lang("x", "hi", 30);
        let tiers = [
            Tier {
                sessions: vec![x.clone()],
                speeds: speeds_of(&[("hi", 1.0)]),
                all_or_nothing: true,
            },
            Tier {
                sessions: vec![x],
                speeds: speeds_of(&[("hi", 1.0)]),
                all_or_nothing: false,
            },
        ];
        let t = plan_tiered(at(18, 0), &tiers);
        assert_eq!(t.planned, vec![vec!["x".to_string()], Vec::<String>::new()]);
        assert!(t.dropped.is_empty());
        let tier = |sessions: Vec<SessionAudio>, speeds: Speeds| Tier {
            sessions,
            speeds,
            all_or_nothing: false,
        };
        let none = plan_tiered(
            at(18, 0),
            &[tier(vec![one_lang("x", "en", 30)], Speeds::default())],
        );
        assert_eq!(none.plan, Plan::Skip(SkipReason::NoNightModel));
        let nothing = plan_tiered(
            at(18, 0),
            &[tier(
                vec![one_lang("x", "en", 30)],
                speeds_of(&[("hi", 1.0)]),
            )],
        );
        assert_eq!(nothing.plan, Plan::Skip(SkipReason::NothingToDo));
        let late = plan_tiered(
            at(18, 0),
            &[tier(
                vec![one_lang("x", "en", 600)],
                speeds_of(&[("en", 3.0)]),
            )],
        );
        assert!(matches!(
            late.plan,
            Plan::Skip(SkipReason::DoesNotFit { .. })
        ));
    }
}
