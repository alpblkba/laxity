# STM32U585 firmware project

Generated territory. STM32CubeMX owns everything here except `USER CODE` regions.

`laxity-u585.ioc` is the source of truth for clocks, GPIO, DMA, interrupts, peripheral instances, ThreadX, NetX Duo and generated initialisation.

To change hardware configuration, edit the `.ioc`, then run:

```sh
./tools/stm32/generate.sh
./tools/stm32/build.sh
./tools/stm32/flash.sh
```

Then smoke test. A firmware change that only compiles is not validated, which matters most for DMA, interrupt routing, clock configuration, ThreadX, NetX Duo and activation placement.

Application logic belongs in separate modules outside this tree. `USER CODE` regions are for the minimum wiring that has to sit inside generated files.

STM32CubeIDE stays available for interactive debugging. It is not required for building, generating, flashing or running tests.
