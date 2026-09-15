/* a GPDMA1 bus aggressor whose shape is set at run time, for separating why a region costs more.
 *
 * gpdma_m2m answers whether contention exists. this answers what the cost follows, and it is a
 * separate module rather than an extension because the two are different instruments: this one
 * runs on channels 12 to 15 and it is gated by a timer, so its numbers are not comparable with
 * the ones gpdma_m2m produced and nothing should be tempted to compare them.
 *
 * channels 12 to 15 are not an arbitrary choice. they are the only GPDMA1 channels on this part
 * that implement 2D addressing, which is where the per transfer address offset lives, so they are
 * the only ones that can sweep stride. using them for every sweep keeps the channel class out of
 * the comparison: the stride sweep and the channel count sweep run on the same hardware.
 *
 * every knob is a field in the structure below and every one is read when the channels are
 * started. nothing here is a compile time constant, because a configuration per binary would make
 * a relink part of every measured difference, and a relink alone has already moved a median on
 * this project by 85 cycles.
 */
#ifndef LAXITY_GPDMA_STRESS_H
#define LAXITY_GPDMA_STRESS_H

#include <stdbool.h>
#include <stdint.h>

#define QOS_STRESS_MAX_CHANNELS  4u

typedef struct {
    /* how many channels move data at once, 1, 2 or 4. each one gets its own buffer pair inside
     * the same region, so more channels means more masters on the matrix rather than more bytes
     * from one master. */
    uint8_t  channels;

    /* bytes per bus transaction, 1, 2 or 4. this is the knob that separates bytes from
     * transactions: at a fixed block size and a fixed trigger rate, moving from 4 to 1 leaves the
     * byte rate untouched and multiplies the transaction rate by four. */
    uint8_t  width;

    /* bytes released by one trigger, per channel. */
    uint32_t block_bytes;

    /* bytes skipped after every transaction, so the address advances by width + stride. zero is
     * the contiguous case. the hardware field is signed and bounded at 8191. */
    uint32_t stride;

    /* triggers per second, which is what turns a channel that runs flat out into one with a
     * bandwidth. byte rate is block_bytes times this, per channel. zero means ungated, which is
     * the old behaviour and is not a point on any bandwidth axis. */
    uint32_t trigger_hz;
} qos_stress_cfg_t;

bool qos_stress_init(void);

/* start the configured channels inside [base, base + span). the buffers are carved from that
 * window, so the caller decides the region by deciding the window. returns false and starts
 * nothing when the configuration does not fit or the hardware refuses it. */
bool qos_stress_start(uint32_t base, uint32_t span, const qos_stress_cfg_t *cfg);

void qos_stress_stop(void);

/* completions observed by polling the transfer complete flag of every running channel, summed.
 * a count that does not advance between two records of an aggressor bearing cell is a channel
 * that never started, which is otherwise indistinguishable from a channel that cost nothing. */
uint32_t qos_stress_completions(void);

bool qos_stress_running(void);

/* the byte and transaction rates the configuration asks for, so a capture can be read against
 * what was requested rather than against what someone remembers requesting. */
uint32_t qos_stress_bytes_per_s(const qos_stress_cfg_t *cfg);
uint32_t qos_stress_xacts_per_s(const qos_stress_cfg_t *cfg);

#endif /* LAXITY_GPDMA_STRESS_H */
