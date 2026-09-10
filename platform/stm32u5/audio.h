/* the board's second digital microphone as a continuous PCM source.
 *
 * MDF1 filter 0 takes the bitstream on PB1 and clocks the part from PF10, which is the wiring
 * the board support package already describes. capture runs into a double buffer forever and a
 * level is computed over each half as it completes, so the status line can show that the part is
 * alive without anything having to decode audio.
 *
 * nothing here raises an interrupt. the board support package enables the channel's line in its
 * MSP and this module disables it again, for the same reason the mem2mem aggressor has no
 * handler: a service routine that fires every half buffer would land inside some measured
 * inference windows and add cycles that have nothing to do with the bus.
 */
#ifndef LAXITY_AUDIO_H
#define LAXITY_AUDIO_H

#include <stdbool.h>
#include <stdint.h>

/* 16 kHz is the sample rate asked of the board support package. the part actually runs at
 * 2.857 MHz / 176, or 16.23 kHz, because the decimation ratio is an integer and the table in the
 * board support package picks the nearest one. that is the rate the buffers fill at. */
#define LAXITY_AUDIO_RATE_HZ  16000u

/* one half is 512 samples, about 32 ms of sound. mono 16 bit at this rate is 32 KB/s, which is
 * three orders of magnitude under what the mem2mem channel moves. */
#define LAXITY_AUDIO_HALF     512u
#define LAXITY_AUDIO_SAMPLES  (2u * LAXITY_AUDIO_HALF)
#define LAXITY_AUDIO_BYTES    (LAXITY_AUDIO_SAMPLES * 2u)

bool qos_audio_init(void);

/* fold any half that has completed since the last call into the level. returns true when one
 * had. cheap to call often and safe to call from outside a measured window only, since it reads
 * 512 samples. */
bool qos_audio_poll(void);

/* halves folded in since init. a count that does not advance is a stalled capture, which is the
 * same evidence the aggressor's completion count gives. */
uint32_t qos_audio_buffers(void);

/* largest absolute sample in the last completed half, 0 to 32767. */
int32_t qos_audio_peak(void);

bool qos_audio_running(void);

#endif /* LAXITY_AUDIO_H */
