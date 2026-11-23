//! Work-unit cutting (Phase 4 step 2). A job is cut into units of about 5 minutes that start and end at VAD
//! pauses (AGENTS.md: "cuts happen only at VAD pauses"). The pauses come in as plain data: the app gets them
//! from the engine's `chunks` output for each closed part (a pause is a gap of at least 0.3 s between two
//! chunks; chunks that abut are forced cuts of the chunker, not pauses, and arrive as `fallback` points), so
//! everything here is a pure function with fixture tests.
//!
//! Rules:
//! * the target is 5 minutes; the cut is the middle of the longest pause within +/-60 s of the target, widening
//!   to +/-120 s when none qualifies; with no pause at all it falls back to the chunker's own forced cut nearest
//!   the target, else the target itself, and the unit records `forced_cut` (it is never silent);
//! * a unit never crosses a sample-rate change between parts (the engine refuses such a unit): the audio is cut
//!   at the change and that unit records `rate_change`;
//! * a rolling pass covers audio up to 2 seconds before the end of the last closed part; the final call (at Stop)
//!   makes the last unit end at the end of the session;
//! * units tile the audio from where the previous unit ended: no gap and no overlap.

use std::cmp::Reverse;

/// A closed part on the audio clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Part {
    pub start_ms: u64,
    pub end_ms: u64,
    pub sample_rate: u32,
}

/// A stretch the VAD calls non-speech, on the audio clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pause {
    pub start_ms: u64,
    pub end_ms: u64,
}

impl Pause {
    pub fn len_ms(&self) -> u64 {
        self.end_ms.saturating_sub(self.start_ms)
    }
    pub fn middle_ms(&self) -> u64 {
        self.start_ms + self.len_ms() / 2
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Params {
    pub target_ms: u64,
    pub window_ms: u64,
    pub wide_window_ms: u64,
    /// Shorter gaps are not pauses.
    pub min_pause_ms: u64,
    /// A rolling pass stops this far before the end of the last closed part.
    pub tail_guard_ms: u64,
    /// A cut never leaves less than this at the end of a run that is being closed.
    pub min_tail_ms: u64,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            target_ms: 300_000,
            window_ms: 60_000,
            wide_window_ms: 120_000,
            min_pause_ms: 300,
            tail_guard_ms: 2_000,
            min_tail_ms: 30_000,
        }
    }
}

/// Why a unit's end is not at a VAD pause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seam {
    /// No pause within +/-120 s: cut at the chunker's own forced cut, or at the target.
    ForcedCut,
    /// The sample rate changes here.
    RateChange,
}

