/* see sensors.h. */

#include "sensors.h"

#include "b_u585i_iot02a_motion_sensors.h"
#include "b_u585i_iot02a_env_sensors.h"
#include "stm32u5xx_hal.h"

#include <math.h>
#include <string.h>

/* the inertial module is motion sensor instance 0 on this board. the environmental instance is
 * not assumed: two different parts on this bus both report temperature and which of them is
 * populated varies by board revision, so both are tried and the one that answers is recorded. */
#define LAXITY_ACC_INSTANCE  0u
#define LAXITY_ENV_COUNT     2u

/* the training set was recorded at 20 Hz and this part does not offer it. 26 Hz is the nearest
 * rate it has, so the window covers 0.92 s where the model was trained on 1.2 s. that mismatch is
 * a stated limit rather than something corrected here, because resampling would put an untested
 * filter between the sensor and the only classifier this project has. */
#define LAXITY_ACC_ODR_HZ    26.0f
#define LAXITY_ACC_PERIOD_MS 38u

/* the golden window reaches 2 g, and a hand shaking the board passes that easily. 4 g leaves room
 * for normal handling without clipping, at the cost of half the resolution. */
#define LAXITY_ACC_FS_G      4

#define LAXITY_ENV_PERIOD_MS 1000u

static float    s_win[LAXITY_SENSOR_STEPS][LAXITY_SENSOR_AXES];
static uint32_t s_head;
static uint32_t s_count;
static uint32_t s_last_ms;
static bool     s_acc_ok;

static float    s_temp_c;
static uint32_t s_env_last_ms;
static bool     s_env_ok;
static uint32_t s_env_instance;

bool qos_sensors_init(void)
{
    s_acc_ok = false;
    s_env_ok = false;

    if (BSP_MOTION_SENSOR_Init(LAXITY_ACC_INSTANCE, MOTION_ACCELERO) == BSP_ERROR_NONE) {
        /* order matters less than checking each one. a full scale that did not take would show up
         * as every reading being out by a constant factor, which is the failure this project is
         * most likely to mistake for a bad classifier. */
        (void)BSP_MOTION_SENSOR_SetFullScale(LAXITY_ACC_INSTANCE, MOTION_ACCELERO, LAXITY_ACC_FS_G);
        (void)BSP_MOTION_SENSOR_SetOutputDataRate(LAXITY_ACC_INSTANCE, MOTION_ACCELERO, LAXITY_ACC_ODR_HZ);
        s_acc_ok = (BSP_MOTION_SENSOR_Enable(LAXITY_ACC_INSTANCE, MOTION_ACCELERO) == BSP_ERROR_NONE);
    }

    for (uint32_t i = 0u; i < LAXITY_ENV_COUNT && !s_env_ok; ++i) {
        if (BSP_ENV_SENSOR_Init(i, ENV_TEMPERATURE) != BSP_ERROR_NONE) { continue; }
        if (BSP_ENV_SENSOR_Enable(i, ENV_TEMPERATURE) != BSP_ERROR_NONE) { continue; }
        s_env_instance = i;
        s_env_ok = true;
    }

    return s_acc_ok;
}

bool qos_sensors_sample(void)
{
    BSP_MOTION_SENSOR_Axes_t axes;
    uint32_t now = HAL_GetTick();

    if (!s_acc_ok) { return false; }
    if ((now - s_last_ms) < LAXITY_ACC_PERIOD_MS) { return false; }
    s_last_ms = now;

    if (BSP_MOTION_SENSOR_GetAxes(LAXITY_ACC_INSTANCE, MOTION_ACCELERO, &axes) != BSP_ERROR_NONE) {
        return false;
    }

    /* the component driver returns milli g, since its sensitivity constants are mg per count.
     * the golden window is in g, so this is the one conversion the live path performs. */
    s_win[s_head][0] = (float)axes.xval / 1000.0f;
    s_win[s_head][1] = (float)axes.yval / 1000.0f;
    s_win[s_head][2] = (float)axes.zval / 1000.0f;

    s_head = (s_head + 1u) % LAXITY_SENSOR_STEPS;
    if (s_count < 0xFFFFFFFFu) { s_count++; }
    return true;
}

void qos_sensors_window(float *dst)
{
    if (dst == NULL) { return; }

    if (s_count < LAXITY_SENSOR_STEPS) {
        memset(dst, 0, LAXITY_SENSOR_LEN * sizeof(float));
        return;
    }

    /* oldest first, which is the order the model's timestep axis expects. s_head is the slot the
     * next sample goes into, so it is also the oldest one currently held. */
    for (uint32_t i = 0u; i < LAXITY_SENSOR_STEPS; ++i) {
        uint32_t src = (s_head + i) % LAXITY_SENSOR_STEPS;
        dst[i * LAXITY_SENSOR_AXES + 0u] = s_win[src][0];
        dst[i * LAXITY_SENSOR_AXES + 1u] = s_win[src][1];
        dst[i * LAXITY_SENSOR_AXES + 2u] = s_win[src][2];
    }
}

bool qos_sensors_ready(void)
{
    return s_acc_ok && (s_count >= LAXITY_SENSOR_STEPS);
}

uint32_t qos_sensors_count(void)
{
    return s_count;
}

float qos_sensors_peak_g(void)
{
    float peak = 0.0f;
    for (uint32_t i = 0u; i < LAXITY_SENSOR_STEPS; ++i) {
        for (uint32_t a = 0u; a < LAXITY_SENSOR_AXES; ++a) {
            float m = fabsf(s_win[i][a]);
            if (m > peak) { peak = m; }
        }
    }
    return peak;
}

float qos_sensors_temperature(void)
{
    uint32_t now = HAL_GetTick();
    float v = 0.0f;

    if (s_env_ok && (now - s_env_last_ms) >= LAXITY_ENV_PERIOD_MS) {
        s_env_last_ms = now;
        if (BSP_ENV_SENSOR_GetValue(s_env_instance, ENV_TEMPERATURE, &v) == BSP_ERROR_NONE) {
            s_temp_c = v;
        }
    }
    return s_temp_c;
}

bool qos_sensors_env_ok(void)
{
    return s_env_ok;
}

uint32_t qos_sensors_env_instance(void)
{
    return s_env_instance;
}
