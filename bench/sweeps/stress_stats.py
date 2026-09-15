#!/usr/bin/env python3
"""median victim cycles per sweep point, read out of the captures.

the framed stream is parsed by tools/telemetry_parse.py rather than re-read here, so there is one
implementation of the format. this only groups the records it returns.

the sweep a capture belongs to is not in the records. the wire format was not changed for this
experiment, so it is in stress.txt beside the capture and in the directory name.

    ./bench/sweeps/stress_stats.py results/raw/*-stress-*
"""

import pathlib
import statistics
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[2] / "tools"))
import telemetry_parse

# the knob values each point index stands for, in the same order as laxity_sweeps in the firmware.
# point 0 is the aggressor off in every sweep. this table is a second copy of what the firmware
# holds and it is written out on the status line every second, which is how the two are checked
# against each other rather than assumed to agree.
SWEEPS = {
    "bw":     ["off", "50kHz 12.8MB/s", "100kHz 25.6MB/s", "200kHz 51.2MB/s",
               "400kHz 102MB/s", "800kHz 205MB/s"],
    "xact":   ["off", "w4 12.8Mxact/s", "w2 25.6Mxact/s", "w1 51.2Mxact/s"],
    "chan":   ["off", "1ch x512B", "2ch x256B", "4ch x128B"],
    "stride": ["off", "stride 0", "stride 4", "stride 12", "stride 28", "stride 60", "stride 124"],
}

# bytes per second and transactions per second the firmware asks for at each point, so the report
# can say which of the two the cost follows instead of only that it moved.
RATES = {
    "bw":     [(0, 0), (12800000, 3200000), (25600000, 6400000), (51200000, 12800000),
               (102400000, 25600000), (204800000, 51200000)],
    "xact":   [(0, 0), (51200000, 12800000), (51200000, 25600000), (51200000, 51200000)],
    "chan":   [(0, 0), (102400000, 25600000), (102400000, 25600000), (102400000, 25600000)],
    "stride": [(0, 0)] + [(51200000, 12800000)] * 6,
}


def read_kv(path):
    out = {}
    if path.exists():
        for line in path.read_text().splitlines():
            if "=" in line:
                k, _, v = line.partition("=")
                out[k.strip()] = v.strip()
    return out


def entry(run_dir):
    meta_kv = read_kv(run_dir / "stress.txt")
    sweep = meta_kv.get("sweep")
    if sweep not in SWEEPS:
        return None

    _, _, _, records = telemetry_parse.parse((run_dir / "telemetry.bin").read_bytes())

    # the point index is the high byte of aggressor_idx and the aggressor region is the low byte,
    # which is the layout the firmware writes and the header documents.
    groups, xfer = {}, {}
    for rec in records:
        point = (rec["aggressor_idx"] >> 8) & 0xFF
        groups.setdefault(point, []).append(rec["exec_cyc"])
        xfer.setdefault(point, []).append(rec["reserved"])

    rows = []
    labels = SWEEPS[sweep]
    base = statistics.median(groups[0]) if 0 in groups and groups[0] else None
    for point in sorted(groups):
        cycles = sorted(groups[point])
        if not cycles:
            continue
        counts = xfer[point]
        # a channel that failed to start is indistinguishable from one that cost nothing, so a
        # point whose transfer count never moved is marked rather than reported as a null result.
        advanced = (point == 0) or (max(counts) > min(counts))
        rows.append({
            "point": point,
            "label": labels[point] if point < len(labels) else "point %d" % point,
            "n": len(cycles),
            "median": statistics.median(cycles),
            "p99": cycles[min(len(cycles) - 1, int(0.99 * len(cycles)))],
            "delta": (statistics.median(cycles) - base) if base is not None else 0,
            "advanced": advanced,
            "bytes_s": RATES[sweep][point][0] if point < len(RATES[sweep]) else 0,
            "xact_s": RATES[sweep][point][1] if point < len(RATES[sweep]) else 0,
        })
    return {"sweep": sweep, "region": meta_kv.get("aggressor_region", "?"),
            "dir": run_dir.name, "rows": rows}


def main():
    if len(sys.argv) < 2:
        raise SystemExit(__doc__)
    for arg in sys.argv[1:]:
        out = entry(pathlib.Path(arg))
        if out is None:
            continue
        print("\n%s sweep, aggressor in %s   (%s)" % (out["sweep"], out["region"], out["dir"]))
        print("  %-18s %6s %9s %9s %9s %12s %12s" %
              ("point", "n", "median", "p99", "delta", "bytes/s", "xact/s"))
        for r in out["rows"]:
            flag = "" if r["advanced"] else "  NO TRANSFER"
            print("  %-18s %6d %9d %9d %+9d %12d %12d%s" %
                  (r["label"], r["n"], r["median"], r["p99"], r["delta"],
                   r["bytes_s"], r["xact_s"], flag))


if __name__ == "__main__":
    main()
