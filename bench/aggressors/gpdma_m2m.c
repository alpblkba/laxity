// see gpdma_m2m.h for why this is a DMA channel and not a thread. 

#include "gpdma_m2m.h"

#include "stm32u5xx_hal.h"

static DMA_HandleTypeDef s_ch;
static DMA_QListTypeDef  s_queue;
static DMA_NodeTypeDef   s_node;
static uint32_t          s_completions;
static bool              s_running;
static bool              s_ready;

bool qos_gpdma_m2m_init(void)
{
    __HAL_RCC_GPDMA1_CLK_ENABLE();

    s_ch.Instance = GPDMA1_Channel0;

    // DMA_HIGH_PRIORITY biases the matrix arbiter toward the channel. an aggressor set to yield would make a null result unreadable, since it could mean the matrix has headroom or it could mean the aggressor was too polite to take any. priority is itself a knob, s o a placement policy could later lower it rather than move an arena.
    s_ch.InitLinkedList.Priority          = DMA_HIGH_PRIORITY;
    s_ch.InitLinkedList.LinkStepMode      = DMA_LSM_FULL_EXECUTION;
    s_ch.InitLinkedList.LinkAllocatedPort = DMA_LINK_ALLOCATED_PORT1;
    s_ch.InitLinkedList.TransferEventMode = DMA_TCEM_LAST_LL_ITEM_TRANSFER;
    s_ch.InitLinkedList.LinkedListMode    = DMA_LINKEDLIST_CIRCULAR;

    // no interrupt is enabled anywhere in this module. the completion count is polled instead, so nothing this channel does can land inside a measured window.
    s_ready = (HAL_DMAEx_List_Init(&s_ch) == HAL_OK);
    return s_ready;
}

bool qos_gpdma_m2m_start(uint32_t src, uint32_t dst, uint32_t bytes)
{
    DMA_NodeConfTypeDef cfg = {0};

    if (!s_ready || bytes == 0u) {
        return false;
    }

    qos_gpdma_m2m_stop();

    cfg.NodeType = DMA_GPDMA_LINEAR_NODE;
    cfg.Init.Request       = DMA_REQUEST_SW;
    cfg.Init.BlkHWRequest  = DMA_BREQ_SINGLE_BURST;
    cfg.Init.Direction     = DMA_MEMORY_TO_MEMORY;
    cfg.Init.SrcInc        = DMA_SINC_INCREMENTED;
    cfg.Init.DestInc       = DMA_DINC_INCREMENTED;
    cfg.Init.SrcDataWidth  = DMA_SRC_DATAWIDTH_WORD;
    cfg.Init.DestDataWidth = DMA_DEST_DATAWIDTH_WORD;
    cfg.Init.SrcBurstLength  = 1u;
    cfg.Init.DestBurstLength = 1u;

    // source on one matrix port and destination on the other, so a pass reads and writes at the same time rather than alternating on one port. that is the shape that actually competes with the core for the region under test.
    cfg.Init.TransferAllocatedPort = DMA_SRC_ALLOCATED_PORT0 | DMA_DEST_ALLOCATED_PORT1;

    // the flag is raised at the end of every block rather than only at the end of the list, so polling it sees progress within a pass instead of once per lap. 
    cfg.Init.TransferEventMode = DMA_TCEM_BLOCK_TRANSFER;
    cfg.Init.Mode              = DMA_NORMAL;

    cfg.SrcAddress = src;
    cfg.DstAddress = dst;
    cfg.DataSize   = bytes;

    if (HAL_DMAEx_List_BuildNode(&cfg, &s_node) != HAL_OK) { return false; }
    if (HAL_DMAEx_List_InsertNode_Tail(&s_queue, &s_node) != HAL_OK) { return false; }
    if (HAL_DMAEx_List_SetCircularMode(&s_queue) != HAL_OK) { return false; }
    if (HAL_DMAEx_List_LinkQ(&s_ch, &s_queue) != HAL_OK) { return false; }
    if (HAL_DMAEx_List_Start(&s_ch) != HAL_OK) { return false; }

    s_running = true;
    return true;
}

void qos_gpdma_m2m_stop(void)
{
    if (s_running) {
        (void)HAL_DMA_Abort(&s_ch);
        (void)HAL_DMAEx_List_UnLinkQ(&s_ch);
        s_running = false;
    }
    // the queue is reset rather than reused, since a second start would otherwise append a node to a list that already loops back on itself. 
    (void)HAL_DMAEx_List_ResetQ(&s_queue);
}

uint32_t qos_gpdma_m2m_completions(void)
{
    if (s_running && __HAL_DMA_GET_FLAG(&s_ch, DMA_FLAG_TC) != 0u) {
        __HAL_DMA_CLEAR_FLAG(&s_ch, DMA_FLAG_TC);
        s_completions++;
    }
    return s_completions;
}

bool qos_gpdma_m2m_running(void)
{
    return s_running;
}
