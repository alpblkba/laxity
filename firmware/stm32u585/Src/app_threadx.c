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
#include "gpdma_stress.h"
#include "read_loop.h"
#include "qos/telemetry.h"
#include "net_udp_export.h"
#include "sensors.h"
#include "audio.h"
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

/* below the driver's own threads and above the exporter, so joining a network never delays a measurement and never starves the thing draining the ring. */
#define LAXITY_NET_STACK     2048
#define LAXITY_NET_PRIO      12

/* the network stack is off by default. with no reachable access point the driver's own threads
 * sit above every thread here at priorities 8 and 9, and about forty seconds after start up they
 * stop yielding: the UART goes silent, the measurement loop stops advancing, and the board looks
 * halted. measured as steady output for 43 seconds and then nothing at all.
 *
 * the sensor and telemetry paths do not need it, so it is opt in until that is understood. set
 * this to 1 to bring it back. */
#define LAXITY_NET_ENABLE    1

/* lower priority than the measurement thread, so a blocking transmit that takes several milliseconds at 921600 baud cannot delay an inference. that is what best effort export means here. */

/* a header frame with four placement entries is 136 bytes, so 512 still carries eleven records per batch against a ring of thirty two and a producer at fifty hertz. */
#define LAXITY_EXPORT_BUF    512

/* what the .ioc configures, reported in the header frame beside the rate qos_dwt_measure_hz() actually observed, since a capture has to be able to show a clock configuration that did not take. */
#define LAXITY_SYSCLK_HZ     160000000u

#define LAXITY_ARENA_BYTES   STAI_NETWORK_ACTIVATIONS_SIZE_BYTES

/* every arena starts on the same 4 KB boundary, so offset within a region is held constant and the region is the only thing that differs between the four labels. 4096 is a bound rather than a measured bank size: the SRAM bank structure is in RM0456, which is the one thing this block could not source, so the alignment is chosen large enough to cover any plausible granularity instead of matching a known one. */
#define LAXITY_ARENA_ALIGN   4096u

/* the arena may not reach the aggressor source buffer. laxity_place() puts that buffer at the arena base plus LAXITY_ARENA_ALIGN and checks only that it stays inside its region, so an arena larger than the alignment would overlap it in all three regions and the run would still look healthy. this is next to the size constant rather than next to the placement, because the size comes from the model and this is where a new model changes it. */
_Static_assert(LAXITY_ARENA_BYTES <= LAXITY_ARENA_ALIGN,
               "activation arena is larger than LAXITY_ARENA_ALIGN, so it overlaps the aggressor source buffer that laxity_place() puts at arena base + LAXITY_ARENA_ALIGN in SRAM1, SRAM2 and SRAM3");

/* one reservation that reaches from wherever bss puts it up to SRAM3, so all three arenas come out of memory the linker actually allocated. sized for the worst case of starting at the base of SRAM1; starting later only makes it reach further.
 *
 * the alternative was a linker script edit, and it was rejected on a real failure mode rather than on taste. the generated script maps SRAM1, SRAM2 and SRAM3 as one 768K RAM region with _estack at its top, so placing sections in the upper two means either overlapping MEMORY regions, which GNU ld does not check for collisions between, or splitting RAM, which moves the stack out of SRAM3 and changes the vendor default layout every comparison is made against. one C object cannot overlap anything, because the linker allocated it. */
/* the largest buffer the aggressor cycles through, per buffer. two of them plus the arena have to fit in the smallest region, and SRAM2 is 64 KB, so 16 KB each leaves room to spare. */
#define LAXITY_AGGR_MAX      (16u * 1024u)

/* one region's worth of the reservation: the arena on its own alignment, then the two aggressor buffers after it. */
#define LAXITY_REGION_SLICE  (LAXITY_ARENA_ALIGN + 2u * LAXITY_AGGR_MAX)

/* the stress experiment gets its own window in every region, clear of the arenas and of the mem2mem buffers. the offsets differ per region because SRAM2 is 64 KB and cannot hold a window at the offset the other two use. each one is checked against its region at run time rather than trusted, the same way the arenas are. */
/* the aggressor window in each region, then a descriptor page above it. the sizes differ because SRAM1 has to give up pages for the measurement thread's stack and for its descriptors, and SRAM4 is 16 KiB in total. no base moves, so the addresses the aggressor touches are the ones the earlier campaign measured and the two matrices stay comparable. */
#define LAXITY_STRESS_BYTES_S1 (12u * 1024u)
#define LAXITY_STRESS_BYTES_S2 (12u * 1024u)
#define LAXITY_STRESS_BYTES_S3 (20u * 1024u)
#define LAXITY_STRESS_BYTES_S4 (12u * 1024u)

/* one page per region for the linked list nodes the hardware fetches at every block boundary. they are static objects in bss otherwise, which lands them in SRAM3 whatever region the traffic is in, and that is the shape the per block term of the cost model has. */
#define LAXITY_DESC_OFF_S1 0x0E000u
#define LAXITY_DESC_OFF_S2 0x0E000u
#define LAXITY_DESC_OFF_S3 0x0B000u
#define LAXITY_DESC_OFF_S4 0x03000u
#define LAXITY_DESC_BYTES  4096u

/* the measurement thread's stack, one page in each region.
 *
 * ThreadX allocates thread stacks from a byte pool that sits in SRAM3, and at -O0 four of every
 * five read loop accesses are stack accesses, so the victim region knob controlled a fifth of the
 * victim's traffic and the rest went to SRAM3 whatever it was set to. the published matrix was
 * measured with the inference victim on that same pool, and whether its rows mean placement or
 * mean stack exposure is what the three pages below exist to separate.
 *
 * the bottom quarter of a page is not handed to ThreadX. an overflow descends into it and shows up
 * as a guard word that changed, rather than as silent corruption of whatever sits underneath. */
#define LAXITY_STACK_OFF_S1 0x0F000u
#define LAXITY_STACK_OFF_S2 0x0D000u
#define LAXITY_STACK_OFF_S3 0x0C000u
#define LAXITY_STACK_BYTES  4096u
#define LAXITY_STACK_GUARD  1024u
#define LAXITY_STACK_FILL   0xA5A5A5A5u

/* a fourth source for the same stack, which is not a page: the ThreadX byte pool, which is where a plain ThreadX application's thread stacks come from and where this one's came from before the three pages existed. the pages are deterministic by construction, so how certain a placement is cannot be asked of them, and it is the only question the vendor path can answer. the value sits outside the region ids rather than beside them, since it names an allocator and not a region. */
#define LAXITY_STACK_SRC_POOL 0x0Fu

/* the ballast sizes, in bytes, taken from the same byte pool before the measurement thread is created. zero allocates nothing rather than allocating an empty block, because an allocator called once more is not the same baseline as one that was not called. */
#define LAXITY_BALLASTS      5u

/* which source the stack comes from, and how much ballast is taken before it, survive a reset in a word of SRAM4, which carries no section and which the startup code does not clear. a thread's stack is fixed when the thread is created and the ballast has to be taken before that, so both choices have to arrive before the kernel starts, and a reset is the only way to get there without a binary per configuration. the low byte is the source and the next one is the ballast index, so the magic is checked over the top half only. */
#define LAXITY_BOOT_SEL_ADDR  (QOS_SRAM4_BASE + 0x3FF0u)
#define LAXITY_BOOT_SEL_MAGIC 0x4C580000u

/* the SRAM1 window moved down from 0x10000 to sit immediately above the mem2mem buffers, which frees the top 128 KiB of SRAM1 in one piece for the victim footprint sweep. it is still SRAM1 traffic, and the stride sweep found the cost flat across a thirtyfold change in address reach, so the move is a layout change rather than a change to what the aggressor does. */
#define LAXITY_STRESS_OFF_S1 0x0B000u
#define LAXITY_STRESS_OFF_S2 0x0A000u
#define LAXITY_STRESS_OFF_S3 0x10000u

/* the read loop victim, one window per region it can run in.
 *
 * SRAM1 gets the whole top 128 KiB of the region because the footprint sweep walks a prefix of it,
 * and every footprint being a prefix of one window means the sweep changes the size without also
 * changing the address. SRAM2 and SRAM3 only ever host the 4 KiB point of the coefficient matrix,
 * and SRAM2 has no more than that left after two arenas and the two mem2mem buffers. */
