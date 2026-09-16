// see gpdma_stress.h for why this is a second module and why it lives on channels 12 to 15.

#include "gpdma_stress.h"

#include "stm32u5xx_hal.h"

/* the 2D addressing channels, in the order they are taken. the channel count knob takes a prefix
 * of this list, so the one channel case is always channel 12 and the two channel case is always
 * 12 and 13, which keeps the comparison between counts from also being a comparison between
 * different channels. */
static DMA_Channel_TypeDef *const s_instances[QOS_STRESS_MAX_CHANNELS] = {
    GPDMA1_Channel12, GPDMA1_Channel13, GPDMA1_Channel14, GPDMA1_Channel15,
};

static DMA_HandleTypeDef s_ch[QOS_STRESS_MAX_CHANNELS];
static DMA_QListTypeDef  s_queue[QOS_STRESS_MAX_CHANNELS];
static DMA_NodeTypeDef   s_node_static[QOS_STRESS_MAX_CHANNELS];

/* the nodes the hardware reads. the queue beside them is a software structure the DMA never
 * fetches, so only this one moves when the descriptor region changes. */
static DMA_NodeTypeDef  *s_node = s_node_static;

static TIM_HandleTypeDef s_tim;

/* the linked list mode the channels are currently initialised in.
 *
 * HAL_DMAEx_List_LinkQ refuses a queue with no first circular node when the channel is in circular
 * mode, which is stm32u5xx_hal_dma_ex.c under "check circularity compatibility". the sweeps need
 * circular and the one shot needs normal, so the mode follows whichever is being set up. */
static uint32_t s_ll_mode = DMA_LINKEDLIST_CIRCULAR;

static uint32_t s_completions;
static uint8_t  s_active;
static bool     s_ready;
static bool     s_running;

/* TIM2 counts at the timer clock and raises TRGO on update, which is the trigger the channels
 * wait for. it is not otherwise used by this project and CubeMX does not initialise it, so
 * claiming it here takes nothing away. */
static uint32_t timer_hz(void)
{
    /* the APB1 timer clock is the bus clock when the APB prescaler is 1 and twice it otherwise.
     * both frequencies are read back from RCC rather than assumed from SYSCLK, so a clock tree
     * that did not take produces a wrong trigger rate visibly instead of silently. */
    uint32_t hclk = HAL_RCC_GetHCLKFreq();
    uint32_t pclk = HAL_RCC_GetPCLK1Freq();
    return (pclk == hclk) ? pclk : (pclk * 2u);
}

static bool trigger_start(uint32_t hz)
{
    TIM_MasterConfigTypeDef master = {0};
    uint32_t ticks;

    /* ungated and armed both leave TIM2 alone. ungated masks the trigger in the node so the channel
     * runs flat out, and armed selects a trigger from a timer this function never starts. */
    if (hz == 0u || hz == QOS_STRESS_TRIGGER_ARMED) {
        return true;
    }

    ticks = timer_hz() / hz;
    if (ticks < 2u) {
        return false;
    }

    __HAL_RCC_TIM2_CLK_ENABLE();
    s_tim.Instance               = TIM2;
    s_tim.Init.Prescaler         = 0u;
    s_tim.Init.CounterMode       = TIM_COUNTERMODE_UP;
    s_tim.Init.Period            = ticks - 1u;
    s_tim.Init.ClockDivision     = TIM_CLOCKDIVISION_DIV1;
    s_tim.Init.AutoReloadPreload = TIM_AUTORELOAD_PRELOAD_DISABLE;
    if (HAL_TIM_Base_Init(&s_tim) != HAL_OK) { return false; }

    master.MasterOutputTrigger = TIM_TRGO_UPDATE;
    master.MasterSlaveMode     = TIM_MASTERSLAVEMODE_DISABLE;
    if (HAL_TIMEx_MasterConfigSynchronization(&s_tim, &master) != HAL_OK) { return false; }

    // no interrupt is enabled on the timer. it exists to raise TRGO into the DMA trigger input,
    // and a handler would put the core back inside the measured window.
    return HAL_TIM_Base_Start(&s_tim) == HAL_OK;
}

