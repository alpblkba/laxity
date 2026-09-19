#!/usr/bin/env python3
"""read the interference campaign captures and print the tables the report is written from.

    ./bench/sweeps/campaign_report.py [results/raw/...]

with no argument it takes every capture whose stress.txt names this campaign. the framed stream is
parsed by tools/telemetry_parse.py rather than re-read here, so there is one implementation of the
format, and bench/sweeps/stress_stats.py stays the per point view of a single capture.

the coefficient reported for a cell is a slope and not a single measurement. cost per aggressor
transaction is the least squares slope of the victim's median delta against the number of aggressor
transactions the window contains, fitted through the origin over the points below the saturation
the earlier sweeps found. transactions in a window are the requested transaction rate times the
uncontended window, so the normaliser does not move with the effect being measured.
"""

import pathlib
import statistics
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[2] / "tools"))
import telemetry_parse

# both rounds are read by one script, since the second one re-measures cells the first one
# reported and the two tables only mean something side by side.
CAMPAIGNS = ("interference-2026-09-15", "mechanism-2026-09-16", "arena-or-stack-2026-09-16",
             "closing-2026-09-19", "wifi-2026-09-19", "paired-2026-09-19",
             "final-2026-09-19")

# the cells every capture of the closing campaign is read against, measured at the start and again
# at the end. the expected values are the decomposition's, and the gate is 0.010 cycles per
# aggressor transaction, which is one and a third of that campaign's worst fit residual expressed
# in the same unit.
GATE = {
    1: ("arena SRAM1, stack SRAM1, aggressor SRAM1", 0.238),
    2: ("arena SRAM1, stack SRAM2, aggressor SRAM1", 0.139),
    3: ("arena SRAM1, stack SRAM2, aggressor SRAM2", 0.108),
}
# the expected values belong to an image, so a campaign that runs on a different one carries its
# own. comparing a gate cell against a value measured on another binary is not a drift check.
GATE_BY_CAMPAIGN = {
    "final-2026-09-19": {
        1: ("arena SRAM1, stack SRAM1, aggressor SRAM1", 0.249),
        2: ("arena SRAM1, stack SRAM2, aggressor SRAM1", 0.144),
        3: ("arena SRAM1, stack SRAM2, aggressor SRAM2", 0.106),
    },
    "paired-2026-09-19": {
        1: ("arena SRAM1, stack SRAM1, aggressor SRAM1", 0.249),
        2: ("arena SRAM1, stack SRAM2, aggressor SRAM1", 0.144),
        3: ("arena SRAM1, stack SRAM2, aggressor SRAM2", 0.106),
    },
}
GATE_TOL = 0.010

# when set, the gate section reports only this campaign.
CAMPAIGN_FOCUS = "final-2026-09-19"

# the knob values each point index stands for, in the same order as laxity_sweeps in the firmware.
# point 0 is the aggressor off in every sweep. this is a second copy of what the firmware holds,
# and the status line stored in stress.txt is what checks the two against each other.
SWEEPS = {
    "bw":     ["off", "50kHz", "100kHz", "200kHz", "400kHz", "800kHz"],
    "xact":   ["off", "w4", "w2", "w1"],
    "chan":   ["off", "1ch", "2ch", "4ch"],
    "stride": ["off", "s0", "s4", "s12", "s28", "s60", "s124"],
    "sat":    ["off", "w2 800kHz", "w1 800kHz", "ungated w4", "ungated w1", "armed, no trigger"],
    "low":    ["off", "6.25kHz", "12.5kHz", "25kHz", "50kHz"],
    # not a victim sweep. each point is one block moved once and timed on the DMA side, and the
    # label is the transaction count that block contains.
    "dmat":   ["-", "1024 xact", "2048 xact", "4096 xact", "512 xact"],
    "none":   ["no aggressor"],
}

# transactions in one timed block, by point, for the dmat sweep.
DMAT_XACTS = {1: 1024, 2: 2048, 3: 4096, 4: 512}

# requested bytes and transactions per second at each point. requested is the word that matters on
# the saturation sweep, where the hardware is not expected to deliver what the configuration asks.
RATES = {
    "bw":     [(0, 0), (12800000, 3200000), (25600000, 6400000), (51200000, 12800000),
               (102400000, 25600000), (204800000, 51200000)],
    "xact":   [(0, 0), (51200000, 12800000), (51200000, 25600000), (51200000, 51200000)],
    "chan":   [(0, 0), (102400000, 25600000), (102400000, 25600000), (102400000, 25600000)],
    "stride": [(0, 0)] + [(51200000, 12800000)] * 6,
    # the two ungated points and the armed point carry no requested rate. ungated means the rate is
    # whatever the matrix allows, and armed means there is no traffic at all.
    "sat":    [(0, 0), (204800000, 102400000), (204800000, 204800000), (0, 0), (0, 0), (0, 0)],
    "low":    [(0, 0), (1600000, 400000), (3200000, 800000), (6400000, 1600000), (12800000, 3200000)],
    "dmat":   [(0, 0)] * 5,
    # no points, so the only cell is the aggressor off one and there is no rate to declare.
    "none":   [(0, 0)],
}

