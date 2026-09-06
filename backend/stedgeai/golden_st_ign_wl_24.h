/* Golden input window and reference output for st_ign_wl_24, compiled into the firmware so the target result can be checked without a sensor.
 *
 * Provenance, because a golden value whose origin is not written down is not a check. The input is ours and the reference is ST's, which is the only arrangement that tests our integration rather than ST's conversion.
 *
 * The input is a fixed synthetic window rather than a recorded trace, so it is reproducible from its formula alone. For t in 0 to 23, laid out t major then axis, matching the C order of the model's (1, 24, 3, 1) input:
 *     x = 2.0 * sin(2*pi*t/12)
 *     y = 1.0 * cos(2*pi*t/12)
 *     z = 0.5 * sin(4*pi*t/12)
 *
 * The reference is the output of the original Keras model, not of the generated C model, taken from network_val_m_outputs_1.csv produced by:
 *     stedgeai validate --model st_ign_wl_24.keras --target stm32u5 --c-api st-ai --name network --mode host --valinput input.csv --save-csv --classifier
 * with ST Edge AI Core v4.0.1-20581. The generated C model agreed with it to seven significant figures on the host, snr 188.1 dB, cos 1.000000.
 *
 * The expected class is index 1, Stationary, at 0.99986. The margin over the next class is four orders of magnitude, so the argmax cannot be flipped by float rounding on the target and the comparison is a real check rather than a coin toss.
 *
 * Class order comes from the model zoo training config: Jogging, Stationary, Stairs, Walking.
 */
#ifndef LAXITY_GOLDEN_ST_IGN_WL_24_H
#define LAXITY_GOLDEN_ST_IGN_WL_24_H

#define LAXITY_GOLDEN_IN_LEN   72
#define LAXITY_GOLDEN_OUT_LEN  4
#define LAXITY_GOLDEN_CLASS    1

static const char *const laxity_golden_class_names[LAXITY_GOLDEN_OUT_LEN] = {
  "Jogging", "Stationary", "Stairs", "Walking"
};

static const float laxity_golden_input[LAXITY_GOLDEN_IN_LEN] = {
  0.000000000e+00f, 1.000000000e+00f, 0.000000000e+00f,
  1.000000000e+00f, 8.660249710e-01f, 4.330129921e-01f,
  1.732051015e+00f, 5.000000000e-01f, 4.330129921e-01f,
  2.000000000e+00f, 0.000000000e+00f, 0.000000000e+00f,
  1.732051015e+00f, -5.000000000e-01f, -4.330129921e-01f,
  1.000000000e+00f, -8.660249710e-01f, -4.330129921e-01f,
  0.000000000e+00f, -1.000000000e+00f, -0.000000000e+00f,
  -1.000000000e+00f, -8.660249710e-01f, 4.330129921e-01f,
  -1.732051015e+00f, -5.000000000e-01f, 4.330129921e-01f,
  -2.000000000e+00f, -0.000000000e+00f, 0.000000000e+00f,
  -1.732051015e+00f, 5.000000000e-01f, -4.330129921e-01f,
  -1.000000000e+00f, 8.660249710e-01f, -4.330129921e-01f,
  -0.000000000e+00f, 1.000000000e+00f, -0.000000000e+00f,
  1.000000000e+00f, 8.660249710e-01f, 4.330129921e-01f,
  1.732051015e+00f, 5.000000000e-01f, 4.330129921e-01f,
  2.000000000e+00f, 0.000000000e+00f, 0.000000000e+00f,
  1.732051015e+00f, -5.000000000e-01f, -4.330129921e-01f,
  1.000000000e+00f, -8.660249710e-01f, -4.330129921e-01f,
  0.000000000e+00f, -1.000000000e+00f, -0.000000000e+00f,
  -1.000000000e+00f, -8.660249710e-01f, 4.330129921e-01f,
  -1.732051015e+00f, -5.000000000e-01f, 4.330129921e-01f,
  -2.000000000e+00f, -0.000000000e+00f, 0.000000000e+00f,
  -1.732051015e+00f, 5.000000000e-01f, -4.330129921e-01f,
  -1.000000000e+00f, 8.660249710e-01f, -4.330129921e-01f,
};

static const float laxity_golden_output[LAXITY_GOLDEN_OUT_LEN] = {
  6.531077190e-08f, 9.998555183e-01f, 1.399237517e-04f, 4.386870387e-06f,
};

#endif /* LAXITY_GOLDEN_ST_IGN_WL_24_H */
