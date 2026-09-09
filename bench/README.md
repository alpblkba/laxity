# Benchmarks

Two things live here. `kernels/` measures a unit of work in isolation. `aggressors/` adds controlled competing traffic. `sweeps/` holds the configurations that cross them.

The tuning study is not a third directory. It is a sweep whose findings become the runtime's default placement policy, which is what closes the loop between measurement and product.

Every run warms up first, reports median and p99 rather than the mean, uses at least thirty repetitions, randomises the order of configurations so drift does not correlate with run index, and records the null probe overhead beside the result.
