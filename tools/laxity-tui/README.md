# Laxity TUI

`laxity-tui` is the live operator view for the placement experiment. It reads the existing mixed ASCII and binary telemetry stream, records every received byte unchanged, and renders logical SRAM traffic from the metadata and records already carried on the wire.

The display does not represent silicon coordinates. A memory row is a measured SRAM region, and its address is the activation arena address reported by the current telemetry header.

## Build and launch

Build from the repository root so Cargo output stays under the ignored `build/` tree.

```sh
cargo build --manifest-path tools/laxity-tui/Cargo.toml --release --target-dir build/laxity-tui
```

Use one source.

```sh
./build/laxity-tui/release/laxity-tui --serial
./build/laxity-tui/release/laxity-tui --serial /dev/cu.usbmodem21303
./build/laxity-tui/release/laxity-tui --udp 50505
./build/laxity-tui/release/laxity-tui --serial --duration 60 --record /absolute/path/telemetry.bin
./build/laxity-tui/release/laxity-tui --file results/raw/RUN/telemetry.bin
```

Live serial and UDP sessions create a new `telemetry.bin` by default. If that name exists, the TUI chooses a timestamped name instead of appending to it. `--record PATH` selects an explicit new file. Replay does not record unless `--record` is present.

The default recording is created in the directory from which the TUI is launched. Use an absolute `--record PATH` for an automated capture or scripted demo.

Live monitoring continues until `q`, `Esc` or `Ctrl-C`. `--duration SECONDS` gives UDP or serial collection a positive whole-second limit and still closes the recorder normally. File replay always runs to EOF and rejects `--duration`.

Serial auto-detection follows the same selection contract as the STM32 capture tools. `LAXITY_PORT` overrides discovery. Otherwise the TUI selects only an ST-LINK USB callout device and honors `LAXITY_STLINK_SN` when more than one board is attached. An explicit `--serial PATH` has highest priority.

File replay is paced near the current capture byte rate so topology changes remain visible. `--headless` parses without display pacing and prints every parser counter, including CYCCNT wraps and skipped bytes.

## What the display means

The status panel keeps source health, byte rate, accepted frames, records, CRC rejections, sequence gaps, target drops, and last byte and frame ages visible. A source or recorder failure remains readable instead of leaving a blank terminal.

The logical SRAM topology shows the primary placement regions from the telemetry header. Each row uses the reported arena address and size. `INFERENCE` marks the record's `region_id`, `DMA` marks the low byte of `aggressor_idx`, and `INFERENCE + DMA` marks an exact same-region collision. Control and alternate-address labels remain available to the parser but are not rendered as additional physical banks.

The current observed cell is held briefly for readability while every incoming record still updates statistics and the recorder. `latest` is one record. `cell p50` is the upper median over the newest 4,096 samples for the exact inference placement, DMA target, and footprint index. `off p50` is the same rolling measurement with `aggressor_idx` equal to zero. Existing experiment captures fit inside that window; a long-running monitor remains memory bounded while its byte-for-byte recording preserves the full run for offline analysis.

`SAME BANK` means the inference placement and DMA target resolve to the same logical SRAM region. `CROSS BANK` means they resolve to distinct SRAM regions, but both still use a shared memory fabric. It does not claim that cross-bank traffic is contention-free. Missing placement metadata produces `RELATION UNKNOWN` instead of a guessed mapping.

The placement comparison holds the DMA target and footprint constant and compares rolling p50 values for the primary inference placements observed for that exact cell. Every penalty is measured against that row's own rolling off median. `observed best` appears only after every primary placement has samples for the cell and marks every tie at the lowest median. It is not a planner choice or an optimality claim.

`exec_cyc` is a wall-cycle window. It can contain both thread preemption and memory contention, so the TUI does not label the full delta as hardware contention.

The current footprint byte table is a property of the benchmark run rather than telemetry version 2. Unknown footprint indices are displayed as indices instead of being assigned a size.

## Baseline, contention and replay

The current firmware schedule interleaves aggressor-off and GPDMA cells. The TUI derives `BASELINE / AGGRESSOR OFF` when the low byte of `aggressor_idx` is zero and `CONTENTION` otherwise, so both modes can appear without restarting the display. Aggressor-off describes the controlled benchmark input; it does not claim that every other source of memory traffic or preemption is absent.

For a board-free demonstration, replay any capture present in the local ignored `results/raw/` tree. Replace `RUN` in the example above with that capture directory. The same parser and experiment-state path is used for serial, UDP and file sources.

Only one process can consume a serial stream. Do not run `tools/stm32/capture.sh` and `laxity-tui --serial` against the same port. Use the TUI as the sole reader when presenting live; its recorder preserves a parser-compatible raw stream. Use `capture.sh` alone when a new evidence directory with build, commit and run provenance is required, then replay that directory's `telemetry.bin` in the TUI.

Validate a raw recording directly with the reference parser.

```sh
python3 tools/telemetry_parse.py telemetry.bin
```

`tools/analyse.py` additionally requires the provenance files created by `capture.sh`; a standalone TUI recording is deliberately not presented as a complete evidence run.
