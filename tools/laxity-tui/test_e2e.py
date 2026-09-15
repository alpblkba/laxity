#!/usr/bin/env python3
"""exercise simulator and replay through the TUI live source."""

import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
TUI = ROOT / "build/laxity-tui/debug/laxity-tui"
SIM = ROOT / "build/laxity-sim/laxity-sim"
REPLAY = ROOT / "tools/laxity_replay.py"
sys.path.insert(0, str(ROOT / "tools"))
import telemetry_parse  # noqa: E402

PYTHON_COUNTERS = (
    "batch_frames",
    "dropped",
    "false_sync",
    "frames",
    "gaps",
    "header_frames",
    "records",
    "records_before_header",
    "skipped_bytes",
    "wrapped",
)


def checked(command, timeout=90):
    result = subprocess.run(
        command,
        cwd=ROOT,
        check=False,
        capture_output=True,
        text=True,
        timeout=timeout,
    )
    if result.returncode != 0:
        raise AssertionError(
            "command failed: %s\nstdout:\n%s\nstderr:\n%s"
            % (" ".join(map(str, command)), result.stdout, result.stderr)
        )
    return result


def build_tools():
    checked([str(ROOT / "tools/laxity-sim/build.sh")])
    checked(
        [
            "cargo",
            "build",
            "--manifest-path",
            str(ROOT / "tools/laxity-tui/Cargo.toml"),
            "--locked",
            "--target-dir",
            str(ROOT / "build/laxity-tui"),
        ]
    )


def unused_port():
    listener = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    listener.bind(("127.0.0.1", 0))
    port = listener.getsockname()[1]
    listener.close()
    return port


def summary(output):
    counters = {}
    for line in output.splitlines():
        name, value = line.split("=", 1)
        counters[name] = int(value)
    return counters


def receive_live(temp, label, sender):
    port = unused_port()
    record = temp / (label + ".bin")
    receiver = subprocess.Popen(
        [
            str(TUI),
            "--udp",
            str(port),
            "--headless",
            "--duration",
            "2",
            "--record",
            str(record),
        ],
        cwd=ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    try:
        time.sleep(0.1)
        if receiver.poll() is not None:
            stdout, stderr = receiver.communicate()
            raise AssertionError("receiver exited early\n%s\n%s" % (stdout, stderr))
        sender("127.0.0.1:%d" % port)
        stdout, stderr = receiver.communicate(timeout=5)
    finally:
        if receiver.poll() is None:
            receiver.terminate()
            receiver.wait(timeout=2)
    if receiver.returncode != 0:
        raise AssertionError("receiver failed\n%s\n%s" % (stdout, stderr))
    return summary(stdout), record


def assert_subset(actual, expected):
    for name, value in expected.items():
        assert actual[name] == value, (name, actual[name], value)


def assert_python_parity(path, rust):
    python, _, _, _ = telemetry_parse.parse(path.read_bytes())
    for name in PYTHON_COUNTERS:
        assert rust[name] == python[name], (name, rust[name], python[name])


def live_scenario(temp, scenario, expected):
    def send(endpoint):
        checked(
            [
                str(SIM),
                "--scenario",
                scenario,
                "--seed",
                "1",
                "--udp",
                endpoint,
            ]
        )

    rust, record = receive_live(temp, scenario, send)
    assert_subset(rust, expected)
    assert_python_parity(record, rust)


def replay_parity(temp):
    source = temp / "replay-source.bin"
    checked(
        [
            str(SIM),
            "--scenario",
            "baseline",
            "--seed",
            "7",
            "--output",
            str(source),
        ]
    )
    direct = summary(checked([str(TUI), "--file", str(source), "--headless"]).stdout)

    def send(endpoint):
        checked(
            [
                sys.executable,
                str(REPLAY),
                str(source),
                "--udp",
                endpoint,
                "--rate",
                "40",
            ]
        )

    live, record = receive_live(temp, "replay-live", send)
    assert live == direct
    assert record.read_bytes() == source.read_bytes()
    assert_python_parity(record, live)


def main():
    build_tools()
    with tempfile.TemporaryDirectory(prefix="laxity-tui-e2e-") as directory:
        temp = Path(directory)
        live_scenario(
            temp,
            "baseline",
            {"accepted_frames": 20, "frames": 20, "records": 40},
        )
        live_scenario(
            temp,
            "same-region-contention",
            {"accepted_frames": 18, "frames": 18, "records": 36},
        )
        live_scenario(
            temp,
            "crc-corruption",
            {
                "accepted_frames": 9,
                "crc_rejections": 1,
                "false_sync": 1,
                "frames": 9,
                "gaps": 1,
                "records": 4,
                "skipped_bytes": 40,
            },
        )
        replay_parity(temp)
    print("laxity TUI boardless end-to-end tests ok")


if __name__ == "__main__":
    main()