static void trigger_stop(void)
{
    if (s_tim.Instance != NULL) {
        (void)HAL_TIM_Base_Stop(&s_tim);
        (void)HAL_TIM_Base_DeInit(&s_tim);
        s_tim.Instance = NULL;
    }
}

void qos_stress_descriptors(uint32_t addr, uint32_t bytes)
{
    if (addr == 0u || bytes < sizeof s_node_static || (addr & 3u) != 0u) {
        s_node = s_node_static;
        return;
    }
    s_node = (DMA_NodeTypeDef *)(uintptr_t)addr;
}

uint32_t qos_stress_descriptor_addr(void)
{
    return (uint32_t)(uintptr_t)s_node;
}

bool qos_stress_init(void)
{
    __HAL_RCC_GPDMA1_CLK_ENABLE();

    for (uint32_t i = 0u; i < QOS_STRESS_MAX_CHANNELS; ++i) {
        s_ch[i].Instance = s_instances[i];
        s_ch[i].InitLinkedList.Priority          = DMA_HIGH_PRIORITY;
        s_ch[i].InitLinkedList.LinkStepMode      = DMA_LSM_FULL_EXECUTION;
        s_ch[i].InitLinkedList.LinkAllocatedPort = DMA_LINK_ALLOCATED_PORT1;
        s_ch[i].InitLinkedList.TransferEventMode = DMA_TCEM_LAST_LL_ITEM_TRANSFER;
        s_ch[i].InitLinkedList.LinkedListMode    = DMA_LINKEDLIST_CIRCULAR;
        if (HAL_DMAEx_List_Init(&s_ch[i]) != HAL_OK) {
            s_ready = false;
            return false;
        }
    }

    s_ready = true;
    return true;
}

static bool ensure_ll_mode(uint32_t mode)
{
    if (s_ll_mode == mode) {
        return true;
    }
    for (uint32_t i = 0u; i < QOS_STRESS_MAX_CHANNELS; ++i) {
        s_ch[i].InitLinkedList.LinkedListMode = mode;
        if (HAL_DMAEx_List_Init(&s_ch[i]) != HAL_OK) {
            return false;
        }
    }
    s_ll_mode = mode;
    return true;
}

static uint32_t width_src(uint8_t width)
{
    return (width == 1u) ? DMA_SRC_DATAWIDTH_BYTE
         : (width == 2u) ? DMA_SRC_DATAWIDTH_HALFWORD
                         : DMA_SRC_DATAWIDTH_WORD;
}

static uint32_t width_dst(uint8_t width)
{
    return (width == 1u) ? DMA_DEST_DATAWIDTH_BYTE
         : (width == 2u) ? DMA_DEST_DATAWIDTH_HALFWORD
                         : DMA_DEST_DATAWIDTH_WORD;
}

uint32_t qos_stress_bytes_per_s(const qos_stress_cfg_t *cfg)
{
    if (cfg->trigger_hz == 0u || cfg->trigger_hz == QOS_STRESS_TRIGGER_ARMED) {
        /* ungated means the rate is whatever the matrix allows and is not declared. armed means
         * there is no traffic at all, and both are wrong to report as a number. */
        return 0u;
    }
    return cfg->block_bytes * cfg->trigger_hz * (uint32_t)cfg->channels;
}

uint32_t qos_stress_xacts_per_s(const qos_stress_cfg_t *cfg)
{
    if (cfg->trigger_hz == 0u || cfg->trigger_hz == QOS_STRESS_TRIGGER_ARMED || cfg->width == 0u) {
        return 0u;
    }
    return (cfg->block_bytes / cfg->width) * cfg->trigger_hz * (uint32_t)cfg->channels;
}

