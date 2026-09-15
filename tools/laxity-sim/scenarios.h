#ifndef LAXITY_SIM_SCENARIOS_H
#define LAXITY_SIM_SCENARIOS_H

#include "qos/telemetry.h"

#include <stdint.h>

/* return one static placement table used by the checked-in scenarios. */
const qos_placement_t *laxity_sim_placements(const char *name, uint8_t *count);

#endif /* LAXITY_SIM_SCENARIOS_H */
