/* USER CODE BEGIN Header */
/**
  ******************************************************************************
  * @file    app_threadx.c
  * @author  MCD Application Team
  * @brief   ThreadX applicative file
  ******************************************************************************
    * @attention
  *
  * Copyright (c) 2026 STMicroelectronics.
  * All rights reserved.
  *
  * This software is licensed under terms that can be found in the LICENSE file
  * in the root directory of this software component.
  * If no LICENSE file comes with this software, it is provided AS-IS.
  *
  ******************************************************************************
  */
/* USER CODE END Header */

/* Includes ------------------------------------------------------------------*/
#include "app_threadx.h"

/* Private includes ----------------------------------------------------------*/
/* USER CODE BEGIN Includes */
#include "main.h"
#include "counters_dwt.h"
#include "memory_map.h"
#include "null_probe.h"
#include "gpdma_m2m.h"
#include "qos/telemetry.h"
#include "net_udp_export.h"
#include "network.h"
#include "network_data.h"
#include "golden_st_ign_wl_24.h"
#include <stdio.h>
#include <string.h>
#include <math.h>

extern UART_HandleTypeDef huart1;

/* USER CODE END Includes */

/* Private typedef -----------------------------------------------------------*/
/* USER CODE BEGIN PTD */

/* USER CODE END PTD */

/* Private define ------------------------------------------------------------*/
/* USER CODE BEGIN PD */
/* snprintf through newlib takes several hundred bytes of stack on its own, and a stack or pool shortfall here shows up as a silent UART rather than as an error, so the margin is bought up front. both stacks come out of TX_APP_MEM_POOL_SIZE, which the .ioc raised from 4096 to 8192 for the second thread. */
#define LAXITY_INFER_STACK   3072
#define LAXITY_INFER_PRIO    15
#define LAXITY_EXPORT_STACK  2048
#define LAXITY_EXPORT_PRIO   20

/* below the driver's own threads and above the exporter, so joining a network never delays a
 * measurement and never starves the thing draining the ring. */
#define LAXITY_NET_STACK     2048
#define LAXITY_NET_PRIO      12

/* lower priority than the measurement thread, so a blocking transmit that takes several milliseconds at 921600 baud cannot delay an inference. that is what best effort export means here. */

/* a header frame with four placement entries is 136 bytes, so 512 still carries eleven records per batch against a ring of thirty two and a producer at fifty hertz. */
#define LAXITY_EXPORT_BUF    512

/* what the .ioc configures, reported in the header frame beside the rate qos_dwt_measure_hz() actually observed, since a capture has to be able to show a clock configuration that did not take. */
#define LAXITY_SYSCLK_HZ     160000000u

#define LAXITY_ARENA_BYTES   STAI_NETWORK_ACTIVATIONS_SIZE_BYTES

/* every arena starts on the same 4 KB boundary, so offset within a region is held constant and the region is the only thing that differs between the four labels. 4096 is a bound rather than a measured bank size: the SRAM bank structure is in RM0456, which is the one thing this block could not source, so the alignment is chosen large enough to cover any plausible granularity instead of matching a known one. */
#define LAXITY_ARENA_ALIGN   4096u

/* one reservation that reaches from wherever bss puts it up to SRAM3, so all three arenas come out of memory the linker actually allocated. sized for the worst case of starting at the base of SRAM1; starting later only makes it reach further.
 *
 * the alternative was a linker script edit, and it was rejected on a real failure mode rather than on taste. the generated script maps SRAM1, SRAM2 and SRAM3 as one 768K RAM region with _estack at its top, so placing sections in the upper two means either overlapping MEMORY regions, which GNU ld does not check for collisions between, or splitting RAM, which moves the stack out of SRAM3 and changes the vendor default layout every comparison is made against. one C object cannot overlap anything, because the linker allocated it. */
/* the largest buffer the aggressor cycles through, per buffer. two of them plus the arena have
 * to fit in the smallest region, and SRAM2 is 64 KB, so 16 KB each leaves room to spare. */
#define LAXITY_AGGR_MAX      (16u * 1024u)

/* one region's worth of the reservation: the arena on its own alignment, then the two aggressor
 * buffers after it. */
#define LAXITY_REGION_SLICE  (LAXITY_ARENA_ALIGN + 2u * LAXITY_AGGR_MAX)