/* how far a block walks in address space, which is what has to fit in the buffer.
 *
 * with a stride the address advances by width + stride per transaction, so a block of the same
 * byte count covers more memory. the caller's window has to hold that reach and not merely the
 * byte count, and getting this wrong would have the aggressor writing outside its region while
 * every liveness check still passed. */
static uint32_t block_reach(const qos_stress_cfg_t *cfg)
{
    uint32_t xacts = cfg->block_bytes / cfg->width;
    return xacts * ((uint32_t)cfg->width + cfg->stride);
}

bool qos_stress_start(uint32_t base, uint32_t span, const qos_stress_cfg_t *cfg)
{
    uint32_t reach, per_channel;

    if (!s_ready || cfg == NULL) { return false; }
    if (cfg->channels == 0u || cfg->channels > QOS_STRESS_MAX_CHANNELS) { return false; }
    if (cfg->width != 1u && cfg->width != 2u && cfg->width != 4u) { return false; }
    if (cfg->block_bytes == 0u || (cfg->block_bytes % cfg->width) != 0u) { return false; }
    /* the hardware field is signed and bounded at 8191, and a value it would truncate has to be
     * refused rather than silently reduced to something nobody asked for. */
    if (cfg->stride > 8191u) { return false; }

    qos_stress_stop();
    if (!ensure_ll_mode(DMA_LINKEDLIST_CIRCULAR)) { return false; }

    reach = block_reach(cfg);
    /* two buffers per channel, source and destination, both inside the caller's window. */
    per_channel = 2u * reach;
    if (per_channel == 0u || (per_channel * cfg->channels) > span) { return false; }

    if (!trigger_start(cfg->trigger_hz)) { return false; }

    for (uint32_t i = 0u; i < cfg->channels; ++i) {
        DMA_NodeConfTypeDef node = {0};
        uint32_t src = base + (i * per_channel);
        uint32_t dst = src + reach;

        /* a 2D node in every case, including stride zero. keeping the node type constant across
         * the sweeps means the stride axis moves one field rather than changing the kind of
         * descriptor the channel executes. */
        node.NodeType = DMA_GPDMA_2D_NODE;

        node.Init.Request          = DMA_REQUEST_SW;
        node.Init.BlkHWRequest     = DMA_BREQ_SINGLE_BURST;
        node.Init.Direction        = DMA_MEMORY_TO_MEMORY;
        node.Init.SrcInc           = DMA_SINC_INCREMENTED;
        node.Init.DestInc          = DMA_DINC_INCREMENTED;
        node.Init.SrcDataWidth     = width_src(cfg->width);
        node.Init.DestDataWidth    = width_dst(cfg->width);
        node.Init.SrcBurstLength   = 1u;
        node.Init.DestBurstLength  = 1u;
        node.Init.TransferAllocatedPort = DMA_SRC_ALLOCATED_PORT0 | DMA_DEST_ALLOCATED_PORT1;
        node.Init.TransferEventMode = DMA_TCEM_BLOCK_TRANSFER;
        node.Init.Mode             = DMA_NORMAL;

        node.DataHandlingConfig.DataExchange  = DMA_EXCHANGE_NONE;
        node.DataHandlingConfig.DataAlignment = DMA_DATA_RIGHTALIGN_ZEROPADDED;

        if (cfg->trigger_hz != 0u) {
            /* one trigger releases one block, so the byte rate is block_bytes times the trigger
             * rate and the channel idles in between. that idle is the bandwidth knob. */
            node.TriggerConfig.TriggerMode      = DMA_TRIGM_BLOCK_TRANSFER;
            node.TriggerConfig.TriggerPolarity  = DMA_TRIG_POLARITY_RISING;
            node.TriggerConfig.TriggerSelection = GPDMA1_TRIGGER_TIM2_TRGO;
        } else {
            node.TriggerConfig.TriggerMode      = DMA_TRIGM_BLOCK_TRANSFER;
            node.TriggerConfig.TriggerPolarity  = DMA_TRIG_POLARITY_MASKED;
            node.TriggerConfig.TriggerSelection = GPDMA1_TRIGGER_TIM2_TRGO;
        }

        /* the address offset applies after every single transfer, which is what makes this a per
         * transaction stride rather than a per block one. */
        node.RepeatBlockConfig.RepeatCount       = 1u;
        node.RepeatBlockConfig.SrcAddrOffset     = (int32_t)cfg->stride;
        node.RepeatBlockConfig.DestAddrOffset    = (int32_t)cfg->stride;
        node.RepeatBlockConfig.BlkSrcAddrOffset  = 0;
        node.RepeatBlockConfig.BlkDestAddrOffset = 0;

        node.SrcAddress = src;
        node.DstAddress = dst;
        node.DataSize   = cfg->block_bytes;

        if (HAL_DMAEx_List_BuildNode(&node, &s_node[i]) != HAL_OK) { goto fail; }
        if (HAL_DMAEx_List_InsertNode_Tail(&s_queue[i], &s_node[i]) != HAL_OK) { goto fail; }
        if (HAL_DMAEx_List_SetCircularMode(&s_queue[i]) != HAL_OK) { goto fail; }
        if (HAL_DMAEx_List_LinkQ(&s_ch[i], &s_queue[i]) != HAL_OK) { goto fail; }
        if (HAL_DMAEx_List_Start(&s_ch[i]) != HAL_OK) { goto fail; }

        s_active = (uint8_t)(i + 1u);
    }

    s_running = true;
    return true;

fail:
    qos_stress_stop();
    return false;
}

