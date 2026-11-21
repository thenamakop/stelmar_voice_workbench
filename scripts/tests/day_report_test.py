"""Tests of the estimate check in scripts/gates/day_report.py.

Run: python -I scripts/tests/day_report_test.py

The day simulation of 19-20 Nov 2026 failed this check for Parallel II (+650%) because the scheduler log has a resolution
of one minute: Panel's start line and Parallel II's stop line were both stamped 16:01, and Panel, which had not stopped
(it had not even started), was counted as a transcript outstanding at that Stop.
"""

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "gates"))
import day_report as dr  # noqa: E402

BASE = "D:\\run\\rec\\2026-11-19\\"


def qlog(lines):
    clock = dr.Clock()
    return [(clock.parse(t), x) for t, x in lines]


# The order of the lines of the real run, shortened (the times are the log's, to the minute).
REAL = qlog(
    [
        ("10:00", f'DAY: started "Inaugural" in {BASE}main-hall__inaugural'),
        ("11:30", 'DAY: stopped "Inaugural"'),
        ("11:30", "main-hall__inaugural: stopped. All transcripts ready by 12:04 IST (basis)"),
        ("12:00", f'DAY: started "Parallel I" in {BASE}hall-a__parallel-i'),
        ("13:30", 'DAY: stopped "Parallel I"'),
        ("13:30", "hall-a__parallel-i: stopped. All transcripts ready by 14:18 IST (basis)"),
        ("14:30", f'DAY: started "Parallel II" in {BASE}hall-b__parallel-ii'),
        ("16:01", 'DAY: stopped "Parallel II"'),
        ("16:01", "hall-b__parallel-ii: stopped. All transcripts ready by 16:17 IST (basis)"),
        ("16:01", f'DAY: started "Panel" in {BASE}main-hall__panel'),
        ("17:32", 'DAY: stopped "Panel"'),
        ("17:32", "main-hall__panel: stopped. All transcripts ready by 18:02 IST (basis)"),
    ]
)
NAMES = ["main-hall__inaugural", "hall-a__parallel-i", "hall-b__parallel-ii", "main-hall__panel"]


def line_of(prefix):
    return next(k for k, (_t, x) in enumerate(REAL) if x.startswith(prefix))


class OutstandingAtStop(unittest.TestCase):
    def setUp(self):
        self.order = dr.stop_order_of(REAL)

    def test_the_stop_order_follows_the_lines(self):
        self.assertEqual(
            sorted(self.order, key=self.order.get),
            ["main-hall__inaugural", "hall-a__parallel-i", "hall-b__parallel-ii", "main-hall__panel"],
        )

    def test_a_session_started_in_the_same_minute_as_the_stop_is_not_outstanding(self):
        k = line_of("hall-b__parallel-ii: stopped")
        t = REAL[k][0]
        # Panel's post-1 completes two hours later; Parallel I's at 14:14.
        complete = {
            ("main-hall__inaugural", "post-1"): t - 5000,
            ("main-hall__panel", "post-1"): t + 7200,
            ("hall-a__parallel-i", "post-1"): t - 1000,
        }
        got = dr.outstanding_at_stop(self.order, k, "hall-b__parallel-ii", NAMES, complete, t)
        self.assertEqual(got, ["hall-b__parallel-ii"])

    def test_a_session_that_stopped_earlier_and_is_still_transcribed_is_outstanding(self):
        k = line_of("hall-b__parallel-ii: stopped")
        t = REAL[k][0]
        complete = {("main-hall__inaugural", "post-1"): t - 5000, ("hall-a__parallel-i", "post-1"): t + 600}
        got = dr.outstanding_at_stop(self.order, k, "hall-b__parallel-ii", NAMES, complete, t)
        self.assertEqual(got, ["hall-a__parallel-i", "hall-b__parallel-ii"])

    def test_a_session_whose_transcript_is_complete_by_the_stop_is_not_outstanding(self):
        k = line_of("hall-b__parallel-ii: stopped")
        t = REAL[k][0]
        complete = {("main-hall__inaugural", "post-1"): t - 5000, ("hall-a__parallel-i", "post-1"): t - 1}
        got = dr.outstanding_at_stop(self.order, k, "hall-b__parallel-ii", NAMES, complete, t)
        self.assertEqual(got, ["hall-b__parallel-ii"])

    def test_a_session_with_no_stop_line_is_never_outstanding(self):
        k = line_of("hall-a__parallel-i: stopped")
        got = dr.outstanding_at_stop({"main-hall__inaugural": 2}, k, "hall-a__parallel-i", NAMES, {}, REAL[k][0])
        self.assertEqual(got, ["main-hall__inaugural", "hall-a__parallel-i"])


