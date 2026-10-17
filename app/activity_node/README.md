# Reference application: activity-sensing node

This directory holds this README and no implementation. The node below is a design, and nothing in this repository runs it or measures it.

| Tenant | Kind | Criticality | Notes |
|---|---|---|---|
| IMU acquisition | RTOS thread | hard | periodic sensor stream |
| Audio acquisition and DSP | RTOS thread, DMA fed | hard | microphone capture, RMS and FFT |
| HAR inference | Laxity job | soft | pretrained ST Edge AI model |
| Wi-Fi telemetry | RTOS thread | best effort | optional |
| Environmental sensors | RTOS thread | best effort | temperature, humidity, pressure |

Hard tenants are RTOS threads rather than Laxity jobs, since Laxity rejects hard criticality for inference and does not pretend to schedule the acquisition path.

The firmware does not do this yet. `firmware/stm32u585/Src/app_threadx.c` creates one measurement thread at ThreadX priority 15, an export thread at 20 and a network thread at 12, and it polls the microphone and the IMU inline from the measurement thread rather than running either as a tenant of its own.

The acquisition deadline follows from the acquisition path rather than being chosen.

```
T_block  = N / fs          = 512 / 16000   = 32 ms
C_budget = T_block * f_CPU = 0.032 * 160e6 = 5,120,000 cycles
```

`platform/stm32u5/audio.h` corrects the rate that arithmetic uses. The part runs at 2.857 MHz divided by 176, or 16.23 kHz, because the decimation ratio is an integer, so a half buffer fills about every 31.5 ms. `laxity.toml` declares the deadline as 5046524 cycles, which is the real rate's figure, with the nominal one named in a comment beside it.

The CPU has to finish a half buffer before circular DMA overwrites it. Missing that corrupts the window, which is how a timing failure becomes an observable output failure.

What this node would show is that adding inference to the acquisition path costs that path its windows, and it does not show it. `RESULTS.md` carries no measurement of a lost window or an overrun, and the placement results it does carry are inference execution cycles under bus contention from a mem2mem channel.

The instrument for that measurement is half built. `qos_audio_buffers` in `platform/stm32u5/audio.c` counts the half buffers the poll has folded and the firmware prints it on the sensors status line, so the halves a run loses can be counted against the halves it should have seen. There is no interrupt on half completion: `platform/stm32u5/audio.h` records that a handler firing every half buffer would land inside measured inference windows and add cycles that have nothing to do with the bus. The latency of a late service therefore cannot be read, only the count of what was lost, and that count misses a half overwritten within one buffer period, since the two completion flags are sticky and a third completion sets a bit that is already set.

Audio earns its place without an audio model, since it is a realistic DMA-driven competing workload.

No model is trained. HAR uses the pretrained model from the lab toolchain.
