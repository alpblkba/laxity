#!/usr/bin/env python3
"""exercise the virtual target through its file and UDP boundaries."""

import socket
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
BIN = ROOT / "build/laxity-sim/laxity-sim"
sys.path.insert(0, str(ROOT / "tools"))
import telemetry_parse  # noqa: E402


def run(scenario, output, seed=1, udp=None, check=True):
    command = [str(BIN), "--scenario", scenario, "--seed", str(seed)]
    if output is not None:
        command += ["--output", str(output)]
    if udp is not None:
        command += ["--udp", udp]
    return subprocess.run(command, cwd=ROOT, check=check, capture_output=True)


def parsed(path):
    return telemetry_parse.parse(path.read_bytes())


def scenario_text(name, actions, pace_ms=0):
    return """schema 1
name %s
clock 160000000 159999900
period_cycles 3200000
pace_ms %d
header_flags 1
null_probe 14 14 118 118 128
placements stm32u585
%s
""" % (name, pace_ms, actions)


def check_scenarios(temp):
    first = temp / "baseline-a.bin"
    second = temp / "baseline-b.bin"
    run("baseline", first, seed=7)
    run("baseline", second, seed=7)
    assert first.read_bytes() == second.read_bytes()

    stats, _, placements, records = parsed(first)
    assert stats["records"] == 40
    assert stats["frames"] == 20
    assert stats["false_sync"] == 0
    _, meta, _, _ = parsed(first)
    assert meta["clock_hz"] == 160000000
    assert meta["cyccnt_hz"] == 159999900
    grouped = telemetry_parse.region_stats(placements, records)
    assert grouped["SRAM1"]["median"] == 320898
    assert grouped["SRAM2"]["median"] == 320901
    assert grouped["SRAM3"]["median"] == 320899

    expected = {
        "crc-corruption": {"false_sync": 1, "gaps": 1, "records": 4},
        "sequence-gap": {"dropped": 0, "gaps": 1, "records": 4},
        "dropped-record": {"dropped": 1, "gaps": 1, "records": 33},
        "counter-wrap": {"wrapped": 1},
    }
    for scenario, wanted in expected.items():
        path = temp / (scenario + ".bin")
        run(scenario, path)
        stats, _, _, _ = parsed(path)
        for key, value in wanted.items():
            assert stats[key] == value, (scenario, key, stats[key], value)

    for scenario in (
        "same-region-contention",
        "cross-region-contention",
        "sram3-asymmetry",
        "silent-source",
        "metadata-change",
        "unknown-region",
        "bursty",
    ):
        path = temp / (scenario + ".bin")
        run(scenario, path)
        stats, meta, _, _ = parsed(path)
        assert meta["version"] == 2
        assert stats["records"] > 0

    unknown_stats, _, _, unknown_records = parsed(temp / "unknown-region.bin")
    assert unknown_stats["gaps"] == 0
    assert any(record["region_id"] == 99 for record in unknown_records)

    _, _, changed, _ = parsed(temp / "metadata-change.bin")
    assert changed[6]["name"] == "SRAM2c"
    assert changed[6]["arena_addr"] == 0x2003A000

    exact_cells = {
        "same-region-contention": {
            ("SRAM1", "r1-16K"): 337877,
            ("SRAM2", "r2-16K"): 337845,
            ("SRAM3", "r3-16K"): 352088,
        },
        "cross-region-contention": {
            ("SRAM1", "r2-16K"): 320927,
            ("SRAM1", "r3-16K"): 335843,
            ("SRAM2", "r1-16K"): 320960,
            ("SRAM2", "r3-16K"): 335848,
            ("SRAM3", "r1-16K"): 320967,
            ("SRAM3", "r2-16K"): 320937,
        },
    }
    for scenario, expected_cells in exact_cells.items():
        _, _, scenario_placements, scenario_records = parsed(temp / (scenario + ".bin"))
        cells = telemetry_parse.cell_stats(scenario_placements, scenario_records)
        for cell, median in expected_cells.items():
            assert cells[cell]["median"] == median, (scenario, cell)

    existing = run("baseline", first, check=False)
    assert existing.returncode != 0


def check_deterministic_jitter(temp):
    scenario = temp / "jitter.lxs"
    scenario.write_text(
        scenario_text("jitter", "emit 40 1 0 0 320898 10 4"), encoding="ascii"
    )
    first = temp / "jitter-a.bin"
    second = temp / "jitter-b.bin"
    third = temp / "jitter-c.bin"
    run(str(scenario), first, seed=7)
    run(str(scenario), second, seed=7)
    run(str(scenario), third, seed=8)
    assert first.read_bytes() == second.read_bytes()
    assert first.read_bytes() != third.read_bytes()

    stats, _, _, records = parsed(first)
    assert stats["records"] == 40
    assert all(320888 <= record["exec_cyc"] <= 320908 for record in records)


def check_udp(temp):
    received = bytearray()
    listener = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    listener.bind(("127.0.0.1", 0))
    listener.settimeout(0.3)
    port = listener.getsockname()[1]

    def receive():
        while True:
            try:
                payload, _ = listener.recvfrom(65536)
            except socket.timeout:
                break
            received.extend(payload)

    thread = threading.Thread(target=receive)
    thread.start()
    run("baseline", None, seed=7, udp="127.0.0.1:%d" % port)
    thread.join()
    listener.close()

    direct = temp / "baseline-direct.bin"
    run("baseline", direct, seed=7)
    assert bytes(received) == direct.read_bytes()


def check_live_silence(temp):
    scenario = temp / "brief-silence.lxs"
    scenario.write_text(
        scenario_text(
            "brief-silence",
            "emit 1 1 0 0 320898 0 1\nsilence 60\nemit 1 1 0 0 320898 0 1",
        ),
        encoding="ascii",
    )

    listener = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    listener.bind(("127.0.0.1", 0))
    endpoint = "127.0.0.1:%d" % listener.getsockname()[1]
    start = time.monotonic()
    run(str(scenario), None, udp=endpoint)
    elapsed = time.monotonic() - start
    listener.close()
    assert elapsed >= 0.05


def check_invalid_schema(temp):
    invalid = temp / "invalid.lxs"
    invalid.write_text("schema 2\n", encoding="ascii")
    output = temp / "invalid.bin"
    result = run(str(invalid), output, check=False)
    assert result.returncode != 0

    duplicate_seed = subprocess.run(
        [
            str(BIN),
            "--scenario",
            "baseline",
            "--seed",
            "1",
            "--seed",
            "2",
            "--output",
            str(temp / "duplicate-seed.bin"),
        ],
        cwd=ROOT,
        check=False,
        capture_output=True,
    )
    assert duplicate_seed.returncode == 2


def main():
    with tempfile.TemporaryDirectory(prefix="laxity-sim-") as directory:
        temp = Path(directory)
        check_scenarios(temp)
        check_deterministic_jitter(temp)
        check_udp(temp)
        check_live_silence(temp)
        check_invalid_schema(temp)
    print("laxity simulator tests ok")


if __name__ == "__main__":
    main()
