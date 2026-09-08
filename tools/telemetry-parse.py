#!/usr/bin/env python3
"""read a framed Laxity telemetry stream and report what is in it.

the format is self-docs/docs/TELEMETRY.md. the firmware shares one UART between framed
telemetry and a human readable status line, so a capture is a mixed stream and this parser
recovers by scanning for the magic and validating the CRC before accepting anything.

stdlib only, and no state carried between runs, so it can be pointed at a raw capture that
was taken months earlier.
"""

import argparse
import struct
import sys

MAGIC = b"LX"
VERSION = 1
FRAME_OVERHEAD = 8
HEADER_PAYLOAD = 20
RECORD_SIZE = 32

FLAG_CYCCNT_WRAP = 1 << 0

RECORD_FIELDS = (
    "seq", "release_cyc", "exec_cyc", "cpu_cyc", "stall_cyc",
    "model_id", "region_id", "flags", "aggressor_idx", "padding", "reserved",
)
RECORD_STRUCT = struct.Struct("<5I H B B H H I")


def crc16(data):
    """CRC-16/CCITT-FALSE, poly 0x1021, init 0xFFFF, no reflection, no final xor."""
    crc = 0xFFFF
    for byte in data:
        crc ^= byte << 8
        for _ in range(8):
            crc = ((crc << 1) ^ 0x1021) & 0xFFFF if crc & 0x8000 else (crc << 1) & 0xFFFF
    return crc


def parse(data):
    out = {
        "frames": 0, "header_frames": 0, "batch_frames": 0,
        "records": 0, "dropped": 0, "gaps": 0,
        "false_sync": 0, "skipped_bytes": 0, "records_before_header": 0,
        "wrapped": 0,
    }
    meta = {}
    records = []
    seen_header = False
    expect_seq = None
    pos = 0
    end = len(data)

    while pos + FRAME_OVERHEAD <= end:
        if data[pos:pos + 2] != MAGIC:
            pos += 1
            out["skipped_bytes"] += 1
            continue

        version = data[pos + 2]
        ftype = data[pos + 3]
        plen, want = struct.unpack_from("<HH", data, pos + 4)
        body = pos + FRAME_OVERHEAD

        # a candidate is rejected on the byte after its magic rather than on the length it
        # claims, since "LX" occurs in ordinary text and a claimed length can walk the reader
        # straight over a real frame that starts inside the bytes it would have skipped.
        ok = (version == VERSION and ftype in (0, 1) and body + plen <= end
              and crc16(data[body:body + plen]) == want)
        if not ok:
            pos += 1
            out["skipped_bytes"] += 1
            out["false_sync"] += 1
            continue

        out["frames"] += 1
        if ftype == 0:
            if plen != HEADER_PAYLOAD:
                pos += 1
                out["skipped_bytes"] += 1
                out["false_sync"] += 1
                continue
            clock_hz, cyccnt_hz, seq_next, dropped = struct.unpack_from("<4I", data, body)
            meta = {
                "version": version,
                "clock_hz": clock_hz,
                "cyccnt_hz": cyccnt_hz,
                "seq_next": seq_next,
                "stall_available": (data[body + 16] & 1) != 0,
                "stall_populated": (data[body + 16] & 2) != 0,
                "n_regions": data[body + 17],
                "n_models": data[body + 18],
                "record_size": data[body + 19],
            }
            out["dropped"] = max(out["dropped"], dropped)
            out["header_frames"] += 1
            seen_header = True
        else:
            count = plen // RECORD_SIZE
            if plen % RECORD_SIZE:
                pos += 1
                out["skipped_bytes"] += 1
                out["false_sync"] += 1
                continue
            out["batch_frames"] += 1
            if not seen_header:
                # the document says a header frame opens every capture, so a batch that
                # arrives before one is reported and discarded rather than parsed against
                # metadata this stream never carried.
                out["records_before_header"] += count
            else:
                for i in range(count):
                    values = RECORD_STRUCT.unpack_from(data, body + i * RECORD_SIZE)
                    rec = dict(zip(RECORD_FIELDS, values))
                    if expect_seq is not None and rec["seq"] != expect_seq:
                        out["gaps"] += 1
                    expect_seq = rec["seq"] + 1
                    if rec["flags"] & FLAG_CYCCNT_WRAP:
                        out["wrapped"] += 1
                    records.append(rec)
                out["records"] += count

        pos = body + plen

    out["skipped_bytes"] += end - pos
    return out, meta, records


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("path", help="raw capture, normally results/raw/<run>/telemetry.bin")
    ap.add_argument("--csv", metavar="PATH", help="write the records as CSV as well")
    args = ap.parse_args()

    with open(args.path, "rb") as handle:
        out, meta, records = parse(handle.read())

    for key in sorted(meta):
        value = meta[key]
        print("%s=%s" % (key, int(value) if isinstance(value, bool) else value))
    for key in sorted(out):
        print("%s=%s" % (key, out[key]))

    if args.csv:
        with open(args.csv, "w") as handle:
            handle.write(",".join(RECORD_FIELDS) + "\n")
            for rec in records:
                handle.write(",".join(str(rec[f]) for f in RECORD_FIELDS) + "\n")

    if not meta:
        print("no valid header frame in %s" % args.path, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
