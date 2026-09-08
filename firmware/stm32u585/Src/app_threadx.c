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
#include "null_probe.h"
#include "qos/telemetry.h"
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
/* snprintf through newlib takes several hundred bytes of stack on its own, and the block 2 lesson was that a stack or pool shortfall here shows up as a silent port rather than as an error, so the margin is bought up front. both stacks come out of TX_APP_MEM_POOL_SIZE, which the .ioc raised from 4096 to 8192 for the second thread. the activation buffer, the network context and the export buffer are static rather than stack allocated. */
#define LAXITY_INFER_STACK   3072
#define LAXITY_INFER_PRIO    15
#define LAXITY_EXPORT_STACK  2048
#define LAXITY_EXPORT_PRIO   20

/* lower priority than the measurement thread, so a blocking transmit that takes several milliseconds at 921600 baud cannot delay an inference. that is what best effort export means here. */

/* 512 bytes carries a header frame and fourteen records, against a ring of thirty two and a producer running at ten hertz, so the exporter stays ahead without a larger buffer. */
#define LAXITY_EXPORT_BUF    512

/* what the .ioc configures, reported in the header frame beside the rate qos_dwt_measure_hz() actually observed, since a capture has to be able to show a clock configuration that did not take. */
#define LAXITY_SYSCLK_HZ     160000000u

/* ten hertz. fast enough to fill the ring during a blocked transmit and slow enough that the exporter drains it every time. */
#define LAXITY_INFER_PERIOD  (TX_TIMER_TICKS_PER_SECOND / 10u)

/* USER CODE END PD */

/* Private macro -------------------------------------------------------------*/
/* USER CODE BEGIN PM */

/* USER CODE END PM */

/* Private variables ---------------------------------------------------------*/
/* USER CODE BEGIN PV */
static TX_THREAD    laxity_infer_thread;
static TX_THREAD    laxity_export_thread;

/* the exporter waits on this rather than sleeping for a plausible interval. the null probe pushes records and resets the ring, so a consumer that started draining during boot would both corrupt the probe and put its synthetic records on the wire. */
static TX_SEMAPHORE laxity_boot_done;

static uint8_t laxity_export_buf[LAXITY_EXPORT_BUF];

/* the vendor default, kept exactly as ST writes it: a plain static array whose address the linker chooses. docs/CHARTER.md defines the baseline as the activation buffer wherever the default linker configuration puts it, so placing this deliberately would destroy the comparison block 15 depends on. handing the arena over on purpose is block 6. */
static uint8_t laxity_activations[STAI_NETWORK_ACTIVATIONS_SIZE_BYTES] __attribute__((aligned(8)));
static uint8_t laxity_net_ctx[STAI_NETWORK_CONTEXT_SIZE] __attribute__((aligned(STAI_NETWORK_CONTEXT_ALIGNMENT)));
static float   laxity_out[LAXITY_GOLDEN_OUT_LEN];

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
} laxity_live;

/* USER CODE END PV */

/* Private function prototypes -----------------------------------------------*/
/* USER CODE BEGIN PFP */
static VOID laxity_infer_entry(ULONG argument);
static VOID laxity_export_entry(ULONG argument);

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

/* bind the network and hand over the activations once. doing it per inference would put allocator work in front of every measurement and would measure a cold network on every iteration, which docs/EXPERIMENTS.md rules out by asking for a warm up. */
static int laxity_infer_setup(void)
{
  stai_ptr acts[STAI_NETWORK_ACTIVATIONS_NUM] = { (stai_ptr)laxity_activations };

  laxity_net = (stai_network *)laxity_net_ctx;
  if (stai_network_init(laxity_net) != STAI_SUCCESS) { return -1; }
  if (stai_network_set_activations(laxity_net, acts, STAI_NETWORK_ACTIVATIONS_NUM) != STAI_SUCCESS) { return -2; }
  return 0;
}

/* one inference, with the cycle counter wrapped around stai_network_run() and nothing else. everything the record needs about the window comes out of here, since a caller reading the counter again later would include the output copy in the measurement. */
static int laxity_infer(uint32_t *cycles, bool *wrapped)
{
  stai_ptr inputs[STAI_NETWORK_IN_NUM];
  stai_ptr outputs[STAI_NETWORK_OUT_NUM];
  stai_size n;
  uint32_t t0, t1;

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

  /* QOS_HDR_STALL_POPULATED stays clear. the counters exist on this part, which block 3 measured, and nothing fills stall_cyc from them until the RQ2 attribution work, so a stream that claimed both bits would describe a measurement this build does not make. */
  qos_telemetry_init(LAXITY_SYSCLK_HZ, laxity_boot.cyccnt_hz,
                     laxity_boot.caps.prfcnt_claimed ? QOS_HDR_STALL_AVAILABLE : 0u);

  laxity_boot.probe = qos_null_probe_run();

  rc = laxity_infer_setup();
  laxity_live.rc = rc;

  /* only now may the exporter touch the ring, since the null probe pushes records into it and resets it. */
  (void)tx_semaphore_put(&laxity_boot_done);

  while (1)
  {
    qos_infer_record_t rec = {0};
    uint32_t cycles = 0u;
    bool wrapped = false;

    rec.release_cyc = qos_cyc_now();

    if (rc == 0)
    {
      int run = laxity_infer(&cycles, &wrapped);
      laxity_live.rc = run;
      if (run == 0)
      {
        laxity_live.cycles = cycles;
        laxity_live.argmax = laxity_argmax();
        laxity_live.worst_ppm = laxity_worst_ppm();
      }
    }

    rec.exec_cyc = cycles;
    /* cpu_cyc and stall_cyc stay zero, as do model_id, region_id and aggressor_idx. there is no attribution measurement, no model table and no placement control in this block, and writing a plausible value into a field nothing measured is how a number nobody can defend gets into a capture. */
    rec.flags = wrapped ? (uint8_t)QOS_FLAG_CYCCNT_WRAP : 0u;
    (void)qos_telemetry_push(&rec);

    tx_thread_sleep(LAXITY_INFER_PERIOD);
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
  char line[220];
  uint32_t ticks = 0u;
  int argmax;

  (void)argument;

  (void)tx_semaphore_get(&laxity_boot_done, TX_WAIT_FOREVER);

  while (1)
  {
    size_t n = qos_telemetry_drain(laxity_export_buf, sizeof laxity_export_buf);
    if (n > 0u)
    {
      HAL_UART_Transmit(&huart1, laxity_export_buf, (uint16_t)n, HAL_MAX_DELAY);
    }

    /* one human readable pair per second, so a silent board is still distinguishable from a broken one without running the parser. */
    if ((++ticks % TX_TIMER_TICKS_PER_SECOND) == 0u)
    {
      argmax = (int)laxity_live.argmax;
      laxity_say(line, snprintf(line, sizeof line,
                   "infer rc=%ld cycles=%lu class=%d(%s) expect=%d(%s) %s worst_diff_ppm=%lu opt=-O0\r\n",
                   (long)laxity_live.rc, (unsigned long)laxity_live.cycles,
                   argmax, laxity_golden_class_names[argmax],
                   LAXITY_GOLDEN_CLASS, laxity_golden_class_names[LAXITY_GOLDEN_CLASS],
                   (laxity_live.rc == 0 && argmax == LAXITY_GOLDEN_CLASS) ? "MATCH" : "MISMATCH",
                   (unsigned long)laxity_live.worst_ppm));

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