#define LAXITY_SPAN_BYTES    ((QOS_SRAM3_BASE - QOS_SRAM1_BASE) + LAXITY_REGION_SLICE + LAXITY_ARENA_ALIGN)

/* repetitions of the whole cross per pass. the schedule reshuffles and repeats, so the sample
 * count per cell comes from how long the capture runs rather than from this number, and a small
 * value keeps one pass short enough that drift is spread across cells rather than within one. */
#define LAXITY_REPS          8u
/* the cross this binary measures: three arena regions against three aggressor regions at four
 * footprints, plus an aggressor off column for each arena region, plus the same buffer control.
 * 3*3*4 + 3 + 1 comes to 40. */
#define LAXITY_ARENAS        5u   /* the three regions, then SRAM2 at a second address, then SRAM1 again */
#define LAXITY_AGGR_REGIONS  3u
#define LAXITY_FOOTPRINTS    4u
#define LAXITY_CELLS         (3u * LAXITY_AGGR_REGIONS * LAXITY_FOOTPRINTS + LAXITY_ARENAS)
#define LAXITY_LABELS        LAXITY_ARENAS
#define LAXITY_SCHEDULE      (LAXITY_REPS * LAXITY_CELLS)

/* discarded, not recorded. the first inferences after reset pull the vendor kernels through flash into ICACHE, and a cold first sample would land wherever the shuffle happened to put it. */
#define LAXITY_WARMUP        32u

/* fifty hertz. the exporter drains eleven records every ten milliseconds, so the ring never fills, and one pass over the schedule takes about ten seconds. */
#define LAXITY_INFER_PERIOD  (TX_TIMER_TICKS_PER_SECOND / 50u)

/* USER CODE END PD */

/* Private macro -------------------------------------------------------------*/
/* USER CODE BEGIN PM */

/* USER CODE END PM */

/* Private variables ---------------------------------------------------------*/
/* USER CODE BEGIN PV */
static TX_THREAD    laxity_infer_thread;
static TX_THREAD    laxity_export_thread;
static TX_THREAD    laxity_net_thread;

/* the exporter waits on this rather than sleeping for a plausible interval. the null probe pushes records and resets the ring, so a consumer that started draining during boot would both corrupt the probe and put its synthetic records on the wire. */
static TX_SEMAPHORE laxity_boot_done;

static uint8_t laxity_export_buf[LAXITY_EXPORT_BUF];

/* the address reservation described in the PD block. aligned so every arena carved out of it starts on the same boundary. */
static uint8_t laxity_arena_span[LAXITY_SPAN_BYTES] __attribute__((aligned(LAXITY_ARENA_ALIGN)));

static uint8_t laxity_net_ctx[STAI_NETWORK_CONTEXT_SIZE] __attribute__((aligned(STAI_NETWORK_CONTEXT_ALIGNMENT)));
static float   laxity_out[LAXITY_GOLDEN_OUT_LEN];

/* one entry per placement label, in the order the schedule draws from. the control is the same arena address as SRAM1 under a second id, which is what makes the spread between them a noise floor rather than a comparison. */
static qos_placement_t laxity_placements[LAXITY_LABELS];
static uint8_t        *laxity_arena[LAXITY_LABELS];

/* one source and one destination per aggressor region, both inside that region, so the traffic
 * is region local and the only thing crossing regions is the arbitration. */
static uint8_t        *laxity_aggr_src[LAXITY_AGGR_REGIONS];
static uint8_t        *laxity_aggr_dst[LAXITY_AGGR_REGIONS];

/* bytes per aggressor buffer. the smallest is well under any buffering on the path and the
 * largest is bounded by SRAM2, which is 64 KB and has to hold an arena and both buffers. */
static const uint32_t  laxity_footprints[LAXITY_FOOTPRINTS] = { 1024u, 4096u, 8192u, 16384u };

/* the cross, built at start up rather than written out by hand. */
static struct {
  uint8_t arena_slot;
  uint8_t aggr_region;   /* 0 when the aggressor is off */
  uint8_t foot_idx;
} laxity_cells[LAXITY_CELLS];

static uint8_t  laxity_aggr_ok;
static uint32_t laxity_cur_aggr = 0xFFFFFFFFu;
static uint16_t        laxity_schedule[LAXITY_SCHEDULE];
static uint8_t         laxity_placed_ok;