void qos_stress_stop(void)
{
    for (uint32_t i = 0u; i < QOS_STRESS_MAX_CHANNELS; ++i) {
        if (i < s_active) {
            (void)HAL_DMA_Abort(&s_ch[i]);
            (void)HAL_DMAEx_List_UnLinkQ(&s_ch[i]);
        }
        // the queue is reset rather than reused, since a second start would otherwise append a
        // node to a list that already loops back on itself.
        (void)HAL_DMAEx_List_ResetQ(&s_queue[i]);
    }
    s_active = 0u;
    s_running = false;
    trigger_stop();
}

uint32_t qos_stress_completions(void)
{
    for (uint32_t i = 0u; i < s_active; ++i) {
        if (__HAL_DMA_GET_FLAG(&s_ch[i], DMA_FLAG_TC) != 0u) {
            __HAL_DMA_CLEAR_FLAG(&s_ch[i], DMA_FLAG_TC);
            s_completions++;
        }
    }
    return s_completions;
}

bool qos_stress_running(void)
{
    return s_running;
}

bool qos_stress_once_arm(uint32_t base, uint32_t span, const qos_stress_cfg_t *cfg)
{
    DMA_NodeConfTypeDef node = {0};
    uint32_t reach;

    if (!s_ready || cfg == NULL) { return false; }
    if (cfg->width != 1u && cfg->width != 2u && cfg->width != 4u) { return false; }
    if (cfg->block_bytes == 0u || (cfg->block_bytes % cfg->width) != 0u) { return false; }

    /* a sweep leaves running channels behind and those are torn down the usual way. an earlier one
     * shot leaves a channel that has already finished, and asking that one to abort is what breaks
     * it: HAL_DMA_Abort suspends and waits for the acknowledge, a finished channel never sends one,
     * and the call returns after its timeout with the handle in a state nothing will start again.
     * unlink and reset need the ready state that settle() leaves behind, and neither is an abort. */
    if (s_running) { qos_stress_stop(); }
    if (!ensure_ll_mode(DMA_LINKEDLIST_NORMAL)) { return false; }
    (void)HAL_DMAEx_List_UnLinkQ(&s_ch[0]);
    (void)HAL_DMAEx_List_ResetQ(&s_queue[0]);

    reach = cfg->block_bytes;
    if ((2u * reach) > span) { return false; }

    /* one channel, one node, no trigger and no circular link, so the channel executes exactly one
     * block and stops. everything else matches what the sweeps configure, since a difference in
     * burst length or port allocation would make the two measurements incomparable. */
    node.NodeType = DMA_GPDMA_2D_NODE;
    node.Init.Request          = DMA_REQUEST_SW;
    node.Init.BlkHWRequest     = DMA_BREQ_SINGLE_BURST;
    node.Init.Direction        = DMA_MEMORY_TO_MEMORY;
    node.Init.SrcInc           = DMA_SINC_INCREMENTED;
    node.Init.DestInc          = DMA_DINC_INCREMENTED;
    node.Init.SrcDataWidth     = width_src(cfg->width);
    node.Init.DestDataWidth    = width_dst(cfg->width);
    node.Init.SrcBurstLength   = 1u;
    node.Init.DestBurstLength  = 1u;
    node.Init.TransferAllocatedPort = DMA_SRC_ALLOCATED_PORT0 | DMA_DEST_ALLOCATED_PORT1;
    node.Init.TransferEventMode = DMA_TCEM_BLOCK_TRANSFER;
    node.Init.Mode             = DMA_NORMAL;
    node.DataHandlingConfig.DataExchange  = DMA_EXCHANGE_NONE;
    node.DataHandlingConfig.DataAlignment = DMA_DATA_RIGHTALIGN_ZEROPADDED;
    node.TriggerConfig.TriggerMode      = DMA_TRIGM_BLOCK_TRANSFER;
    node.TriggerConfig.TriggerPolarity  = DMA_TRIG_POLARITY_MASKED;
    node.TriggerConfig.TriggerSelection = GPDMA1_TRIGGER_TIM2_TRGO;
    node.RepeatBlockConfig.RepeatCount       = 1u;
    node.RepeatBlockConfig.SrcAddrOffset     = 0;
    node.RepeatBlockConfig.DestAddrOffset    = 0;
    node.RepeatBlockConfig.BlkSrcAddrOffset  = 0;
    node.RepeatBlockConfig.BlkDestAddrOffset = 0;
    node.SrcAddress = base;
    node.DstAddress = base + reach;
    node.DataSize   = cfg->block_bytes;

    if (HAL_DMAEx_List_BuildNode(&node, &s_node[0]) != HAL_OK) { return false; }
    if (HAL_DMAEx_List_InsertNode_Tail(&s_queue[0], &s_node[0]) != HAL_OK) { return false; }
    if (HAL_DMAEx_List_LinkQ(&s_ch[0], &s_queue[0]) != HAL_OK) { return false; }

    /* s_active stays zero through all of this. it is what qos_stress_completions() walks, and that
     * runs in the exporter thread, so a non zero count here would have a second thread clearing the
     * transfer complete flag this measurement is waiting for. */
    return true;
}

bool qos_stress_once_fire(void)
{
    return HAL_DMAEx_List_Start(&s_ch[0]) == HAL_OK;
}

bool qos_stress_once_pending(void)
{
    /* read without clearing. the flag is what settle() hands to the HAL, and a poll that consumed
     * it would leave the state machine waiting for something that already happened. */
    return __HAL_DMA_GET_FLAG(&s_ch[0], DMA_FLAG_TC) != 0u;
}

bool qos_stress_once_settle(void)
{
    /* the supported way back to the ready state after a polled transfer. it clears the flags and
     * updates the handle, which is what makes the next arm work. */
    if (HAL_DMA_PollForTransfer(&s_ch[0], HAL_DMA_FULL_TRANSFER, 5u) != HAL_OK) {
        return false;
    }
    s_completions++;
    return true;
}
