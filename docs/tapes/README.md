# Terminal recordings

The VHS tapes in this directory generate the GIFs used by the root README. The renderer builds the current checkout before recording, since an installed command can still point to an older viewer. It writes temporary recordings under `build/tapes` and replaces each image in `assets` only after its tape succeeds.

Run from the repository root with VHS, a Rust toolchain and a C compiler installed. VHS uses `ffmpeg` and `ttyd`, which its Homebrew package installs. Three tapes read an attached board and the rest need no hardware.

```sh
brew install vhs
./docs/tapes/render.sh
```

Pass tape names to rebuild selected recordings. The same command rebuilds the host binary and generates the scenario inputs, so source changes are included in the next recording. The README links remain unchanged when the GIF files are replaced.

```sh
./docs/tapes/render.sh overview cross-region crc
```

| Tape | Input | Published image |
| --- | --- | --- |
| `overview.tape` | Attached board, serial | `assets/laxity-tui.gif` |
| `memory.tape` | Attached board, serial | `assets/laxity-memory.gif` |
| `cross-region.tape` | Attached board, serial | `assets/laxity-cross-region.gif` |
| `crc.tape` | `crc-corruption.lxs` | `assets/laxity-crc.gif` |
| `audit.tape` | Reference ELF, profiles and application config | `assets/laxity-audit.gif` |

The three board tapes record whatever the firmware is doing when VHS opens the port. Nothing arms them, because nothing needs to: the reference firmware boots with the inference victim and the bandwidth sweep already selected, so the GPDMA1 aggressor is running before the port is opened. The console alphabet that would select another configuration is documented in `firmware/stm32u585/Src/app_threadx.c` and read over the same UART, and no host tool here writes to it, so what a recording shows is the schedule the board shuffles through rather than one chosen cell.

The simulator generates the remaining byte stream from its checked-in scenario with a fixed seed. A rejected frame is deliberate corruption, and asking the board to produce one on cue is not something the firmware does, so that tape stays a replay and its alt text in the root README says so. The viewer marks the stream as simulated, and the displayed values remain scenario inputs rather than hardware measurements.

`common.tape` fixes the shared terminal settings and selects the binary built under `build/tapes/cargo`. The renderer checks the replayed scenario with the headless reader before VHS starts, and before a board tape it reads the port headlessly as well, which fails when no ST-LINK is there, when something else holds the port and when the stream carries no header frame. A board tape whose check fails is skipped with a message and its published image is left alone, since an empty screen labelled as a measurement is worse than an old recording. Each tape then waits for its expected screen, so a command error fails the recording instead of becoming a README animation.

The audit tape requires `build/target/laxity-u585.elf`, the STM32U585 profile and characterisation, and `examples/stm32u585-reference/laxity.toml`. Build the reference firmware with `./tools/stm32/build.sh` when that ELF is absent, and flash it with `./tools/stm32/flash.sh` before recording a board tape. The other tapes can still be rendered individually without either.
