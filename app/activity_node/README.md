# Reference application: activity-sensing node

A connected sensing node. Deliberately ordinary, since the runtime is the product and this exists to show it carries a real workload.

| Tenant | Kind | Criticality | Notes |
|---|---|---|---|
| IMU acquisition | RTOS thread | hard | periodic sensor stream |
| Audio acquisition and DSP | RTOS thread, DMA fed | hard | microphone capture, RMS and FFT |
| HAR inference | Laxity job | soft | pretrained ST Edge AI model |
| Wi-Fi telemetry | RTOS thread | best effort | optional |
| Environmental sensors | RTOS thread | best effort | temperature, humidity, pressure |

Hard tenants are RTOS threads rather than Laxity jobs, since Laxity rejects hard criticality for inference and does not pretend to schedule the acquisition path.

The visible failure mode is in the hard tenants. A soft tenant missing its deadline is tolerable by definition, so the demonstration has to break the acquisition path: adding inference to a working pipeline causes audio DMA overruns or missed acquisition windows, and Laxity's placement and admission bring them back.

The acquisition deadline follows from the acquisition path rather than being chosen.

```
T_block  = N / fs          = 512 / 16000   = 32 ms
C_budget = T_block * f_CPU = 0.032 * 160e6 = 5,120,000 cycles
```

The CPU has to finish a half buffer before circular DMA overwrites it. Missing that corrupts the window, which is how a timing failure becomes an observable output failure.

Audio earns its place without an audio model, since it is a realistic DMA-driven competing workload.

No model is trained. HAR uses the pretrained model from the lab toolchain.

Reported numbers come from recorded traces with known event timestamps. Live sensors are for demonstration.
