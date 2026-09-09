/* see memory_map.h. */

#include "memory_map.h"

/* base addresses and sizes are ST's own, from firmware/stm32u585/Drivers/CMSIS/Device/ST/STM32U5xx/Include/stm32u585xx.h lines 1708 to 1711 for the sizes and 1727 to 1730 for the non-secure base addresses. TrustZone is disabled in the .ioc, so the non-secure aliases are the addresses that run.
 *
 * the domain column is from Drivers/STM32U5xx_HAL_Driver/Inc/stm32u5xx_ll_pwr.h line 416, which names the SmartRun domain as the AHB3 and APB3 clock domain. SRAM4 is the SRAM in that domain, which is why it is a topology control rather than a fourth placement target.
 *
 * the port column is empty and that is a gap rather than a value. RM0456 is not on this host, two attempts to fetch it timed out and a third returned 403, and naming a bus matrix slave port without reading the table it comes from would be inventing a citation. base, size and domain are all sourced above; only the per region matrix port is missing, and nothing in this block's measurement depends on it.
 *
 * rel_cost is 1000 everywhere on purpose. the planner learns these from measurement under bench/, and seeding them with a guess would make it agree with the guess rather than with the board. */
const qos_mem_region_t qos_mem_regions[] = {
    { QOS_REGION_SRAM1, "SRAM1", QOS_SRAM1_BASE, QOS_SRAM1_SIZE, "main", "", 1000u, 1u },
    { QOS_REGION_SRAM2, "SRAM2", QOS_SRAM2_BASE, QOS_SRAM2_SIZE, "main", "", 1000u, 1u },
    { QOS_REGION_SRAM3, "SRAM3", QOS_SRAM3_BASE, QOS_SRAM3_SIZE, "main", "", 1000u, 1u },
    /* outside the linker's RAM region and in the SmartRun domain, so the span reservation the placement experiment uses cannot reach it and it is measured as a separate case. */
    { QOS_REGION_SRAM4, "SRAM4", QOS_SRAM4_BASE, QOS_SRAM4_SIZE, "SmartRun", "", 1000u, 0u },
};

const uint32_t qos_mem_region_count = (uint32_t)(sizeof qos_mem_regions / sizeof qos_mem_regions[0]);

const qos_mem_region_t *qos_mem_region(uint8_t id)
{
    /* the control label resolves to SRAM1, since it is the same memory measured under a second identity rather than a region of its own. */
    if (id == QOS_REGION_SRAM1_CONTROL) {
        id = QOS_REGION_SRAM1;
    }
    if (id == QOS_REGION_SRAM2_ALT) {
        id = QOS_REGION_SRAM2;
    }
    for (uint32_t i = 0u; i < qos_mem_region_count; ++i) {
        if (qos_mem_regions[i].id == id) {
            return &qos_mem_regions[i];
        }
    }
    return 0;
}
