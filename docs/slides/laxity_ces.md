---
marp: true
theme: default
size: 16:9
paginate: true
header: 'ces.itec.kit.edu'
footer: '10.09.2026 · Alp O. Bolukbasi · Laxity'
---

<style>
:root {
  --kit-teal: #009682;
  --ces-blue: #2b6cb0;
  --baby: #dcefff;
  --baby-2: #ecf7ff;
  --text: #17212b;
  --muted: #657080;
  --edge: #547896;
  --amber: #d7a249;
  --green: #65ad79;
  --red: #d9776a;
  --violet: #9276d8;
}

section {
  font-family: Arial, Helvetica, sans-serif;
  color: var(--text);
  background-color: #fff;
  padding: 54px 64px 70px 64px;
  font-size: 24px;
  line-height: 1.26;
}

section:not(.title) {
  background-image:
    url('./ces-assets/kit-logo.png'),
    url('./ces-assets/ces-logo.png'),
    linear-gradient(#d8d8d8, #d8d8d8);
  background-repeat: no-repeat, no-repeat, no-repeat;
  background-position:
    right 60px top 28px,
    right 58px bottom 18px,
    left 60px bottom 52px;
  background-size:
    130px auto,
    92px auto,
    calc(100% - 120px) 1px;
}

h1 {
  font-size: 34px;
  font-weight: 700;
  margin: 0 0 26px 0;
  padding-right: 180px;
  color: #000;
}

h2 {
  font-size: 28px;
  margin: 0 0 14px 0;
}

p, li { font-size: 24px; }
ul { margin-top: 10px; }
li { margin: 9px 0; }
li::marker { color: var(--kit-teal); }
strong { color: #000; }
code {
  font-family: Menlo, Consolas, monospace;
  background: #f3f7fa;
  padding: 0.06em 0.20em;
  border-radius: 4px;
  font-size: 0.86em;
}

header {
  position: absolute;
  right: 165px;
  left: auto;
  top: auto;
  bottom: 19px;
  font-size: 14px;
  color: #111;
}

footer {
  position: absolute;
  left: 82px;
  bottom: 19px;
  font-size: 14px;
  color: #111;
}

section::after {
  color: #111;
  font-size: 14px;
  left: 29px;
  right: auto;
  bottom: 19px;
}

section.title {
  padding: 0;
  background-color: #fff;
  background-image:
    url('./ces-assets/kit-logo.png'),
    url('./ces-assets/ces-logo.png'),
    url('./ces-assets/title-band.png');
  background-repeat: no-repeat;
  background-position:
    left 50px top 46px,
    right 78px top 54px,
    left 16px bottom 62px;
  background-size:
    220px auto,
    250px auto,
    calc(100% - 32px) 268px;
}

section.title header,
section.title footer,
section.title::after { display: none; }

section.title .titlecopy {
  position: absolute;
  left: 74px;
  right: 86px;
  bottom: 112px;
  color: white;
}

section.title h1 {
  color: white;
  font-size: 48px;
  line-height: 1.08;
  padding: 0;
  margin: 0 0 18px 0;
  max-width: 1010px;
}

section.title .subtitle {
  font-size: 27px;
  line-height: 1.2;
  margin-bottom: 28px;
}

section.title .meta {
  font-size: 20px;
  opacity: 0.95;
}

.columns {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 34px;
  align-items: center;
}
.columns.wide-left { grid-template-columns: 1.16fr 0.84fr; }
.columns.wide-right { grid-template-columns: 0.86fr 1.14fr; }

.metric-grid {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 20px;
  margin-top: 44px;
}
.metric {
  background: linear-gradient(145deg, #eef8ff, #d8edff);
  border: 2px solid #7aaed8;
  border-radius: 16px;
  padding: 26px 22px;
  min-height: 178px;
}
.metric .value {
  font-size: 42px;
  line-height: 1;
  font-weight: 700;
  color: #14558c;
  margin-bottom: 15px;
}
.metric .label {
  font-size: 20px;
  line-height: 1.25;
}

.callout {
  background: #edf7ff;
  border-left: 7px solid var(--kit-teal);
  padding: 18px 22px;
  border-radius: 5px;
  margin-top: 18px;
}
.callout.warn { border-left-color: var(--amber); background: #fff8e8; }
.callout.open { border-left-color: var(--red); background: #fff2f0; }

.card {
  border: 1.5px solid #9fbad0;
  background: #f7fbff;
  border-radius: 10px;
  padding: 18px 20px;
}
.card h2 { font-size: 24px; margin-bottom: 8px; }

.memory-grid {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 16px;
  margin-top: 14px;
}
.memory {
  background: #f3efff;
  border: 2px solid var(--violet);
  border-radius: 10px;
  text-align: center;
  padding: 20px 12px;
}
.memory strong { font-size: 25px; }
.memory code { display: block; margin-top: 8px; background: transparent; }

.figure {
  display: block;
  max-width: 100%;
  max-height: 525px;
  margin: 0 auto;
}
.figure.tall { max-height: 545px; }
.figure.chart { max-height: 475px; }

.tui-shot {
  display: block;
  max-width: 100%;
  max-height: 515px;
  margin: 0 auto;
  border: 1px solid #bcc9d5;
  border-radius: 6px;
}

.caption {
  font-size: 16px;
  color: var(--muted);
  margin-top: 8px;
  text-align: center;
}

.small { font-size: 19px; }
.small li, .small p { font-size: 19px; }
.compact { font-size: 20px; }
.compact li, .compact p { font-size: 20px; }

.matrix-note {
  font-family: Menlo, Consolas, monospace;
  font-size: 20px;
  background: #f5f9fc;
  border: 1px solid #b8c9d8;
  border-radius: 8px;
  padding: 14px 18px;
}

section.full-figure h1 { margin-bottom: 12px; }
section.full-figure .figure { max-height: 560px; }
section.tui h1 { margin-bottom: 14px; }

.refs {
  columns: 2;
  column-gap: 36px;
  font-size: 14px;
  line-height: 1.22;
}
.refs p { font-size: 14px; margin: 0 0 9px 0; break-inside: avoid; }

.closing {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 38px;
  margin-top: 34px;
}

</style>

<!-- _class: title -->
<div class="titlecopy">

# Laxity

<div class="subtitle">Activation placement and memory contention in microcontroller-class neural inference</div>

<div class="meta">Alp O. Bolukbasi · Karlsruhe Institute of Technology · 10 September 2026</div>

</div>

---

# The measured effect is almost zero until memory is shared

<div class="metric-grid">
  <div class="metric">
    <div class="value">3 cycles</div>
    <div class="label">Maximum SRAM placement spread without a competing bus master, out of 320,898 cycles.</div>
  </div>
  <div class="metric">
    <div class="value">+31,189</div>
    <div class="label">Largest measured contention penalty, with inference and DMA in SRAM3.</div>
  </div>
  <div class="metric">
    <div class="value">+9.7%</div>
    <div class="label">Worst observed median penalty. SRAM3 also creates a cross-region asymmetry.</div>
  </div>
</div>

<div class="callout">The claim is a measurement claim, not a scheduler claim. Laxity measures the cost that a later placement policy would need to model.</div>

---

# The question is whether an arena address becomes a timing variable

<div class="columns wide-left">
<div>

A converted network normally treats its activation arena as a size constraint. If it fits, the exact SRAM address is usually left to the linker.

On the STM32U585, the core runs inference while DMA engines and sensor peripherals can move data independently. That creates a second requester for the memory system.

<div class="callout warn">If placement only matters under contention, the useful scheduling signal is not "fast SRAM". It is the current relationship between the arena and the active bus masters.</div>

</div>
<div>

![Memory contention model](../diagrams/generated/memory-contention.svg)

</div>
</div>

---

<!-- _class: full-figure -->
# Laxity separates the embedded experiment from host-side measurement

![Laxity architecture](../diagrams/generated/architecture.svg)

<div class="caption">Application, benchmark, platform support, telemetry, host analysis, and experiment tooling remain separate layers.</div>

---

# Platform and placement space

<div class="columns wide-left">
<div>

- B-U585I-IOT02A with STM32U585, Cortex-M33 at 160 MHz
- ThreadX application environment
- ST Edge AI Core 4.0.1 inference backend
- Human activity recognition model trained on WISDM
- 24 samples across three axes, four output classes
- Activation arena: **2,944 bytes**
- Internal SRAM is not covered by the data cache on this part

</div>
<div>

<div class="memory-grid">
  <div class="memory"><strong>SRAM1</strong><br>192 KiB<code>0x20000000</code></div>
  <div class="memory"><strong>SRAM2</strong><br>64 KiB<code>0x20030000</code></div>
  <div class="memory"><strong>SRAM3</strong><br>512 KiB<code>0x20040000</code></div>
</div>

<div class="callout">Three equally aligned arenas are allocated once. A pointer selects the active arena at run time, so the firmware image does not change between cells.</div>

</div>
</div>

---

# GPDMA is the aggressor because a second thread would measure preemption

<div class="columns wide-left">
<div>

A Cortex-M33 has one core and no simultaneous multithreading. Two software threads therefore do not execute at the same instant.

A higher-priority aggressor thread would add its own execution to the measured interval. That mixes scheduler accounting with memory interference.

GPDMA1 is independent of the core. It performs circular memory-to-memory transfers while inference runs, with no DMA interrupt inside the measured interval.

</div>
<div>

![Runtime sequence](../diagrams/generated/runtime-sequence.svg)

</div>
</div>

---

# The measurement path is small enough to measure itself

<div class="metric-grid">
  <div class="metric">
    <div class="value">14 cyc</div>
    <div class="label">Median and p99 cost of the cycle-counter read in the null probe.</div>
  </div>
  <div class="metric">
    <div class="value">118 cyc</div>
    <div class="label">Median record push cost, measured outside the inference timing interval.</div>
  </div>
  <div class="metric">
    <div class="value">32 B</div>
    <div class="label">Fixed telemetry record carrying timing, sequence, arena, and aggressor identity.</div>
  </div>
</div>

- DWT CYCCNT is 32 bits; wrap is detected and flagged.
- Records pass through an SPSC ring and framed transport with CRC.
- Sequence numbers advance on drop, so loss remains visible at the host.

---

# The sweep crosses arena placement with an independent memory master

<div class="columns wide-right">
<div class="compact">

- 3 arena regions × 3 DMA regions, plus aggressor-off baselines
- DMA working set: 1, 4, 8, and 16 KiB
- Additional controls estimate instrumentation and address-level noise
- Cells are visited in randomized order after warm-up
- Reported statistics: median and p99

<div class="callout">The reported 180 s capture contains 9,062 records, 217 to 225 samples per cell, no drops, no sequence gaps, no CRC rejections, and one detected CYCCNT wrap.</div>

</div>
<div>

![Experiment flow](../diagrams/generated/experiment-flow.svg)

</div>
</div>

---

<!-- _class: tui -->
# The TUI exposes the experiment cell instead of hiding it behind one latency number

![Laxity TUI](../../assets/01-dashboard-wide.png)

<div class="caption">Replay/live state, logical SRAM placement, DMA target, current cell, rolling p50, placement comparison, and raw board log remain visible together.</div>

---

# Without contention, placement changes the median by three cycles

<div class="columns wide-right">
<div class="compact">

The uncontended medians are:

<div class="matrix-note">SRAM1  320,898 cyc<br>SRAM2  320,901 cyc<br>SRAM3  320,899 cyc</div>

The full spread is **3 cycles**, or **9.3 parts per million**.

The control spread at this sample size is zero at the median. The effect is resolvable, but it is too small to act on.

</div>
<div>

![Baseline delta](./figures/baseline-delta.svg)

</div>
</div>

---

# Contention changes the scale from single cycles to tens of thousands

![Contention matrix](./figures/contention-matrix.svg)

<div class="caption">Each cell is the median penalty against the same arena's aggressor-off median.</div>

---

# SRAM3 is asymmetric, and the current experiment does not explain why

<div class="columns">
<div>

<div class="card">
<h2>R1 and R2</h2>

Same-region contention costs about **5.3%**. Cross-region traffic between R1 and R2 stays near baseline at the 16 KiB working set.
</div>

<div class="card" style="margin-top:18px;">
<h2>R3 as aggressor</h2>

Traffic in R3 adds about **4.7%** to arenas in R1 and R2, even though the regions differ.
</div>

</div>
<div>

<div class="card">
<h2>Reverse direction</h2>

R1 or R2 traffic adds only tens of cycles to an arena in R3.
</div>

<div class="callout open">R3 + R3 reaches +31,189 cycles, or +9.7%. The transfer counter advances at the same rate across regions, so the capture does not identify the internal arbitration point.</div>

</div>
</div>

---

# Working-set size changes the direction of the effect

![Working-set sweep](./figures/working-set-sweep.svg)

<div class="caption">R1/R2 same-region penalties fall as the DMA buffer grows; R3 cross-region penalties rise. R3/R3 is almost flat.</div>

---

# The synthetic master is a ceiling, not an operating point

<div class="columns wide-left">
<div>

The board also runs a sensing pipeline:

- inertial sensor sampled at 26 Hz
- environmental sensor
- microphone through the digital filter peripheral
- measured audio rate: 16,230 samples/s
- clock-chain prediction: 16,233 samples/s
- audio traffic: roughly 32 KiB/s
- audio buffer resides in SRAM3

</div>
<div>

![Cross-bank TUI view](../../assets/04-dashboard-100x30.png)

</div>
</div>

<div class="callout warn">The sensing pipeline is always on in the reported firmware. Its timing cost is therefore a baseline shift for the whole sensing workload, not an isolated microphone-on versus microphone-off result.</div>

---

# The measurement has limits that stay attached to the claim

<div class="columns">
<div>

- Arena size is fixed at 2,944 B, so scaling with activation footprint is unknown.
- Measurements use optimization level zero, which lengthens inference.
- Each main region contributes one arena address, so region and address are partly confounded.
- The inertial sensor runs at 26 Hz while the training dataset used 20 Hz; this affects classification quality, not timing.

</div>
<div>

- The DMA channel changed after the audio driver claimed the first channel; equal-priority arbitration is assumed not to depend on channel index.
- Radio-driver threads add about 4%, but they preempt the inference thread, so that number mixes preemption with interference.
- The SRAM3 mechanism remains unresolved.

<div class="callout open">The data supports the asymmetry. It does not support assigning that asymmetry to a specific undocumented internal bus path.</div>

</div>
</div>

---

# Laxity currently measures the cost model; the placement policy is future work

<div class="closing">
<div class="card">
<h2>Implemented now</h2>

- run-time arena placement
- independent GPDMA interference
- cycle-accurate telemetry
- capture and replay
- per-cell comparison and analysis
- live memory-topology TUI
</div>

<div class="card">
<h2>Deliberately not claimed yet</h2>

- automatic planner
- admission control
- optimal placement policy
- automatic migration

A policy would consume the per-region costs measured here instead of assuming one SRAM is globally best.
</div>
</div>

---

# What the measurements support

<div class="metric-grid">
  <div class="metric">
    <div class="value">9.3 ppm</div>
    <div class="label">Placement effect with an idle memory system.</div>
  </div>
  <div class="metric">
    <div class="value">5.3%</div>
    <div class="label">Same-region contention in SRAM1 and SRAM2.</div>
  </div>
  <div class="metric">
    <div class="value">9.7%</div>
    <div class="label">Largest measured penalty, in SRAM3.</div>
  </div>
</div>

<div class="callout">Activation placement becomes a useful scheduling signal when the memory system is shared. The open problem is the SRAM3 topology that a future cost model must represent.</div>

<p style="margin-top:34px; font-size:20px;"><code>github.com/alpblkba/laxity</code></p>

---

# References

<div class="refs">

<p>[1] C. L. Liu and J. W. Layland, "Scheduling algorithms for multiprogramming in a hard-real-time environment," JACM, 1973.</p>
<p>[2] R. Banakar et al., "Scratchpad memory: a design alternative for cache on-chip memory in embedded systems," CODES, 2002.</p>
<p>[3] O. Avissar, R. Barua, and D. Stewart, "An optimal memory allocation scheme for scratch-pad-based embedded systems," ACM TECS, 2002.</p>
<p>[4] S. Steinke et al., "Assigning program and data objects to scratchpad for energy reduction," DATE, 2002.</p>
<p>[5] H. Yun et al., "MemGuard: memory bandwidth reservation system for efficient performance isolation in multi-core platforms," RTAS, 2013.</p>
<p>[6] H. Yun et al., "PALLOC: DRAM bank-aware memory allocator for performance isolation on multicore platforms," RTAS, 2014.</p>
<p>[7] H. Kim et al., "Bounding memory interference delay in COTS-based multi-core systems," RTAS, 2014.</p>
<p>[8] R. Pellizzoni et al., "A predictable execution model for COTS-based embedded systems," RTAS, 2011.</p>
<p>[9] R. David et al., "TensorFlow Lite Micro: embedded machine learning for TinyML systems," MLSys, 2021.</p>
<p>[10] C. Banbury et al., "MLPerf Tiny benchmark," NeurIPS Datasets and Benchmarks, 2021.</p>
<p>[11] J. Lin et al., "MCUNet: tiny deep learning on IoT devices," NeurIPS, 2020.</p>
<p>[12] J. Lin et al., "MCUNetV2: memory-efficient patch-based inference for tiny deep learning," NeurIPS, 2021.</p>
<p>[13] J. R. Kwapisz, G. M. Weiss, and S. A. Moore, "Activity recognition using cell phone accelerometers," ACM SIGKDD Explorations, 2010.</p>

</div>