# the fit stays below the rate where the earlier bandwidth sweep saturated in SRAM3.
FIT_MAX_XACT = 12800000

REGION_ID = {"sram1": 1, "sram2": 2, "sram3": 3, "sram4": 4}


def read_kv(path):
    out = {}
    if path.exists():
        for line in path.read_text().splitlines():
            if "=" in line:
                k, _, v = line.partition("=")
                out[k.strip()] = v.strip()
    return out


def load(run_dir):
    meta_kv = read_kv(run_dir / "stress.txt")
    if meta_kv.get("campaign") not in CAMPAIGNS:
        return None
    # a capture whose stress.txt was voided by hand is kept on disk, since results/raw is append
    # only, and is excluded from every table here. the reason is in the key.
    if "void" in meta_kv:
        return None
    stats, meta, _, records = telemetry_parse.parse((run_dir / "telemetry.bin").read_bytes())
    if not records:
        return None

    # the sequence number restarts at zero when the board resets, and a capture that spans a reset
    # holds records from two different configurations, since the firmware comes back running the
    # arena cross rather than whatever the console had selected. it is detected rather than
    # averaged, because the second half looks like a plausible measurement.
    restarts = sum(1 for a, b in zip(records, records[1:]) if b["seq"] < a["seq"])

    groups, xfer, wrong_region = {}, {}, 0
    want_aggr = REGION_ID[meta_kv["aggressor_region"]]
    for rec in records:
        point = (rec["aggressor_idx"] >> 8) & 0xFF
        region = rec["aggressor_idx"] & 0xFF
        # the record carries the aggressor region and stress.txt carries the name the capture was
        # filed under, so a capture whose records disagree with its own filing is caught here
        # rather than reported as a result.
        if point > 0 and region != want_aggr:
            wrong_region += 1
        groups.setdefault(point, []).append(rec["exec_cyc"])
        xfer.setdefault(point, []).append(rec["reserved"])

    rows = []
    base = statistics.median(groups[0]) if groups.get(0) else None
    # the aggressor off records in stream order, which the contention free table needs whole rather
    # than summarised, since 31 cycles in 329150 is 94 parts per million and a median alone cannot
    # carry that.
    seq0 = list(groups.get(0, []))
    # every point's records in stream order, which the paired comparison needs because it reads one
    # point of a sweep rather than a summary of all of them.
    seq_by_point = {p: list(v) for p, v in groups.items()}
    sweep = meta_kv["sweep"]
    for point in sorted(groups):
        cyc = sorted(groups[point])
        rate = RATES[sweep][point] if point < len(RATES[sweep]) else (0, 0)
        med = statistics.median(cyc)
        rows.append({
            "point": point,
            "label": SWEEPS[sweep][point] if point < len(SWEEPS[sweep]) else str(point),
            "n": len(cyc),
            "median": med,
            "p99": cyc[min(len(cyc) - 1, int(0.99 * len(cyc)))],
            "delta": med - base if base is not None else 0,
            "bytes_s": rate[0],
            "xact_s": rate[1],
            "advanced": point == 0 or max(xfer[point]) > min(xfer[point]),
        })
    return {"dir": run_dir.name, "kv": meta_kv, "rows": rows, "base": base,
            "stats": stats, "cyccnt_hz": meta.get("cyccnt_hz", 160000000),
            "wrong_region": wrong_region, "records": len(records), "restarts": restarts,
            "seq0": seq0, "seq_by_point": seq_by_point}


# blocks per second at each point of a sweep, which is the trigger rate times the channel count.
# the per block term of the cost model is charged once per block, so this is its x axis.
BLOCKS = {
    "bw":     [0, 50000, 100000, 200000, 400000, 800000],
    "xact":   [0, 200000, 200000, 200000],
    "chan":   [0, 200000, 400000, 800000],
    "stride": [0] + [200000] * 6,
    "sat":    [0, 800000, 800000, 0, 0, 0],
    "low":    [0, 6250, 12500, 25000, 50000],
    "dmat":   [0] * 5,
    "none":   [0],
}


def per_block(cap):
    """cycles of victim delay per aggressor block, fitted through the origin."""
    hz = cap["cyccnt_hz"]
    window_s = cap["base"] / hz
    blocks = BLOCKS[cap["kv"]["sweep"]]
    num = den = 0.0
    for r in cap["rows"]:
        if r["point"] == 0 or r["point"] >= len(blocks) or blocks[r["point"]] == 0:
            continue
        x = blocks[r["point"]] * window_s
        num += x * r["delta"]
        den += x * x
    return (num / den) if den else None


def dma_fit(cap):
    """time against transactions for a timed block, with a slope and an intercept."""
    import statistics as st
    pts = [(DMAT_XACTS[r["point"]], r["median"]) for r in cap["rows"] if r["point"] in DMAT_XACTS]
    if len(pts) < 2:
        return None, None
    n = len(pts)
    mx = st.mean(x for x, _ in pts)
    my = st.mean(y for _, y in pts)
    sxy = sum((x - mx) * (y - my) for x, y in pts)
    sxx = sum((x - mx) ** 2 for x, _ in pts)
    slope = sxy / sxx
    return slope, my - slope * mx


