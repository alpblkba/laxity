/* a bare CPU read loop, used as the victim when the question is about the memory system rather
 * than about inference.
 *
 * inference is the wrong victim for a curve. its access pattern depends on the weights and on the
 * layer boundaries, so the number of loads inside the measured window is neither constant nor
 * known, and a curve drawn from it mixes the shape of the contention with the shape of the model.
 * this loop issues a known number of loads over a fixed buffer and does nothing else.
 *
 * it sits beside the inference path rather than replacing it, and which one runs is chosen at run
 * time, because a second binary would move the median on its own.
 */
#ifndef LAXITY_READ_LOOP_H
#define LAXITY_READ_LOOP_H

#include <stdint.h>

/* words read per pass. 1024 words is 4 KB, which is one arena alignment, so the victim buffer is
 * the same size and the same shape as an activation arena and sits where one would sit. */
#define QOS_READ_LOOP_WORDS   1024u

/* passes over the buffer per measured window. eight passes is 8192 loads, long enough that the
 * DWT window is thousands of cycles and short enough that a window never approaches a counter
 * wrap at 160 MHz. */
#define QOS_READ_LOOP_PASSES  8u

#define QOS_READ_LOOP_LOADS   (QOS_READ_LOOP_WORDS * QOS_READ_LOOP_PASSES)

/* read the buffer QOS_READ_LOOP_PASSES times and return the accumulated value.
 *
 * the return value is the reason the loads cannot be deleted. the caller consumes it, so nothing
 * in the chain is dead, which matters at any optimisation level above -O0 and costs nothing at
 * -O0. the body is unrolled by eight so the loop counter and the branch do not dominate what the
 * window measures.
 */
uint32_t qos_read_loop(const volatile uint32_t *buf);

#endif /* LAXITY_READ_LOOP_H */