/* written once during boot and read by the exporter after the semaphore, which publishes it. */
static struct {
  uint32_t cyccnt_hz;
  qos_dwt_caps_t caps;
  qos_null_probe_t probe;
} laxity_boot;

/* written by the measurement thread every iteration and read by the exporter. the fields are word sized and the line is diagnostic, so a torn read costs one stale status line and never a record. */
static volatile struct {
  int32_t  rc;
  uint32_t cycles;
  int32_t  argmax;
  uint32_t worst_ppm;
  uint32_t mismatches;
  uint32_t passes;
  uint32_t aggr_completions;
  int32_t  net_rc;
  uint32_t net_addr;
  uint32_t net_sent;
  uint32_t net_failed;
} laxity_live;

/* USER CODE END PV */

/* Private function prototypes -----------------------------------------------*/
/* USER CODE BEGIN PFP */
static VOID laxity_infer_entry(ULONG argument);
static VOID laxity_export_entry(ULONG argument);
static VOID laxity_net_entry(ULONG argument);

/* USER CODE END PFP */

/**
  * @brief  Application ThreadX Initialization.
  * @param memory_ptr: memory pointer
  * @retval int
  */
UINT App_ThreadX_Init(VOID *memory_ptr)
{
  UINT ret = TX_SUCCESS;
  /* USER CODE BEGIN App_ThreadX_MEM_POOL */
  TX_BYTE_POOL *byte_pool = (TX_BYTE_POOL *)memory_ptr;

  /* USER CODE END App_ThreadX_MEM_POOL */
  /* USER CODE BEGIN App_ThreadX_Init */
  /* the stacks come from the ThreadX byte pool rather than static arrays,
     since the pool is the allocation CubeMX sizes and reports. */
  CHAR *infer_stack;
  CHAR *export_stack;

  if (tx_semaphore_create(&laxity_boot_done, "laxity_boot", 0) != TX_SUCCESS)
  {
    return TX_SEMAPHORE_ERROR;
  }

  if (tx_byte_allocate(byte_pool, (VOID **)&infer_stack, LAXITY_INFER_STACK,
                       TX_NO_WAIT) != TX_SUCCESS)
  {
    return TX_POOL_ERROR;
  }

  if (tx_byte_allocate(byte_pool, (VOID **)&export_stack, LAXITY_EXPORT_STACK,
                       TX_NO_WAIT) != TX_SUCCESS)
  {
    return TX_POOL_ERROR;
  }

  if (tx_thread_create(&laxity_infer_thread, "laxity_infer", laxity_infer_entry, 0,
                       infer_stack, LAXITY_INFER_STACK,
                       LAXITY_INFER_PRIO, LAXITY_INFER_PRIO,
                       TX_NO_TIME_SLICE, TX_AUTO_START) != TX_SUCCESS)
  {
    return TX_THREAD_ERROR;
  }

  if (tx_thread_create(&laxity_export_thread, "laxity_export", laxity_export_entry, 0,
                       export_stack, LAXITY_EXPORT_STACK,
                       LAXITY_EXPORT_PRIO, LAXITY_EXPORT_PRIO,
                       TX_NO_TIME_SLICE, TX_AUTO_START) != TX_SUCCESS)
  {
    return TX_THREAD_ERROR;
  }

  {
    CHAR *net_stack;
    if (tx_byte_allocate(byte_pool, (VOID **)&net_stack, LAXITY_NET_STACK,
                         TX_NO_WAIT) != TX_SUCCESS)
    {
      return TX_POOL_ERROR;
    }
    if (tx_thread_create(&laxity_net_thread, "laxity_net", laxity_net_entry, 0,
                         net_stack, LAXITY_NET_STACK,
                         LAXITY_NET_PRIO, LAXITY_NET_PRIO,
                         TX_NO_TIME_SLICE, TX_AUTO_START) != TX_SUCCESS)
    {
      return TX_THREAD_ERROR;
    }
  }
  /* USER CODE END App_ThreadX_Init */

  return ret;
}

  /**
  * @brief  Function that implements the kernel's initialization.
  * @param  None
  * @retval None
  */
void MX_ThreadX_Init(void)
{
  /* USER CODE BEGIN Before_Kernel_Start */

  /* USER CODE END Before_Kernel_Start */

  tx_kernel_enter();

  /* USER CODE BEGIN Kernel_Start_Error */

  /* USER CODE END Kernel_Start_Error */
}

