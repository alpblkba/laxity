#!/usr/bin/env python3
"""read a framed Laxity telemetry stream and report what is in it.

the format is defined by include/qos/telemetry.h. the firmware shares one UART between framed
telemetry and a human readable status line, so a capture is a mixed stream and this parser
recovers by scanning for the magic and validating the CRC before accepting anything.

stdlib only, and no state carried between runs, so it can be pointed at a raw capture that
was taken months earlier.
"""

import argparse
import struct
import sys

MAGIC = b"LX"
# version 1 carried a 20 byte fixed header and no placement entries. version 2 added the null
# probe overhead and the placement table. both are accepted, since results/raw is append only
# and a capture has to stay readable after the format moves.
VERSIONS = (1, 2)
HEADER_FIXED = {1: 20, 2: 40}
FRAME_OVERHEAD = 8
PLACEMENT_SIZE = 20
RECORD_SIZE = 32

FLAG_CYCCNT_WRAP = 1 << 0

# aggressor_idx packs the competing master's region in the low byte and a footprint index in the
# high byte. the footprint table is a property of the run rather than of the format, so it is
# named here and has to move with the firmware if the levels change.
FOOTPRINT_BYTES = (1024, 4096, 8192, 16384)


def aggressor_of(rec):
    idx = rec["aggressor_idx"]
    region, foot = idx & 0xFF, (idx >> 8) & 0xFF
    if region == 0:
        return "off"
    size = FOOTPRINT_BYTES[foot] if foot < len(FOOTPRINT_BYTES) else foot
    return "r%d-%s" % (region, "%dK" % (size // 1024) if size >= 1024 else str(size))

RECORD_FIELDS = (
    "seq", "release_cyc", "exec_cyc", "cpu_cyc", "stall_cyc",
    "model_id", "region_id", "flags", "aggressor_idx", "padding", "reserved",
)
RECORD_STRUCT = struct.Struct("<5I H B B H H I")
PLACEMENT_STRUCT = struct.Struct("<B B H I I 8s")
PLACEMENT_CONTROL = 1 << 0
PLACEMENT_ALT_ADDR = 1 << 1


def median(values):
    """the firmware's rule, so a host figure and a target figure mean the same thing"""
    return sorted(values)[len(values) // 2]


def p99(values):
    return sorted(values)[(len(values) * 99) // 100]


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
    placements = {}
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
        ok = (version in VERSIONS and ftype in (0, 1) and body + plen <= end
              and crc16(data[body:body + plen]) == want)
        if not ok:
            pos += 1
            out["skipped_bytes"] += 1
            out["false_sync"] += 1
            continue

        out["frames"] += 1
        if ftype == 0:
            fixed = HEADER_FIXED[version]
            n_regions = data[body + 17] if plen > 17 else 0
            if plen != fixed + n_regions * PLACEMENT_SIZE:
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
                "n_regions": n_regions,
                "n_models": data[body + 18],
                "record_size": data[body + 19],
            }
            if version >= 2:
                rm, rp, pm, pp = struct.unpack_from("<4I", data, body + 20)
                meta.update({
                    "null_read_median": rm, "null_read_p99": rp,
                    "null_push_median": pm, "null_push_p99": pp,
                    "null_n": struct.unpack_from("<H", data, body + 36)[0],
                })
            placements.clear()
            for i in range(n_regions):
                pid, pflags, rel, addr, size, raw = PLACEMENT_STRUCT.unpack_from(
                    data, body + fixed + i * PLACEMENT_SIZE)
                placements[pid] = {
                    "name": raw.split(b"\x00")[0].decode("ascii", "replace"),
                    "control": bool(pflags & PLACEMENT_CONTROL),
                    "alt_addr": bool(pflags & PLACEMENT_ALT_ADDR),
                    "rel_cost": rel, "arena_addr": addr, "arena_size": size,
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
    return out, meta, placements, records


def region_stats(placements, records):
    """group exec_cyc by placement label, so the control alias stays a separate column"""
    stats = {}
    for rec in records:
        stats.setdefault(rec["region_id"], []).append(rec["exec_cyc"])
    out = {}
    for rid, values in sorted(stats.items()):
        info = placements.get(rid, {})
        label = info.get("name") or ("id%d" % rid)
        out[label] = {
            "id": rid,
            "n": len(values),
            "min": min(values),
            "median": median(values),
            "p99": p99(values),
            "max": max(values),
            "control": 1 if info.get("control") else 0,
            "alt_addr": 1 if info.get("alt_addr") else 0,
            "arena_addr": "0x%08x" % info["arena_addr"] if "arena_addr" in info else "",
        }
    return out


def cell_stats(placements, records):
    """one row per arena placement crossed with the aggressor that was running

    the transfer count travels with the cell, because a cell whose aggressor never moved is a
    broken measurement and has to be refused rather than reported as a null result.
    """
    cells = {}
    for rec in records:
        info = placements.get(rec["region_id"], {})
        arena = info.get("name") or ("id%d" % rec["region_id"])
        cells.setdefault((arena, aggressor_of(rec)), []).append(rec)
    out = {}
    for (arena, aggr), recs in sorted(cells.items()):
        values = [r["exec_cyc"] for r in recs]
        counts = [r["reserved"] for r in recs]
        out[(arena, aggr)] = {
            "n": len(values),
            "median": median(values),
            "p99": p99(values),
            "min": min(values),
            "max": max(values),
            "xfer_first": counts[0],
            "xfer_last": counts[-1],
            "xfer_advanced": 1 if counts[-1] > counts[0] else 0,
        }
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("path", help="raw capture, normally results/raw/<run>/telemetry.bin")
    ap.add_argument("--csv", metavar="PATH", help="write the records as CSV as well")
    args = ap.parse_args()

    with open(args.path, "rb") as handle:
        out, meta, placements, records = parse(handle.read())

    for key in sorted(meta):
        value = meta[key]
        print("%s=%s" % (key, int(value) if isinstance(value, bool) else value))
    for key in sorted(out):
        print("%s=%s" % (key, out[key]))
    for label, st in region_stats(placements, records).items():
        for key in ("id", "n", "min", "median", "p99", "max", "control", "alt_addr", "arena_addr"):
            print("region.%s.%s=%s" % (label, key, st[key]))
    for (arena, aggr), st in cell_stats(placements, records).items():
        for key in ("n", "min", "median", "p99", "max", "xfer_advanced"):
            print("cell.%s.%s.%s=%s" % (arena, aggr, key, st[key]))

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
