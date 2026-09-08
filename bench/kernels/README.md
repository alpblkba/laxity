# Isolation kernels

`axpy_u8` and `conv1d_i16_valid` come from the course benchmark work and have golden outputs, which is what makes the dead-code-elimination check possible.

`rfft_f32` is CMSIS-DSP and represents the real acquisition-side cost.

`pointer_chase` uses dependent loads and exposes latency rather than bandwidth. `stream_rw` is sequential and exposes bandwidth and arbitration.

`null_probe` measures the measurement. Run it first in every sweep.

Each kernel takes the region its working set lives in as a parameter, so the same binary covers every region.
