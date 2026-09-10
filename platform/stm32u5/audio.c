// see audio.h for why this capture has no interrupt handler.

#include "audio.h"

#include "b_u585i_iot02a_audio.h"
#include "stm32u5xx_hal.h"

/* the DMA writes this and the core reads it. internal SRAM is outside what DCACHE1 covers on
 * this part, so there is no maintenance to do between the two. */
static int16_t  s_pcm[LAXITY_AUDIO_SAMPLES] __attribute__((aligned(4)));
static uint32_t s_buffers;
static int32_t  s_peak;
static bool     s_running;

static void laxity_audio_level(const int16_t *samples)
{
    int32_t peak = 0;

    for (uint32_t i = 0u; i < LAXITY_AUDIO_HALF; ++i) {
        int32_t v = samples[i];
        if (v < 0) { v = -v; }
        if (v > peak) { peak = v; }
    }

    s_peak = peak;
    s_buffers++;
}

bool qos_audio_init(void)
{
    BSP_AUDIO_Init_t cfg;

    cfg.Device        = AUDIO_IN_DEVICE_DIGITAL_MIC2;
    cfg.SampleRate    = AUDIO_FREQUENCY_16K;
    cfg.BitsPerSample = AUDIO_RESOLUTION_16B;
    cfg.ChannelsNbr   = 1u;
    cfg.Volume        = 100u;

    if (BSP_AUDIO_IN_Init(0u, &cfg) != BSP_ERROR_NONE) {
        return false;
    }

    // the line is enabled inside the MSP that the init above runs, and the startup file's default
    // handler for it is an endless loop. it is disabled here rather than after the record below,
    // because between these two calls the channel is idle and cannot raise it.
    HAL_NVIC_DisableIRQ(GPDMA1_Channel0_IRQn);

    if (BSP_AUDIO_IN_Record(0u, (uint8_t *)s_pcm, LAXITY_AUDIO_BYTES) != BSP_ERROR_NONE) {
        return false;
    }

    s_running = true;
    return true;
}

bool qos_audio_poll(void)
{
    bool folded = false;

    if (!s_running) {
        return false;
    }

    // the flags are read straight from the channel rather than through a callback, so a half that
    // completed while the core was inside a measured window is picked up afterwards instead of
    // interrupting it. both can be set on one call when polling was late, and both are honoured.
    if (__HAL_DMA_GET_FLAG(&haudio_mdf[1], DMA_FLAG_HT) != 0u) {
        __HAL_DMA_CLEAR_FLAG(&haudio_mdf[1], DMA_FLAG_HT);
        laxity_audio_level(&s_pcm[0]);
        folded = true;
    }

    if (__HAL_DMA_GET_FLAG(&haudio_mdf[1], DMA_FLAG_TC) != 0u) {
        __HAL_DMA_CLEAR_FLAG(&haudio_mdf[1], DMA_FLAG_TC);
        laxity_audio_level(&s_pcm[LAXITY_AUDIO_HALF]);
        folded = true;
    }

    return folded;
}

uint32_t qos_audio_buffers(void) { return s_buffers; }
int32_t  qos_audio_peak(void)    { return s_peak; }
bool     qos_audio_running(void) { return s_running; }
