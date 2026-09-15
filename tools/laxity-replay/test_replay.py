#!/usr/bin/env python3

import argparse
import pathlib
import sys
import unittest


TOOLS = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(TOOLS))

import laxity_replay
import telemetry_parse


def frame(frame_type, payload):
    return (b"LX" + bytes((2, frame_type)) + len(payload).to_bytes(2, "little")
            + telemetry_parse.crc16(payload).to_bytes(2, "little") + payload)


def header(cyccnt_hz=100):
    payload = bytearray(40)
    payload[0:4] = cyccnt_hz.to_bytes(4, "little")
    payload[4:8] = cyccnt_hz.to_bytes(4, "little")
    payload[19] = telemetry_parse.RECORD_SIZE
    return frame(0, payload)


def batch(seq, release_cyc):
    payload = telemetry_parse.RECORD_STRUCT.pack(
        seq, release_cyc, 10, 0, 0, 0, 0, 0, 0, 0, 0)
    return frame(1, payload)


def timed_stream():
    bad = bytearray(batch(99, 7))
    bad[-1] ^= 1
    return (b"boot ok\n" + header() + batch(1, 0xFFFFFFF0) + bytes(bad)
            + b"status\n" + header() + batch(2, 0x00000010) + b"tail\n")


def rebuild(data, chunks):
    return b"".join(data[start:end] for _, start, end in chunks)


class ReplayPlanTests(unittest.TestCase):
    def test_plan_preserves_every_input_byte_including_bad_crc(self):
        data = timed_stream()

        pacing, chunks, counts = laxity_replay.replay_plan(data, 1.0)

        self.assertEqual(pacing, "release_cyc pacing")
        self.assertEqual(rebuild(data, chunks), data)
        self.assertEqual(counts["records"], 2)
        self.assertEqual(counts["false_sync"], 1)
        self.assertEqual([start for _, start, _ in chunks],
                         [0] + [end for _, _, end in chunks[:-1]])

    def test_release_counter_wrap_uses_unsigned_delta(self):
        _, chunks, _ = laxity_replay.replay_plan(timed_stream(), 1.0)

        self.assertAlmostEqual(chunks[1][0], 0.32)

    def test_rate_scales_target_cycle_deadlines(self):
        _, normal, _ = laxity_replay.replay_plan(timed_stream(), 1.0)
        _, fast, _ = laxity_replay.replay_plan(timed_stream(), 4.0)

        self.assertAlmostEqual(fast[1][0], normal[1][0] / 4.0)

    def test_fixed_pacing_is_the_byte_preserving_fallback(self):
        data = b"status without telemetry\n" * 30

        pacing, chunks, counts = laxity_replay.replay_plan(data, 2.0)

        self.assertEqual(pacing, "fixed byte pacing")
        self.assertEqual(rebuild(data, chunks), data)
        self.assertEqual(counts["records"], 0)
        self.assertEqual(chunks[0][0], 0.0)
        self.assertAlmostEqual(chunks[1][0],
                               laxity_replay.FALLBACK_INTERVAL_SECONDS / 2.0)

    def test_fixed_pacing_covers_unusable_cycle_timestamps(self):
        streams = (
            header() + batch(1, 10),
            header(0) + batch(1, 10) + batch(2, 20),
            header(100) + batch(1, 10) + header(200) + batch(2, 20),
        )

        for data in streams:
            pacing, chunks, _ = laxity_replay.replay_plan(data, 1.0)
            self.assertEqual(pacing, "fixed byte pacing")
            self.assertEqual(rebuild(data, chunks), data)

    def test_batch_observer_does_not_change_parser_results(self):
        data = timed_stream()
        seen = []

        expected = telemetry_parse.parse(data)
        actual = telemetry_parse.parse(data, on_batch=lambda *batch: seen.append(batch))

        self.assertEqual(actual, expected)
        self.assertEqual(len(seen), 2)
        self.assertEqual(seen[-1][2][-1]["seq"], 2)

    def test_rate_and_destination_validation_fail_closed(self):
        for value in ("0", "-1", "nan", "inf", "word"):
            with self.assertRaises(argparse.ArgumentTypeError):
                laxity_replay.positive_rate(value)
        self.assertEqual(laxity_replay.udp_endpoint("127.0.0.1:50505"),
                         ("127.0.0.1", 50505))
        for value in ("50505", ":50505", "host:0", "host:word"):
            with self.assertRaises(argparse.ArgumentTypeError):
                laxity_replay.udp_endpoint(value)


if __name__ == "__main__":
    unittest.main()
