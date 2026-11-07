# Limitations the answer must carry

- **IAU-77/80, not IAU 2006/2000A.** The precession and nutation series are the ones SGP4/TEME is
  defined against. A request for the modern IAU 2006/2000A chain is out of scope.
- **Earth-orientation parameters default to zero.** `Propagator::new` uses
  `EarthOrientation::negligible()`: `UT1 - UTC = 0`, polar motion `x = y = 0`. Until an IERS
  `finals2000A` loader exists, a ground-fixed result carries that approximation (up to hundreds of
  metres at LEO, kilometres at GEO).
- **The leap-second table ends at its published revision.** A date after it is flagged, not given a
  guessed offset.
- **UT1 cannot be derived from UTC or TAI alone.** `DUT1` is an IERS-published input; without it the
  conversion is refused.
- **TDB is the truncated ERFA series**, agreeing with the full series to about 20 microseconds.
- **No sub-daily EOP, no tidal variation, no TCG/TCB chain** is implemented in this package.
- **The expert does not replace the access limitations.** A computed pass also carries the sampling,
  vertex-union and geometric-line-of-sight limits of the access layer.
