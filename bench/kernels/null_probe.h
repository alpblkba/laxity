/* what the measurement costs, measured the same way the measurement is.
 *
 * docs/EXPERIMENTS.md requires the null probe to run first in every sweep and its overhead to be reported beside every result. without it a placement effect of a few hundred cycles cannot be told apart from the cost of looking.
 *
 * nothing here includes a vendor header. it needs the cycle counter and the telemetry ring and nothing else. */
#ifndef LAXITY_NULL_PROBE_H
#define LAXITY_NULL_PROBE_H

#include <stdint.h>

/* one hundred and twenty eight samples, which is the smallest count where the 99th percentile is not simply the maximum. at index 126 of 128 the p99 has two samples above it. */
#define QOS_NULL_PROBE_N  128u

typedef struct {
    uint32_t read_median;  /* cycles between two qos_cyc_now() calls with nothing between them */
    uint32_t read_p99;
    uint32_t push_median;  /* cycles for one record through qos_telemetry_push() */
    uint32_t push_p99;
    uint32_t n;
} qos_null_probe_t;

/* runs QOS_NULL_PROBE_N iterations of each measurement and leaves the telemetry ring empty and its counters at zero, so the probe's own records never reach a capture. the caller must be the telemetry producer and no consumer may be draining. */
qos_null_probe_t qos_null_probe_run(void);

#endif /* LAXITY_NULL_PROBE_H */