/* USER CODE BEGIN 1 */
static stai_network *laxity_net;

/* offset value meaning "the lowest address in this region the span can serve", which is what SRAM1 needs because the bottom of SRAM1 holds the vector table, .data and the rest of .bss. */
#define LAXITY_OFF_FIRST  0xFFFFFFFFu

/* carve one arena out of the span, at a given offset into the named region, on the shared alignment.
 *
 * returns NULL rather than a plausible pointer when the request cannot be served, because a section attribute that silently does not apply and a buffer that landed in the wrong bank look identical in the results. the caller refuses to measure in that case. */
static uint8_t *laxity_carve(uint32_t region_base, uint32_t region_size, uint32_t offset)
{
  uintptr_t lo = (uintptr_t)laxity_arena_span;
  uintptr_t hi = lo + sizeof laxity_arena_span;
  uintptr_t a;

  if (offset == LAXITY_OFF_FIRST) {
    a = (region_base > lo) ? (uintptr_t)region_base : lo;
  } else {
    a = (uintptr_t)region_base + offset;
  }

  a = (a + (LAXITY_ARENA_ALIGN - 1u)) & ~(uintptr_t)(LAXITY_ARENA_ALIGN - 1u);

  if (a < lo || (a + LAXITY_ARENA_BYTES) > hi) { return NULL; }
  if (a < region_base || (a + LAXITY_ARENA_BYTES) > ((uintptr_t)region_base + region_size)) { return NULL; }
  return (uint8_t *)a;
}

static void laxity_name(char *dst, const char *src)
{
  int i;
  for (i = 0; i < 8; ++i) { dst[i] = (src[i] != '\0') ? src[i] : '\0'; if (src[i] == '\0') { break; } }
  for (; i < 8; ++i) { dst[i] = '\0'; }
}

/* the labels this run measures. two of them are controls of different kinds: SRAM1c is the same
 * bytes as SRAM1 under a second id, which gives the noise floor, and SRAM2b is SRAM2 at a second
 * address, which says whether a difference belongs to the region or to the address. */
static const struct {
  uint8_t     id;
  uint8_t     region;
  uint32_t    offset;
  uint8_t     flags;
  const char *name;
} laxity_labels[LAXITY_LABELS] = {
  { QOS_REGION_SRAM1,         QOS_REGION_SRAM1, LAXITY_OFF_FIRST, 0u,                      "SRAM1"  },
  { QOS_REGION_SRAM2,         QOS_REGION_SRAM2, 0x00000u,         0u,                      "SRAM2"  },
  { QOS_REGION_SRAM3,         QOS_REGION_SRAM3, 0x00000u,         0u,                      "SRAM3"  },
  /* past the aggressor buffers, which occupy one arena alignment plus two maximum footprints
   * from the base of every region. */
  { QOS_REGION_SRAM2_ALT,     QOS_REGION_SRAM2, 0x09000u,         QOS_PLACEMENT_ALT_ADDR,  "SRAM2b" },
  { QOS_REGION_SRAM1_CONTROL, QOS_REGION_SRAM1, LAXITY_OFF_FIRST, QOS_PLACEMENT_CONTROL,   "SRAM1c" },
};

/* fill the placement table and prove every arena landed where it was meant to. */
static uint8_t laxity_place(void)
{
  for (uint32_t i = 0u; i < LAXITY_LABELS; ++i)
  {
    const qos_mem_region_t *r = qos_mem_region(laxity_labels[i].region);
    if (r == NULL) { return 0u; }

    laxity_arena[i] = laxity_carve(r->base, r->size, laxity_labels[i].offset);
    if (laxity_arena[i] == NULL) { return 0u; }

    laxity_placements[i].id = laxity_labels[i].id;
    laxity_placements[i].flags = laxity_labels[i].flags;
    laxity_placements[i].rel_cost = r->rel_cost;
    laxity_placements[i].arena_addr = (uint32_t)(uintptr_t)laxity_arena[i];
    laxity_placements[i].arena_size = LAXITY_ARENA_BYTES;
    laxity_name(laxity_placements[i].name, laxity_labels[i].name);
  }

  /* the aggressor buffers sit after the arena inside the same region. */
  for (uint32_t r = 0u; r < LAXITY_AGGR_REGIONS; ++r)
  {
    const qos_mem_region_t *reg = qos_mem_region((uint8_t)(QOS_REGION_SRAM1 + r));
    uintptr_t base;
    if (reg == NULL) { return 0u; }
    base = (uintptr_t)laxity_arena[r] + LAXITY_ARENA_ALIGN;
    laxity_aggr_src[r] = (uint8_t *)base;
    laxity_aggr_dst[r] = (uint8_t *)(base + LAXITY_AGGR_MAX);
    if ((base + 2u * LAXITY_AGGR_MAX) > ((uintptr_t)reg->base + reg->size)) { return 0u; }
    if ((base + 2u * LAXITY_AGGR_MAX) > ((uintptr_t)laxity_arena_span + sizeof laxity_arena_span)) { return 0u; }
  }

  /* the control has to be the same bytes as SRAM1, not merely the same region, or the spread between them would measure a second buffer rather than the noise floor. */
  if (laxity_arena[4] != laxity_arena[0]) { return 0u; }
  /* and the alternate address has to be a different one, or it would measure nothing. */
  if (laxity_arena[3] == laxity_arena[1]) { return 0u; }
  return 1u;
}

