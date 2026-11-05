# The solar-system kernel

**Goal:** make the numerical kernel understand more than the Earth. Time scales reach TDB, body
constants and rotation models follow IAU resolutions, planet and Moon positions come from a
verifiable source, and a scenario can name its central body — so "switch to another planet" is a
capability of the kernel rather than a display illusion.

**The standard is the one in `AGENTS.md`:** every numerical layer is finished only when it matches
a published reference. Sources in order of preference: the primary resolution or standard (IAU,
IERS, NAIF, USNO), then a reference implementation of it (IAU SOFA / ERFA, JPL Horizons, NAIF
kernels), then the paper. "It looks right" is not evidence.

## Provenance and licensing

- **Implemented here.** No dependency on `nyx-space/anise` or `nyx-space/hifitime` (both MPL-2.0)
  or `nyx` (AGPL-3.0), and no copying of their source. They are read as design references only.
- **Data** comes from NAIF's public text kernels (`pck00011.tpc`, `gm_de440.tpc`) and the
  IAU/WGCCRE report. Values enter the source as tables that carry `source + version` in a comment.
- **Verification** uses ERFA (BSD, derived from IAU SOFA) and JPL Horizons. Every test vector
  states the query that produced it and the date it was taken.

## Five phases

Each phase passes `cargo xtask ci` on its own and lands as its own commit.

### Phase 1 — TDB (`stkcraft-time`) — **done**

- `TDB − TT` uses the seven largest terms of the Fairhead & Bretagnon (1990) series in the
  truncated form given by Kaplan (2005), USNO Circular 179; documented as better than 10 µs over
  AD 1600–2200.
- `TT → TDB` is a direct evaluation; `TDB → TT` is a two-iteration fixed-point inversion, because
  the series argument is a TT quantity that is not known until the correction is.
- `TimeScale::Tdb` no longer aliases `Tt` inside `Epoch::to_scale` and `Epoch::as_tai`.
- **Acceptance, measured:** agreement with ERFA's full series (`eraDtdb`) at eight epochs from
  1985 to 2025 within 20 µs — worst case 5.2 µs; the 1.7 ms amplitude envelope is enforced and
  required to be approached; the inverse recovers TT to better than 1 ns; the `UTC → TAI → TT →
  TDB` chain reproduces the ERFA value at 2023-01-01.

Why this is not optional: `pck00011.tpc` states that the independent variable of the IAU 2015
rotation formulas is TDB and that the frame is the ICRF. Phase 2's rotation models and phase 3's
ephemerides both take TDB, so a kernel that treats TDB as TT contaminates all of them.

### Phase 2 — `stkcraft-body` (L1, between `time` and `frames`)

- `Body`: Sun, Mercury, Venus, Earth, Moon, Mars, Jupiter, Saturn, Uranus, Neptune, keyed by both
  NAIF ID and IAU name (the NAIF key is what phase 3's kernel backend will need).
- **GM** from DE440 (`gm_de440.tpc`, Park et al. 2021, AJ 161:105), km³/s².
- **Radii and ellipsoids** from IAU WGCCRE 2015 (Archinal et al. 2018, CMDA 130:22), via
  `pck00011.tpc`'s `BODYnnn_RADII`.
- **Rotation models** from IAU 2015: pole α₀, δ₀ and prime meridian W as polynomials in TDB
  centuries from J2000, with the periodic (`NUT_PREC_*`) terms the kernel carries for Mercury,
  the Moon, Jupiter and Neptune. `r_fixed = R₃(W)·R₁(90° − δ)·R₃(90° + α)·r_icrf`.
- **Planetocentric / planetographic** conversion per body's own ellipsoid; no hard-coded WGS-84.
- Known limitation to state in the code, not to hide: the text kernel carries only the linear
  terms of the Moon's model. The full librational model needs a binary PCK, which is phase 3
  work; until then the omission is documented with its magnitude (about 0.01°, a few hundred
  metres on the surface).
- **Acceptance:** the model reproduces the kernel's constants at J2000 for the bodies without
  periodic terms; rotation matrices are orthonormal with determinant +1 at several epochs;
  icrf → fixed → icrf returns the input to machine precision; planetodetic ↔ planetocentric
  round-trips inside 1 m.

### Phase 3 — `stkcraft-ephemeris` (L2)

- `trait Ephemeris { fn state(&self, body: Body, t: Epoch) -> Result<StateVector>; }` returning
  ICRF position and velocity, with a TDB time argument.
- **Backend A (built in, offline):** truncated VSOP87 (Bretagnon & Francou 1988) for the planets
  and truncated ELP2000-82B for the Moon, rotated to the ICRF with the IAU 2006 obliquity. Small
  generated tables, no data files.
- **Backend B (optional):** a NAIF DAF/SPK reader implemented here (Type 2 and 3) over
  `de440s.bsp` (32 MB) and the JPL satellite kernels. Data is not committed: it is fetched with a
  pinned SHA-256, and CI skips the backend offline without failing.
- **Acceptance:** both backends against JPL Horizons for several epochs and every body; A against
  B within A's stated accuracy. Horizons' sub-observer planetographic latitude and longitude are
  used to cross-check phase 2's body-fixed frames against an independent source.

### Phase 4 — central body in the engine (`stkcraft-engine`)

- `Scenario` gains `central_body`; satellites support planet-centred two-body plus J2/J4 in
  addition to Earth TLEs; sites sit on the body's ellipsoid in its body-fixed frame; occultation
  uses that body's radius; commands and the scenario document follow.
- **Acceptance:** hand-computed Kepler solutions, body-fixed round-trips, and the Earth path
  unchanged bit for bit (regression).

### Phase 5 — planetary systems

- Moons (Phobos, Deimos, the Galileans, Titan and the rest) as second-level bodies whose central
  body is their planet, with accuracy taken from the kernels. The analytic backend returns
  `Error::Unsupported` for them rather than guessing.

## Not in scope

- Interplanetary transfer design and planetary perturbations. Two-body plus J2/J4 is what this
  kernel claims; it does not pretend to integrate a chaotic regime.
- Full IERS Earth-orientation data beyond what the existing EOP path already carries; that is its
  own piece of work.

## Error handling

The rules from `AGENTS.md` apply unchanged: no panics, `Error::Unsupported` for what is not built
yet, input validated before use, and NaN expressed as `Option` rather than as a value.
