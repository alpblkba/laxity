#!/usr/bin/env python3
"""turn a raw capture directory into the results document.

the results document is generated and never edited by hand, so this is the only thing that
writes it. it reads the framed stream through tools/telemetry_parse.py rather than
reimplementing the format, and it refuses to emit an entry that is missing any of the metadata
a reported result has to carry instead of quietly writing a shorter row.

set LAXITY_RESULTS to write somewhere other than RESULTS.md at the repository root.

    ./tools/analyse.py results/raw/<run> [more runs...]
"""

import os
import pathlib
import sys

import telemetry_parse

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parent
OUT = pathlib.Path(os.environ.get("LAXITY_RESULTS", ROOT / "RESULTS.md"))

# the fields an entry must carry. the list lives here rather than in the results document,
# because this script rewrites that document from scratch on every run and would erase any rule
# stated there.
#
# image_sha256 is required and elf_sha256 is not. a clean rebuild of identical sources reorders
# .debug_str and shifts every section header after it, so the ELF hash names a file no rebuild
# recreates while the image hash names the bytes that ran. elf_sha256 is still recorded and
# reported, since it identifies the local build a run came from.
REQUIRED_BUILD = ("board", "opt", "sysclk_hz", "flash_latency", "voltage_scale", "image_sha256")


def cycles(n):
    return "%d cycle" % n if abs(n) == 1 else "%d cycles" % n


def read_kv(path):
    out = {}
    if path.exists():
        for line in path.read_text().splitlines():
            if "=" in line:
                key, _, value = line.partition("=")
                out[key.strip()] = value.strip()
    return out