class EngineDeathCount(unittest.TestCase):
    def test_counts_supervisor_death_lines_not_the_restarts_of_done_lines(self):
        log = [
            (1.0, "hall-a__s: unit 1 of post-1 job 1 on cpu: Died { exit: Some(1), restart: 1, committed_until_ms: 0 }"),
            (2.0, "hall-a__s: unit 1 of post-1 job 1 on cpu: Died { exit: Some(1), restart: 2, committed_until_ms: 0 }"),
            (3.0, "hall-a__s: unit 2 of post-1 job 1 on gpu: Died { exit: Some(1), restart: 1, committed_until_ms: 1475350 }"),
            (4.0, "hall-a__s: unit 1 of post-1 done in 3975.0 s on cpu (1 restart(s), engine peak 1450 MiB against limit 2807 MiB)"),
            (5.0, "hall-a__s: unit 3 of post-1 job 1 on gpu: engine warning chunk_rescued: chunk 2: rescued by step a"),
        ]
        self.assertEqual(dr.engine_deaths(log), 3)


class LeadIn(unittest.TestCase):
    def test_the_lead_in_is_the_recorders_first_audio_to_the_players_first_frame(self):
        import json
        import tempfile

        with tempfile.TemporaryDirectory() as d:
            run = Path(d)
            for i, (rec_at, play_at) in enumerate(
                [("2026-11-20T15:07:03.138+05:30", "2026-11-20T15:07:04.838+05:30"),
                 ("2026-11-20T19:37:03.138+05:30", "2026-11-20T19:38:32.138+05:30")], start=1):
                sess = run / "rec" / "2026-11-20" / f"hall-{i}__s"
                sess.mkdir(parents=True)
                (sess / "session.json").write_text(json.dumps({"events": [
                    {"kind": "opened", "at": rec_at}, {"kind": "audio_started", "at": rec_at}]}), encoding="utf-8")
                (run / f"play-{i}.json").write_text(json.dumps({"started_at": play_at}), encoding="utf-8")
            got = dr.lead_in_seconds(run)
            self.assertEqual([n for n, _ in got], ["hall-1__s", "hall-2__s"])
            self.assertAlmostEqual(got[0][1], 1.7, places=2)
            self.assertAlmostEqual(got[1][1], 89.0, places=2)


class LeadInSkipsImportedCopies(unittest.TestCase):
    def test_a_session_nobody_plays_does_not_shift_the_pairing(self):
        import json
        import tempfile

        with tempfile.TemporaryDirectory() as d:
            run = Path(d)
            # The kill test: a played session, then an imported copy whose session.json carries the copied events.
            for name, rec_at in [("hall-a__kill", "2026-11-20T12:14:00+05:30"), ("hall-a__kill-copy", "2026-11-20T12:14:00+05:30"), ("hall-b__other", "2026-11-20T12:20:00+05:30")]:
                sess = run / "rec" / "2026-11-20" / name
                sess.mkdir(parents=True)
                (sess / "session.json").write_text(json.dumps({"events": [{"kind": "audio_started", "at": rec_at}]}), encoding="utf-8")
            (run / "play-1.json").write_text(json.dumps({"started_at": "2026-11-20T12:14:02+05:30"}), encoding="utf-8")
            (run / "play-2.json").write_text(json.dumps({"started_at": "2026-11-20T12:20:02+05:30"}), encoding="utf-8")
            got = dr.lead_in_seconds(run, frozenset({"hall-a__kill-copy"}))
            self.assertEqual([n for n, _ in got], ["hall-a__kill", "hall-b__other"])
            self.assertAlmostEqual(got[0][1], 2.0, places=2)
            self.assertAlmostEqual(got[1][1], 2.0, places=2)


class EstimateBar(unittest.TestCase):
    """Owner, 21 Nov 2026: at most 10% too optimistic (finished later than estimated), at most 30% too pessimistic."""

    def test_late_by_up_to_ten_percent_passes_and_more_fails(self):
        self.assertTrue(dr.estimate_within_bar(0.0))
        self.assertTrue(dr.estimate_within_bar(0.10))
        self.assertFalse(dr.estimate_within_bar(0.101))
        self.assertFalse(dr.estimate_within_bar(0.65))

    def test_early_by_up_to_thirty_percent_passes_and_more_fails(self):
        self.assertTrue(dr.estimate_within_bar(-0.25))  # Parallel II of 21 Nov 2026: -25%, passes the new early bar
        self.assertTrue(dr.estimate_within_bar(-0.30))
        self.assertFalse(dr.estimate_within_bar(-0.301))
        self.assertFalse(dr.estimate_within_bar(-0.5))

    def test_the_old_symmetric_bar_would_have_failed_parallel_ii(self):
        # The recorded run keeps its numbers: -25.0% was a FAIL against the old +-10% and is a PASS against this bar.
        self.assertFalse(abs(-0.25) <= 0.10)
        self.assertTrue(dr.estimate_within_bar(-0.25))

    def test_the_estimate_checks_are_information_in_the_kill_test_and_the_rehearsal_only(self):
        self.assertEqual(set(dr.ESTIMATE_INFO_NOTE), {"rehearse", "kill"})
        self.assertNotIn("day", dr.ESTIMATE_INFO_NOTE)


if __name__ == "__main__":
    unittest.main(verbosity=2)