impl Seam {
    pub fn as_str(self) -> &'static str {
        match self {
            Seam::ForcedCut => "forced_cut",
            Seam::RateChange => "rate_change",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnitSpec {
    pub idx: u32,
    pub start_ms: u64,
    pub end_ms: u64,
    /// Set when the unit's end is not at a VAD pause.
    pub seam: Option<Seam>,
}

/// Groups the parts into runs of one sample rate: `(start, end, has_following_run)`.
fn runs(parts: &[Part]) -> Vec<(u64, u64, bool)> {
    let mut out: Vec<(u64, u64, u32)> = Vec::new();
    for p in parts {
        match out.last_mut() {
            Some(last) if last.2 == p.sample_rate && last.1 == p.start_ms => last.1 = p.end_ms,
            // A different rate, or a hole between parts (never made by the recorder): a new run.
            _ => out.push((p.start_ms, p.end_ms, p.sample_rate)),
        }
    }
    let n = out.len();
    out.iter()
        .enumerate()
        .map(|(i, r)| (r.0, r.1, i + 1 < n))
        .collect()
}

/// The cut for a unit starting at `start`: the middle of the longest pause near `start + target`, else a forced cut.
/// `hi_cap` keeps the cut clear of the end of a run that is being closed.
fn choose_cut(
    start: u64,
    hi_cap: u64,
    pauses: &[Pause],
    fallback: &[u64],
    p: &Params,
) -> (u64, Option<Seam>) {
    let t = start + p.target_ms;
    for window in [p.window_ms, p.wide_window_ms] {
        let lo = t.saturating_sub(window).max(start + 1);
        let hi = (t + window).min(hi_cap);
        let best = pauses
            .iter()
            .filter(|z| z.len_ms() >= p.min_pause_ms)
            .map(|z| (z.len_ms(), z.middle_ms()))
            .filter(|(_, m)| *m >= lo && *m <= hi)
            .max_by_key(|(len, m)| (*len, Reverse(m.abs_diff(t)), Reverse(*m)));
        if let Some((_, m)) = best {
            return (m, None);
        }
    }
    let lo = t.saturating_sub(p.wide_window_ms).max(start + 1);
    let hi = (t + p.wide_window_ms).min(hi_cap);
    let forced = fallback
        .iter()
        .copied()
        .filter(|m| *m >= lo && *m <= hi)
        .min_by_key(|m| (m.abs_diff(t), *m))
        .unwrap_or(t.min(hi_cap));
    (forced, Some(Seam::ForcedCut))
}

/// Cuts the audio from `from_ms` into units. `next_idx` numbers the first new unit. `finished` is true at Stop
/// (the last unit then ends at the end of the last part); while recording it is false, and the rolling pass
/// stops 2 s before the end of the last closed part and waits until every pause it would need is known.
///
/// Calling it again with more parts continues from the end of the last unit it returned.
pub fn plan(
    next_idx: u32,
    from_ms: u64,
    parts: &[Part],
    pauses: &[Pause],
    fallback: &[u64],
    finished: bool,
    p: &Params,
) -> Vec<UnitSpec> {
    let mut out = Vec::new();
    let mut idx = next_idx;
    let mut start = from_ms;
    for (run_start, run_end, has_next) in runs(parts) {
        if run_end <= start {
            continue;
        }
        start = start.max(run_start);
        // A run followed by another run is complete; the last run is complete only at Stop.
        let closed = has_next || finished;
        loop {
            if closed {
                if run_end - start <= p.target_ms + p.window_ms {
                    out.push(UnitSpec {
                        idx,
                        start_ms: start,
                        end_ms: run_end,
                        seam: has_next.then_some(Seam::RateChange),
                    });
                    idx += 1;
                    start = run_end;
                    break;
                }
                let (cut, seam) = choose_cut(
                    start,
                    run_end.saturating_sub(p.min_tail_ms),
                    pauses,
                    fallback,
                    p,
                );
                out.push(UnitSpec {
                    idx,
                    start_ms: start,
                    end_ms: cut,
                    seam,
                });
                idx += 1;
                start = cut;
            } else {
                // Open run: the rolling limit is 2 s before the end; wait until the widest window is inside it.
                let limit = run_end.saturating_sub(p.tail_guard_ms);
                if start + p.target_ms + p.wide_window_ms > limit {
                    return out;
                }
                let (cut, seam) = choose_cut(start, limit, pauses, fallback, p);
                out.push(UnitSpec {
                    idx,
                    start_ms: start,
                    end_ms: cut,
                    seam,
                });
                idx += 1;
                start = cut;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const MIN: u64 = 60_000;

    fn part(start: u64, end: u64, rate: u32) -> Part {
        Part {
            start_ms: start,
            end_ms: end,
            sample_rate: rate,
        }
    }

    fn pause(start: u64, end: u64) -> Pause {
        Pause {
            start_ms: start,
            end_ms: end,
        }
    }

    /// Ten-minute parts at one rate from 0.
    fn parts(n: u64, rate: u32) -> Vec<Part> {
        (0..n)
            .map(|i| part(i * 10 * MIN, (i + 1) * 10 * MIN, rate))
            .collect()
    }

    #[test]
    fn the_cut_is_the_middle_of_the_longest_pause_within_a_minute_of_five_minutes() {
        let p = Params::default();
        let pauses = [
            pause(250_000, 250_800), // in the window, short
            pause(298_000, 299_000), // 1 s, near the target
            pause(330_000, 332_000), // 2 s: the longest
            pause(300_500, 300_700), // too short to count as a pause
        ];
        let u = plan(0, 0, &parts(2, 48_000), &pauses, &[], true, &p);
        assert_eq!(
            u[0],
            UnitSpec {
                idx: 0,
                start_ms: 0,
                end_ms: 331_000,
                seam: None
            }
        );
    }

    #[test]
    fn a_tie_goes_to_the_pause_closest_to_five_minutes() {
        let pauses = [pause(270_000, 271_000), pause(310_000, 311_000)];
        let u = plan(
            0,
            0,
            &parts(2, 48_000),
            &pauses,
            &[],
            true,
            &Params::default(),
        );
        assert_eq!(u[0].end_ms, 310_500);
    }

    #[test]
    fn the_window_widens_to_two_minutes_when_none_qualifies() {
        // 90 s past the target: outside +/-60 s, inside +/-120 s.
        let pauses = [pause(389_000, 390_000)];
        let u = plan(
            0,
            0,
            &parts(2, 48_000),
            &pauses,
            &[],
            true,
            &Params::default(),
        );
        assert_eq!(
            u[0],
            UnitSpec {
                idx: 0,
                start_ms: 0,
                end_ms: 389_500,
                seam: None
            }
        );
    }

    #[test]
    fn with_no_pause_the_unit_records_a_forced_cut_at_the_choppers_cut_or_the_target() {
        let p = Params::default();
        let u = plan(
            0,
            0,
            &parts(2, 48_000),
            &[pause(100_000, 101_000)],
            &[],
            true,
            &p,
        );
        assert_eq!(
            u[0],
            UnitSpec {
                idx: 0,
                start_ms: 0,
                end_ms: 300_000,
                seam: Some(Seam::ForcedCut)
            }
        );
        let u = plan(
            0,
            0,
            &parts(2, 48_000),
            &[],
            &[250_000, 340_000, 600_000],
            true,
            &p,
        );
        assert_eq!(
            (u[0].end_ms, u[0].seam),
            (340_000, Some(Seam::ForcedCut)),
            "340 s is closer than 250 s"
        );
    }

    #[test]
    fn a_short_session_is_one_unit_and_the_last_unit_ends_at_the_end_of_the_session() {
        let p = Params::default();
        let u = plan(0, 0, &[part(0, 200_000, 48_000)], &[], &[], true, &p);
        assert_eq!(
            u,
            vec![UnitSpec {
                idx: 0,
                start_ms: 0,
                end_ms: 200_000,
                seam: None
            }]
        );
        let u = plan(
            0,
            0,
            &parts(2, 48_000),
            &[pause(299_000, 301_000)],
            &[],
            true,
            &p,
        );
        assert_eq!(u.last().map(|x| x.end_ms), Some(1_200_000));
        // 5 min tail after the last cut is still within target + 1 minute: one unit.
        assert!(
            u.iter()
                .all(|x| x.end_ms - x.start_ms <= p.target_ms + p.wide_window_ms)
        );
    }

    #[test]
    fn a_rolling_pass_waits_for_the_pauses_it_needs_and_stops_two_seconds_before_the_end() {
        let p = Params::default();
        let pauses = [pause(299_000, 301_000), pause(598_000, 599_000)];
        // One closed part (10 min): the widest window around 5 min (to 7 min) is inside 598 s: one unit.
        let u = plan(0, 0, &parts(1, 48_000), &pauses, &[], false, &p);
        assert_eq!(
            u,
            vec![UnitSpec {
                idx: 0,
                start_ms: 0,
                end_ms: 300_000,
                seam: None
            }]
        );
        // Nothing past 598 s: the next unit would need to reach 300 s + 300 s + 120 s.
        assert!(u.iter().all(|x| x.end_ms <= 598_000));
        // The second part closes: the second unit (300 s to ~600 s) can be cut, still 2 s short of the end.
        let u2 = plan(1, 300_000, &parts(2, 48_000), &pauses, &[], false, &p);
        // Two more units fit before the guard: one cut at the 598.5 s pause, one forced at the target
        // (no pause near 898 s). A third would need the widest window to end past 1,198 s.
        assert_eq!(u2.len(), 2);
        assert_eq!(
            (u2[0].idx, u2[0].start_ms, u2[0].end_ms, u2[0].seam),
            (1, 300_000, 598_500, None)
        );
        assert_eq!(
            (u2[1].start_ms, u2[1].seam),
            (598_500, Some(Seam::ForcedCut))
        );
        assert!(u2.iter().all(|x| x.end_ms <= 1_198_000));
        // At Stop the rest is cut and the last unit ends at the end.
        let rest = plan(3, u2[1].end_ms, &parts(2, 48_000), &pauses, &[], true, &p);
        assert_eq!(rest.last().map(|x| x.end_ms), Some(1_200_000));
    }

    #[test]
    fn a_rate_change_is_a_boundary_and_the_unit_before_it_records_rate_change() {
        let p = Params::default();
        // Parts 1 and 2 at 48 kHz, part 3 at 44.1 kHz (a reconnect), part 4 at 44.1 kHz.
        let mut ps = parts(2, 48_000);
        ps.push(part(1_200_000, 1_800_000, 44_100));
        ps.push(part(1_800_000, 2_400_000, 44_100));
        let pauses = [pause(299_000, 301_000), pause(899_000, 901_000)];
        let u = plan(0, 0, &ps, &pauses, &[], true, &p);
        let boundary = u
            .iter()
            .find(|x| x.end_ms == 1_200_000)
            .expect("a unit ends at the rate change");
        assert_eq!(boundary.seam, Some(Seam::RateChange));
        for x in &u {
            let crosses = x.start_ms < 1_200_000 && x.end_ms > 1_200_000;
            assert!(!crosses, "{x:?} crosses the rate change");
        }
        assert_eq!(u.last().map(|x| x.end_ms), Some(2_400_000));
    }

    #[test]
    fn a_rolling_pass_closes_an_earlier_run_even_before_the_session_ends() {
        let p = Params::default();
        let mut ps = parts(1, 48_000);
        ps.push(part(600_000, 800_000, 44_100));
        let u = plan(0, 0, &ps, &[pause(299_000, 301_000)], &[], false, &p);
        // The first run (0 to 600 s) is closed by the rate change: its last unit ends exactly there.
        assert_eq!(
            u.last().map(|x| (x.end_ms, x.seam)),
            Some((600_000, Some(Seam::RateChange)))
        );
        // The open run (600 s to 800 s, 44.1 kHz) is too short to cut yet.
        assert!(u.iter().all(|x| x.end_ms <= 600_000));
    }

    #[test]
    fn planning_again_with_the_same_data_continues_and_does_not_repeat() {
        let p = Params::default();
        let pauses = [
            pause(299_000, 301_000),
            pause(599_000, 601_000),
            pause(899_000, 901_000),
        ];
        let all = plan(0, 0, &parts(3, 48_000), &pauses, &[], true, &p);
        let first = &all[..2];
        let more = plan(
            2,
            first[1].end_ms,
            &parts(3, 48_000),
            &pauses,
            &[],
            true,
            &p,
        );
        assert_eq!([first, &more[..]].concat(), all);
    }

    // ---- property tests ----

    fn arb_parts() -> impl Strategy<Value = Vec<Part>> {
        prop::collection::vec((100_000u64..700_000, prop::bool::weighted(0.25)), 1..14).prop_map(
            |v| {
                let mut out = Vec::new();
                let mut t = 0;
                let mut rate = 48_000;
                for (len, flip) in v {
                    if flip {
                        rate = if rate == 48_000 { 44_100 } else { 48_000 };
                    }
                    out.push(Part {
                        start_ms: t,
                        end_ms: t + len,
                        sample_rate: rate,
                    });
                    t += len;
                }
                out
            },
        )
    }

    fn arb_pauses(span: u64) -> impl Strategy<Value = Vec<Pause>> {
        prop::collection::vec((0..span, 100u64..4_000), 0..120).prop_map(|v| {
            v.into_iter()
                .map(|(s, l)| Pause {
                    start_ms: s,
                    end_ms: s + l,
                })
                .collect()
        })
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 200, failure_persistence: None, ..ProptestConfig::default() })]

        /// At Stop: units tile the audio exactly, none crosses a rate change, none is empty or over 7 minutes,
        /// every cut is at a pause or is marked, and the last unit ends at the end of the session.
        #[test]
        fn units_tile_the_audio_and_never_cross_a_rate_change(
            ps in arb_parts(),
            seed_pauses in arb_pauses(9_000_000),
            fallback in prop::collection::vec(0u64..9_000_000, 0..20),
        ) {
            let p = Params::default();
            let end = ps.last().map_or(0, |x| x.end_ms);
            let units = plan(0, 0, &ps, &seed_pauses, &fallback, true, &p);
            prop_assert!(!units.is_empty());
            let mut at = 0;
            for (i, u) in units.iter().enumerate() {
                prop_assert_eq!(u.idx as usize, i);
                prop_assert_eq!(u.start_ms, at, "gap or overlap before unit {}", i);
                prop_assert!(u.end_ms > u.start_ms);
                prop_assert!(u.end_ms - u.start_ms <= p.target_ms + p.wide_window_ms + p.min_tail_ms);
                at = u.end_ms;
            }
            prop_assert_eq!(at, end);
            // No unit crosses a rate change.
            for w in ps.windows(2) {
                if w[0].sample_rate != w[1].sample_rate {
                    let b = w[1].start_ms;
                    for u in &units {
                        prop_assert!(!(u.start_ms < b && u.end_ms > b), "{:?} crosses {}", u, b);
                    }
                    prop_assert!(units.iter().any(|u| u.end_ms == b && u.seam == Some(Seam::RateChange)));
                }
            }
            // Every other cut is in the middle of a real pause, or is a marked forced cut.
            for u in units.iter().filter(|u| u.end_ms != end) {
                let at_rate_change = ps.windows(2).any(|w| w[0].sample_rate != w[1].sample_rate && w[1].start_ms == u.end_ms);
                if at_rate_change {
                    prop_assert_eq!(u.seam, Some(Seam::RateChange));
                } else if u.seam.is_none() {
                    prop_assert!(
                        seed_pauses.iter().any(|z| z.len_ms() >= p.min_pause_ms && z.middle_ms() == u.end_ms),
                        "cut {} is not at a pause", u.end_ms
                    );
                } else {
                    prop_assert_eq!(u.seam, Some(Seam::ForcedCut));
                }
            }
        }

        /// While recording: nothing is cut within 2 s of the end of the last closed part (except at a rate
        /// change, which is a closed boundary), and growing the parts one by one continues without a gap or
        /// an overlap and ends the same way a single call at Stop does.
        #[test]
        fn a_rolling_pass_keeps_its_distance_and_continues_without_gaps(
            ps in arb_parts(),
            seed_pauses in arb_pauses(9_000_000),
        ) {
            let p = Params::default();
            let mut units: Vec<UnitSpec> = Vec::new();
            for n in 1..=ps.len() {
                let so_far = &ps[..n];
                let end = so_far[n - 1].end_ms;
                let next = u32::try_from(units.len()).unwrap();
                let from = units.last().map_or(0, |u| u.end_ms);
                let new = plan(next, from, so_far, &seed_pauses, &[], false, &p);
                for u in &new {
                    let at_rate_change = u.seam == Some(Seam::RateChange);
                    prop_assert!(at_rate_change || u.end_ms + p.tail_guard_ms <= end, "{:?} ends too late for end {}", u, end);
                }
                units.extend(new);
            }
            let next = u32::try_from(units.len()).unwrap();
            let from = units.last().map_or(0, |u| u.end_ms);
            units.extend(plan(next, from, &ps, &seed_pauses, &[], true, &p));
            let mut at = 0;
            for (i, u) in units.iter().enumerate() {
                prop_assert_eq!(u.idx as usize, i);
                prop_assert_eq!(u.start_ms, at);
                at = u.end_ms;
            }
            prop_assert_eq!(at, ps.last().map_or(0, |x| x.end_ms));
        }
    }
}
