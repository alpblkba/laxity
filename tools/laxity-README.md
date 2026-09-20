# laxity, the operator command

One command that finds the board, works out which profile describes it, and drives the scripts already in `tools/`. Every subcommand is a wrapper: the build, the flash, the capture and the analysis all stay where they were, so there is one definition of each and this cannot drift away from it.

Python 3, standard library only. No virtual environment and nothing to install.

## Install

```sh
./tools/install.sh
```

That symlinks `tools/laxity` into `~/.local/bin/laxity`. It asks before replacing anything already there, and if `~/.local/bin` is not on your PATH it prints the line to add and leaves your shell rc file alone. A symlink rather than a copy, because `laxity` resolves the link to find the repository.

Then check the machine:

```sh
laxity doctor
```

## Subcommands

```
laxity doctor              toolchain check, then the boards attached
laxity boards              each board: serial number, serial port, matching profile
laxity build               build the firmware
laxity flash               flash the built firmware
laxity capture             record a telemetry run, --seconds and --out
laxity analyse [capture]   turn a capture into RESULTS.md, newest if you name none
laxity run                 build, flash, capture, analyse, in one go
laxity tui                 the live viewer, or --file to read a capture back
laxity sim <scenario>      play a scenario from the simulator into the viewer
laxity audit [laxity.toml] audit declared memory placement without a board
laxity wifi                put the board on a network and report what actually worked
```

Only `run`, `capture`, `flash`, `tui` and `wifi` need a board. Everything else, including `sim`, `analyse` and `tui --file`, runs on the host alone.

## Audit

`laxity audit` reads the declared workloads, objects and requester endpoints from `laxity.toml`, resolves their addresses and sizes from the linker map, and evaluates them against the platform topology and its measured characterisation. Pass a config path when `laxity.toml` is not in the current directory.

The command does not discover objects; it reads `laxity.toml`. It does not apply a plan; `laxity plan` does that. It does not measure the target; `laxity characterise` does that. It does not resolve placement below a region.

The characterisation is separate from the topology in `profiles/<platform>.characterisation.toml`. A measured coefficient is a point estimate for that platform. A borrowed coefficient must carry a range and is reported as order of magnitude only. A missing coefficient or transaction rate stays `unknown`.

## Choosing between boards

Every command that touches a board takes `--board <serial>`. With one board attached you are never asked. With several and no `--board`, you get a numbered list and pick one, and the choice is passed to the scripts as `LAXITY_STLINK_SN`, which is the variable they already use to narrow themselves to one probe. The viewer is started on the chosen port, so there is no board picker inside it.

With no board attached, the command tells you what it looked for: ST-LINK probes in `STM32_Programmer_CLI -l`, and a serial port matching `/dev/cu.usbmodem*` on macOS or `/dev/ttyACM*` on Linux.

## Adding a board

Add a TOML file under `profiles/`. There is no code to change.

A profile matches a board when its `device.id`, lowercased with every non-alphanumeric character removed, is one of the part numbers the probe reports under the same normalisation, or a prefix of one. So `device.id = "stm32u585"` matches a probe reporting `STM32U585AI`, and it also matches `STM32U575/585`, because the programmer prints families with the tail after the slash standing for a suffix onto the head and `laxity` expands that form before comparing.

To find the part number a board reports, run `laxity boards` with it attached and read the `device` line. Identification connects to the probe in hotplug mode, which reads the part number without resetting or halting the core, so listing boards does not disturb a running experiment.

## Wi-Fi

`laxity wifi` uses the credentials already in `firmware/stm32u585/Inc/wifi_credentials.h` when they are real. Otherwise it reads the network this host is on and joins that, writing the header with both the SSID and the password quoted, and with this host's current address as the telemetry destination.

It reports two observations separately and never merges them:

```
1. association and DHCP     did the board join and take an address
2. UDP reached this host    did any datagram actually arrive here
```

They are separate because a guest network with a captive portal lets a client associate and then drops its traffic, so the first can pass while the second fails. That is a result, not a failure to report.

On current macOS the SSID is often unreadable. The old `airport` binary is gone, `networksetup -getairportnetwork` answers "not associated" even when you are, and `ipconfig getsummary` returns `<redacted>`, all because the SSID needs Location Services permission. When that happens `laxity wifi` says so and stops rather than writing a placeholder. Pass `--ssid` to get past it, or grant your terminal Location Services access.