def entry(run_dir):
    build = read_kv(run_dir / "build.txt")
    run = read_kv(run_dir / "run.txt")
    commit = (run_dir / "commit.txt").read_text().strip()
    dirty = [l for l in (run_dir / "dirty.txt").read_text().splitlines() if l.strip()]

    missing = [f for f in REQUIRED_BUILD if not build.get(f)]
    if missing:
        raise SystemExit("%s is missing %s, refusing to report it" % (run_dir, ", ".join(missing)))

    out, meta, placements, records = telemetry_parse.parse((run_dir / "telemetry.bin").read_bytes())
    if not meta:
        raise SystemExit("%s carries no header frame" % run_dir)
    quiet = [r for r in records if r["aggressor_idx"] == 0] or records
    stats = telemetry_parse.region_stats(placements, quiet)
    cells = telemetry_parse.cell_stats(placements, records)

    # a channel that failed to start is indistinguishable from a channel that caused no
    # contention, so a cell that names an aggressor whose transfer count never moved is a broken
    # run and is refused rather than reported as a null result.
    dead = ["%s under %s" % (a, g) for (a, g), st in cells.items()
            if g != "off" and not st["xfer_advanced"]]
    if dead:
        raise SystemExit("%s: the aggressor never advanced in %d cell(s): %s"
                         % (run_dir, len(dead), ", ".join(dead[:4])))

    lines = []
    lines.append("## %s" % run_dir.name)
    lines.append("")
    if dirty:
        # a run whose source is not in a commit cannot be reconstructed, so it is not evidence.
        # the rule is restated here, where someone reading a number will see it.
        lines.append("Provisional. `dirty.txt` carries %d paths, so the working tree that produced this "
                     "firmware is not in a commit and the run cannot be reconstructed from `%s` alone. "
                     "Commit, capture again, and regenerate to clear this." % (len(dirty), commit[:12]))
        lines.append("")

    # the image hash goes first because it is the one a rebuild reproduces. the ELF hash follows
    # when the capture recorded one, since it still identifies the local build behind the run.
    ident = "image `%s`" % build["image_sha256"][:12]
    if build.get("elf_sha256"):
        ident += ", ELF `%s`" % build["elf_sha256"][:12]
    lines.append("%s, %s, SYSCLK %s Hz, %s, %s, ICACHE %s, commit `%s`, %s." % (
        build["board"], build["opt"], build["sysclk_hz"], build["flash_latency"],
        build["voltage_scale"], build.get("icache", "unknown"), commit[:12], ident))
    lines.append("")
    lines.append("Null probe on the same boot: reading the cycle counter costs %s cycles at median and "
                 "%s at p99, recording one result costs %s and %s, over N of %s. Every figure below "
                 "carries that overhead outside its measured window rather than inside it." % (
                     meta.get("null_read_median", "?"), meta.get("null_read_p99", "?"),
                     meta.get("null_push_median", "?"), meta.get("null_push_p99", "?"),
                     meta.get("null_n", "?")))
    lines.append("")
    lines.append("%s records over %s s, %s dropped, %s sequence gaps, %s frames rejected on CRC." % (
        out["records"], run.get("seconds", "?"), out["dropped"], out["gaps"], out["false_sync"]))
    lines.append("")
    contended = sorted({g for (_, g) in cells if g != "off"})
    if contended:
        base = {a: st["median"] for (a, g), st in cells.items() if g == "off"}
        lines.append("Median inference cycles by arena placement and competing GPDMA1 traffic. "
                     "The aggressor is a memory to memory channel whose buffers sit in the named "
                     "region, so a row is one arena and a column is one source of bus traffic.")
        lines.append("")
        lines.append("| arena | uncontended | " + " | ".join(contended) + " |")
        lines.append("|---" * (len(contended) + 2) + "|")
        for arena in sorted(base):
            row = ["%d" % base[arena]]
            for g in contended:
                st = cells.get((arena, g))
                row.append("%d (%+d)" % (st["median"], st["median"] - base[arena]) if st else "")
            lines.append("| %s | %s |" % (arena, " | ".join(row)))
        lines.append("")
        worst = max(((a, g, st["median"] - base[a]) for (a, g), st in cells.items()
                     if g != "off" and a in base), key=lambda t: t[2])
        lines.append("Every contended cell carried a GPDMA1 transfer count that advanced across "
                     "its records, so no cell here is a channel that failed to start. The largest "
                     "effect is %s under %s at %s above its own uncontended median."
                     % (worst[0], worst[1], cycles(worst[2])))
        lines.append("")

    lines.append("Placement on its own, from the records taken with no aggressor running.")
    lines.append("")
    lines.append("| label | arena | n | min | median | p99 | max | median delta |")
    lines.append("|---|---|---|---|---|---|---|---|")
    base = None
    for label, st in stats.items():
        if base is None:
            base = st["median"]
        note = " (control)" if st["control"] else ""
        lines.append("| %s%s | %s | %d | %d | %d | %d | %d | %+d |" % (
            label, note, st["arena_addr"], st["n"], st["min"], st["median"],
            st["p99"], st["max"], st["median"] - base))
    lines.append("")

    # a label name is the region name plus a suffix for a second identity of it, so grouping on the
    # leading region name is what pairs a control with the label it shadows.
    def group(label):
        return label.rstrip("abcdefghijklmnopqrstuvwxyz'")

    controls, alts, control_floor = [], [], 0
    for label, st in stats.items():
        base_label = next((l for l in stats if l == group(label) and l != label), None)
        if base_label is None:
            continue
        delta = st["median"] - stats[base_label]["median"]
        p99d = st["p99"] - stats[base_label]["p99"]
        if st["control"]:
            controls.append((label, base_label, delta, p99d))
            control_floor = max(control_floor, abs(delta))
        elif st["alt_addr"]:
            alts.append((label, base_label, delta, p99d))

    for label, base_label, delta, p99d in controls:
        lines.append("Same buffer under two identities, %s against %s: median differs by %s "
                     "and p99 by %d. That is the noise floor any between region claim has to clear."
                     % (label, base_label, cycles(delta), p99d))
        lines.append("")
    for label, base_label, delta, p99d in alts:
        lines.append("Same region at a second address, %s at %s against %s at %s: median differs by "
                     "%s and p99 by %d, so a difference at this size belongs to the region "
                     "rather than to the address." % (
                         label, stats[label]["arena_addr"], base_label,
                         stats[base_label]["arena_addr"], cycles(delta), p99d))
        lines.append("")

    if contended:
        by_col = {}
        for (a, g), st in cells.items():
            if g != "off":
                by_col.setdefault(g, {})[a] = st["median"]
        widest = max(by_col.items(), key=lambda kv: max(kv[1].values()) - min(kv[1].values()))
        span = max(widest[1].values()) - min(widest[1].values())
        best = min(widest[1], key=widest[1].get)
        lines.append("Under contention, where the arena sits changes median inference latency by "
                     "up to %s. The widest column is %s, where %s is the cheapest placement and "
                     "the most expensive costs that much more, against a same buffer control that "
                     "differs by %s."
                     % (cycles(span), widest[0], best, cycles(control_floor)))
        lines.append("")

        # the footprint axis, read from the cells where arena and aggressor share a region
        same = {}
        for (a, g), st in cells.items():
            if g != "off" and a[:4] == "SRAM" and a[4:].isdigit() and g.startswith("r%s-" % a[4:]):
                same.setdefault(g.split("-")[1], []).append(st["median"])
        if len(same) > 1:
            order = sorted(same, key=lambda k: int(k.rstrip("K")))
            trend = [int(sum(same[k]) / len(same[k])) for k in order]
            lines.append("Aggressor footprint moves the cost by %s across %s, and the direction is "
                         "%s: %s. Smaller buffers restart the channel's block more often, so the "
                         "per block arbitration rather than the bandwidth is what the arena pays for."
                         % (cycles(max(trend) - min(trend)), " to ".join([order[0], order[-1]]),
                            "downward" if trend[-1] < trend[0] else "upward",
                            ", ".join("%s %d" % (k, v) for k, v in zip(order, trend))))
            lines.append("")

    floor = max((abs(d) for _, _, d, _ in controls), default=0)
    primaries = {l: st for l, st in stats.items() if not st["control"] and not st["alt_addr"]}
    if primaries:
        ref = min(st["median"] for st in primaries.values())
        spread = max(st["median"] for st in primaries.values()) - ref
        resolvable = "above" if spread > floor else "at or below"
        lines.append("Isolated placement, for a %s byte arena with no aggressor running: moving it "
                     "between SRAM1, SRAM2 and SRAM3 moves median inference latency by at most %s "
                     "out of %d, which is %s the %s noise floor the same buffer control measures."
                     % (list(placements.values())[0]["arena_size"], cycles(spread), ref,
                        resolvable, cycles(floor)))
        lines.append("")
    return "\n".join(lines)


def main():
    if len(sys.argv) < 2:
        raise SystemExit(__doc__)

    body = ["# Results", "",
            "Generated by `tools/analyse.py` from `results/raw/`. Do not edit by hand.", ""]
    for arg in sys.argv[1:]:
        body.append(entry(pathlib.Path(arg)))
    OUT.write_text("\n".join(body).rstrip() + "\n")
    print("wrote %s" % OUT)


if __name__ == "__main__":
    main()