static void laxity_build_cells(void)
{
  uint32_t n = 0u;
  for (uint32_t a = 0u; a < LAXITY_ARENAS; ++a)
  {
    /* every arena label gets an aggressor off cell, which is what the contended numbers are
     * compared against and what the control measures its noise floor in. */
    laxity_cells[n].arena_slot = (uint8_t)a;
    laxity_cells[n].aggr_region = 0u;
    laxity_cells[n].foot_idx = 0u;
    ++n;

    /* the control is only ever measured uncontended, since its job is the noise floor. */
    if (a >= LAXITY_AGGR_REGIONS) { continue; }

    for (uint32_t r = 0u; r < LAXITY_AGGR_REGIONS; ++r)
    {
      for (uint32_t f = 0u; f < LAXITY_FOOTPRINTS; ++f)
      {
        laxity_cells[n].arena_slot = (uint8_t)a;
        laxity_cells[n].aggr_region = (uint8_t)(QOS_REGION_SRAM1 + r);
        laxity_cells[n].foot_idx = (uint8_t)f;
        ++n;
      }
    }
  }
}

/* switch the aggressor only when the cell asks for something different, since a stop and start
 * costs more than the comparison does and the schedule repeats cells. */
static void laxity_set_aggressor(uint8_t region, uint8_t foot_idx)
{
  uint32_t want = ((uint32_t)region << 8) | foot_idx;
  if (want == laxity_cur_aggr) { return; }
  laxity_cur_aggr = want;

  if (region == 0u || !laxity_aggr_ok)
  {
    qos_gpdma_m2m_stop();
    return;
  }
  {
    uint32_t r = (uint32_t)region - QOS_REGION_SRAM1;
    (void)qos_gpdma_m2m_start((uint32_t)(uintptr_t)laxity_aggr_src[r],
                              (uint32_t)(uintptr_t)laxity_aggr_dst[r],
                              laxity_footprints[foot_idx]);
  }
}

/* xorshift32, seeded from the cycle counter at boot so the order differs between runs. the realised order does not have to be reproducible because every record carries its own region_id, so the schedule that actually ran is in the data rather than in a seed someone has to keep. */
static uint32_t laxity_rand_state;

static uint32_t laxity_rand(void)
{
  uint32_t x = laxity_rand_state;
  x ^= x << 13; x ^= x >> 17; x ^= x << 5;
  laxity_rand_state = x;
  return x;
}

/* the order is randomised so thermal and flash state do not correlate with run index. a Fisher-Yates shuffle over the whole schedule spreads each label across the run instead of blocking it. */
static void laxity_shuffle(void)
{
  for (uint32_t i = 0u; i < LAXITY_SCHEDULE; ++i) { laxity_schedule[i] = (uint16_t)(i % LAXITY_CELLS); }
  for (uint32_t i = LAXITY_SCHEDULE - 1u; i > 0u; --i)
  {
    uint32_t j = laxity_rand() % (i + 1u);
    uint16_t t = laxity_schedule[i];
    laxity_schedule[i] = laxity_schedule[j];
    laxity_schedule[j] = t;
  }
}

static int laxity_infer_setup(void)
{
  laxity_net = (stai_network *)laxity_net_ctx;
  return (stai_network_init(laxity_net) == STAI_SUCCESS) ? 0 : -1;
}

