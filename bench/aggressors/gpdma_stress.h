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

/* a trigger_hz value meaning "configure and enable the channels, then never trigger them".
 *
 * the channels are armed on the same TIM2 trigger every gated point uses and the timer is left
 * stopped, so they move zero bytes while everything else about the hardware state matches a point
 * that does move bytes. it exists to separate a channel being active from the traffic it carries,
 * which is the only way to ask whether an effect comes from the transfers or from the matrix
 * having a master attached to it. a point using this moves no data, so its completion count stays
 * flat on purpose and the usual reading of a flat count as a channel that failed to start does
 * not apply to it.
 */
#define QOS_STRESS_TRIGGER_ARMED 0xFFFFFFFFu

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

/* where the linked list nodes the hardware fetches at every block boundary are kept.
 *
 * by default they are static objects in bss, which on this build lands in SRAM3 for every
 * aggressor region. that makes the descriptor fetch a constant of the rig rather than a property
 * of the region under test, and the per block term of the cost model is the same everywhere,
 * which is what a constant looks like. passing an address here moves the nodes and turns that
 * constant into a variable. zero restores the static storage.
 *
 * the memory has to hold QOS_STRESS_MAX_CHANNELS nodes, be 32 bit aligned, and sit in the same
 * 64 KiB block as itself, since the hardware carries only the low 16 bits of the next node
 * address in the link register. */
void qos_stress_descriptors(uint32_t addr, uint32_t bytes);
uint32_t qos_stress_descriptor_addr(void);

/* one block, once, timed by the caller.
 *
 * the sweeps measure what an aggressor costs a victim. this measures what the transfer itself
 * takes, with no victim in the window, which is the only way to ask whether a region is slower on
 * the DMA side without the victim's own access pattern in the answer. the caller owns the cycle
 * counter, because every other measured window in this project is taken by the measurement thread
 * and this one should not be different.
 *
 * start returns false when the configuration does not fit or the hardware refuses it. done returns
 * true once the transfer complete flag is set, and clears it. */
bool qos_stress_once_arm(uint32_t base, uint32_t span, const qos_stress_cfg_t *cfg);
bool qos_stress_once_fire(void);
bool qos_stress_once_pending(void);
bool qos_stress_once_settle(void);

/* the byte and transaction rates the configuration asks for, so a capture can be read against
 * what was requested rather than against what someone remembers requesting. */
uint32_t qos_stress_bytes_per_s(const qos_stress_cfg_t *cfg);
uint32_t qos_stress_xacts_per_s(const qos_stress_cfg_t *cfg);

#endif /* LAXITY_GPDMA_STRESS_H */
