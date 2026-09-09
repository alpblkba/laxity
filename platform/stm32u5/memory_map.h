/* the STM32U585 internal SRAM regions, as the placement experiment sees them.
 *
 * this is a local header rather than a port interface. retrofitting the table behind a qos_port_memory_t later does not need the file to move, which is the same call counters_dwt.h makes.
 *
 * nothing here includes a vendor header. the addresses are constants with their source named beside them, so this file stays free of the CMSIS include chain in the same way platform/cortex-m33 does for the DWT registers.
 */
#ifndef LAXITY_MEMORY_MAP_H
#define LAXITY_MEMORY_MAP_H

#include <stdint.h>

/* placement label ids, carried in qos_infer_record_t::region_id.
 *
 * 0 stays reserved for a record whose placement was not chosen. QOS_REGION_SRAM1_CONTROL is not a region: it is SRAM1 measured a second time under a second identity, and the spread between the two is the noise floor that says whether any between region difference is resolvable at all. */
#define QOS_REGION_NONE            0u
#define QOS_REGION_SRAM1           1u
#define QOS_REGION_SRAM2           2u
#define QOS_REGION_SRAM3           3u
#define QOS_REGION_SRAM4           4u
#define QOS_REGION_SRAM1_CONTROL   5u
/* SRAM2 at a second address. region and address are confounded when each region gets one arena, so this label is what separates "SRAM2 is slower" from "that address is slower". */
#define QOS_REGION_SRAM2_ALT       6u

/* the same constants the table below carries, in macro form, because the span reservation in the firmware needs its size at compile time and duplicating the addresses there is how the two would drift apart. */
#define QOS_SRAM1_BASE  0x20000000u
#define QOS_SRAM1_SIZE  0x00030000u
#define QOS_SRAM2_BASE  0x20030000u
#define QOS_SRAM2_SIZE  0x00010000u
#define QOS_SRAM3_BASE  0x20040000u
#define QOS_SRAM3_SIZE  0x00080000u
#define QOS_SRAM4_BASE  0x28000000u
#define QOS_SRAM4_SIZE  0x00004000u

typedef struct {
    uint8_t     id;
    const char *name;
    uint32_t    base;
    uint32_t    size;
    const char *domain;    /* which power and clock domain the region sits in */
    const char *port;      /* bus matrix slave port, empty until RM0456 is read */
    uint16_t    rel_cost;  /* 1000 everywhere until measurement replaces it */
    uint8_t     activation_target;  /* 0 for a region this project does not place arenas in */
} qos_mem_region_t;

extern const qos_mem_region_t qos_mem_regions[];
extern const uint32_t qos_mem_region_count;

const qos_mem_region_t *qos_mem_region(uint8_t id);

#endif /* LAXITY_MEMORY_MAP_H */