#define LAXITY_VICTIM_OFF_S1   0x10000u
#define LAXITY_VICTIM_BYTES_S1 (128u * 1024u)
#define LAXITY_VICTIM_OFF_S2   0x0F000u
#define LAXITY_VICTIM_BYTES_S2 (4u * 1024u)
#define LAXITY_VICTIM_OFF_S3   0x09000u
#define LAXITY_VICTIM_BYTES_S3 (4u * 1024u)

/* the reservation now has to reach the SRAM3 stress window as well as the SRAM3 arena, and the window is the one further in. */
#define LAXITY_SPAN_BYTES    ((QOS_SRAM3_BASE - QOS_SRAM1_BASE) + LAXITY_STRESS_OFF_S3 + LAXITY_STRESS_BYTES_S3)

/* repetitions of the whole cross per pass. the schedule reshuffles and repeats, so the sample count per cell comes from how long the capture runs rather than from this number, and a small value keeps one pass short enough that drift is spread across cells rather than within one. */
#define LAXITY_REPS          8u
/* the cross this binary measures: three arena regions against three aggressor regions at four footprints, plus an aggressor off column for each arena region, plus the same buffer control. 3*3*4 + 3 + 1 comes to 40. */
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
#if LAXITY_NET_ENABLE
static TX_THREAD    laxity_net_thread;
#endif

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

/* one source and one destination per aggressor region, both inside that region, so the traffic is region local and the only thing crossing regions is the arbitration. */
static uint8_t        *laxity_aggr_src[LAXITY_AGGR_REGIONS];
static uint8_t        *laxity_aggr_dst[LAXITY_AGGR_REGIONS];

/* bytes per aggressor buffer. the smallest is well under any buffering on the path and the largest is bounded by SRAM2, which is 64 KB and has to hold an arena and both buffers. */
static const uint32_t  laxity_footprints[LAXITY_FOOTPRINTS] = { 1024u, 4096u, 8192u, 16384u };

/* the stress experiment.
 *
 * which victim runs, which sweep is active and which region the aggressor sits in are all chosen
 * at run time from the console, so one image produces every configuration. a configuration per
 * binary would put a relink inside every measured difference, and a relink on its own has already
 * moved a median here by 85 cycles.
 *
 * a sweep is a list of aggressor configurations that differ in one knob. point 0 of the schedule
 * is always the aggressor off, which is what the rest of the sweep is read against. */
#define LAXITY_VICTIM_INFER  0u
#define LAXITY_VICTIM_READ   1u
/* inference driven from the stress schedule rather than from the arena cross. the arena stays in SRAM1 and the aggressor sweeps, which is the only arrangement that puts both victims under the same aggressor configuration and makes a per transaction cost comparable between them. */
#define LAXITY_VICTIM_ISTRESS 2u
/* no victim at all. the DMA transfer is what is timed, which is the only way to ask whether a region is slower on the DMA side without a victim's access pattern in the answer. */
#define LAXITY_VICTIM_DMATIME 3u

#define LAXITY_SWEEP_BW      0u
#define LAXITY_SWEEP_XACT    1u
#define LAXITY_SWEEP_CHAN    2u
#define LAXITY_SWEEP_STRIDE  3u
#define LAXITY_SWEEP_SAT     4u
#define LAXITY_SWEEP_LOW     5u
#define LAXITY_SWEEP_DMAT    6u
#define LAXITY_SWEEP_NONE    7u
#define LAXITY_SWEEPS        8u
#define LAXITY_SWEEP_MAX_PTS 6u

/* the aggressor can sit in SRAM4 as well as in the three the arenas use, and SRAM4 holds no arena and no victim, so the stress region count is one more than the aggressor region count. */
#define LAXITY_STRESS_REGIONS 4u

/* an upper bound on the one shot poll, in iterations. the longest transfer any point asks for is 4096 transactions, so this is two orders of magnitude of headroom and it exists only so that a channel that never raises its flag ends the measurement instead of the run. */
#define LAXITY_DMA_POLL_MAX   200000u

/* cells per pass: the off cell plus the points of the active sweep. */
#define LAXITY_STRESS_CELLS    (1u + LAXITY_SWEEP_MAX_PTS)
#define LAXITY_STRESS_REPS     8u
#define LAXITY_STRESS_SCHEDULE (LAXITY_STRESS_REPS * LAXITY_STRESS_CELLS)

static const struct {
  const char      *name;
  uint8_t          points;
  qos_stress_cfg_t pt[LAXITY_SWEEP_MAX_PTS];
} laxity_sweeps[LAXITY_SWEEPS] = {
  /* bandwidth at a fixed transaction size. width and block stay put and only the trigger rate moves, so bytes per second and transactions per second rise together and this sweep on its own cannot separate them. it is the axis the other three are read against. */
  { "bw", 5, {
      { 1u, 4u, 256u, 0u,  50000u },
      { 1u, 4u, 256u, 0u, 100000u },
      { 1u, 4u, 256u, 0u, 200000u },
      { 1u, 4u, 256u, 0u, 400000u },
      { 1u, 4u, 256u, 0u, 800000u },
  } },
  /* transactions at a fixed bandwidth. the block is a byte count, so holding it and the trigger rate fixed while the width falls from 4 to 1 leaves the byte rate exactly where it was and multiplies the transaction rate by four. this is the sweep that separates the first two hypotheses, and it is the reason the width knob exists. */
  { "xact", 3, {
      { 1u, 4u, 256u, 0u, 200000u },
      { 1u, 2u, 256u, 0u, 200000u },
      { 1u, 1u, 256u, 0u, 200000u },
  } },
  /* channel count at a fixed total bandwidth. the block is divided by the channel count, so the bytes and the transactions per second are the same at every point and the only thing that changes is how many masters are asking. */
  { "chan", 3, {
      { 1u, 4u, 512u, 0u, 200000u },
      { 2u, 4u, 256u, 0u, 200000u },
      { 4u, 4u, 128u, 0u, 200000u },
  } },
  /* address stride. bytes and transactions are constant across the whole sweep and only where they land moves, so anything this finds belongs to the address rather than to the load. */
  { "stride", 6, {
      { 1u, 4u, 256u,   0u, 200000u },
      { 1u, 4u, 256u,   4u, 200000u },
      { 1u, 4u, 256u,  12u, 200000u },
      { 1u, 4u, 256u,  28u, 200000u },
      { 1u, 4u, 256u,  60u, 200000u },
      { 1u, 4u, 256u, 124u, 200000u },
  } },
  /* past the top of the bandwidth sweep, which reached 51.2 million transactions per second and was already saturating there. the first two points ask for two and four times that rate at the same trigger, the next two remove the trigger so the channel runs flat out, and the last one arms the channel on a timer that is never started so it is enabled and moves nothing. the requested rates of the first two are what the configuration asks for and not what the hardware delivers, which is the question this sweep exists to answer. */
  { "sat", 5, {
      { 1u, 2u, 256u, 0u, 800000u },
      { 1u, 1u, 256u, 0u, 800000u },
      { 1u, 4u, 256u, 0u, 0u },
      { 1u, 1u, 256u, 0u, 0u },
      { 1u, 4u, 256u, 0u, QOS_STRESS_TRIGGER_ARMED },
  } },
  /* below the bandwidth sweep, which starts where this one ends. 0.4 to 3.2 million transactions per second, which is where a port that is busy most of the time at the old rates should have room again, and where a diagonal that saturation is hiding would reappear. */
  { "low", 4, {
      { 1u, 4u, 256u, 0u,  6250u },
      { 1u, 4u, 256u, 0u, 12500u },
      { 1u, 4u, 256u, 0u, 25000u },
      { 1u, 4u, 256u, 0u, 50000u },
  } },
  /* not a sweep of an aggressor against a victim. each point is one block moved once and timed on the DMA side, and the four points change the transaction count at a fixed byte count and then the byte count at a fixed width, so time against transactions has both a slope and an intercept. the slope is what a transaction costs the DMA, the intercept is what a block costs before any data moves. */
  { "dmat", 4, {
      { 1u, 4u, 4096u, 0u, 0u },
      { 1u, 2u, 4096u, 0u, 0u },
      { 1u, 1u, 4096u, 0u, 0u },
      { 1u, 4u, 2048u, 0u, 0u },
  } },
  /* no points at all, so the schedule is the aggressor off cell and nothing else and no channel is ever started. taking the off records out of a sweep that also runs an aggressor would work and would mean measuring a quiet window in a capture that was not quiet, which is the distinction the contention free table has to be able to make. */
  { "none", 0, { { 0u, 0u, 0u, 0u, 0u } } },
};

