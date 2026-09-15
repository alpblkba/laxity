#!/usr/bin/env python3
"""replay an unchanged Laxity capture over UDP with target-cycle pacing."""

import argparse
import math
import pathlib
import socket
import sys
import time

import telemetry_parse


FALLBACK_CHUNK_BYTES = 320
FALLBACK_INTERVAL_SECONDS = 0.033
UDP_PAYLOAD_BYTES = 1400


def positive_rate(value):
    try:
        rate = float(value)
    except ValueError as error:
        raise argparse.ArgumentTypeError("rate must be a positive number") from error
    if not math.isfinite(rate) or rate <= 0.0:
        raise argparse.ArgumentTypeError("rate must be a positive finite number")
    return rate


def udp_endpoint(value):
    host, separator, raw_port = value.rpartition(":")
    if not separator or not host:
        raise argparse.ArgumentTypeError("UDP destination must be HOST:PORT")
    try:
        port = int(raw_port)
    except ValueError as error:
        raise argparse.ArgumentTypeError("UDP port must be an integer") from error
    if not 1 <= port <= 65535:
        raise argparse.ArgumentTypeError("UDP port must be between 1 and 65535")
    return host, port


def replay_plan(data, rate):
    points = []

    def observe_batch(end, cyccnt_hz, records):
        points.append((end, records[-1]["release_cyc"], cyccnt_hz))

    counts, _, _, _ = telemetry_parse.parse(data, on_batch=observe_batch)
    clocks = {point[2] for point in points}
    if len(points) < 2 or len(clocks) != 1 or 0 in clocks:
        chunks = []
        for index, start in enumerate(range(0, len(data), FALLBACK_CHUNK_BYTES)):
            end = min(start + FALLBACK_CHUNK_BYTES, len(data))
            at = index * FALLBACK_INTERVAL_SECONDS / rate
            chunks.append((at, start, end))
        return "fixed byte pacing", chunks, counts

    chunks = [(0.0, 0, points[0][0])]
    elapsed = 0.0
    previous_release = points[0][1]
    previous_end = points[0][0]
    for end, release_cyc, cyccnt_hz in points[1:]:
        delta = (release_cyc - previous_release) & 0xFFFFFFFF
        elapsed += delta / cyccnt_hz / rate
        chunks.append((elapsed, previous_end, end))
        previous_release = release_cyc
        previous_end = end
    if previous_end < len(data):
        chunks.append((elapsed, previous_end, len(data)))
    return "release_cyc pacing", chunks, counts


def send_plan(sock, destination, data, chunks):
    started = time.monotonic()
    sent = 0
    for at, start, end in chunks:
        delay = started + at - time.monotonic()
        if delay > 0.0:
            time.sleep(delay)
        for offset in range(start, end, UDP_PAYLOAD_BYTES):
            packet = data[offset:min(offset + UDP_PAYLOAD_BYTES, end)]
            count = sock.sendto(packet, destination)
            if count != len(packet):
                raise OSError("UDP socket accepted only part of a datagram")
            sent += count
    return sent


def parse_args(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("path", type=pathlib.Path, help="raw telemetry capture")
    parser.add_argument("--udp", required=True, type=udp_endpoint, metavar="HOST:PORT",
                        help="UDP destination, for example 127.0.0.1:50505")
    parser.add_argument("--rate", default=1.0, type=positive_rate,
                        help="playback speed multiplier, default 1.0")
    parser.add_argument("--loop", action="store_true", help="repeat until interrupted")
    return parser.parse_args(argv)


def main(argv=None):
    args = parse_args(argv)
    try:
        data = args.path.read_bytes()
    except OSError as error:
        print("cannot read %s: %s" % (args.path, error), file=sys.stderr)
        return 1
    if not data:
        print("cannot replay an empty capture", file=sys.stderr)
        return 1

    pacing, chunks, counts = replay_plan(data, args.rate)
    print("%s: %d bytes, %d records, %s at %gx" % (
        args.path, len(data), counts["records"], pacing, args.rate))

    try:
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
            while True:
                send_plan(sock, args.udp, data, chunks)
                if not args.loop:
                    break
    except KeyboardInterrupt:
        return 130
    except OSError as error:
        print("UDP replay failed: %s" % error, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
