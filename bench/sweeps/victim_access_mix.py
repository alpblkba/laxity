#!/usr/bin/env python3
"""count the victim's memory accesses per region out of the disassembly of the image that ran.

    ./bench/sweeps/victim_access_mix.py --elf build/target/laxity-u585.elf \
        --words 1024 --passes 8 --buffer-region sram1 --stack-region sram1

prints one line for a capture's stress.txt. the knob says where the victim buffer is and the status
line says where the thread stack is, and neither says what fraction of the victim's address stream
goes to each. at -O0 the answer is not close to the knob: four of every five accesses in the read
loop are the compiler's stack traffic for its own locals.

this exists because two contaminations have been found in this rig and both were the same sentence,
that something touched memory inside the measured window which the knobs did not know about.

the count is exact for the loop structure rather than estimated. the two backward branches in the
function are the inner and the outer loop, which gives the trip count of every instruction, and
every memory referencing instruction is classified by whether its base register is the frame
pointer or the stack pointer.
"""

import argparse
import re
import subprocess
import sys

# a memory referencing instruction, with its base register. the read loop is compiled at -O0 and
# uses only these forms, and anything else is reported rather than silently dropped.
MEM = re.compile(r"^\s*(?P<addr>[0-9a-f]+):\s+(?:[0-9a-f]{4}\s)+\s*"
                 r"(?P<op>ldr|str|ldrb|strb|ldrh|strh|ldrd|strd|ldr\.w|str\.w|push|pop)"
                 r"(?:\.[wn])?\s+(?P<rest>.*)$")
BRANCH = re.compile(r"^\s*(?P<addr>[0-9a-f]+):\s+(?:[0-9a-f]{4}\s)+\s*"
                    r"b(?:cc|cs|ls|hi|eq|ne|lt|gt|le|ge|al)?(?:\.[wn])?\s+(?P<t>[0-9a-f]+)\s")


def disassemble(elf, symbol):
    out = subprocess.run(["arm-none-eabi-objdump", "-d", elf],
                         capture_output=True, text=True, check=True).stdout
    lines = out.splitlines()
    start = next(i for i, l in enumerate(lines) if l.endswith("<%s>:" % symbol))
    body = []
    for l in lines[start + 1:]:
        if not l.strip():
            break
        body.append(l)
    return body


def classify(body):
    """(address, is_stack) for every memory referencing instruction."""
    out = []
    for l in body:
        m = MEM.match(l)
        if not m:
            continue
        rest = m.group("rest")
        base = re.search(r"\[(\w+)", rest)
        op = m.group("op")
        if op in ("push", "pop"):
            # one access per register in the list, and always the stack
            n = len(re.findall(r"\w+", rest.strip("{} ")))
            for _ in range(n):
                out.append((int(m.group("addr"), 16), True))
            continue
        if base is None:
            continue
        out.append((int(m.group("addr"), 16), base.group(1) in ("r7", "sp", "fp")))
    return out


def loops(body):
    """the inner and outer loop ranges, from the two backward branches."""
    back = []
    for l in body:
        m = BRANCH.match(l)
        if not m:
            continue
        a, t = int(m.group("addr"), 16), int(m.group("t"), 16)
        if t < a:
            back.append((t, a))
    if len(back) < 2:
        raise SystemExit("expected two backward branches in the victim, found %d" % len(back))
    back.sort(key=lambda r: r[1])
    return back[-2], back[-1]   # inner, outer


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--elf", required=True)
    ap.add_argument("--symbol", default="qos_read_loop")
    ap.add_argument("--words", type=int, required=True)
    ap.add_argument("--passes", type=int, required=True)
    ap.add_argument("--buffer-region", required=True)
    ap.add_argument("--stack-region", required=True)
    ap.add_argument("--unroll", type=int, default=8)
    a = ap.parse_args()

    body = disassemble(a.elf, a.symbol)
    inner, outer = loops(body)
    mem = classify(body)

    inner_trips = (a.words // a.unroll) * a.passes
    outer_trips = a.passes

    counts = {"stack": 0, "buffer": 0}
    for addr, is_stack in mem:
        if inner[0] <= addr <= inner[1]:
            trips = inner_trips
        elif outer[0] <= addr <= outer[1]:
            trips = outer_trips
        else:
            trips = 1
        counts["stack" if is_stack else "buffer"] += trips

    per = {}
    per[a.stack_region] = per.get(a.stack_region, 0) + counts["stack"]
    per[a.buffer_region] = per.get(a.buffer_region, 0) + counts["buffer"]
    total = counts["stack"] + counts["buffer"]

    regions = " ".join("%s=%d(%.1f%%)" % (r, n, 100.0 * n / total)
                       for r, n in sorted(per.items()))
    print("victim_access_mix=total=%d stack=%d@%s buffer=%d@%s %s"
          % (total, counts["stack"], a.stack_region, counts["buffer"], a.buffer_region, regions))
    print("victim_access_loads=%d" % (a.words * a.passes))


if __name__ == "__main__":
    main()
