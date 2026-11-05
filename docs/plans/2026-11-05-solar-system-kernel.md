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

### Phase 2 — status: convention settled, crate not yet written

The rotation convention below is now pinned against an authoritative implementation rather than
assumed. A throwaway harness loaded `pck00011.tpc` into CSPICE (spiceypy 8.3, `CSPICE_N0067`) and
compared `pxform('IAU_<body>', 'J2000', et)` against our formula at three epochs (ET = 0, 3.2e8,
8.0e8 s). Agreed conventions and their evidence:

- `r_fixed = R3(W) · R1(90° − δ) · R3(90° + α) · r_icrf`, with the passive matrices
  `R3(θ) = [[c, s, 0], [−s, c, 0], [0, 0, 1]]` and `R1(θ) = [[1, 0, 0], [0, c, s], [0, −s, c]]`.
- The trigonometric terms are **sine for α, cosine for δ, sine for W** — the PCK's own Jupiter
  example spells this out and a first attempt with cosine for α disagreed with CSPICE by order 1.
- `T` is Julian centuries and `d` Julian days of **TDB** from J2000.
- With those, Mercury, the Moon, Jupiter and Earth match CSPICE to 1e−12 or better.

**Open item: Mars.** Its `NUT_PREC_*` arrays in `pck00011.tpc` have different lengths
(15, 20, 26) from its 39-entry angle list, and no positional offset of the coefficients
reproduces CSPICE's `IAU_MARS` matrix (best residual 1.5e−2 after trying offsets 0–11, against
≤1e−12 for the other bodies). Two readings are still possible — the file's quadratic phase-angle
extension, or a different model revision — and neither is being guessed at. Until the 2015
report's F-angle table is transcribed and checked the same way, Mars ships with its polynomial
terms only and a test that pins the known mismatch. This is the first thing to finish in phase 2.

### Phase 2 — `stkcraft-body` (L2, beside `frames`) — **done except Mars**

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
- **Acceptance, measured:** every rotation matrix agrees with CSPICE's `pxform` over the same
  kernel to 1e−8 or better (nine bodies × three epochs; a wrong trigonometric function, a
  transposed matrix or a wrong angle index miss by six orders of magnitude). Orthonormality and a
  determinant of +1 hold for all ten bodies at four epochs. Planetodetic ↔ planetocentric and
  surface-point round trips return to a nanoradian and a tenth of a millimetre. The Earth's pole
  is the ICRF pole at J2000, and a day advances its prime meridian by 0.9856°, which is the
  sidereal-to-solar day difference.

### Phase 3 — `stkcraft-ephemeris` (L3) — **in progress**

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

**Landed:** the crate, the `Ephemeris` trait, the Chebyshev evaluator (checked against the closed
forms and a finite difference), the DAF/SPK structure reader, and **type-2 evaluation, verified
against CSPICE**: 15 reference states — the Sun, Mercury, Venus, the Earth and the Moon at three
epochs each — agree to better than 1e−3 km in position and 1e−9 km/s in velocity over
`de440s.bsp`.

**The layout, and the mistake worth recording.** The first attempt assumed a segment begins with
six words of initial state, an interval length, a coefficient count and a record count, and no
parameter fit reproduced `spkezr`. NAIF's SPK Required Reading says the opposite: the records come
first and a four-number directory — `INIT`, `INTLEN`, `RSIZE`, `N` — sits at the *end* of the
segment. `RSIZE` is a whole record in array elements, not a coefficient count (`(RSIZE − 2) / 3` is
the count), the four directory entries are stored as doubles, and `N = 0` is not an empty segment
but a degenerate one: the target *is* its centre, which is how DE440 writes a barycentre sitting on
its planet. Reading the two counts as integers from the low half of their words is what made the
Sun appear to have no records at all.

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
