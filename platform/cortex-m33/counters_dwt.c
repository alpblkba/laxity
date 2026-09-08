/* see counters_dwt.h for what this module promises and what it deliberately does not. */

#include "counters_dwt.h"

/* ARMv8-M debug and DWT registers, addressed directly so this file needs no CMSIS device header and no STM32 header. addresses are architectural and come from the ARMv8-M architecture reference manual. */
#define REG32(addr)     (*(volatile uint32_t *)(addr))

#define DEMCR           REG32(0xE000EDFCu)
#define DEMCR_TRCENA    (1u << 24)

#define DWT_CTRL        REG32(0xE0001000u)
#define DWT_CYCCNT      REG32(0xE0001004u)
#define DWT_CPICNT      REG32(0xE0001008u)
#define DWT_EXCCNT      REG32(0xE000100Cu)
#define DWT_LSUCNT      REG32(0xE0001014u)
#define DWT_FOLDCNT     REG32(0xE0001018u)

#define DWT_CTRL_CYCCNTENA    (1u << 0)
#define DWT_CTRL_CPIEVTENA    (1u << 17)
#define DWT_CTRL_EXCEVTENA    (1u << 18)
#define DWT_CTRL_LSUEVTENA    (1u << 20)
#define DWT_CTRL_FOLDEVTENA   (1u << 21)

/* read only capability bits. a part that does not implement a counter reads 1 here, which is the architectural answer to the question the probe asks empirically. */
#define DWT_CTRL_NOPRFCNT     (1u << 24)
#define DWT_CTRL_NOCYCCNT     (1u << 25)

bool qos_dwt_init(void)
{
    DEMCR |= DEMCR_TRCENA;

    if ((DWT_CTRL & DWT_CTRL_NOCYCCNT) != 0u) {
        return false;
    }

    DWT_CYCCNT = 0u;
    DWT_CTRL |= DWT_CTRL_CYCCNTENA;

    if ((DWT_CTRL & DWT_CTRL_CYCCNTENA) == 0u) {
        return false;
    }

    /* the enable bit reading back is not the same claim as the counter running, and on ARMv8-M some parts gate DWT behind a lock access register, so the only honest check is to look twice. */
    uint32_t first = DWT_CYCCNT;
    for (volatile int i = 0; i < 16; ++i) {
    }
    return DWT_CYCCNT != first;
}

uint32_t qos_cyc_now(void)
{
    return DWT_CYCCNT;
}

uint32_t qos_cyc_delta(uint32_t start, uint32_t end)
{
    return end - start;
}

bool qos_cyc_wrapped(uint32_t start, uint32_t end)
{
    return end < start;
}

uint32_t qos_dwt_measure_hz(uint32_t (*millis)(void), uint32_t window_ms)
{
    if (millis == 0 || window_ms == 0u) {
        return 0u;
    }

    /* wait for a tick edge before starting, since entering mid tick makes the window shorter than window_ms by up to one tick and biases the result high. */
    uint32_t edge = millis();
    while (millis() == edge) {
    }

    uint32_t base = millis();
    uint32_t c0 = DWT_CYCCNT;
    while ((millis() - base) < window_ms) {
    }
    uint32_t cycles = qos_cyc_delta(c0, DWT_CYCCNT);

    return (uint32_t)(((uint64_t)cycles * 1000u) / window_ms);
}

qos_dwt_caps_t qos_dwt_probe(uint32_t (*millis)(void), uint32_t burst_ms)
{
    qos_dwt_caps_t caps = {0};

    DEMCR |= DEMCR_TRCENA;

    caps.cyccnt = (DWT_CTRL & DWT_CTRL_NOCYCCNT) == 0u;
    caps.prfcnt_claimed = (DWT_CTRL & DWT_CTRL_NOPRFCNT) == 0u;

    DWT_CTRL |= DWT_CTRL_CPIEVTENA | DWT_CTRL_EXCEVTENA |
                DWT_CTRL_LSUEVTENA | DWT_CTRL_FOLDEVTENA;
    caps.ctrl_after_enable = DWT_CTRL;

    if (millis == 0 || burst_ms == 0u) {
        return caps;
    }

    DWT_CPICNT = 0u;
    DWT_EXCCNT = 0u;
    DWT_LSUCNT = 0u;
    DWT_FOLDCNT = 0u;

    /* the load mixes loads, stores and arithmetic so a working LSUCNT sees memory accesses and a working CPICNT sees multi cycle instructions, and it runs for whole milliseconds so the timebase interrupt fires several times and a working EXCCNT has something to count. volatile keeps the compiler from deleting all of it. */
    static volatile uint32_t scratch[8];
    uint32_t base = millis();
    uint32_t first_cpi = 0u, first_exc = 0u, first_lsu = 0u, first_fold = 0u;
    bool sampled = false;

    for (uint32_t i = 0u; (millis() - base) < burst_ms; ++i) {
        scratch[i & 7u] = scratch[(i + 3u) & 7u] + i;
        if (!sampled && i == 256u) {
            /* one early sample, because these counters are eight bits wide and wrap. comparing an early reading against a late one detects a counter that is running even when the late reading happens to land back on zero. */
            first_cpi  = DWT_CPICNT  & 0xFFu;
            first_exc  = DWT_EXCCNT  & 0xFFu;
            first_lsu  = DWT_LSUCNT  & 0xFFu;
            first_fold = DWT_FOLDCNT & 0xFFu;
            sampled = true;
        }
    }

    uint32_t last_cpi  = DWT_CPICNT  & 0xFFu;
    uint32_t last_exc  = DWT_EXCCNT  & 0xFFu;
    uint32_t last_lsu  = DWT_LSUCNT  & 0xFFu;
    uint32_t last_fold = DWT_FOLDCNT & 0xFFu;

    caps.cpicnt  = (first_cpi  != 0u) || (last_cpi  != first_cpi);
    caps.exccnt  = (first_exc  != 0u) || (last_exc  != first_exc);
    caps.lsucnt  = (first_lsu  != 0u) || (last_lsu  != first_lsu);
    caps.foldcnt = (first_fold != 0u) || (last_fold != first_fold);

    return caps;
}
