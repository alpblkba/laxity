/* the board's inertial module as a live input for the classifier, and one environmental reading.
 *
 * this sits beside the compiled in golden window rather than replacing it. the golden window is
 * the only end to end integrity check this firmware has, so the run time choice between the two
 * is a choice, and the caller is expected to say out loud which one produced a classification.
 */
#ifndef LAXITY_SENSORS_H
#define LAXITY_SENSORS_H

#include <stdbool.h>
#include <stdint.h>

/* the model's input window: 24 timesteps of three axes, timestep major. */
#define LAXITY_SENSOR_STEPS  24u
#define LAXITY_SENSOR_AXES   3u
#define LAXITY_SENSOR_LEN    (LAXITY_SENSOR_STEPS * LAXITY_SENSOR_AXES)

bool qos_sensors_init(void);

/* take one accelerometer sample if the output data rate says another is due. returns true when a
 * sample was actually stored, so a caller can tell a working sensor from a quiet one. */
bool qos_sensors_sample(void);

/* the newest window, oldest sample first, in g. safe to call before the window has filled, in
 * which case it is padded with zeros and qos_sensors_ready() is false. */
void qos_sensors_window(float *dst);
bool qos_sensors_ready(void);

/* how many samples have ever been stored, so a stalled sensor is visible on a status line. */
uint32_t qos_sensors_count(void);

/* the largest absolute acceleration in the current window, in g.
 *
 * this is on the status line on purpose. the golden window reaches 2 g, and if the conversion
 * from the sensor's units were wrong this number would be out by the same factor, which makes a
 * scaling mistake visible without guessing at the classifier's output. */
float qos_sensors_peak_g(void);

/* degrees Celsius, refreshed about once a second. */
float qos_sensors_temperature(void);
bool qos_sensors_env_ok(void);

/* which environmental instance answered, since which part is populated varies by board. */
uint32_t qos_sensors_env_instance(void);

#endif /* LAXITY_SENSORS_H */
