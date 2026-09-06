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
/* snprintf through newlib takes several hundred bytes of stack on its own, and the block 2 lesson was that a stack or pool shortfall here shows up as a silent port rather than as an error, so the margin is bought up front. It stays under TX_APP_MEM_POOL_SIZE, which is 4096, because raising the pool would mean editing the .ioc and block 4 does not regenerate. The activation buffer and the network context are static rather than stack allocated, so the inference call itself adds little to this. */
#define LAXITY_HELLO_STACK  3072
#define LAXITY_HELLO_PRIO   15

/* USER CODE END PD */

/* Private macro -------------------------------------------------------------*/
/* USER CODE BEGIN PM */

/* USER CODE END PM */

/* Private variables ---------------------------------------------------------*/
/* USER CODE BEGIN PV */
static TX_THREAD laxity_hello_thread;

/* The vendor default, kept exactly as ST writes it: a plain static array whose address the linker chooses. docs/CHARTER.md defines the baseline as the activation buffer wherever the default linker configuration puts it, so placing this deliberately would destroy the comparison block 15 depends on. Handing the arena over on purpose is block 6. */
static uint8_t laxity_activations[STAI_NETWORK_ACTIVATIONS_SIZE_BYTES] __attribute__((aligned(8)));
static uint8_t laxity_net_ctx[STAI_NETWORK_CONTEXT_SIZE] __attribute__((aligned(STAI_NETWORK_CONTEXT_ALIGNMENT)));
static float   laxity_out[LAXITY_GOLDEN_OUT_LEN];

/* USER CODE END PV */

/* Private function prototypes -----------------------------------------------*/
/* USER CODE BEGIN PFP */
static VOID laxity_hello_entry(ULONG argument);

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
  CHAR *stack_ptr;

  /* USER CODE END App_ThreadX_MEM_POOL */
  /* USER CODE BEGIN App_ThreadX_Init */
  /* The stack comes from the ThreadX byte pool rather than a static array,
     since the pool is the allocation CubeMX sizes and reports. */
  if (tx_byte_allocate(byte_pool, (VOID **)&stack_ptr, LAXITY_HELLO_STACK,
                       TX_NO_WAIT) != TX_SUCCESS)
  {
    return TX_POOL_ERROR;
  }

  if (tx_thread_create(&laxity_hello_thread, "laxity_hello", laxity_hello_entry, 0,
                       stack_ptr, LAXITY_HELLO_STACK,
                       LAXITY_HELLO_PRIO, LAXITY_HELLO_PRIO,
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
/* Run the vendor path once and report what it cost and what it produced. The weights are bound inside network.c at init, so only the activations have to be handed over. */
static int laxity_infer(uint32_t *cycles)
{
  stai_network *net = (stai_network *)laxity_net_ctx;
  stai_ptr acts[STAI_NETWORK_ACTIVATIONS_NUM] = { (stai_ptr)laxity_activations };
  stai_ptr inputs[STAI_NETWORK_IN_NUM];
  stai_ptr outputs[STAI_NETWORK_OUT_NUM];
  stai_size n;
  uint32_t t0, t1;

  if (stai_network_init(net) != STAI_SUCCESS) { return -1; }
  if (stai_network_set_activations(net, acts, STAI_NETWORK_ACTIVATIONS_NUM) != STAI_SUCCESS) { return -2; }

  n = STAI_NETWORK_IN_NUM;
  if (stai_network_get_inputs(net, inputs, &n) != STAI_SUCCESS) { return -3; }
  memcpy(inputs[0], laxity_golden_input, STAI_NETWORK_IN_1_SIZE_BYTES);

  t0 = qos_cyc_now();
  if (stai_network_run(net, STAI_MODE_SYNC) != STAI_SUCCESS) { return -4; }
  t1 = qos_cyc_now();
  *cycles = qos_cyc_delta(t0, t1);

  n = STAI_NETWORK_OUT_NUM;
  if (stai_network_get_outputs(net, outputs, &n) != STAI_SUCCESS) { return -5; }
  memcpy(laxity_out, outputs[0], STAI_NETWORK_OUT_1_SIZE_BYTES);
  return 0;
}

/* One report per second over the ST-LINK virtual COM port. It repeats so a capture attached after reset still sees it. */
static VOID laxity_hello_entry(ULONG argument)
{
  char line[200];
  uint32_t cycles = 0u;
  uint32_t worst_ppm = 0u;
  int rc, argmax = 0, i, n;

  (void)argument;

  (void)qos_dwt_init();

  rc = laxity_infer(&cycles);

  for (i = 1; i < LAXITY_GOLDEN_OUT_LEN; ++i)
  {
    if (laxity_out[i] > laxity_out[argmax]) { argmax = i; }
  }

  /* The difference is reported in parts per million as an integer rather than as a float, since printing floats would pull newlib's formatting onto a path that later carries a measurement. */
  for (i = 0; i < LAXITY_GOLDEN_OUT_LEN; ++i)
  {
    uint32_t ppm = (uint32_t)(fabsf(laxity_out[i] - laxity_golden_output[i]) * 1000000.0f);
    if (ppm > worst_ppm) { worst_ppm = ppm; }
  }

  while (1)
  {
    n = snprintf(line, sizeof line,
                 "infer rc=%d cycles=%lu class=%d(%s) expect=%d(%s) %s worst_diff_ppm=%lu opt=-O0\r\n",
                 rc, (unsigned long)cycles,
                 argmax, laxity_golden_class_names[argmax],
                 LAXITY_GOLDEN_CLASS, laxity_golden_class_names[LAXITY_GOLDEN_CLASS],
                 (rc == 0 && argmax == LAXITY_GOLDEN_CLASS) ? "MATCH" : "MISMATCH",
                 (unsigned long)worst_ppm);
    if (n > 0)
    {
      HAL_UART_Transmit(&huart1, (uint8_t *)line, (uint16_t)n, HAL_MAX_DELAY);
    }
    tx_thread_sleep(TX_TIMER_TICKS_PER_SECOND);
  }
}

/* USER CODE END 1 */
