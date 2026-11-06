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

### Phase 2 — `stkcraft-astro-body` (L2, beside `frames`) — **done except Mars**

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

### Phase 3 — `stkcraft-astro-ephemeris` (L3) — **in progress**

- `trait Ephemeris { fn state(&self, body: Body, t: Epoch) -> Result<StateVector>; }` returning
  ICRF position and velocity, with a TDB time argument.
- **Backend A (built in, offline) — landed as JPL's Keplerian elements, not VSOP87.** The offline
  backend uses *Keplerian Elements for Approximate Positions of the Major Planets* (JPL SSD,
  Table 1, 1800–2050) rather than the VSOP87/ELP2000 series: it is the same publisher, a far smaller
  transcription, and its accuracy is stated by JPL and checked against CSPICE in the tests (15″ to
  600″). A VSOP87/ELP2000 backend would be more accurate and much larger; it is the next accuracy
  step, not a correction.
- **VSOP87 attempt, and where it stopped.** The `VSOP87A.*` files were fetched from IMCCE (they are
  public and the series is the published standard) and truncated at 1e-7 au, which would give about
  0.02″. The record layout is the blocker: a data line is `[id, iv, it, 12 α-powers, then five
  numbers]`, and no assignment of the last five to `(A, B, C, S, K)` that was tried reproduces
  CSPICE for Mercury at J2000 — the best was 657 000 km, three orders off the truncation error. The
  next step is the published record description, not more guessing at columns.
- **ELP2000 attempted, not landed.** The three main-problem files (`ELP1` longitude, `ELP2` latitude,
  `ELP3` distance) parse and evaluate: at J2000 the prototype agrees with CSPICE to about 20″ of
  longitude, 3″ of latitude and 2 km of distance — the accuracy the main problem alone should have.
  Five years away it is out by 1.3°, which is a coefficient problem, not truncation: the leading
  latitude term is stored with a second-column value of 412 529 where a linear-in-`T` term cannot
  be that large, so ELP's polynomial columns carry a scaling this reading does not apply. Nothing
  was shipped: a Moon off by a degree is worse than a Moon refused by name, which is what the
  VSOP87 backend already does. The notice (`elp82b.ps`, extracted with `ps2ascii`) confirms what the
  files contain — main-problem longitude, latitude and distance series in arcseconds and kilometres,
  with Delaunay arguments as polynomials in TDB centuries — but it does not state the term-record
  column layout, and the latitude file's second column is where the reading breaks. The next step is
  the record layout in the original 1983/1988 papers (A&A 124, 50 and A&A 190, 342), not the
  distribution notice. CDS (`VI/79`), which the README names as the catalogue copy that documents the
  format, is unreachable from this machine, and the distribution carries no Fortran source to read
  the layout out of. That is the concrete blocker: three rounds (10, 11, 12) have stopped at the same
  place for the same reason, and the Moon is placed by the SPK backend in the meantime.
- **VSOP87 landed: 0.1.14.** `Vsop87Ephemeris` reads the published Poisson series (9 537 terms
  truncated at 1e-7 au, in `generated_vsop.rs` with its provenance) and agrees with CSPICE to under
  an arcsecond over fifteen heliocentric states — Mercury at 0.04″. The Moon needs ELP2000 and is
  refused by name; that is the remaining half of this item.
- **VSOP87 parse now provably correct; the residual is systematic, not a misread.** Reading the
  last five whitespace fields of a record as `S, K, A, B, C` and taking the coordinate and the power
  of `T` from the header block parses all 6359 Mercury records, and the identity
  `sqrt(S² + K²) = A` holds on every one of them — so the columns are right. The position still
  differs from CSPICE by 657 000 km at J2000, and because a misread column would not survive that
  identity, the difference is systematic: something about the frame or the time argument, not the
  arithmetic. The next step is to evaluate at several epochs and watch whether the residual rotates
  (a phase or frequency error) or stays fixed (a frame offset), which tells the two apart in one
  test.
- **VSOP87 record format, now read from the distribution's own `vsop87.doc`.** A term record is
  `1x,4i1,i5,12i3,f15.11,2f18.11,f14.11,f20.11`: version code, body code, coordinate index and the
  power of `T` packed into the first four characters (which is why a line begins `1110` for
  `iv=1, ib=1, ic=1, it=0`), then the rank, then the twelve mean-longitude coefficients, then
  **S, K, A, B, C**. The term is `A·cos(B + C·T)` with `T` in thousands of Julian years from J2000,
  and the documented time argument is TT. So the columns used in the failed attempt (16, 17, 18 as
  A, B, C) were already the right ones, which means the 657 000 km residual is *not* a column
  problem: the next suspects are the coordinate index (the first character of the packed field must
  be the *version* code 1 for VSOP87A, not the coordinate) and the ecliptic-to-ICRF step used to
  build the reference. That is where the next attempt starts, with the doc's own
  `sqrt(S²+K²) = A` identity as the check on the parse.
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

### Phase 4 — central body in the engine (`stkcraft-astro-engine`)

- `Scenario` gains `central_body`; satellites support planet-centred two-body plus J2/J4 in
  addition to Earth TLEs; sites sit on the body's ellipsoid in its body-fixed frame; occultation
  uses that body's radius; commands and the scenario document follow.
- **Landed:** `Scenario::central_body` (default Earth, serialised by name) and `PlanetOrbit`, the
  exact two-body solution about any body's own GM, checked against `v = sqrt(mu/r)`, the conserved
  energy and angular momentum, and the antipode after half a period. SGP4 remains the Earth model;
  the two are separate types rather than a flag.
- **Still to do:** sites on another body's ellipsoid, and an access problem in that body's
  body-fixed frame (the body crate's `Orientation` supplies the rotation).

### Phase 5 — planetary systems — **core landed**

### Phase 4 — central body in the engine (`stkcraft-astro-engine`) — **core landed**

- **Landed:** Phobos, Deimos, Io, Europa, Ganymede, Callisto and Titan are bodies with IAU 2015
  constants and rotation models, and their ephemerides are in the CSPICE table (eight planets and
  eight moons, 54 reference states, three epochs each, better than a metre). Triton needed its own
  2.1 GB kernel (`nep097xl-801.bsp`, now in the fetch table with its verified checksum); without it
  the reader refused the body by name rather than substituting Neptune.
- **Not done:** an explicit "central body of each moon" link in the engine (the chain is resolved by
  the ephemeris kernels, not by the document).

## Not in scope

- Interplanetary transfer design and planetary perturbations. Two-body plus J2/J4 is what this
  kernel claims; it does not pretend to integrate a chaotic regime.
- Full IERS Earth-orientation data beyond what the existing EOP path already carries; that is its
  own piece of work.

## Error handling

The rules from `AGENTS.md` apply unchanged: no panics, `Error::Unsupported` for what is not built
yet, input validated before use, and NaN expressed as `Option` rather than as a value.
