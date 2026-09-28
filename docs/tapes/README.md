# Terminal recordings

The VHS tapes in this directory generate the GIFs used by the root README. The renderer builds the current checkout before recording, since an installed command can still point to an older viewer. It writes temporary recordings under `build/tapes` and replaces each image in `assets` only after its tape succeeds.

Run from the repository root with VHS, a Rust toolchain and a C compiler installed. VHS uses `ffmpeg` and `ttyd`, which its Homebrew package installs. The scenario recordings need no board or private capture directory.

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
| `overview.tape` | `same-region-contention.lxs` | `assets/laxity-tui.gif` |
| `memory.tape` | `same-region-contention.lxs` | `assets/laxity-memory.gif` |
| `cross-region.tape` | `cross-region-contention.lxs` | `assets/laxity-cross-region.gif` |
| `crc.tape` | `crc-corruption.lxs` | `assets/laxity-crc.gif` |
| `audit.tape` | Reference ELF, profiles and application config | `assets/laxity-audit.gif` |

The simulator generates each byte stream from its checked-in scenario with a fixed seed. The tapes replay those files with the viewer's STM32U585 profile, which matches their addresses. The viewer marks the streams as simulated, and the displayed values remain scenario inputs rather than new hardware measurements.

`common.tape` fixes the shared terminal settings and selects the binary built under `build/tapes/cargo`. The renderer checks every scenario with the headless reader before VHS starts. Each tape then waits for its expected screen, so a command error fails the recording instead of becoming a README animation.

The audit tape requires `build/target/laxity-u585.elf`, the STM32U585 profile and characterisation, and `examples/stm32u585-reference/laxity.toml`. Build the reference firmware with `./tools/stm32/build.sh` when that ELF is absent. Scenario tapes can still be rendered individually without it.
