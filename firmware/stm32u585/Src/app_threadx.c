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
#include <stdio.h>

extern UART_HandleTypeDef huart1;

/* USER CODE END Includes */

/* Private typedef -----------------------------------------------------------*/
/* USER CODE BEGIN PTD */

/* USER CODE END PTD */

/* Private define ------------------------------------------------------------*/
/* USER CODE BEGIN PD */
/* snprintf through newlib takes several hundred bytes of stack on its own, and the block 2 lesson was that a stack or pool shortfall here shows up as a silent port rather than as an error, so the margin is bought up front out of the 4096 byte pool. */
#define LAXITY_HELLO_STACK  2048
#define LAXITY_HELLO_PRIO   15

/* USER CODE END PD */

/* Private macro -------------------------------------------------------------*/
/* USER CODE BEGIN PM */

/* USER CODE END PM */

/* Private variables ---------------------------------------------------------*/
/* USER CODE BEGIN PV */
static TX_THREAD laxity_hello_thread;

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
/* One report per second over the ST-LINK virtual COM port. It repeats so a capture attached after reset still sees it. */
static VOID laxity_hello_entry(ULONG argument)
{
  char line[160];
  bool dwt_ok;
  uint32_t hz;
  qos_dwt_caps_t caps;
  int n;

  (void)argument;

  dwt_ok = qos_dwt_init();
  hz = dwt_ok ? qos_dwt_measure_hz(HAL_GetTick, 200u) : 0u;
  caps = qos_dwt_probe(HAL_GetTick, 5u);

  while (1)
  {
    /* The measurement runs once at start up and is reported every second, since re measuring each second would report the same clock tree over and over and put a 200 ms busy wait in the loop for nothing. */
    n = snprintf(line, sizeof line,
                 "dwt=%s cyccnt_hz=%lu ctrl=0x%08lX noprfcnt=%s "
                 "cpi=%s exc=%s lsu=%s fold=%s\r\n",
                 dwt_ok ? "ok" : "FAIL",
                 (unsigned long)hz,
                 (unsigned long)caps.ctrl_after_enable,
                 caps.prfcnt_claimed ? "clear" : "set",
                 caps.cpicnt ? "yes" : "no",
                 caps.exccnt ? "yes" : "no",
                 caps.lsucnt ? "yes" : "no",
                 caps.foldcnt ? "yes" : "no");
    if (n > 0)
    {
      HAL_UART_Transmit(&huart1, (uint8_t *)line, (uint16_t)n, HAL_MAX_DELAY);
    }
    tx_thread_sleep(TX_TIMER_TICKS_PER_SECOND);
  }
}

/* USER CODE END 1 */