/* the victim footprints of the scaling sweep, in bytes. the read loop buffer size is already a free parameter, so how the cost scales with the victim's working set can be measured without converting a second network, which is what the placement report could not say anything about. */
#define LAXITY_VICTIM_FOOTPRINTS 8u
static const uint32_t laxity_victim_footprints[LAXITY_VICTIM_FOOTPRINTS] = {
  1024u, 2048u, 4096u, 8192u, 16384u, 32768u, 65536u, 131072u
};

static uint8_t  laxity_victim = LAXITY_VICTIM_INFER;
static uint8_t  laxity_sweep = LAXITY_SWEEP_BW;
static uint8_t  laxity_stress_region = QOS_REGION_SRAM3;
static uint8_t  laxity_stress_ok;
static uint8_t  laxity_stress_point = 0xFFu;   /* the point the channels are currently set to */
static uint32_t laxity_stress_base[LAXITY_STRESS_REGIONS];
static uint32_t laxity_stress_span[LAXITY_STRESS_REGIONS];
static uint32_t laxity_desc_base[LAXITY_STRESS_REGIONS];
static uint8_t  laxity_desc_region = QOS_REGION_SRAM3;  /* where the nodes were before the knob existed */
static uint8_t  laxity_desc_ok;
/* where the measurement thread's stack actually is, resolved from its address rather than from the request, because a fallback that nobody noticed would invalidate every capture after it. */
static uint8_t  laxity_stack_region = QOS_REGION_NONE;
/* the page the thread actually got, or NULL when it fell back to the byte pool. the high water reading uses this rather than the region, so a fallback cannot be read as a stack that was never there. */
static uint8_t *laxity_stack_used_page;
/* the source this boot ran with, kept as a selector rather than as a region because the byte pool is in SRAM3 and a request for the pool would otherwise read as a request for the SRAM3 page already in use and reset nothing. */
static uint8_t  laxity_stack_src = QOS_REGION_SRAM1;
/* where the thread's stack actually starts: the page plus its guard for a page, and whatever the allocator answered for the pool. the status line and the high water reading both use this rather than the request, since the address is the only thing that decides the bank. */
static uint8_t *laxity_stack_addr;
/* the ballast, which models a feature added elsewhere in the firmware taking memory ahead of the measurement thread. it is allocated and never touched, so it moves what comes after it without adding any traffic of its own. */
static const uint32_t laxity_ballast_sizes[LAXITY_BALLASTS] = { 0u, 64u, 256u, 1024u, 4096u };
static uint8_t  laxity_ballast_idx;
static uint32_t laxity_ballast_bytes;
static uint32_t laxity_ballast_addr;

/* which placement the stress schedule's inference runs against. the arena cross picks its own slot per cell; this is the one the decomposition varies. */
static uint8_t  laxity_arena_slot;
static uint32_t laxity_read_acc;               /* consumes the loads so none of them is dead */

/* where the read loop victim runs, how much of its window it walks and how many loads it issues. all three are run time choices for the same reason the aggressor knobs are: a build per point would put a relink inside the curve. */
/* leave the mem2mem aggressor running through a stress pass instead of stopping it.
 *
 * this exists to reproduce a fault rather than to measure anything new. before the stop was added,
 * a stress pass inherited whatever the last cell of the arena cross had left on GPDMA1 channel 1,
 * so twelve earlier captures were taken with a second uncontrolled master running. setting this
 * puts that state back deliberately, which is the difference between inferring the contamination
 * from an arithmetic match and demonstrating it. */
static uint8_t  laxity_m2m_hold;

static uint8_t  laxity_victim_region = QOS_REGION_SRAM1;
static uint8_t  laxity_victim_foot = 2u;                   /* 4096 bytes, what every earlier stress capture used */
static uint32_t laxity_victim_loads = QOS_READ_LOOP_LOADS; /* loads asked for per window */
static uint8_t *laxity_victim_base[LAXITY_AGGR_REGIONS];
static uint32_t laxity_victim_window[LAXITY_AGGR_REGIONS];
/* what the measurement thread settled on for the current pass, published for the status line so a capture can be checked against the configuration it was filed under. */
static uint32_t laxity_victim_words;
static uint32_t laxity_victim_passes;
static uint16_t laxity_stress_schedule[LAXITY_STRESS_SCHEDULE];

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

/* which window the classifier is fed. the golden window is a fixed synthetic trace with a known answer and is the only end to end check this firmware has, so the live sensor is a second mode beside it rather than a replacement. the user button toggles between them. */
#define LAXITY_INPUT_GOLDEN  0u
#define LAXITY_INPUT_LIVE    1u
static uint8_t  laxity_input_mode = LAXITY_INPUT_GOLDEN;
static uint8_t  laxity_btn_idle;
static uint8_t  laxity_btn_last;
static uint32_t laxity_btn_ms;
static uint8_t  laxity_sensor_ok;
static uint8_t  laxity_audio_ok;
static float    laxity_live_window[LAXITY_SENSOR_LEN];

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
#if LAXITY_NET_ENABLE
static VOID laxity_net_entry(ULONG argument);
#endif

/* toggle on a change away from the level the pin sat at during start up, which avoids assuming
   whether the button pulls high or low. 200 ms of settling is longer than any contact bounce. */
static void laxity_poll_button(void)
{
  uint8_t now = (uint8_t)HAL_GPIO_ReadPin(USER_Button_GPIO_Port, USER_Button_Pin);
  uint32_t t = HAL_GetTick();

  if (now != laxity_btn_last)
  {
    laxity_btn_last = now;
    if (now != laxity_btn_idle && (t - laxity_btn_ms) > 200u)
    {
      laxity_btn_ms = t;
      laxity_input_mode = (laxity_input_mode == LAXITY_INPUT_GOLDEN)
                            ? LAXITY_INPUT_LIVE : LAXITY_INPUT_GOLDEN;
    }
  }
}

/* which region an address is in, resolved from the region table rather than from whichever knob was supposed to put it there. two contaminations have been found in this rig and both were the same sentence: something touched memory inside the window that the knobs did not know about. */
static uint8_t laxity_region_of(uintptr_t a)
{
  for (uint32_t i = 0u; i < qos_mem_region_count; ++i)
  {
    const qos_mem_region_t *r = &qos_mem_regions[i];
    if (a >= (uintptr_t)r->base && a < ((uintptr_t)r->base + r->size)) { return r->id; }
  }
  return QOS_REGION_NONE;
}

/* the measurement thread's stack page in one region, inside the span reservation.
 *
 * it returns NULL rather than an address when the page is not inside the reservation, because the
 * span starts wherever bss puts it and an address that drifted outside it would be memory the
 * linker gave to something else. the caller falls back to the byte pool and the status line says
 * which region the run actually got. */
static uint8_t *laxity_stack_page(uint8_t region)
{
  const qos_mem_region_t *reg = qos_mem_region(region);
  uintptr_t span_lo = (uintptr_t)laxity_arena_span;
  uintptr_t span_hi = span_lo + sizeof laxity_arena_span;
  uintptr_t lo;
  uint32_t off;

  switch (region)
  {
    case QOS_REGION_SRAM1: off = LAXITY_STACK_OFF_S1; break;
    case QOS_REGION_SRAM2: off = LAXITY_STACK_OFF_S2; break;
    case QOS_REGION_SRAM3: off = LAXITY_STACK_OFF_S3; break;
    default: return NULL;
  }
  if (reg == NULL) { return NULL; }
  lo = (uintptr_t)reg->base + off;
  if ((lo + LAXITY_STACK_BYTES) > ((uintptr_t)reg->base + reg->size)) { return NULL; }
  if (lo < span_lo || (lo + LAXITY_STACK_BYTES) > span_hi) { return NULL; }
  return (uint8_t *)lo;
}

