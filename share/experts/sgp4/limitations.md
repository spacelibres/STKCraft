# Limitations the answer must carry

- **SGP4 is a fitted model, not a precision orbit determination.** The AIAA vectors prove the
  implementation reproduces the published SGP4 output; they do not prove metre-level truth against
  a real satellite.
- **TLE age matters.** Accuracy degrades as the propagation time moves away from the element set
  epoch. The answer states the epoch and the propagation interval.
- **No maneuvers or covariance.** A TLE has no maneuver information and SGP4 has no covariance.
- **Deep-space resonance is the model's approximation.** The lunar-solar periodics and resonance
  integration are the published series, not a numerical integration of the full force model.
- **A decayed or sub-orbital element set is refused.** The error code is the answer; a position is
  not invented.
- **The output is TEME.** Converting to GCRF/ITRF uses the Time & Frames limitations (IAU-77/80 and
  zero Earth orientation until an IERS loader exists).
- **The verification file has three deliberately broken element sets** (`33333`, `33334`, `33335`);
  they are refused with a stated code, as the reference also reports.