/* one inference against the arena for label slot, with the cycle counter around stai_network_run() and nothing else.
 *
 * the activations are handed over on every iteration rather than only when the label changes, so the work in front of the measured window is identical whatever the shuffle produced. the input and output pointers live inside the activation buffer, so they move with it and have to be fetched again after every handover. */
static int laxity_infer(uint32_t slot, uint32_t *cycles, bool *wrapped)
{
  stai_ptr acts[STAI_NETWORK_ACTIVATIONS_NUM] = { (stai_ptr)laxity_arena[slot] };
  stai_ptr inputs[STAI_NETWORK_IN_NUM];
  stai_ptr outputs[STAI_NETWORK_OUT_NUM];
  stai_size n;
  uint32_t t0, t1;

  if (stai_network_set_activations(laxity_net, acts, STAI_NETWORK_ACTIVATIONS_NUM) != STAI_SUCCESS) { return -2; }

  n = STAI_NETWORK_IN_NUM;
  if (stai_network_get_inputs(laxity_net, inputs, &n) != STAI_SUCCESS) { return -3; }
  memcpy(inputs[0], laxity_golden_input, STAI_NETWORK_IN_1_SIZE_BYTES);

  t0 = qos_cyc_now();
  if (stai_network_run(laxity_net, STAI_MODE_SYNC) != STAI_SUCCESS) { return -4; }
  t1 = qos_cyc_now();
  *cycles = qos_cyc_delta(t0, t1);
  *wrapped = qos_cyc_wrapped(t0, t1);

  n = STAI_NETWORK_OUT_NUM;
  if (stai_network_get_outputs(laxity_net, outputs, &n) != STAI_SUCCESS) { return -5; }
  memcpy(laxity_out, outputs[0], STAI_NETWORK_OUT_1_SIZE_BYTES);
  return 0;
}

static int laxity_argmax(void)
{
  int best = 0;
  for (int i = 1; i < LAXITY_GOLDEN_OUT_LEN; ++i)
  {
    if (laxity_out[i] > laxity_out[best]) { best = i; }
  }
  return best;
}

/* the difference is reported in parts per million as an integer rather than as a float, since printing floats would pull newlib's formatting onto a path that later carries a measurement. */
static uint32_t laxity_worst_ppm(void)
{
  uint32_t worst = 0u;
  for (int i = 0; i < LAXITY_GOLDEN_OUT_LEN; ++i)
  {
    uint32_t ppm = (uint32_t)(fabsf(laxity_out[i] - laxity_golden_output[i]) * 1000000.0f);
    if (ppm > worst) { worst = ppm; }
  }
  return worst;
}

