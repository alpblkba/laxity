// see read_loop.h.

#include "read_loop.h"

uint32_t qos_read_loop(const volatile uint32_t *buf, uint32_t words, uint32_t passes)
{
    uint32_t acc = 0u;

    // rounded down here rather than trusted from the caller, because the unrolled body reads eight
    // words past the index and a count that is not a multiple of eight would run off the buffer.
    words &= ~7u;

    for (uint32_t pass = 0u; pass < passes; ++pass) {
        // eight loads per iteration. the index arithmetic and the branch are paid once per eight
        // loads instead of once per load, so the window is dominated by the traffic it is meant
        // to measure rather than by loop overhead.
        for (uint32_t i = 0u; i < words; i += 8u) {
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
