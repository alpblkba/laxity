#!/usr/bin/env python3
"""one cell's coefficient in two capture sets, side by side.

    ./bench/sweeps/cell_compare.py results/raw/20260926T22*-df-* -- results/raw/20260930T2*-df-*

the two sets are separated by a bare --, and a cell is paired across them by the name its capture
directory ends in, which is the name bench/sweeps/descriptor_free_run.sh files it under. the left
set is the reference and the right is the one being read against it.

the fit is not here. campaign_report.py is imported and its load() and coefficient() do the whole
computation, so this prints two numbers from one implementation rather than adding a third. a
capture the reporter will not read, because its campaign is not in that module's CAMPAIGNS or its
stress.txt is missing, is reported as unreadable rather than skipped silently.
"""

import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import campaign_report


def cell_name(path):
    """the campaign's own name for a cell, which is what the timestamp prefix is followed by."""
    return path.name.split("-", 1)[1] if "-" in path.name else path.name


def read(path):
    """the coefficient and the worst residual of one capture, or a reason there is none."""
    cap = campaign_report.load(path)
    if cap is None:
        return None, "campaign_report.load() will not read it"
    if cap["restarts"]:
        return None, "the board reset %d times inside it" % cap["restarts"]
    k, _, worst = campaign_report.coefficient(cap)
    if k is None:
        return None, "no point of it is inside the fit"
    return (k, worst, cap), None


def by_cell(paths):
    """the newest capture of each cell in one set, since a cell taken twice is a repeat and the later one is the run being reported."""
    found = {}
    for path in sorted(paths):
        if path.is_dir():
            found[cell_name(path)] = path
    return found


def main():
    args = sys.argv[1:]
    if "--" not in args:
        raise SystemExit("usage: cell_compare.py <reference dirs> -- <dirs to read against them>")
    split = args.index("--")
    left = by_cell(pathlib.Path(a) for a in args[:split])
    right = by_cell(pathlib.Path(a) for a in args[split + 1:])
    if not left or not right:
        raise SystemExit("both sides need at least one capture directory")

    print("%-14s %-6s %-6s %10s %10s %10s   %s" %
          ("cell", "arena", "aggr", "right", "left", "diff", "right directory"))
    # every cell either side names, so one missing from a set is a row that says so rather than a row nobody sees.
    for name in sorted(set(left) | set(right)):
        if name not in right:
            print("%-14s no capture in the set being read" % name)
            continue
        if name not in left:
            print("%-14s no capture in the reference set" % name)
            continue
        this, why = read(right[name])
        if this is None:
            print("%-14s %s: %s" % (name, right[name].name, why))
            continue
        other, why = read(left[name])
        if other is None:
            print("%-14s %s: %s" % (name, left[name].name, why))
            continue
        (kr, _, cap), (kl, _, _) = this, other
        kv = cap["kv"]
        print("%-14s %-6s %-6s %10.5f %10.5f %+10.5f   %s" %
              (name, kv.get("arena_region", "?"), kv.get("aggressor_region", "?"),
               kr, kl, kr - kl, right[name].name))


if __name__ == "__main__":
    main()
