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

The reference firmware boots with the inference victim and the bandwidth sweep already selected, so the GPDMA1 aggressor is running before the port is opened and a board tape never records an idle part. What it boots into is the arena cross, which shuffles, so an unarmed recording catches a cell rather than choosing one.

`laxity console` selects one. It resolves the port the viewer resolves, checks each key against the alphabet the firmware's switch implements, refuses the nine that reboot the board unless `--allow-reset` is passed, sends them paced clear of the board's one byte per tick poll, and reads the three once a second status lines until the board reports the state the keys ask for. A key that reboots goes first whatever order it was written in, and the writer says so, because a reboot returns everything not held in the word of SRAM4 to its power on default and would otherwise discard the selections sent before it. It exits non zero when the board does not, so a tape never records a cell nobody selected. `render.sh` calls it before a tape that names keys in `tape_keys`, and `cross-region.tape` names `i7c`: the victim moves to the stress schedule, where the arena is a slot rather than the cross, the arena goes to SRAM1 and the aggressor to SRAM3.

The simulator generates the remaining byte stream from its checked-in scenario with a fixed seed. A rejected frame is deliberate corruption, and asking the board to produce one on cue is not something the firmware does, so that tape stays a replay and its alt text in the root README says so. The viewer marks the stream as simulated, and the displayed values remain scenario inputs rather than hardware measurements.

`common.tape` fixes the shared terminal settings and selects the binary built under `build/tapes/cargo`. The renderer checks the replayed scenario with the headless reader before VHS starts, and before a board tape it reads the port headlessly as well, which fails when no ST-LINK is there, when something else holds the port and when the stream carries no header frame. A board tape whose check fails is skipped with a message and its published image is left alone, since an empty screen labelled as a measurement is worse than an old recording. Each tape then waits for its expected screen, so a command error fails the recording instead of becoming a README animation.

The audit tape requires `build/target/laxity-u585.elf`, the STM32U585 profile and characterisation, and the `laxity.toml` at the repository root. Build the reference firmware with `./tools/stm32/build.sh` when that ELF is absent, and flash it with `./tools/stm32/flash.sh` before recording a board tape. The other tapes can still be rendered individually without either.
