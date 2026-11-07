# Limitations the answer must carry

- **Sampling can miss a short window.** A window that opens and closes entirely between two samples
  is invisible to the sampler; the answer states the step it used.
- **The line of sight is geometric.** No atmospheric refraction, light time, aberration or Doppler.
- **Earth orientation is zero.** `UT1 - UTC` and polar motion are not modelled until an IERS loader
  exists, so ground-fixed positions can be off by hundreds of metres.
- **Target access is the union of vertex windows.** It is not coverage of the polygon interior, and
  a point target is the one-vertex case.
- **Occultation uses a sphere of the ellipsoid's volume**, not the ellipsoid itself; the difference
  is about 7 km at the limb, roughly 0.06 degrees of grazing elevation. The elevation comparison
  itself uses the WGS84 ellipsoid.
- **No terrain or local horizon.** A station is a point on the ellipsoid with a scalar mask.
- **The orbit is whatever the propagator supplied.** For SGP4 this inherits the SGP4 and Time &
  Frames limitations.
