# Benchmarks

Two things live here. `kernels/` measures a unit of work in isolation. `aggressors/` adds controlled competing traffic. `sweeps/` holds the configurations that cross them.

The tuning study is not a third directory. It is a sweep whose findings become the runtime's default placement policy, which is what closes the loop between measurement and product.

Methodology, the aggressor set and the experiment matrix are in `docs/EXPERIMENTS.md`.
