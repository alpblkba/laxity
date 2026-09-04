/* DWT cycle counting and profiling counter capability probe for Cortex-M33.
 *
 * This is a local header rather than a port interface. include/qos/port/ is block 9, and retrofitting this module behind qos_port_counters_t does not need the file to move.
 *
 * Nothing here includes a vendor header. The DWT and DEBUG blocks are architectural on ARMv8-M, so the registers are addressed directly and the module stays buildable on any Cortex-M33.
 */
#ifndef LAXITY_COUNTERS_DWT_H
#define LAXITY_COUNTERS_DWT_H

#include <stdbool.h>
#include <stdint.h>

/* What this part actually implements, as opposed to what it accepts writes to. CPICNT, EXCCNT, LSUCNT and FOLDCNT are IMPLEMENTATION DEFINED on Cortex-M33, so each flag records whether the counter was observed to advance, not whether its enable bit read back. */
typedef struct {
    bool cyccnt;          /* DWT_CTRL.NOCYCCNT reads 0 and CYCCNT advances. */
    bool prfcnt_claimed;  /* DWT_CTRL.NOPRFCNT reads 0, so the part claims the profiling counters. */
    bool cpicnt;
    bool exccnt;
    bool lsucnt;
    bool foldcnt;
    uint32_t ctrl_after_enable;  /* DWT_CTRL read back after the enable writes, kept for the tracelog. */
} qos_dwt_caps_t;

/* Enable TRCENA and CYCCNTENA, then confirm by reading back and by watching the counter move. Returns false if CYCCNT does not run, in which case no timing in this project means anything. */
bool qos_dwt_init(void);

uint32_t qos_cyc_now(void);

/* Correct across one wrap of the 32 bit counter and wrong across two. Unsigned subtraction carries the single wrap for free. Two wraps are out of contract rather than handled: at 160 MHz that is 53.7 s between the two reads, and any interval that long is not an inference measurement. */
uint32_t qos_cyc_delta(uint32_t start, uint32_t end);

/* Measure the rate CYCCNT actually runs at, in Hz, over window_ms of wall clock supplied by millis.
 *
 * The point is to catch a counter that runs at some divided rate or a clock tree that is not what the .ioc claims. It is a ratio check, not a calibration: on this board the HAL timebase and the core both descend from the same PLL, so agreement confirms that CYCCNT counts core cycles and that the timebase prescaler is right, and it says nothing about the absolute accuracy of MSI. An independent oscillator would be needed for that.
 *
 * window_ms must stay well under 26800 so the cycle count cannot wrap inside the window. */
uint32_t qos_dwt_measure_hz(uint32_t (*millis)(void), uint32_t window_ms);

/* Write every profiling counter enable bit, read DWT_CTRL back, then run a loop and see which counters moved. Reading back a bit proves the register accepts a write, which is why the advance test exists. The load runs for burst_ms of wall clock so the timebase interrupt fires and EXCCNT has something to count, since a burst measured in microseconds would report a working EXCCNT as absent. */
qos_dwt_caps_t qos_dwt_probe(uint32_t (*millis)(void), uint32_t burst_ms);

#endif /* LAXITY_COUNTERS_DWT_H */
