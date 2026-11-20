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


if __name__ == "__main__":
    unittest.main(verbosity=2)
