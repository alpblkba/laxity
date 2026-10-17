#!/usr/bin/env python3
"""the folded half count against the halves that fit in the elapsed cycles, on captures whose counts are known.

a synthetic record list rather than a capture directory, since what is under test is the arithmetic and not the reader that feeds it. the reader has its own check in run.sh, over a stream the C side produced.
"""
import pathlib
import sys
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
import analyse

# one half buffer at the rate the part really runs, in cycles at the clock the header declares.
HZ = 160_000_000
PERIOD = HZ * analyse.AUDIO_HALF_SAMPLES * analyse.AUDIO_DECIMATION / analyse.AUDIO_CLOCK_HZ


def records(count, halves_per_step, period=PERIOD):
    """records one period apart, each folding the given number of halves."""
    out, folded = [], 0
    for i in range(count):
        out.append({"release_cyc": int(i * period) & 0xFFFFFFFF, "audio_halves": folded & 0xFFFF})
        folded += halves_per_step
    return out


class AudioLossTests(unittest.TestCase):
    def test_the_period_is_the_fraction_and_not_the_rounded_rate(self):
        # 512 samples at 2857000/176 Hz is 31.5407 ms, which the declared deadline matches exactly.
        self.assertEqual(round(PERIOD), 5_046_524)

    def test_a_capture_that_folded_every_half_reports_no_loss(self):
        loss = analyse.audio_loss({"version": 3, "cyccnt_hz": HZ}, records(40, 1))
        self.assertEqual(loss["folded"], 39)
        self.assertAlmostEqual(loss["expected"], 39.0, places=3)
        self.assertAlmostEqual(loss["lost"], 0.0, places=3)

    def test_a_capture_that_folded_one_half_in_three_reports_the_rest_lost(self):
        # thirty steps of three periods each, folding one half per step, so two in three are lost.
        rows = records(31, 1, period=PERIOD * 3)
        loss = analyse.audio_loss({"version": 3, "cyccnt_hz": HZ}, rows)
        self.assertEqual(loss["folded"], 30)
        self.assertAlmostEqual(loss["expected"], 90.0, places=2)
        self.assertAlmostEqual(loss["lost"], 60.0, places=2)

    def test_the_first_record_is_outside_both_totals(self):
        # two records one period apart carry one interval, so one half is expected and not two.
        loss = analyse.audio_loss({"version": 3, "cyccnt_hz": HZ}, records(2, 1))
        self.assertEqual(loss["folded"], 1)
        self.assertAlmostEqual(loss["expected"], 1.0, places=3)

    def test_a_count_that_never_advances_is_named_rather_than_reported_as_total_loss(self):
        loss = analyse.audio_loss({"version": 3, "cyccnt_hz": HZ}, records(40, 0))
        self.assertEqual(loss["folded"], 0)
        self.assertIsNone(loss["lost"])

    def test_a_capture_older_than_the_field_has_no_figure(self):
        self.assertIsNone(analyse.audio_loss({"version": 2, "cyccnt_hz": HZ}, records(40, 1)))

    def test_both_counters_wrap(self):
        # the cycle counter at 32 bits and the half count at 16, which a 75 second capture crosses.
        rows = [
            {"release_cyc": 0xFFFFFFFF - int(PERIOD) + 1, "audio_halves": 0xFFFF},
            {"release_cyc": 0, "audio_halves": 0},
        ]
        loss = analyse.audio_loss({"version": 3, "cyccnt_hz": HZ}, rows)
        self.assertEqual(loss["folded"], 1)
        self.assertAlmostEqual(loss["expected"], 1.0, places=3)


if __name__ == "__main__":
    unittest.main(verbosity=2)
