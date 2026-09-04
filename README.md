# Laxity

Laxity is a memory QoS and telemetry middleware for microcontrollers. It sits between an inference backend and an RTOS.

It is intended to measure the cost of each inference in CPU cycles and attributed memory stall cycles. The target workload runs TinyML inference, DSP, and DMA traffic on one SRAM and one bus. ThreadX schedules CPU time, but it does not arbitrate the memory system, so memory interference is otherwise hard to observe directly.

This project is brand new and none of these above implemented yet. The repository currently contains the design, measurement method, and tooling needed to bring up the board. Any numbers in the repository are assumptions or examples, not measured results.

The target platform is the B-U585I-IOT02A running ThreadX and ST Edge AI.



## The name

In the context of embedded systems or computer architecture, scheduling is the action of assigning resources to perform tasks. The resources may be processors, network links or expansion cards. The tasks may be threads, processes or data flows.

Laxity, also called slack, is the amount of time a job can still lose before it can no longer meet its deadline. [Laxity](https://microcontrollerslab.com/least-laxity-first-llf/) is the flexibility of scheduling, the maximum amount of time a task can be delayed or wait before it misses its hard deadline

```text
laxity = deadline - now - estimated_remaining_execution
```

A least laxity first scheduler runs the job with the smallest remaining laxity. 

See [least slack time scheduling](https://en.wikipedia.org/wiki/Least_slack_time_scheduling), and [Liu and Layland (1973)](https://doi.org/10.1145/321738.321743) for the hard real-time model it belongs to.


## License

```text
software: Apache-2.0

hardware designs, if they ever exist: CERN-OHL-P-2.0

third-party STMicroelectronics components: their respective ST licenses
```

