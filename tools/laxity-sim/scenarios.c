#include "scenarios.h"

#include <string.h>

/* these values are the five placement labels carried by the measured contention capture. */
static const qos_placement_t stm32u585[] = {
    { 1u, 0u,                      1000u, 0x20002000u, 2944u, "SRAM1"  },
    { 2u, 0u,                      1000u, 0x20030000u, 2944u, "SRAM2"  },
    { 3u, 0u,                      1000u, 0x20040000u, 2944u, "SRAM3"  },
    { 6u, QOS_PLACEMENT_ALT_ADDR, 1000u, 0x20039000u, 2944u, "SRAM2b" },
    { 5u, QOS_PLACEMENT_CONTROL,  1000u, 0x20002000u, 2944u, "SRAM1c" },
};

/* this synthetic table changes one placement address and label. it represents a metadata change, not a newly measured topology. */
static const qos_placement_t stm32u585_changed[] = {
    { 1u, 0u,                      1000u, 0x20002000u, 2944u, "SRAM1"  },
    { 2u, 0u,                      1000u, 0x20030000u, 2944u, "SRAM2"  },
    { 3u, 0u,                      1000u, 0x20040000u, 2944u, "SRAM3"  },
    { 6u, QOS_PLACEMENT_ALT_ADDR, 1000u, 0x2003A000u, 2944u, "SRAM2c" },
    { 5u, QOS_PLACEMENT_CONTROL,  1000u, 0x20002000u, 2944u, "SRAM1c" },
};

const qos_placement_t *laxity_sim_placements(const char *name, uint8_t *count)
{
    if (name == NULL || count == NULL) {
        return NULL;
    }
    if (strcmp(name, "stm32u585") == 0) {
        *count = (uint8_t)(sizeof stm32u585 / sizeof stm32u585[0]);
        return stm32u585;
    }
    if (strcmp(name, "stm32u585-changed") == 0) {
        *count = (uint8_t)(sizeof stm32u585_changed / sizeof stm32u585_changed[0]);
        return stm32u585_changed;
    }
    return NULL;
}
