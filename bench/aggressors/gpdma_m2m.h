/* a GPDMA1 memory to memory channel used as a controlled bus aggressor.

the point of this module is that it is not a thread. there is one Cortex-M33 core here and no SMP, so two threads never execute in the same instant: an aggressor thread that preempted the measured thread would put its own cycles inside the cycle counter window and inflate the result without anything being contended, and the curve that produced would look like  contention while being scheduler accounting.

GPDMA1 is an independent master on the bus matrix. a channel moving bytes while the core runs inference puts two masters on the matrix at once, which is the only arrangement on this silicon where the arbiter has a decision to make.
 
the channel runs a circular linked list so it repeats without the core touching it. a design that restarted the transfer from an interrupt would put an interrupt handler back inside the measured window, which is the same mistake as the thread.

*/
#ifndef LAXITY_GPDMA_M2M_H
#define LAXITY_GPDMA_M2M_H

#include <stdbool.h>
#include <stdint.h>

// enable the peripheral and prepare the channel. safe to call once at start up. */
bool qos_gpdma_m2m_init(void);

// move bytes endlessly between two buffers, both inside the region the caller chose. src and dst  are byte addresses and bytes is the size of each buffer, so a pass moves that many bytes. calling this while running restarts with the new parameters.
bool qos_gpdma_m2m_start(uint32_t src, uint32_t dst, uint32_t bytes);

void qos_gpdma_m2m_stop(void);

// completions observed since init, sampled by polling the transfer complete flag. this is the evidence that the channel is actually moving data. a channel that failed to start produces exactly the telemetry of a channel that causes no contention, so a count that does not advance between two records is a broken run rather than a null result.
 
// it is a lower bound: several blocks can complete between two polls and be counted once. that is enough to prove liveness, which is all it is for. 
uint32_t qos_gpdma_m2m_completions(void);

bool qos_gpdma_m2m_running(void);

#endif /*LAXITY_GPDMA_M2M_H*/