static VOID laxity_infer_entry(ULONG argument)
{
  int rc;

  (void)argument;

  (void)qos_dwt_init();
  laxity_boot.caps = qos_dwt_probe(HAL_GetTick, 5u);
  laxity_boot.cyccnt_hz = qos_dwt_measure_hz(HAL_GetTick, 200u);

  /* QOS_HDR_STALL_POPULATED stays clear. the counters exist on this part, measured rather than assumed, and nothing fills stall_cyc from them yet, so a stream that claimed both bits would describe a measurement this build does not make. */
  qos_telemetry_init(LAXITY_SYSCLK_HZ, laxity_boot.cyccnt_hz,
                     laxity_boot.caps.prfcnt_claimed ? QOS_HDR_STALL_AVAILABLE : 0u);

  laxity_boot.probe = qos_null_probe_run();
  qos_telemetry_set_null_probe(laxity_boot.probe.read_median, laxity_boot.probe.read_p99,
                               laxity_boot.probe.push_median, laxity_boot.probe.push_p99,
                               (uint16_t)laxity_boot.probe.n);

  laxity_placed_ok = laxity_place();
  if (laxity_placed_ok) { qos_telemetry_set_placements(laxity_placements, LAXITY_LABELS); }
  laxity_aggr_ok = qos_gpdma_m2m_init() ? 1u : 0u;
  laxity_build_cells();

  laxity_rand_state = qos_cyc_now() | 1u;

  rc = laxity_infer_setup();
  laxity_live.rc = rc;

  /* only now may the exporter touch the ring, since the null probe pushes records into it and resets it. */
  (void)tx_semaphore_put(&laxity_boot_done);

  if (rc != 0 || !laxity_placed_ok)
  {
    /* nothing measurable. the status line carries the reason and no record is emitted, because a capture of records taken against arenas that did not land is worse than an empty one. */
    while (1) { tx_thread_sleep(TX_TIMER_TICKS_PER_SECOND); }
  }

  for (uint32_t i = 0u; i < LAXITY_WARMUP; ++i)
  {
    uint32_t cycles = 0u; bool wrapped = false;
    (void)laxity_infer(i % LAXITY_ARENAS, &cycles, &wrapped);
    tx_thread_sleep(LAXITY_INFER_PERIOD);
  }

  while (1)
  {
    laxity_shuffle();

    for (uint32_t i = 0u; i < LAXITY_SCHEDULE; ++i)
    {
      qos_infer_record_t rec = {0};
      uint32_t cell = laxity_schedule[i];
      uint32_t slot = laxity_cells[cell].arena_slot;
      uint8_t  aggr = laxity_cells[cell].aggr_region;
      uint8_t  foot = laxity_cells[cell].foot_idx;
      uint32_t cycles = 0u;
      bool wrapped = false;
      int run;

      /* the aggressor is set before the window opens and left alone inside it, so nothing this
       * loop does to the channel is counted as part of the inference. */
      laxity_set_aggressor(aggr, foot);

      rec.release_cyc = qos_cyc_now();
      run = laxity_infer(slot, &cycles, &wrapped);
      laxity_live.rc = run;

      if (run == 0)
      {
        int argmax = laxity_argmax();
        laxity_live.cycles = cycles;
        laxity_live.argmax = argmax;
        laxity_live.worst_ppm = laxity_worst_ppm();
        /* the golden check runs on every inference rather than once, since a wrongly placed arena that still returns a number would otherwise pass unnoticed. */
        if (argmax != LAXITY_GOLDEN_CLASS) { laxity_live.mismatches++; }
      }

      rec.exec_cyc = cycles;
      rec.region_id = laxity_placements[slot].id;

      /* which aggressor was running, and the proof that it was. the count is polled after the
       * window so it covers it, and it advances only when the channel actually completed a
       * block, which is what tells a null result apart from a channel that never started. */
      rec.aggressor_idx = (uint16_t)(((uint16_t)foot << 8) | aggr);
      rec.reserved = qos_gpdma_m2m_completions();
      laxity_live.aggr_completions = rec.reserved;
      /* cpu_cyc and stall_cyc stay zero, as do model_id and aggressor_idx. there is no attribution measurement and no aggressor in this block, and writing a plausible value into a field nothing measured is how a number nobody can defend gets into a capture. */
      rec.flags = wrapped ? (uint8_t)QOS_FLAG_CYCCNT_WRAP : 0u;
      (void)qos_telemetry_push(&rec);

      tx_thread_sleep(LAXITY_INFER_PERIOD);
    }
    laxity_live.passes++;
  }
}

/* bring the link up once, then report it. the result is published for the status line rather
   than printed here, because one thread owns the UART and it is not this one. */
static VOID laxity_net_entry(ULONG argument)
{
  (void)argument;

  laxity_mxchip_irq_enable();
  laxity_live.net_rc = (int32_t)qos_net_start();

  while (1)
  {
    ULONG addr = 0u, mask = 0u;
    (void)qos_net_address(&addr, &mask);
    laxity_live.net_addr = (uint32_t)addr;
    (void)qos_net_stats((UINT *)&laxity_live.net_sent, (UINT *)&laxity_live.net_failed);
    tx_thread_sleep(TX_TIMER_TICKS_PER_SECOND);
  }
}

/* send one line, with the frame magic scrubbed out of it first.
 *
 * the magic is "LX" in ASCII and the status line shares this UART with the framed stream, so a status line that happened to contain those two bytes would cost the parser a rescan. the parser recovers from it either way, and keeping the sequence off the wire is cheaper than relying on the recovery. */
static void laxity_say(char *line, int n)
{
  int i;

  if (n <= 0) { return; }
  for (i = 0; i + 1 < n; ++i)
  {
    if ((uint8_t)line[i] == QOS_TELEMETRY_MAGIC0 && (uint8_t)line[i + 1] == QOS_TELEMETRY_MAGIC1)
    {
      line[i + 1] = 'x';
    }
  }
  HAL_UART_Transmit(&huart1, (uint8_t *)line, (uint16_t)n, HAL_MAX_DELAY);
}

