/* see null_probe.h. */

#include "null_probe.h"

#include "counters_dwt.h"
#include "qos/telemetry.h"

/* static rather than automatic. two arrays of 128 words are 1024 bytes, which is a third of the measurement thread's stack, and block 2 established that a stack shortfall on this target presents as a silent port rather than as an error. */
static uint32_t s_read[QOS_NULL_PROBE_N];
static uint32_t s_push[QOS_NULL_PROBE_N];

/* insertion sort on 128 elements, run once at boot. qsort would pull newlib's implementation and a comparison callback into the image for no gain at this size. */
static void sort(uint32_t *a, uint32_t n)
{
    for (uint32_t i = 1u; i < n; ++i) {
        uint32_t v = a[i];
        uint32_t j = i;
        while (j > 0u && a[j - 1u] > v) {
            a[j] = a[j - 1u];
            --j;
        }
        a[j] = v;
    }
}

qos_null_probe_t qos_null_probe_run(void)
{
    qos_null_probe_t out = {0};
    qos_infer_record_t rec = {0};

    for (uint32_t i = 0u; i < QOS_NULL_PROBE_N; ++i) {
        uint32_t t0 = qos_cyc_now();
        uint32_t t1 = qos_cyc_now();
        s_read[i] = qos_cyc_delta(t0, t1);
    }

    for (uint32_t i = 0u; i < QOS_NULL_PROBE_N; ++i) {
        /* the ring holds 32 records, so emptying it every 32 pushes keeps every sample on the accepting path. measuring a mixture of accepted and dropped pushes would report the cheaper drop path as if it were the cost of recording. */
        if ((i % 32u) == 0u) {
            qos_telemetry_reset();
        }
        uint32_t t0 = qos_cyc_now();
        (void)qos_telemetry_push(&rec);
        uint32_t t1 = qos_cyc_now();
        s_push[i] = qos_cyc_delta(t0, t1);
    }

    qos_telemetry_reset();

    sort(s_read, QOS_NULL_PROBE_N);
    sort(s_push, QOS_NULL_PROBE_N);

    out.n = QOS_NULL_PROBE_N;
    out.read_median = s_read[QOS_NULL_PROBE_N / 2u];
    out.read_p99    = s_read[(QOS_NULL_PROBE_N * 99u) / 100u];
    out.push_median = s_push[QOS_NULL_PROBE_N / 2u];
    out.push_p99    = s_push[(QOS_NULL_PROBE_N * 99u) / 100u];
    return out;
}