static uint8_t laxity_boot_sel_read(void)
{
  uint32_t w;
  __HAL_RCC_SRAM4_CLK_ENABLE();
  w = *(volatile uint32_t *)LAXITY_BOOT_SEL_ADDR;
  if ((w & 0xFFFF0000u) != LAXITY_BOOT_SEL_MAGIC) { return QOS_REGION_SRAM1; }
  w &= 0xFFu;
  if (w == LAXITY_STACK_SRC_POOL) { return (uint8_t)w; }
  return (w >= QOS_REGION_SRAM1 && w <= QOS_REGION_SRAM3) ? (uint8_t)w : QOS_REGION_SRAM1;
}

static uint8_t laxity_boot_ballast_read(void)
{
  uint32_t w;
  __HAL_RCC_SRAM4_CLK_ENABLE();
  w = *(volatile uint32_t *)LAXITY_BOOT_SEL_ADDR;
  if ((w & 0xFFFF0000u) != LAXITY_BOOT_SEL_MAGIC) { return 0u; }
  w = (w >> 8) & 0xFFu;
  return (w < LAXITY_BALLASTS) ? (uint8_t)w : 0u;
}

static void laxity_boot_sel_write(uint8_t src, uint8_t ballast_idx)
{
  __HAL_RCC_SRAM4_CLK_ENABLE();
  *(volatile uint32_t *)LAXITY_BOOT_SEL_ADDR =
      LAXITY_BOOT_SEL_MAGIC | ((uint32_t)ballast_idx << 8) | (uint32_t)src;
}

/* how deep the measurement thread's stack has been, and whether it went past what it was given.
 *
 * ThreadX fills a new stack with TX_STACK_FILL, so the lowest word that is not that pattern is the
 * deepest the thread has reached. the guard below carries a different pattern, which is what makes
 * an overflow distinguishable from ordinary depth instead of being read as more of it. inference
 * through a vendor runtime can be expensive in stack and an overflow corrupts silently. */
static uint32_t laxity_stack_high_water(uint8_t *guard_hit)
{
  /* the guard exists only under a page. a stack out of the byte pool has whatever the allocator
     put below it, so there is nothing to check and the reading says so instead of reading the
     neighbouring block as a guard that changed. */
  const uint32_t *g = (const uint32_t *)laxity_stack_used_page;
  const uint32_t *p = (const uint32_t *)laxity_stack_addr;
  uint32_t i;

  *guard_hit = 0u;
  if (p == NULL) { return 0u; }
  if (g != NULL)
  {
    for (i = 0u; i < (LAXITY_STACK_GUARD / 4u); ++i)
    {
      if (g[i] != LAXITY_STACK_FILL) { *guard_hit = 1u; break; }
    }
  }
  for (i = 0u; i < (LAXITY_INFER_STACK / 4u); ++i)
  {
    if (p[i] != 0xEFEFEFEFu) { break; }
  }
  return ((LAXITY_INFER_STACK / 4u) - i) * 4u;
}

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

  /* the measurement thread's stack comes from one of three pages, since the pool is in SRAM3 and
     the victim's stack traffic would otherwise land in the region half the experiment is about,
     or from the pool itself when the vendor path is what is being measured.
     every page is filled, including the ones this run will not use, so a page that is not a stack
     reads as untouched and a high water figure cannot be taken from the wrong one. */
  {
    uint8_t want;
    for (uint8_t reg = QOS_REGION_SRAM1; reg <= QOS_REGION_SRAM3; ++reg)
    {
      uint32_t *page = (uint32_t *)laxity_stack_page(reg);
      if (page == NULL) { continue; }
      for (uint32_t i = 0u; i < (LAXITY_STACK_BYTES / 4u); ++i) { page[i] = LAXITY_STACK_FILL; }
    }
    want = laxity_boot_sel_read();
    laxity_stack_src = want;
    laxity_ballast_idx = laxity_boot_ballast_read();
    laxity_ballast_bytes = laxity_ballast_sizes[laxity_ballast_idx];

    /* the ballast is taken before the measurement thread's stack rather than after it, because
       what it models is a feature that was already there when the thread was created. taken
       afterwards it would move nothing and measure nothing. */
    if (laxity_ballast_bytes > 0u)
    {
      VOID *block;
      if (tx_byte_allocate(byte_pool, &block, laxity_ballast_bytes, TX_NO_WAIT) != TX_SUCCESS)
      {
        return TX_POOL_ERROR;
      }
      laxity_ballast_addr = (uint32_t)(uintptr_t)block;
    }

    if (want == LAXITY_STACK_SRC_POOL)
    {
      laxity_stack_used_page = NULL;
      infer_stack = NULL;
    }
    else
    {
      laxity_stack_used_page = laxity_stack_page(want);
      if (laxity_stack_used_page == NULL) { laxity_stack_used_page = laxity_stack_page(QOS_REGION_SRAM1); }
      infer_stack = (CHAR *)laxity_stack_used_page;
      if (infer_stack != NULL) { infer_stack += LAXITY_STACK_GUARD; }
    }
  }
  if (infer_stack == NULL &&
      tx_byte_allocate(byte_pool, (VOID **)&infer_stack, LAXITY_INFER_STACK,
                       TX_NO_WAIT) != TX_SUCCESS)
  {
    return TX_POOL_ERROR;
  }
  laxity_stack_addr = (uint8_t *)infer_stack;
  laxity_stack_region = laxity_region_of((uintptr_t)infer_stack);

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

#if LAXITY_NET_ENABLE
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
#endif
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

