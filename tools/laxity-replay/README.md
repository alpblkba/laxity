# Live capture replay

`tools/laxity_replay.py` sends an existing capture to the TUI over its UDP source without changing a byte. Start the receiver before the sender, since UDP does not retain packets for a listener that is not bound yet.

```sh
./build/laxity-tui/release/laxity-tui --udp 50505 --record /tmp/replayed.bin
python3 tools/laxity_replay.py results/raw/RUN/telemetry.bin --udp 127.0.0.1:50505
```

`--rate 0.25` runs at one quarter speed and `--rate 4` runs four times faster. `--loop` repeats until interrupted. A loop sends the original sequence numbers again, so a receiver that keeps its parser state can report a sequence discontinuity at each loop boundary. Rewriting those numbers would break byte preservation.

The replay parser uses the last `release_cyc` in each accepted record batch and the active `cyccnt_hz`. Unsigned subtraction preserves one 32 bit counter wrap, and absolute monotonic deadlines prevent sleep overhead from accumulating across the run. The first batch starts immediately because the capture has no timestamp for the time before its first record.

The capture has no wall timestamp for each frame or ASCII line. Replay therefore reconstructs record release cadence, not the original UART transmission timing. ASCII and rejected frame candidates keep their original byte positions and travel with the next accepted batch.

When fewer than two accepted record batches exist, the cycle clock is zero, or the clock changes inside the capture, replay falls back to 320 bytes every 33 ms. The rate multiplier applies to this path as well. Empty files and invalid rates fail instead of producing a successful run with no traffic.

UDP can still lose or reorder datagrams, particularly when replay runs faster than the receiver can drain its socket. Validate a local run by comparing the sender input with the TUI recording, then parse both files through the reference reader.

```sh
cmp results/raw/RUN/telemetry.bin /tmp/replayed.bin
python3 tools/telemetry_parse.py /tmp/replayed.bin
```

Run the focused checks from the repository root.

```sh
python3 -m unittest discover -s tools/laxity-replay -p 'test_*.py'
```