def coefficient(cap):
    """least squares slope through the origin of delta cycles against aggressor transactions."""
    hz = cap["cyccnt_hz"]
    window_s = cap["base"] / hz
    num = den = 0.0
    used = []
    for r in cap["rows"]:
        if r["point"] == 0 or r["xact_s"] == 0 or r["xact_s"] > FIT_MAX_XACT:
            continue
        x = r["xact_s"] * window_s
        num += x * r["delta"]
        den += x * x
        used.append(r["label"])
    if den == 0.0:
        return None, [], 0.0
    k = num / den
    worst = 0.0
    for r in cap["rows"]:
        if r["point"] == 0 or r["xact_s"] == 0 or r["xact_s"] > FIT_MAX_XACT:
            continue
        x = r["xact_s"] * window_s
        worst = max(worst, abs(r["delta"] - k * x))
    return k, used, worst


def per_point_table(cap):
    print("  %-18s %6s %10s %10s %10s %13s %13s" %
          ("point", "n", "median", "p99", "delta", "bytes/s", "xact/s"))
    for r in cap["rows"]:
        flag = "" if r["advanced"] else "  NO TRANSFER"
        print("  %-18s %6d %10d %10d %+10d %13d %13d%s" %
              (r["label"], r["n"], r["median"], r["p99"], r["delta"],
               r["bytes_s"], r["xact_s"], flag))