/* the labels this run measures. two of them are controls of different kinds: SRAM1c is the same bytes as SRAM1 under a second id, which gives the noise floor, and SRAM2b is SRAM2 at a second address, which says whether a difference belongs to the region or to the address. */
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
  /* past the aggressor buffers, which occupy one arena alignment plus two maximum footprints from the base of every region. */
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

  /* the stress windows, one per region, and the read loop victim in SRAM1.
   *
   * every one is checked against its region and against the reservation before it is used. a
   * window that fell outside its region would have the aggressor competing for memory the
   * experiment is not measuring, and it would do so while every liveness count still advanced. */
  {
    static const uint32_t stress_off[LAXITY_AGGR_REGIONS] = {
      LAXITY_STRESS_OFF_S1, LAXITY_STRESS_OFF_S2, LAXITY_STRESS_OFF_S3
    };
    static const uint32_t stress_size[LAXITY_AGGR_REGIONS] = {
      LAXITY_STRESS_BYTES_S1, LAXITY_STRESS_BYTES_S2, LAXITY_STRESS_BYTES_S3
    };
    static const uint32_t desc_off[LAXITY_AGGR_REGIONS] = {
      LAXITY_DESC_OFF_S1, LAXITY_DESC_OFF_S2, LAXITY_DESC_OFF_S3
    };
    static const uint32_t victim_off[LAXITY_AGGR_REGIONS] = {
      LAXITY_VICTIM_OFF_S1, LAXITY_VICTIM_OFF_S2, LAXITY_VICTIM_OFF_S3
    };
    static const uint32_t victim_size[LAXITY_AGGR_REGIONS] = {
      LAXITY_VICTIM_BYTES_S1, LAXITY_VICTIM_BYTES_S2, LAXITY_VICTIM_BYTES_S3
    };
    uintptr_t span_lo = (uintptr_t)laxity_arena_span;
    uintptr_t span_hi = span_lo + sizeof laxity_arena_span;

    for (uint32_t r = 0u; r < LAXITY_AGGR_REGIONS; ++r)
    {
      const qos_mem_region_t *reg = qos_mem_region((uint8_t)(QOS_REGION_SRAM1 + r));
      uintptr_t reg_hi, taken, lo, hi, vlo, vhi;
      if (reg == NULL) { return 0u; }
      reg_hi = (uintptr_t)reg->base + reg->size;
      /* the arena and the mem2mem buffers end one arena alignment plus two maximum footprints above the arena, and nothing else in this region may start below that. */
      taken = (uintptr_t)laxity_arena[r] + LAXITY_ARENA_ALIGN + 2u * LAXITY_AGGR_MAX;

      lo = (uintptr_t)reg->base + stress_off[r];
      hi = lo + stress_size[r];
      if (hi > reg_hi) { return 0u; }
      if (lo < span_lo || hi > span_hi) { return 0u; }
      if (lo < taken) { return 0u; }
      laxity_stress_base[r] = (uint32_t)lo;
      laxity_stress_span[r] = stress_size[r];

      vlo = (uintptr_t)reg->base + victim_off[r];
      vhi = vlo + victim_size[r];
      if (vhi > reg_hi) { return 0u; }
      if (vlo < span_lo || vhi > span_hi) { return 0u; }
      if (vlo < taken) { return 0u; }
      /* the victim window is above the aggressor window in SRAM1 and SRAM2 and below it in SRAM3, so what is checked is disjointness rather than an order. a victim reading the memory the aggressor is writing would still return a number, and that number would look like contention while measuring something else entirely. */
      if (vlo < hi && lo < vhi) { return 0u; }
      laxity_victim_base[r] = (uint8_t *)vlo;
      laxity_victim_window[r] = victim_size[r];

      /* the descriptor page. it sits above the aggressor window in SRAM1 and SRAM2 and below it in SRAM3, so this is disjointness against both windows rather than an order. writing it as an order is what made the first build of this refuse to place at all. */
      {
        uintptr_t dlo = (uintptr_t)reg->base + desc_off[r];
        uintptr_t dhi = dlo + LAXITY_DESC_BYTES;
        uintptr_t klo = (uintptr_t)laxity_stack_page((uint8_t)(QOS_REGION_SRAM1 + r));
        uintptr_t khi = klo + LAXITY_STACK_BYTES;

        if (dhi > reg_hi) { return 0u; }
        if (dlo < span_lo || dhi > span_hi) { return 0u; }
        if (dlo < taken) { return 0u; }
        if (dlo < hi && lo < dhi) { return 0u; }
        if (dlo < vhi && vlo < dhi) { return 0u; }
        laxity_desc_base[r] = (uint32_t)dlo;

        /* the stack page for this region. the thread has been running on one of the three since before this function existed in the boot order, so what is still possible here is to refuse to measure when a page overlaps something rather than to move it. */
        if (klo == 0u) { return 0u; }
        if (khi > reg_hi) { return 0u; }
        if (klo < taken) { return 0u; }
        if (klo < hi && lo < khi) { return 0u; }
        if (klo < vhi && vlo < khi) { return 0u; }
        if (klo < dhi && dlo < khi) { return 0u; }
      }
    }

    /* SRAM4 carries no arena and no victim, only an aggressor window, and it is outside the reservation because it cannot be inside one: it sits at 0x28000000 in the SmartRun domain, outside the linker's RAM region, and the map file places no section in the SRAM4 region it declares, so nothing the linker allocated can collide with it. the clock is enabled here rather than assumed, although reading RCC_AHB3ENR on the running board showed bit 31 already set out of reset. */
    __HAL_RCC_SRAM4_CLK_ENABLE();
    laxity_stress_base[LAXITY_AGGR_REGIONS] = QOS_SRAM4_BASE;
    laxity_stress_span[LAXITY_AGGR_REGIONS] = LAXITY_STRESS_BYTES_S4;
    laxity_desc_base[LAXITY_AGGR_REGIONS] = QOS_SRAM4_BASE + LAXITY_DESC_OFF_S4;
    if ((LAXITY_DESC_OFF_S4 + LAXITY_DESC_BYTES) > QOS_SRAM4_SIZE) { return 0u; }
    if (LAXITY_STRESS_BYTES_S4 > LAXITY_DESC_OFF_S4) { return 0u; }
    laxity_desc_ok = 1u;
  }

  return 1u;
}

/* put the channels into the configuration the cell asks for, or stop them for the off cell.
 *
 * only on a change, since a stop and a start cost more than the comparison and the schedule
 * repeats points. the region is a run time choice and the window for it was validated above. */
static void laxity_set_stress(uint8_t point)
{
  uint32_t r;

  if (point == laxity_stress_point) { return; }
  laxity_stress_point = point;

  if (point == 0u || !laxity_stress_ok)
  {
    qos_stress_stop();
    return;
  }

  r = (uint32_t)laxity_stress_region - QOS_REGION_SRAM1;
  if (r >= LAXITY_STRESS_REGIONS) { qos_stress_stop(); return; }

  (void)qos_stress_start(laxity_stress_base[r], laxity_stress_span[r],
                         &laxity_sweeps[laxity_sweep].pt[point - 1u]);
}

