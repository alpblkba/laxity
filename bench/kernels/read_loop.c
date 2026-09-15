// see read_loop.h.

#include "read_loop.h"

uint32_t qos_read_loop(const volatile uint32_t *buf)
{
    uint32_t acc = 0u;

    for (uint32_t pass = 0u; pass < QOS_READ_LOOP_PASSES; ++pass) {
        // eight loads per iteration. the index arithmetic and the branch are paid once per eight
        // loads instead of once per load, so the window is dominated by the traffic it is meant
        // to measure rather than by loop overhead.
        for (uint32_t i = 0u; i < QOS_READ_LOOP_WORDS; i += 8u) {
            acc += buf[i + 0u];
            acc += buf[i + 1u];
            acc += buf[i + 2u];
            acc += buf[i + 3u];
            acc += buf[i + 4u];
            acc += buf[i + 5u];
            acc += buf[i + 6u];
            acc += buf[i + 7u];
        }
    }

    return acc;
}