def main():
    args = sys.argv[1:]
    dirs = [pathlib.Path(a) for a in args] if args else sorted(pathlib.Path("results/raw").iterdir())
    caps = []
    for d in dirs:
        if not d.is_dir():
            continue
        cap = load(d)
        if cap is not None:
            caps.append(cap)
    if not caps:
        raise SystemExit("no campaign captures found")

    by_name = {c["dir"].split("-", 1)[1]: c for c in caps}

    print("# campaign inventory\n")
    print("  %-26s %8s %8s %6s %6s %7s %7s %s" %
          ("capture", "records", "min n", "drop", "gaps", "badregn", "resets", "experiment"))
    for c in sorted(caps, key=lambda c: c["dir"]):
        print("  %-26s %8d %8d %6d %6d %7d %7d %s%s" %
              (c["dir"].split("-", 1)[1], c["records"], min(r["n"] for r in c["rows"]),
               c["stats"]["dropped"], c["stats"]["gaps"], c["wrong_region"], c["restarts"],
               c["kv"].get("experiment", "?"),
               "   SPANS A RESET" if c["restarts"] else ""))

    paired = [c for c in caps if c["kv"].get("campaign") == CAMPAIGN_FOCUS
              and c["kv"].get("experiment", "") in
              ("default-off", "default-on", "placed-off", "placed-on")]
    if paired:
        # the standard point of the bandwidth sweep and the two above it. the on captures run the
        # whole sweep, so all three come out of the same captures.
        ON_POINTS = (3, 4, 5)
        ON_POINT = 3
        hz = paired[0]["cyccnt_hz"]
        window = 0.032 * hz          # the audio half buffer, 512 samples at 16 kHz

        def pool(exp, point=None):
            out = []
            for c in sorted(paired, key=lambda c: c["dir"]):
                if c["kv"]["experiment"] != exp:
                    continue
                out += c["seq_by_point"].get(0 if exp.endswith("-off") else point, [])
            return out

        def stats(v):
            q = sorted(v)
            return (q[0], q[len(q) // 2], q[min(len(q) - 1, int(0.99 * len(q)))], q[-1], len(q))

        print("\n# job A, paired comparison, one image, aggressor SRAM3\n")
        print("  deadline window %.0f cycles, which is 32 ms of audio half buffer at %d Hz" % (window, hz))
        print("  bandwidth sweep points read: 3 at 200 kHz, 4 at 400 kHz, 5 at 800 kHz\n")
        for pt, label in ((3, "200 kHz, the standard point"), (4, "400 kHz"), (5, "800 kHz")):
            print("  point %d, %s" % (pt, label))
            print("    %-9s %-14s %9s %9s %9s %9s %7s" %
                  ("config", "state", "min", "p50", "p99", "max", "n"))
            keep = {}
            for cfg in ("default", "placed"):
                off, on = pool(cfg + "-off"), pool(cfg + "-on", pt)
                if not off or not on:
                    continue
                so, sn = stats(off), stats(on)
                keep[cfg] = (so, sn)
                print("    %-9s %-14s %9d %9d %9d %9d %7d" % ((cfg, "aggressor off") + so))
                print("    %-9s %-14s %9d %9d %9d %9d %7d" % ((cfg, "aggressor on") + sn))
            for cfg in ("default", "placed"):
                if cfg not in keep:
                    continue
                so, sn = keep[cfg]
                print("    %-9s penalty        p50 %+8d (%+.3f%%)   p99 %+8d (%+.3f%%)" %
                      (cfg, sn[1] - so[1], 100.0 * (sn[1] - so[1]) / so[1],
                       sn[2] - so[2], 100.0 * (sn[2] - so[2]) / so[2]))
            if len(keep) == 2:
                d, pl = keep["default"], keep["placed"]
                print("    difference     penalty p50 %+8d          penalty p99 %+8d" %
                      ((pl[1][1] - pl[0][1]) - (d[1][1] - d[0][1]),
                       (pl[1][2] - pl[0][2]) - (d[1][2] - d[0][2])))
            print()

        rows = {}
        for cfg in ("default", "placed"):
            off, on = pool(cfg + "-off"), pool(cfg + "-on", ON_POINT)
            rows[cfg] = (stats(off), stats(on))
            print("  %s" % cfg)
            for label, st in (("aggressor off", rows[cfg][0]), ("aggressor on ", rows[cfg][1])):
                print("    %-14s min %8d  p50 %8d  p99 %8d  max %8d  n %6d" % ((label,) + st))
            o50, o99 = rows[cfg][0][1], rows[cfg][0][2]
            n50, n99 = rows[cfg][1][1], rows[cfg][1][2]
            print("    penalty        p50 %+8d (%+.3f%%)   p99 %+8d (%+.3f%%)"
                  % (n50 - o50, 100.0 * (n50 - o50) / o50, n99 - o99, 100.0 * (n99 - o99) / o99))
            miss_off = sum(1 for v in off if v > window)
            miss_on = sum(1 for v in on if v > window)
            print("    margin         off p50 %.3f%%  p99 %.3f%%   on p50 %.3f%%  p99 %.3f%%"
                  % (100.0 * o50 / window, 100.0 * o99 / window,
                     100.0 * n50 / window, 100.0 * n99 / window))
            print("    windows missed off %d of %d, on %d of %d\n"
                  % (miss_off, len(off), miss_on, len(on)))

        if "default" in rows and "placed" in rows:
            d, pl = rows["default"], rows["placed"]
            print("  %-22s %12s %12s %12s" % ("", "default", "placed", "difference"))
            for label, i in (("aggressor off p50", 1), ("aggressor off p99", 2),
                             ("aggressor on p50", 1), ("aggressor on p99", 2)):
                a = d[0 if "off" in label else 1][i]
                b = pl[0 if "off" in label else 1][i]
                print("  %-22s %12d %12d %+12d" % (label, a, b, b - a))
            dp50 = d[1][1] - d[0][1]
            pp50 = pl[1][1] - pl[0][1]
            dp99 = d[1][2] - d[0][2]
            pp99 = pl[1][2] - pl[0][2]
            print("  %-22s %12d %12d %+12d" % ("penalty p50", dp50, pp50, pp50 - dp50))
            print("  %-22s %12d %12d %+12d" % ("penalty p99", dp99, pp99, pp99 - dp99))
            print("  %-22s %11.3f%% %11.3f%% %+11.3f%%" %
                  ("margin on p50", 100.0 * d[1][1] / window, 100.0 * pl[1][1] / window,
                   100.0 * (pl[1][1] - d[1][1]) / window))
            print("  %-22s %11.3f%% %11.3f%% %+11.3f%%" %
                  ("margin on p99", 100.0 * d[1][2] / window, 100.0 * pl[1][2] / window,
                   100.0 * (pl[1][2] - d[1][2]) / window))
        print()

        print("  per block, aggressor on p50, to show whether anything drifted\n")
        print("    %-10s %10s %10s" % ("block", "default", "placed"))
        for b in (1, 2, 3, 4):
            cells = []
            for cfg in ("default", "placed"):
                # the block name carries the campaign prefix, which differs between campaigns, so
                # the match is on the block and configuration rather than on a fixed name.
                m = [c for c in paired
                     if c["dir"].endswith("b%d-%s-on" % (b, cfg))]
                if not m:
                    cells.append("        --")
                    continue
                q = sorted(m[0]["seq_by_point"].get(ON_POINT, []))
                cells.append("%10d" % q[len(q) // 2] if q else "        --")
            print("    %-10s %s" % ("block %d" % b, " ".join(cells)))
        print()

    # gate cells are grouped by campaign, since the expected values belong to an image and pairing
    # an opening cell with a closing cell from another campaign would compare two runs.
    jb = [c for c in caps if c["kv"].get("experiment") == "b-descriptor"]
    if jb:
        print("\n# job B, the residual of the placed configuration against the descriptor page\n")
        print("  arena SRAM1, stack SRAM1, aggressor data SRAM3 at the standard point\n")
        print("  %-22s %-22s %10s %10s %12s" %
              ("capture", "descriptor page", "baseline", "coeff", "residual"))
        for c in sorted(jb, key=lambda c: c["kv"].get("descriptor_region", "")):
            k, _, worst = coefficient(c)
            print("  %-22s %-22s %10d %10.3f %12.1f" %
                  (c["dir"].split("-", 1)[1], c["kv"].get("descriptor_region", "?"),
                   c["base"], k if k is not None else float("nan"), worst))
        print()

    c1 = [c for c in caps if c["kv"].get("experiment") == "c1-shape"]
    if c1:
        cells = {}
        for c in c1:
            vr = int(c["kv"]["victim_region"][-1])
            sr = int(c["kv"]["stack_region"][-1])
            cells[(vr, sr)] = c
        n = min(len(c["seq0"]) for c in c1)
        med = {k: sorted(v["seq0"][:n])[n // 2] for k, v in cells.items()}
        print("\n# job C1, read loop victim, no channel started at any point\n")
        print("  vwords %s, vpasses %s, every cell truncated to the first %d records\n"
              % (c1[0]["kv"].get("victim_words"), c1[0]["kv"].get("victim_passes"), n))
        print("    %-10s %10s %10s %10s" % ("buffer", "stack s1", "stack s2", "stack s3"))
        for vr in (1, 2, 3):
            row = ["%10d" % med[(vr, sr)] if (vr, sr) in med else "        --" for sr in (1, 2, 3)]
            print("    %-10s %s" % ("sram%d" % vr, " ".join(row)))
        if len(med) == 9:
            ref = med[(1, 1)]
            f = {r: med[(r, 1)] - ref for r in (1, 2, 3)}
            g = {r: med[(1, r)] - ref for r in (1, 2, 3)}
            cc = {r: (f[r] + g[r]) / 2.0 for r in (1, 2, 3)}
            print("\n    deltas against buffer sram1 with stack sram1, which is %d" % ref)
            print("    buffer alone: " + ", ".join("sram%d %+d" % (r, f[r]) for r in (1, 2, 3)))
            print("    stack alone:  " + ", ".join("sram%d %+d" % (r, g[r]) for r in (1, 2, 3)))
            shapes = {
                "additive": lambda vr, sr: f[vr] + g[sr],
                "maximum": lambda vr, sr: max(f[vr], g[sr]),
                "region sum": lambda vr, sr: sum(cc[r] for r in set((vr, sr))),
            }
            print("\n    %-14s %s" % ("shape", "worst error over the nine cells, cycles"))
            for name, fn in shapes.items():
                worst = max(abs((med[(vr, sr)] - ref) - fn(vr, sr))
                            for vr in (1, 2, 3) for sr in (1, 2, 3))
                print("    %-14s %.1f" % (name, worst))
        print()

    c2 = [c for c in caps if c["kv"].get("experiment") == "c2-window"]
    if c2:
        pts = {}
        for c in c2:
            vp = int(c["kv"]["victim_passes"])
            pts.setdefault(vp, {})[c["kv"]["victim_region"]] = c
        n = min(len(c["seq0"]) for c in c2)
        print("\n# job C2, buffer in SRAM3 against buffer in SRAM1, stack SRAM1, no channel started\n")
        print("  every cell truncated to the first %d records\n" % n)
        print("  %-9s %8s %8s %10s %10s %10s" %
              ("vpasses", "vwords", "loads", "buf sram3", "buf sram1", "difference"))
        diffs = []
        for vp in sorted(pts):
            d = pts[vp]
            if "sram3" not in d or "sram1" not in d:
                continue
            m3 = sorted(d["sram3"]["seq0"][:n])[n // 2]
            m1 = sorted(d["sram1"]["seq0"][:n])[n // 2]
            loads = int(d["sram3"]["kv"]["loads_per_window"])
            print("  %-9d %8s %8d %10d %10d %+10d" %
                  (vp, d["sram3"]["kv"]["victim_words"], loads, m3, m1, m3 - m1))
            diffs.append((vp, loads, m3 - m1))
        if len(diffs) >= 3:
            ys = [d for _, _, d in diffs]
            xs = [l for _, l, _ in diffs]
            const = sum(ys) / len(ys)
            k = sum(x * y for x, y in zip(xs, ys)) / sum(x * x for x in xs)
            mx = sum(xs) / len(xs)
            my = sum(ys) / len(ys)
            sxx = sum((x - mx) ** 2 for x in xs)
            slope = sum((x - mx) * (y - my) for x, y in zip(xs, ys)) / sxx if sxx else 0.0
            icept = my - slope * mx
            print("\n  %-34s %s" % ("shape", "worst error over the points, cycles"))
            print("  %-34s %.1f" % ("constant, one off per window", max(abs(y - const) for y in ys)))
            print("  %-34s %.1f" % ("proportional to accesses",
                                    max(abs(y - k * x) for x, y in zip(xs, ys))))
            print("  %-34s %.1f" % ("constant plus proportional",
                                    max(abs(y - (icept + slope * x)) for x, y in zip(xs, ys))))
        print()

    gates = [c for c in caps if c["kv"].get("experiment", "").startswith("gate-")
             and c["kv"].get("campaign") == CAMPAIGN_FOCUS] if CAMPAIGN_FOCUS else \
            [c for c in caps if c["kv"].get("experiment", "").startswith("gate-")]
    if gates:
        print("\n# repeat cells, start and end of the campaign\n")
        print("  %-24s %-44s %9s %9s %9s %s" %
              ("capture", "cell", "expected", "measured", "diff", "gate"))
        worst = 0.0
        for c in sorted(gates, key=lambda c: c["dir"]):
            name = c["dir"].split("-", 1)[1]
            idx = int(name[-1])
            label, exp = GATE_BY_CAMPAIGN.get(c["kv"].get("campaign"), GATE)[idx]
            k, _, _ = coefficient(c)
            diff = k - exp
            worst = max(worst, abs(diff))
            print("  %-24s %-44s %9.3f %9.3f %+9.3f %s" %
                  (name, label, exp, k, diff, "pass" if abs(diff) <= GATE_TOL else "FAIL"))
        opens = {int(c["dir"][-1]): c for c in gates if c["kv"]["experiment"] == "gate-open"}
        closes = {int(c["dir"][-1]): c for c in gates if c["kv"]["experiment"] == "gate-close"}
        if opens and closes:
            print("\n  %-24s %9s %9s %9s" % ("cell", "open", "close", "drift"))
            for idx in sorted(set(opens) & set(closes)):
                ko = coefficient(opens[idx])[0]
                kc = coefficient(closes[idx])[0]
                table = GATE_BY_CAMPAIGN.get(opens[idx]["kv"].get("campaign"), GATE)
                print("  %-24s %9.3f %9.3f %+9.3f" % (table[idx][0], ko, kc, kc - ko))
        print("\n  worst absolute difference from expected: %.3f against a gate of %.3f\n"
              % (worst, GATE_TOL))

    d88 = {c["dir"].split("-", 1)[1]: c for c in caps
           if c["kv"].get("experiment") == "descriptor-88"}
    if d88:
        print("\n# the descriptor page or the permanent 88 bytes\n")
        print("  arena in SRAM1, aggressor data in SRAM2, which holds nothing of the victim\n")
        print("    %-26s %10s %10s %10s %10s" %
              ("victim stack", "desc SRAM1", "desc SRAM2", "desc SRAM3", "desc SRAM4"))
        for st in (1, 3):
            cells = []
            for pi in (1, 2, 3, 4):
                c = d88.get("stress-cl-desc-s%d-p%d" % (st, pi))
                k, _, _ = coefficient(c) if c else (None, None, None)
                cells.append("%10.3f" % k if k is not None else "        --")
            print("    %-26s %s" % ("SRAM%d" % st, " ".join(cells)))
        print("\n  %-24s %10s %10s %12s %s" %
              ("cell", "baseline", "coeff", "residual", "descriptor page"))
        for name in sorted(d88):
            c = d88[name]
            k, _, worst = coefficient(c)
            page = "?"
            for f in c["kv"].get("status_line", "").split():
                if f.startswith("desc="):
                    page = f[5:]
            print("  %-24s %10d %10.3f %12.1f %s" % (name, c["base"], k, worst, page))
        print()

    quiet = {c["dir"].split("-", 1)[1]: c for c in caps
             if c["kv"].get("experiment") == "quiet-place"}
    if quiet:
        n = min(len(c["seq0"]) for c in quiet.values())
        print("\n# contention free placement, no channel started at any point\n")
        print("  every cell truncated to the first %d records, since a percentile compared across\n"
              "  unequal counts is not a comparison\n" % n)
        print("  %-8s %-8s %10s %10s %10s %10s %10s" %
              ("arena", "stack", "min", "p50", "p99", "max", "p50 delta"))
        ref = None
        for ar in (1, 2, 3):
            for st in (1, 2, 3):
                c = quiet.get("stress-cl-quiet-a%d-s%d" % (ar, st))
                if c is None:
                    continue
                cyc = sorted(c["seq0"][:n])
                med = cyc[len(cyc) // 2]
                if ref is None:
                    ref = med
                print("  %-8s %-8s %10d %10d %10d %10d %+10d" %
                      ("sram%d" % ar, "sram%d" % st, cyc[0], med,
                       cyc[min(len(cyc) - 1, int(0.99 * len(cyc)))], cyc[-1], med - ref))
        print("\n  medians as a table, arena down and stack across\n")
        print("    %-8s %10s %10s %10s" % ("", "stack s1", "stack s2", "stack s3"))
        for ar in (1, 2, 3):
            cells = []
            for st in (1, 2, 3):
                c = quiet.get("stress-cl-quiet-a%d-s%d" % (ar, st))
                if c is None:
                    cells.append("        --")
                    continue
                cyc = sorted(c["seq0"][:n])
                cells.append("%10d" % cyc[len(cyc) // 2])
            print("    %-8s %s" % ("arena s%d" % ar, " ".join(cells)))
        print()

    dec = {c["dir"].split("-", 1)[1]: c for c in caps
           if c["kv"].get("experiment") == "decomposition"}
    if dec:
        print("\n# inference victim, arena and stack as separate knobs\n")
        print("  cycles of victim delay per aggressor transaction, three point slope\n")
        for ar in (1, 2, 3):
            rows = [k for k in dec if k.startswith("stress-as-a%d-" % ar)]
            if not rows:
                continue
            print("  arena in SRAM%d" % ar)
            print("    %-10s %10s %10s %10s %10s" %
                  ("stack", "a:sram1", "a:sram2", "a:sram3", "a:sram4"))
            for st in (1, 2, 3):
                cells = []
                for ag in (1, 2, 3, 4):
                    c = dec.get("stress-as-a%d-s%d-g%d" % (ar, st, ag))
                    k, _, _ = coefficient(c) if c else (None, None, None)
                    cells.append("%10.3f" % k if k is not None else "        --")
                print("    %-10s %s" % ("sram%d" % st, " ".join(cells)))
            print()
        print("  %-22s %10s %10s %12s %8s %8s" %
              ("cell", "baseline", "coeff", "residual", "stack hw", "guard"))
        for name in sorted(dec):
            c = dec[name]
            k, _, worst = coefficient(c)
            hw, guard = "?", "?"
            for f in c["kv"].get("status_line", "").split():
                if f.startswith("shw="):
                    hw = f[4:]
                if f.startswith("sguard="):
                    guard = f[7:]
            print("  %-22s %10d %10.3f %12.1f %8s %8s" %
                  (name, c["base"], k, worst, hw, guard))
        print()

    dma = {c["kv"]["aggressor_region"]: c for c in caps
           if c["kv"].get("experiment") == "dma-throughput"}
    if dma:
        print("\n# DMA side, one block timed with no victim running\n")
        print("  %-8s %10s %10s %10s %10s %12s %12s" %
              ("region", "512 xact", "1024", "2048", "4096", "cyc/xact", "intercept"))
        for r in ("sram1", "sram2", "sram3", "sram4"):
            c = dma.get(r)
            if c is None:
                continue
            by = {p: None for p in DMAT_XACTS}
            for row in c["rows"]:
                if row["point"] in DMAT_XACTS:
                    by[row["point"]] = row["median"]
            slope, icept = dma_fit(c)
            print("  %-8s %10s %10s %10s %10s %12.3f %12.1f" %
                  (r, by[4], by[1], by[2], by[3], slope, icept))
        base = dma.get("sram2")
        if base is not None:
            b, _ = dma_fit(base)
            print("\n  ratio against sram2: " + ", ".join(
                "%s %.2f" % (r, dma_fit(dma[r])[0] / b) for r in
                ("sram1", "sram2", "sram3", "sram4") if r in dma))

    k2 = {c["dir"].split("-", 1)[1]: c for c in caps if c["kv"].get("experiment") == "k-matrix-2"}
    if k2:
        print("\n# the twelve cells again, with the victim stack in SRAM1\n")
        print("  %-8s %10s %10s %10s %10s" %
              ("victim", "a:sram1", "a:sram2", "a:sram3", "a:sram4"))
        for v in (1, 2, 3):
            cells = []
            for a in (1, 2, 3, 4):
                c = k2.get("stress-k2-v%d-a%d" % (v, a))
                k, _, _ = coefficient(c) if c else (None, None, None)
                cells.append("%10.3f" % k if k is not None else "        --")
            print("  %-8s %s" % ("sram%d" % v, " ".join(cells)))
        print("\n  %-16s %10s %10s %12s %s" %
              ("cell", "baseline", "coeff", "residual", "victim access mix"))
        for v in (1, 2, 3):
            for a in (1, 2, 3, 4):
                c = k2.get("stress-k2-v%d-a%d" % (v, a))
                if c is None:
                    continue
                k, _, worst = coefficient(c)
                print("  %-16s %10d %10.3f %12.1f %s" %
                      ("v%d-a%d" % (v, a), c["base"], k, worst,
                       c["kv"].get("victim_access_mix", "?").split(" ", 3)[-1]))

    low = {c["dir"].split("-", 1)[1]: c for c in caps if c["kv"].get("experiment") == "low-rate"}
    if low:
        print("\n# below the bandwidth sweep, where a saturated port should have room again\n")
        for name in sorted(low):
            c = low[name]
            k, _, _ = coefficient(c)
            print("%s   (%s)   coefficient %.3f" % (name, c["dir"], k))
            per_point_table(c)
            print()

    desc = {c["dir"].split("-", 1)[1]: c for c in caps if c["kv"].get("experiment") == "descriptor"}
    if desc:
        print("\n# where the linked list descriptors live, against the per block cost\n")
        print("  %-16s %-22s %10s %12s %12s" %
              ("capture", "descriptor page", "baseline", "cyc/block", "coeff/xact"))
        for name in sorted(desc):
            c = desc[name]
            page = "?"
            for f in c["kv"].get("status_line", "").split():
                if f.startswith("desc="):
                    page = f[5:]
            k, _, _ = coefficient(c)
            # the channel sweep runs above the rate the per transaction fit is allowed to use, so
            # the coefficient is absent there by design and the per block figure is the one to read.
            print("  %-16s %-22s %10d %12.3f %12s" %
                  (name, page, c["base"], per_block(c) or 0.0,
                   "%.3f" % k if k is not None else "n/a"))
        print()
        for name in sorted(desc):
            c = desc[name]
            print("%s   (%s)" % (name, c["dir"]))
            per_point_table(c)
            print()

    print("\n# experiment 1, cycles per aggressor transaction\n")
    print("  %-8s %10s %10s %10s %10s" % ("victim", "a:sram1", "a:sram2", "a:sram3", "a:sram4"))
    for v in (1, 2, 3):
        cells = []
        for a in (1, 2, 3, 4):
            c = by_name.get("stress-k-v%d-a%d" % (v, a))
            if c is None:
                cells.append("        --")
                continue
            k, _, _ = coefficient(c)
            cells.append("%10.3f" % k if k is not None else "        --")
        print("  %-8s %s" % ("sram%d" % v, " ".join(cells)))

    print("\n  uncontended victim median and fit residual per cell\n")
    print("  %-16s %10s %10s %12s" % ("cell", "baseline", "coeff", "max residual"))
    for v in (1, 2, 3):
        for a in (1, 2, 3, 4):
            c = by_name.get("stress-k-v%d-a%d" % (v, a))
            if c is None:
                continue
            k, used, worst = coefficient(c)
            print("  %-16s %10d %10.3f %12.1f" % ("v%d-a%d" % (v, a), c["base"], k, worst))

    print("\n# experiment 1, full per point tables\n")
    for v in (1, 2, 3):
        for a in (1, 2, 3, 4):
            c = by_name.get("stress-k-v%d-a%d" % (v, a))
            if c is None:
                continue
            print("victim sram%d, aggressor sram%d   (%s)" % (v, a, c["dir"]))
            per_point_table(c)
            print()

    print("# experiment 2, victim footprint\n")
    print("  %-10s %8s %8s %10s %12s %14s %12s" %
          ("footprint", "words", "loads", "baseline", "delta@12.8M", "cycles/access", "coeff"))
    for name in ("1k", "2k", "4k", "8k", "16k", "32k", "64k", "128k"):
        c = by_name.get("stress-foot-%s" % name)
        if c is None:
            continue
        loads = int(c["kv"]["loads_per_window"])
        hit = [r for r in c["rows"] if r["xact_s"] == 12800000]
        k, _, _ = coefficient(c)
        d = hit[0]["delta"] if hit else 0
        print("  %-10s %8s %8d %10d %+12d %14.5f %12.3f" %
              (name, c["kv"]["victim_words"], loads, c["base"], d, d / loads, k))

    print("\n  per point tables\n")
    for name in ("1k", "2k", "4k", "8k", "16k", "32k", "64k", "128k"):
        c = by_name.get("stress-foot-%s" % name)
        if c is None:
            continue
        print("footprint %s, %s loads   (%s)" % (name, c["kv"]["loads_per_window"], c["dir"]))
        per_point_table(c)
        print()

    print("# experiment 3, past saturation\n")
    for key in ("stress-sat-a1", "stress-sat-a2", "stress-sat-a2-4096loads",
                "stress-sat-a3", "stress-sat-a4"):
        c = by_name.get(key)
        if c is None:
            continue
        print("%s, aggressor in %s, %s loads   (%s)" %
              (key, c["kv"]["aggressor_region"], c["kv"]["loads_per_window"], c["dir"]))
        per_point_table(c)
        print("  saving per victim load at the largest saving: %.4f cycles" %
              (min(r["delta"] for r in c["rows"]) / int(c["kv"]["loads_per_window"])))
        print()

    holds = [c for c in caps if c["kv"].get("experiment") == "contamination"]
    if holds:
        print("# the mem2mem aggressor left running through a stress pass, as the earlier captures had it\n")
        for c in sorted(holds, key=lambda c: c["dir"]):
            print("mem2mem in %s at 16 KiB, plus the stress sweep   (%s)" %
                  (c["kv"]["aggressor_region"], c["dir"]))
            per_point_table(c)
            print()

    others = [c for c in caps if c["kv"].get("experiment") == "re-measure"]
    if others:
        print("# the transaction and channel sweeps, re-measured with no second aggressor\n")
        for c in sorted(others, key=lambda c: c["dir"]):
            print("%s sweep, aggressor in %s   (%s)" %
                  (c["kv"]["sweep"], c["kv"]["aggressor_region"], c["dir"]))
            per_point_table(c)
            print()

    print("# experiment 4, the two victims under the same aggressor\n")
    print("  %-18s %-10s %10s %10s %12s" %
          ("capture", "victim", "baseline", "coeff", "window us"))
    for a in (1, 2, 3):
        for key, victim in (("stress-k-v1-a%d" % a, "read_loop"),
                            ("stress-infer-a%d" % a, "inference")):
            c = by_name.get(key)
            if c is None:
                continue
            k, _, _ = coefficient(c)
            print("  %-18s %-10s %10d %10.3f %12.1f" %
                  (key, victim, c["base"], k, 1e6 * c["base"] / c["cyccnt_hz"]))
    print()
    for a in (1, 2, 3):
        c = by_name.get("stress-infer-a%d" % a)
        if c is None:
            continue
        print("inference victim, aggressor sram%d   (%s)" % (a, c["dir"]))
        per_point_table(c)
        print()


if __name__ == "__main__":
    main()