static void laxity_build_cells(void)
{
  uint32_t n = 0u;
  for (uint32_t a = 0u; a < LAXITY_ARENAS; ++a)
  {
    /* every arena label gets an aggressor off cell, which is what the contended numbers are compared against and what the control measures its noise floor in. */
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

/* switch the aggressor only when the cell asks for something different, since a stop and start costs more than the comparison does and the schedule repeats cells. */
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

/* the off cell plus the active sweep's points, each repeated, then shuffled.
 *
 * shuffled for the same reason the arena cross is: thermal drift and flash state otherwise
 * correlate with position in the run, and a sweep read off a monotonically ordered schedule would
 * carry that drift as if it were the shape of the curve. */
static void laxity_stress_shuffle(uint8_t first_point)
{
  uint32_t cells = (first_point == 0u) ? (1u + (uint32_t)laxity_sweeps[laxity_sweep].points)
                                       : (uint32_t)laxity_sweeps[laxity_sweep].points;
  uint32_t n = LAXITY_STRESS_REPS * cells;

  for (uint32_t i = 0u; i < n; ++i)
  {
    laxity_stress_schedule[i] = (uint16_t)(first_point + (i % cells));
  }
  for (uint32_t i = n - 1u; i > 0u; --i)
  {
    uint32_t j = laxity_rand() % (i + 1u);
    uint16_t t = laxity_stress_schedule[i];
    laxity_stress_schedule[i] = laxity_stress_schedule[j];
    laxity_stress_schedule[j] = t;
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
  /* live mode only reaches the model once a whole window has been collected, so a half filled
     buffer never gets classified and reported as if it were motion. */
  if (laxity_input_mode == LAXITY_INPUT_LIVE && qos_sensors_ready())
  {
    qos_sensors_window(laxity_live_window);
    memcpy(inputs[0], laxity_live_window, STAI_NETWORK_IN_1_SIZE_BYTES);
  }
  else
  {
    memcpy(inputs[0], laxity_golden_input, STAI_NETWORK_IN_1_SIZE_BYTES);
  }

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
  laxity_stress_ok = qos_stress_init() ? 1u : 0u;
  laxity_sensor_ok = qos_sensors_init() ? 1u : 0u;
  laxity_audio_ok = qos_audio_init() ? 1u : 0u;
  laxity_btn_idle = (uint8_t)HAL_GPIO_ReadPin(USER_Button_GPIO_Port, USER_Button_Pin);
  laxity_btn_last = laxity_btn_idle;
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
    /* the victim, its region, its footprint and its load count are read once per pass rather than
       per iteration, so a console command never lands between the shuffle and the schedule it
       produced. */
    uint8_t victim = laxity_victim;
    uint8_t stress_sched = (victim != LAXITY_VICTIM_INFER) ? 1u : 0u;
    /* the timing mode has no aggressor off cell to measure, since there is no victim to leave
       alone, so its schedule starts at the first real point. */
    uint8_t first_point = (victim == LAXITY_VICTIM_DMATIME) ? 1u : 0u;
    uint32_t vr = (uint32_t)laxity_victim_region - QOS_REGION_SRAM1;
    uint32_t vwords, vpasses, entries;

    if (vr >= LAXITY_AGGR_REGIONS) { vr = 0u; }
    vwords = laxity_victim_footprints[laxity_victim_foot] / 4u;
    /* clamped to the window the region has. SRAM2 keeps one arena alignment once two arenas and
       two mem2mem buffers are placed, so a request for 128 KiB there has to become what fits
       instead of a read past the end of the region. */
    if (vwords > (laxity_victim_window[vr] / 4u)) { vwords = laxity_victim_window[vr] / 4u; }
    vpasses = laxity_victim_loads / vwords;
    if (vpasses == 0u) { vpasses = 1u; }
    laxity_victim_words = vwords;
    laxity_victim_passes = vpasses;

    if (stress_sched)
    {
      /* the mem2mem aggressor belongs to the arena cross and no stress pass touches it, so a pass
         that inherited it from the last inference cell would run a second uncontrolled master for
         the whole capture. stopping it is what makes the stress aggressor the only one there. */
      if (laxity_m2m_hold)
      {
        laxity_set_aggressor((uint8_t)laxity_stress_region, LAXITY_FOOTPRINTS - 1u);
      }
      else
      {
        laxity_set_aggressor(0u, 0u);
      }
      /* the descriptor page is chosen before any channel is started, because the nodes are built
         into it and a channel already running would keep reading the old ones. */
      if (laxity_desc_ok)
      {
        uint32_t dr = (uint32_t)laxity_desc_region - QOS_REGION_SRAM1;
        qos_stress_descriptors((dr < LAXITY_STRESS_REGIONS) ? laxity_desc_base[dr] : 0u,
                               LAXITY_DESC_BYTES);
      }
      laxity_stress_shuffle(first_point);
      entries = LAXITY_STRESS_REPS *
                ((first_point == 0u) ? (1u + (uint32_t)laxity_sweeps[laxity_sweep].points)
                                     : (uint32_t)laxity_sweeps[laxity_sweep].points);
    }
    else
    {
      laxity_shuffle();
      entries = LAXITY_SCHEDULE;
    }

    for (uint32_t i = 0u; i < entries; ++i)
    {
      qos_infer_record_t rec = {0};
      uint32_t cycles = 0u;
      bool wrapped = false;
      uint32_t slot = (victim == LAXITY_VICTIM_ISTRESS) ? (uint32_t)laxity_arena_slot : 0u;
      uint8_t  point = 0u;
      int run = 0;

      if (stress_sched)
      {
        point = (uint8_t)laxity_stress_schedule[i];
        /* set before the window opens and left alone inside it, so nothing this loop does to the
           channels is counted as part of the measured read. the timing mode is the exception:
           there the transfer is what the window contains, so it is started inside it. */
        if (victim != LAXITY_VICTIM_DMATIME) { laxity_set_stress(point); }
      }
      else
      {
        uint32_t cell = laxity_schedule[i];
        slot = laxity_cells[cell].arena_slot;
        laxity_set_aggressor(laxity_cells[cell].aggr_region, laxity_cells[cell].foot_idx);
      }

      /* all three are outside the cycle counter window below. the sensor decides its own spacing
         from its output data rate, so this only offers it the chance, and the microphone poll
         does nothing unless a half buffer has completed since the last pass. */
      (void)qos_sensors_sample();
      (void)qos_audio_poll();
      laxity_poll_button();

      rec.release_cyc = qos_cyc_now();

      if (victim == LAXITY_VICTIM_DMATIME)
      {
        /* one block, started and waited for. the window holds the channel start, the transfer and
           the poll that observes its completion flag, and nothing else. the start is register
           writes to the same peripheral at every point, so it lands in the intercept of the fit
           rather than in the slope. */
        const qos_stress_cfg_t *c = &laxity_sweeps[laxity_sweep].pt[point - 1u];
        uint32_t r = (uint32_t)laxity_stress_region - QOS_REGION_SRAM1;
        uint32_t t0, t1, k = 0u;
        bool ok;

        if (r >= LAXITY_STRESS_REGIONS) { r = 0u; }
        /* the node is built and linked before the window opens, so what the window holds is the
           channel start, the transfer and the poll that sees it finish. the build is the same work
           at every point and it would otherwise sit in the intercept of the fit, which is the one
           number the descriptor question needs to be readable. */
        ok = qos_stress_once_arm(laxity_stress_base[r], laxity_stress_span[r], c);
        t0 = qos_cyc_now();
        if (ok)
        {
          ok = qos_stress_once_fire();
          while (ok && k < LAXITY_DMA_POLL_MAX && !qos_stress_once_pending()) { ++k; }
          ok = ok && (k < LAXITY_DMA_POLL_MAX);
        }
        t1 = qos_cyc_now();
        cycles = qos_cyc_delta(t0, t1);
        wrapped = qos_cyc_wrapped(t0, t1);
        /* outside the window, and it is what returns the channel to a state the next arm can use. */
        if (ok) { ok = qos_stress_once_settle(); }
        run = ok ? 0 : -6;
      }
      else if (victim == LAXITY_VICTIM_READ)
      {
        /* a known number of loads over a fixed buffer, and nothing else inside the window. the
           accumulator is kept so the loads cannot be removed as dead. */
        uint32_t t0 = qos_cyc_now();
        laxity_read_acc += qos_read_loop((const volatile uint32_t *)laxity_victim_base[vr],
                                         vwords, vpasses);
        uint32_t t1 = qos_cyc_now();
        cycles = qos_cyc_delta(t0, t1);
        wrapped = qos_cyc_wrapped(t0, t1);
      }
      else
      {
        run = laxity_infer(slot, &cycles, &wrapped);
      }
      laxity_live.rc = run;

      if (run == 0)
      {
        laxity_live.cycles = cycles;
        if (victim == LAXITY_VICTIM_INFER || victim == LAXITY_VICTIM_ISTRESS)
        {
          int argmax = laxity_argmax();
          laxity_live.argmax = argmax;
          laxity_live.worst_ppm = laxity_worst_ppm();
          /* the golden check runs on every inference rather than once, since a wrongly placed arena
             that still returns a number would otherwise pass unnoticed. it only means anything
             against the golden window, so live classifications are never counted as mismatches. */
          if (laxity_input_mode == LAXITY_INPUT_GOLDEN && argmax != LAXITY_GOLDEN_CLASS)
          {
            laxity_live.mismatches++;
          }
        }
      }

      rec.exec_cyc = cycles;

      if (stress_sched)
      {
        /* region_id names where the victim is, which is the read loop window for a read pass and
           the SRAM1 arena for an inference pass. the point index goes where the footprint index
           goes, and which sweep those points belong to is in the capture metadata, because the
           record has no field left and the wire format is not being changed for this. */
        rec.region_id = (victim == LAXITY_VICTIM_READ) ? (uint8_t)(QOS_REGION_SRAM1 + vr)
                      : (victim == LAXITY_VICTIM_DMATIME) ? laxity_stress_region
                      : laxity_placements[slot].id;
        rec.aggressor_idx = (uint16_t)(((uint16_t)point << 8)
                                       | (point ? laxity_stress_region : 0u));
        rec.reserved = qos_stress_completions();
      }
      else
      {
        rec.region_id = laxity_placements[slot].id;
        rec.aggressor_idx = (uint16_t)(((uint16_t)laxity_cells[laxity_schedule[i]].foot_idx << 8)
                                       | laxity_cells[laxity_schedule[i]].aggr_region);
        rec.reserved = qos_gpdma_m2m_completions();
      }
      laxity_live.aggr_completions = rec.reserved;

      /* cpu_cyc, stall_cyc and model_id stay zero. there is no attribution measurement here, and
         writing a plausible value into a field nothing measured is how a number nobody can defend
         gets into a capture. */
      rec.flags = wrapped ? (uint8_t)QOS_FLAG_CYCCNT_WRAP : 0u;
      (void)qos_telemetry_push(&rec);

      tx_thread_sleep(LAXITY_INFER_PERIOD);
    }
    laxity_live.passes++;
  }
}

#if LAXITY_NET_ENABLE
/* bring the link up once, then report it. the result is published for the status line rather
   than printed here, because one thread owns the UART and it is not this one. */
/* the stack is started on request rather than at boot.
 *
 * starting it at boot makes the board stop scheduling this application entirely: the MXCHIP SPI
 * transmit and receive thread the driver creates runs at ThreadX priority 8, against 12, 15 and 20
 * here, and it holds the processor. the core keeps retiring instructions and no fault is raised,
 * so this is starvation rather than a hang. compiling the stack in and leaving it stopped is what
 * lets the rest of the campaign measure what the stack costs when the radio is idle. */
static uint8_t laxity_net_want;

/* and the telemetry export stays on the UART whatever the radio does. a measurement whose own transport is the aggressor mixes the two, which this project has already paid for twice. */
static uint8_t laxity_net_export;

static VOID laxity_net_entry(ULONG argument)
{
  (void)argument;

  while (!laxity_net_want) { tx_thread_sleep(TX_TIMER_TICKS_PER_SECOND / 10u); }

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
#endif

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
/* read one console command byte if one is waiting.
 *
 * this is what makes the four knobs run time choices. a configuration per binary would put a
 * relink inside every measured difference, and a relink on its own has moved a median here by 85
 * cycles, so the whole experiment would be unreadable.
 *
 * it runs in the exporter thread because that thread owns USART1. a second thread touching the
 * same peripheral would interleave with a frame and cost a record. the timeout is zero, so a tick
 * with nothing waiting costs a register read.
 *
 *   v  victim is the inference path        r  victim is the read loop
 *   i  victim is inference on the stress schedule, arena in SRAM1
 *   t  no victim, the DMA transfer itself is timed
 *   0  bandwidth sweep                     1  transactions at fixed bandwidth
 *   2  channel count at fixed bandwidth    3  address stride
 *   4  past the top of the bandwidth sweep, ending in a channel that never transfers
 *   5  below the bandwidth sweep, 0.4 to 3.2 million transactions per second
 *   6  one block moved once and timed, four points of transaction count
 *   o  no aggressor at all, for the contention free table
 *   W  start the network stack, which is not started at boot
 *   e  also send telemetry over UDP, which is off and should stay off during a measurement
 *   a  aggressor in SRAM1                  b  SRAM2        c  SRAM3        d  SRAM4
 *   X  read loop victim in SRAM1           Y  SRAM2        Z  SRAM3
 *   A to H  victim footprint 1, 2, 4, 8, 16, 32, 64 and 128 KiB
 *   P  descriptors in SRAM1                  Q  SRAM2        R  SRAM3        S  SRAM4
 *   j  stack in SRAM1                        k  SRAM2        n  SRAM3
 *   p  stack from the ThreadX byte pool, which is the vendor default and not a page at all
 *      the stack is fixed when the thread is created, so these reset the board through a word of
 *      SRAM4 and the board comes back on the page that was asked for
 *   f  ballast 0 bytes   g  64   h  256   m  1024   q  4096
 *      a block taken from the byte pool before the measurement thread is created, which moves
 *      whatever the pool hands out after it. it resets the board for the same reason
 *   7  inference arena in SRAM1              8  SRAM2        9  SRAM3
 *   l  4096 loads per window               L  8192 loads per window
 *   M  run the mem2mem aggressor through a stress pass, in the aggressor region, largest
 *      footprint, which reproduces the contamination the earlier stress captures were taken with
 *   N  stop it again, which is the normal state
 */
/* ask for a stack source, which means asking for a reset.
 *
 * nothing here can move a running thread onto another stack, so the choice is written where a warm
 * reset preserves it and the board comes back having made it. a request for the source already in
 * use resets nothing, so a capture that repeats the current setting costs no reboot. what is
 * compared is the source and not the region the stack landed in, since the byte pool is in SRAM3
 * and a request for it would otherwise be read as a request for the SRAM3 page. */
static void laxity_set_stack_src(uint8_t src)
{
  if (src == laxity_stack_src) { return; }
  laxity_boot_sel_write(src, laxity_ballast_idx);
  NVIC_SystemReset();
}

/* ask for a ballast size, which means asking for a reset for the same reason: the block is taken before the measurement thread exists and nothing after that can take it again. */
static void laxity_set_ballast(uint8_t idx)
{
  if (idx >= LAXITY_BALLASTS || idx == laxity_ballast_idx) { return; }
  laxity_boot_sel_write(laxity_stack_src, idx);
  NVIC_SystemReset();
}

static void laxity_poll_console(void)
{
  uint8_t c;

  if (HAL_UART_Receive(&huart1, &c, 1u, 0u) != HAL_OK) { return; }

  switch (c)
  {
    case 'v': laxity_victim = LAXITY_VICTIM_INFER;   break;
    case 'r': laxity_victim = LAXITY_VICTIM_READ;    break;
    case 'i': laxity_victim = LAXITY_VICTIM_ISTRESS; break;
    case 't': laxity_victim = LAXITY_VICTIM_DMATIME; laxity_stress_point = 0xFFu; break;
#if LAXITY_NET_ENABLE
    case 'W': laxity_net_want = 1u; break;
    case 'e': laxity_net_export = 1u; break;
#endif
    case 'o':
      laxity_sweep = LAXITY_SWEEP_NONE;
      laxity_stress_point = 0xFFu;
      break;
    case '0': case '1': case '2': case '3': case '4': case '5': case '6':
      laxity_sweep = (uint8_t)(c - '0');
      /* the point index is invalidated rather than kept, so the next cell reprograms the channels
         even if it happens to carry the same number as the last one did under the old sweep. */
      laxity_stress_point = 0xFFu;
      break;
    case 'a': laxity_stress_region = QOS_REGION_SRAM1; laxity_stress_point = 0xFFu; break;
    case 'b': laxity_stress_region = QOS_REGION_SRAM2; laxity_stress_point = 0xFFu; break;
    case 'c': laxity_stress_region = QOS_REGION_SRAM3; laxity_stress_point = 0xFFu; break;
    case 'd': laxity_stress_region = QOS_REGION_SRAM4; laxity_stress_point = 0xFFu; break;
    /* the descriptor page invalidates the point for the same reason a sweep change does: the
       nodes are rebuilt from it and a cell that kept its number would not be reprogrammed. */
    case 'P': laxity_desc_region = QOS_REGION_SRAM1; laxity_stress_point = 0xFFu; break;
    case 'Q': laxity_desc_region = QOS_REGION_SRAM2; laxity_stress_point = 0xFFu; break;
    case 'R': laxity_desc_region = QOS_REGION_SRAM3; laxity_stress_point = 0xFFu; break;
    case 'S': laxity_desc_region = QOS_REGION_SRAM4; laxity_stress_point = 0xFFu; break;
    case 'j': laxity_set_stack_src(QOS_REGION_SRAM1); break;
    case 'k': laxity_set_stack_src(QOS_REGION_SRAM2); break;
    case 'n': laxity_set_stack_src(QOS_REGION_SRAM3); break;
    case 'p': laxity_set_stack_src(LAXITY_STACK_SRC_POOL); break;
    case 'f': laxity_set_ballast(0u); break;
    case 'g': laxity_set_ballast(1u); break;
    case 'h': laxity_set_ballast(2u); break;
    case 'm': laxity_set_ballast(3u); break;
    case 'q': laxity_set_ballast(4u); break;
    case '7': laxity_arena_slot = 0u; break;
    case '8': laxity_arena_slot = 1u; break;
    case '9': laxity_arena_slot = 2u; break;
    case 'X': laxity_victim_region = QOS_REGION_SRAM1; break;
    case 'Y': laxity_victim_region = QOS_REGION_SRAM2; break;
    case 'Z': laxity_victim_region = QOS_REGION_SRAM3; break;
    case 'A': case 'B': case 'C': case 'D':
    case 'E': case 'F': case 'G': case 'H':
      laxity_victim_foot = (uint8_t)(c - 'A');
      break;
    /* the load count is a knob of its own rather than a consequence of the footprint, because one
       prediction under test is that the SRAM2 speedup is proportional to the victim's access count
       and halving the loads at a fixed footprint is the only way to ask that. */
    case 'l': laxity_victim_loads = 4096u; break;
    case 'L': laxity_victim_loads = QOS_READ_LOOP_LOADS; break;
    case 'M': laxity_m2m_hold = 1u; break;
    case 'N': laxity_m2m_hold = 0u; break;
    default: break;
  }
}

static VOID laxity_export_entry(ULONG argument)
{
  char line[288];
  uint32_t ticks = 0u;
  int argmax;
  int golden;

  (void)argument;

  (void)tx_semaphore_get(&laxity_boot_done, TX_WAIT_FOREVER);

  while (1)
  {
    laxity_poll_console();

    size_t n = qos_telemetry_drain(laxity_export_buf, sizeof laxity_export_buf);
    if (n > 0u)
    {
      /* the UART is the path that always works and it stays first. the datagram carries the
         same bytes, so the host parser reads either without knowing which it received. */
      HAL_UART_Transmit(&huart1, laxity_export_buf, (uint16_t)n, HAL_MAX_DELAY);
#if LAXITY_NET_ENABLE
      if (laxity_net_export) { (void)qos_net_send(laxity_export_buf, (UINT)n); }
#endif
    }

    /* one human readable set per second, so a silent board is still distinguishable from a broken one without running the parser. */
    if ((++ticks % TX_TIMER_TICKS_PER_SECOND) == 0u)
    {
      argmax = (int)laxity_live.argmax;
      golden = (laxity_input_mode == LAXITY_INPUT_GOLDEN) || !qos_sensors_ready();
      laxity_say(line, snprintf(line, sizeof line,
                   "infer mode=%s rc=%ld cycles=%lu class=%d(%s) expect=%d(%s) %s mismatch=%lu pass=%lu dma=%u/%lu opt=-O0\r\n",
                   golden ? "golden" : "live",
                   (long)laxity_live.rc, (unsigned long)laxity_live.cycles,
                   argmax, laxity_golden_class_names[argmax],
                   LAXITY_GOLDEN_CLASS, laxity_golden_class_names[LAXITY_GOLDEN_CLASS],
                   /* the verdict is only a verdict against the golden window. saying MATCH while
                      reading a sensor would claim a check that is not being made. */
                   golden ? ((laxity_live.rc == 0 && argmax == LAXITY_GOLDEN_CLASS) ? "MATCH" : "MISMATCH")
                          : "live-no-verdict",
                   (unsigned long)laxity_live.mismatches, (unsigned long)laxity_live.passes,
                   (unsigned)laxity_aggr_ok, (unsigned long)laxity_live.aggr_completions));

      /* the addresses are on the wire in the header frame as well. printing them makes a wrong placement visible with head -c, before anyone runs the parser.
         the arena addresses here are the candidates every run carries, which is what the line said before the vendor stack existed. the stack fields are the address the thread was actually given and the region that address is in, because with the byte pool as the source neither is a link time constant and neither can be read off any candidate. */
      laxity_say(line, snprintf(line, sizeof line,
                   "placement ok=%u span=0x%08lx..0x%08lx s1=0x%08lx s2=0x%08lx s3=0x%08lx s2b=0x%08lx ctl=0x%08lx bytes=%u"
                   " stack_src=%u stack_addr=0x%08lx stack_region=%u ballast=%lu ballast_addr=0x%08lx\r\n",
                   (unsigned)laxity_placed_ok,
                   (unsigned long)(uintptr_t)laxity_arena_span,
                   (unsigned long)((uintptr_t)laxity_arena_span + sizeof laxity_arena_span),
                   (unsigned long)laxity_placements[0].arena_addr,
                   (unsigned long)laxity_placements[1].arena_addr,
                   (unsigned long)laxity_placements[2].arena_addr,
                   (unsigned long)laxity_placements[3].arena_addr,
                   (unsigned long)laxity_placements[4].arena_addr,
                   (unsigned)LAXITY_ARENA_BYTES,
                   (unsigned)laxity_stack_src,
                   (unsigned long)(uintptr_t)laxity_stack_addr,
                   (unsigned)laxity_stack_region,
                   (unsigned long)laxity_ballast_bytes,
                   (unsigned long)laxity_ballast_addr));

      {
        /* the active configuration, so a capture taken while this was on the wire can be checked
           against the name it was filed under rather than against someone's memory of it. */
        uint8_t  pt = (laxity_stress_point == 0xFFu) ? 0u : laxity_stress_point;
        uint8_t  sguard = 0u;
        const qos_stress_cfg_t *c = (pt > 0u) ? &laxity_sweeps[laxity_sweep].pt[pt - 1u] : NULL;
        laxity_say(line, snprintf(line, sizeof line,
                     "stress victim=%s m2m=%u stack=%u arena=%u desc=%u(0x%08lx) "
                     "vregion=%u vwords=%lu vpasses=%lu vloads=%lu "
                     "sweep=%s region=%u ok=%u point=%u/%u "
                     "chan=%u width=%u block=%lu stride=%lu hz=%lu Bps=%lu xps=%lu run=%u xfer=%lu "
                     "shw=%lu sguard=%u\r\n",
                     (laxity_victim == LAXITY_VICTIM_READ) ? "read"
                       : (laxity_victim == LAXITY_VICTIM_DMATIME) ? "dma"
                       : (laxity_victim == LAXITY_VICTIM_ISTRESS) ? "infer-stress" : "infer",
                     (unsigned)laxity_m2m_hold,
                     (unsigned)laxity_stack_region,
                     (unsigned)laxity_placements[laxity_arena_slot].id,
                     (unsigned)laxity_region_of((uintptr_t)qos_stress_descriptor_addr()),
                     (unsigned long)qos_stress_descriptor_addr(),
                     (unsigned)laxity_victim_region,
                     (unsigned long)laxity_victim_words,
                     (unsigned long)laxity_victim_passes,
                     (unsigned long)(laxity_victim_words * laxity_victim_passes),
                     laxity_sweeps[laxity_sweep].name,
                     (unsigned)laxity_stress_region, (unsigned)laxity_stress_ok,
                     (unsigned)pt, (unsigned)laxity_sweeps[laxity_sweep].points,
                     (unsigned)(c ? c->channels : 0u), (unsigned)(c ? c->width : 0u),
                     (unsigned long)(c ? c->block_bytes : 0u),
                     (unsigned long)(c ? c->stride : 0u),
                     (unsigned long)(c ? c->trigger_hz : 0u),
                     (unsigned long)(c ? qos_stress_bytes_per_s(c) : 0u),
                     (unsigned long)(c ? qos_stress_xacts_per_s(c) : 0u),
                     /* whether the channels are enabled, which the transfer count cannot say on
                        the one point that is meant to move nothing. a point with run=1 and a flat
                        transfer count is a configured channel carrying no traffic, and a point
                        with run=0 is a channel that did not start. */
                     (unsigned)(qos_stress_running() ? 1u : 0u),
                     (unsigned long)qos_stress_completions(),
                     (unsigned long)laxity_stack_high_water(&sguard), (unsigned)sguard));
      }

      {
        float t = qos_sensors_temperature();
        int   ti = (int)t;
        int   tf = (int)((t - (float)ti) * 10.0f); if (tf < 0) { tf = -tf; }
        laxity_say(line, snprintf(line, sizeof line,
                     "sensors acc=%u env=%u(inst%lu) samples=%lu window=%s peak_mg=%d temp_c=%d.%d mic=%u bufs=%lu mic_peak=%ld\r\n",
                     (unsigned)laxity_sensor_ok, (unsigned)(qos_sensors_env_ok() ? 1u : 0u),
                     (unsigned long)qos_sensors_env_instance(),
                     (unsigned long)qos_sensors_count(),
                     qos_sensors_ready() ? "full" : "filling",
                     (int)(qos_sensors_peak_g() * 1000.0f), ti, tf,
                     (unsigned)laxity_audio_ok, (unsigned long)qos_audio_buffers(),
                     (long)qos_audio_peak()));
      }

#if !LAXITY_NET_ENABLE
      /* saying rc=0 while the stack was never started would read as a link that came up. */
      laxity_say(line, snprintf(line, sizeof line, "net disabled\r\n"));
#else
      laxity_say(line, snprintf(line, sizeof line,
                   "net started=%u export=%u rc=%ld ip=%lu.%lu.%lu.%lu sent=%lu failed=%lu\r\n",
                   (unsigned)laxity_net_want, (unsigned)laxity_net_export,
                   (long)laxity_live.net_rc,
                   (unsigned long)((laxity_live.net_addr >> 24) & 0xFFu),
                   (unsigned long)((laxity_live.net_addr >> 16) & 0xFFu),
                   (unsigned long)((laxity_live.net_addr >> 8) & 0xFFu),
                   (unsigned long)(laxity_live.net_addr & 0xFFu),
                   (unsigned long)laxity_live.net_sent,
                   (unsigned long)laxity_live.net_failed));
#endif

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