/* the exporter owns USART1. nothing else transmits, so a status line cannot interleave into a frame and cost a record. */
static VOID laxity_export_entry(ULONG argument)
{
  char line[240];
  uint32_t ticks = 0u;
  int argmax;

  (void)argument;

  (void)tx_semaphore_get(&laxity_boot_done, TX_WAIT_FOREVER);

  while (1)
  {
    size_t n = qos_telemetry_drain(laxity_export_buf, sizeof laxity_export_buf);
    if (n > 0u)
    {
      /* the UART is the path that always works and it stays first. the datagram carries the
         same bytes, so the host parser reads either without knowing which it received. */
      HAL_UART_Transmit(&huart1, laxity_export_buf, (uint16_t)n, HAL_MAX_DELAY);
      (void)qos_net_send(laxity_export_buf, (UINT)n);
    }

    /* one human readable set per second, so a silent board is still distinguishable from a broken one without running the parser. */
    if ((++ticks % TX_TIMER_TICKS_PER_SECOND) == 0u)
    {
      argmax = (int)laxity_live.argmax;
      laxity_say(line, snprintf(line, sizeof line,
                   "infer rc=%ld cycles=%lu class=%d(%s) expect=%d(%s) %s mismatch=%lu pass=%lu dma=%u/%lu opt=-O0\r\n",
                   (long)laxity_live.rc, (unsigned long)laxity_live.cycles,
                   argmax, laxity_golden_class_names[argmax],
                   LAXITY_GOLDEN_CLASS, laxity_golden_class_names[LAXITY_GOLDEN_CLASS],
                   (laxity_live.rc == 0 && argmax == LAXITY_GOLDEN_CLASS) ? "MATCH" : "MISMATCH",
                   (unsigned long)laxity_live.mismatches, (unsigned long)laxity_live.passes,
                   (unsigned)laxity_aggr_ok, (unsigned long)laxity_live.aggr_completions));

      /* the addresses are on the wire in the header frame as well. printing them makes a wrong placement visible with head -c, before anyone runs the parser. */
      laxity_say(line, snprintf(line, sizeof line,
                   "placement ok=%u span=0x%08lx..0x%08lx s1=0x%08lx s2=0x%08lx s3=0x%08lx s2b=0x%08lx ctl=0x%08lx bytes=%u\r\n",
                   (unsigned)laxity_placed_ok,
                   (unsigned long)(uintptr_t)laxity_arena_span,
                   (unsigned long)((uintptr_t)laxity_arena_span + sizeof laxity_arena_span),
                   (unsigned long)laxity_placements[0].arena_addr,
                   (unsigned long)laxity_placements[1].arena_addr,
                   (unsigned long)laxity_placements[2].arena_addr,
                   (unsigned long)laxity_placements[3].arena_addr,
                   (unsigned long)laxity_placements[4].arena_addr,
                   (unsigned)LAXITY_ARENA_BYTES));

      laxity_say(line, snprintf(line, sizeof line,
                   "net rc=%ld ip=%lu.%lu.%lu.%lu sent=%lu failed=%lu\r\n",
                   (long)laxity_live.net_rc,
                   (unsigned long)((laxity_live.net_addr >> 24) & 0xFFu),
                   (unsigned long)((laxity_live.net_addr >> 16) & 0xFFu),
                   (unsigned long)((laxity_live.net_addr >> 8) & 0xFFu),
                   (unsigned long)(laxity_live.net_addr & 0xFFu),
                   (unsigned long)laxity_live.net_sent,
                   (unsigned long)laxity_live.net_failed));

      laxity_say(line, snprintf(line, sizeof line,
                   "telemetry v%u cyccnt_hz=%lu stall_avail=%d null_read_med=%lu null_read_p99=%lu null_push_med=%lu null_push_p99=%lu n=%lu\r\n",
                   (unsigned)QOS_TELEMETRY_VERSION, (unsigned long)laxity_boot.cyccnt_hz,
                   qos_telemetry_has_stall_attribution() ? 1 : 0,
                   (unsigned long)laxity_boot.probe.read_median,
                   (unsigned long)laxity_boot.probe.read_p99,
                   (unsigned long)laxity_boot.probe.push_median,
                   (unsigned long)laxity_boot.probe.push_p99,
                   (unsigned long)laxity_boot.probe.n));
    }

    tx_thread_sleep(1);
  }
}

/* USER CODE END 1 */
