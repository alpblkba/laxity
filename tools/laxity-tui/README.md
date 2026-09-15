# Laxity TUI

`laxity-tui` is the operator surface for Laxity telemetry. It decodes the existing mixed ASCII and LX binary stream, records every received byte unchanged, adapts LX v2 records into a hardware-independent host model, and renders the resulting measurements without assigning terminal coordinates to the protocol or experiment layers.

The default profile describes the measured B-U585I-IOT02A and STM32U585 backend. You can load another profile, but STM32U585 remains the only measured hardware backend. Generic host types and a synthetic target do not establish support for arbitrary hardware.

## Build and launch

Build from the repository root so Cargo output stays under the ignored `build/` tree.

```sh
cargo build --manifest-path tools/laxity-tui/Cargo.toml \
    --release \
    --locked \
    --target-dir build/laxity-tui
```

Choose one source. `--profile` is optional and defaults to `profiles/stm32u585.toml` through the embedded copy of that profile.

```sh
./build/laxity-tui/release/laxity-tui --serial
./build/laxity-tui/release/laxity-tui --serial /dev/cu.usbmodem21303
./build/laxity-tui/release/laxity-tui --udp 50505
./build/laxity-tui/release/laxity-tui --file results/raw/RUN/telemetry.bin
./build/laxity-tui/release/laxity-tui \
    --udp 50505 \
    --profile profiles/virtual-generic.toml \
    --record /absolute/path/telemetry.bin
```

Press `m` to switch between the diagnostic dashboard and memory view. In memory view, press `Tab` to select `latest`, `mean`, `p50`, `p95`, `p99`, `penalty`, `stall`, or release-time `slack`. Use the up and down keys to select a logical memory region. `q`, `Esc`, and `Ctrl-C` stop the process.

## Host-side layers

`TelemetryState` owns LX framing, CRC recovery, sequence accounting, session metadata, and ASCII recovery. `LxV2Adapter` is the only layer that understands the packed requester target and footprint index used by the STM32 experiment. It maps wire placements into `DeviceState`, where a placement range and a containing memory region remain separate objects.

LX v2 carries neither a device ID nor a requester ID. The operator selects the matching profile. The adapter names a requester only when that profile declares exactly one DMA requester; otherwise it keeps an explicit unknown LX requester ID instead of choosing between candidates.

`MeasurementStore` retains at most 4,096 measurements per exact experiment cell and at most 1,024 cells. It exposes count, latest, mean, p50, p95, and p99 without growing for the lifetime of the process. A structural placement metadata change advances the host topology revision and starts fresh measurement windows, while repeated LX headers preserve the current windows.

`ExperimentState` consumes only canonical device and measurement state. `MemorySceneModel` supplies renderer-independent regions, placement ranges, requester flows, metrics, selections, and semantic effect requests. TachyonFX maps those requests to the terminal after layout, so an arrival, relation change, CRC rejection, or source error can affect its local object without making the model depend on a `Rect`.

## Display semantics

The dashboard preserves the existing exact-cell comparison. `latest` is one record. `cell p50` is the upper median for the current placement, requester target, and working set. `off p50` is the same placement with the controlled requester disabled. Every penalty uses that placement's own off baseline, and `observed best` appears only after each comparison placement has samples for the exact cell. It reports an observation, not a planner choice.

`SAME REGION` means the workload placement and requester target map into the same profile memory region. `CROSS REGION` means they map into distinct logical regions. Neither label proves a physical bus path or freedom from contention. The measured `exec_cyc` window can contain both thread preemption and memory contention.

The memory view renders profile address ranges as proportional logical slabs. Highlighted subranges are the exact placement ranges carried by telemetry, so unmeasured addresses are not presented as measured. Slab depth is visual separation, not a die coordinate. The STM32 profile declares physical topology unavailable, which keeps the physical interconnect disclaimer visible.

Stall and slack never fall back to zero. Stall remains unavailable when the selected capability or stream metric is absent. Slack is calculated only for a workload with a deadline, using its estimated remaining execution at release; current LX captures carry no deadline, so their slack view remains unavailable.

## Recording and source health

Live serial and UDP sessions create a new `telemetry.bin` in the launch directory unless `--record PATH` is present. If that name already exists, the TUI chooses a timestamped name instead of appending. File replay does not record unless you request it. Use an absolute output path for automated collection.

Live monitoring has no default time limit. `--duration SECONDS` gives UDP or serial collection a positive whole-second limit and closes the recorder normally. `--headless` prints every parser counter. A live headless run also requires `--duration`, since a machine-run receiver needs a bounded completion condition.

Serial auto-detection checks `LAXITY_PORT` first. Otherwise it selects an ST-LINK USB callout device and honors `LAXITY_STLINK_SN` when more than one board is attached. An explicit `--serial PATH` has highest priority. Only one process can consume a serial stream, so do not run `tools/stm32/capture.sh` and the TUI against the same port.

The status panel distinguishes a silent source, stale bytes, text without valid frames, CRC rejection, recorder failure, and source failure. It keeps the last ASCII line visible and never replaces a failure with a blank terminal.

## Boardless workflows

Build the production-backed virtual target, start the TUI, then emit a deterministic scenario.

```sh
./tools/laxity-sim/build.sh
./build/laxity-tui/release/laxity-tui \
    --udp 50505 \
    --record /tmp/laxity-sim.bin
./build/laxity-sim/laxity-sim \
    --scenario sram3-asymmetry \
    --seed 1 \
    --udp 127.0.0.1:50505
```

Replay a real capture with its record release cadence by running the receiver before the sender.

```sh
./build/laxity-tui/release/laxity-tui \
    --udp 50505 \
    --record /tmp/laxity-replay.bin
python3 tools/laxity_replay.py \
    results/raw/RUN/telemetry.bin \
    --udp 127.0.0.1:50505 \
    --rate 1
cmp results/raw/RUN/telemetry.bin /tmp/laxity-replay.bin
```

The live replay utility preserves every source byte and reconstructs pacing from `release_cyc` and `cyccnt_hz`. A capture does not timestamp ASCII or individual UART bytes, so this is record release pacing rather than original serial timing.

## Validation

Run the TUI tests without an STM32 SDK or board.

```sh
cargo test --manifest-path tools/laxity-tui/Cargo.toml \
    --locked \
    --target-dir build/laxity-tui
cargo clippy --manifest-path tools/laxity-tui/Cargo.toml \
    --locked \
    --all-targets \
    --target-dir build/laxity-tui \
    -- -D warnings
python3 tools/laxity-tui/test_e2e.py
```

The end-to-end check streams baseline, contention, and CRC scenarios through the live UDP source, then requires paced replay to match direct file parsing and byte recording. It builds both host tools under `build/` and needs no board or STM32 SDK.

`results/raw/` is intentionally ignored, so the full real-capture regression is an explicit local check rather than a test that silently passes in a clean clone. Run it on a workspace containing the known capture:

```sh
cargo test --manifest-path tools/laxity-tui/Cargo.toml \
    --locked \
    --target-dir build/laxity-tui \
    -- --ignored known_real_capture_keeps_golden_accounting_and_experiment_state
```

Validate a recorded byte stream with the reference parser. `tools/analyse.py` still requires the provenance files created by `tools/stm32/capture.sh`, so a standalone TUI recording is not presented as a complete evidence run.

```sh
python3 tools/telemetry_parse.py /absolute/path/telemetry.bin
```
