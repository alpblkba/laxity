# Virtual target

`laxity-sim` runs the production `runtime/telemetry.c` ring, sequence numbering, header generation, framing and CRC on a host. Only the record producer is synthetic. It reproduces checked-in telemetry patterns for host development, but it does not model STM32 arbitration or predict hardware behaviour.

Build and stream a scenario from the repository root:

```sh
./tools/laxity-sim/build.sh
./build/laxity-sim/laxity-sim \
    --scenario sram3-asymmetry \
    --seed 1 \
    --udp 127.0.0.1:50505
```

Use `--output PATH` instead of UDP, or use both destinations together. An output path must not exist, since the simulator never overwrites a capture. A scenario name resolves to `scenarios/NAME.lxs`; an explicit path reads that file instead.

The STM32-derived scenarios use the medians in `RESULTS.md`. Their constant samples preserve those measured values without presenting a synthetic distribution as measured data. The `jitter` field is available for deterministic visual testing, and the same scenario plus seed produces identical bytes.

## Scenario format

Header fields have a fixed order so malformed scenarios fail before their first action:

```text
schema 1
name example
clock 160000000 159999900
period_cycles 3200000
pace_ms 20
header_flags 1
null_probe 14 14 118 118 128
placements stm32u585
```

Actions use decimal integers:

```text
emit COUNT PLACEMENT TARGET FOOTPRINT EXEC_CYC JITTER BATCH
overflow COUNT PLACEMENT TARGET FOOTPRINT EXEC_CYC JITTER
set_release CYCLE
crc_next
gap_next
silence MILLISECONDS
placements PLACEMENT_SET
```

`TARGET` is the placement identifier packed into the low byte of the LX v2 `aggressor_idx`. `FOOTPRINT` is its high-byte index, where the current experiment uses 0, 1, 2 and 3 for 1 KiB, 4 KiB, 8 KiB and 16 KiB. This encoding stays at the LX v2 boundary.

`overflow` fills the real 32-record telemetry ring before draining it, so target-drop accounting comes from production code. `crc_next` corrupts the next batch after production framing. `gap_next` discards one complete drain after sequence numbers have advanced, which represents transport loss rather than a target drop. `silence` delays only live UDP output because a byte file cannot encode idle time.

`pace_ms` delays each UDP datagram. File-only generation has no artificial delay.

## Checks

Run the host-only checks without an STM32 SDK or board:

```sh
./tools/laxity-sim/test.sh
```

The checks compile the production telemetry implementation into the simulator, parse every checked-in scenario with `tools/telemetry_parse.py`, verify measured cell medians, exercise CRC, gap, drop, wrap and metadata behaviour, compare file and UDP bytes, and prove seeded jitter is deterministic.
