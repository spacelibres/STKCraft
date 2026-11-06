# Access on a body other than the Earth

`planet.access` answers the same question as `access.compute` for a station standing on Mars, Titan
or any other body the kernel models, with that body's own ellipsoid and rotation.

## Procedure

1. Call `planet.access` with `body` (the IAU name: `Mars`, `Titan`, `Io`, …), `radius-km` for a
   circular orbit about that body's centre, `inclination-deg`, `latitude-deg`, `longitude-deg`, and
   optionally `mask-deg`, `hours`, `step-seconds`.
2. The answer counts the passes over the window and the total time in view, then lists each pass with
   its UTC start and end and its duration. The same list is in the call's structured content.

## What is and is not modelled

The orbit is two-body about the body's gravitational parameter: exact and checkable, with no J2, no
third body and no drag. For a low orbit that is a kilometre-a-minute along the track, so quote the
geometry, not a precise contact time. The station sits on the body's own ellipsoid and its visibility
uses the IAU 2015 rotation model, so the body-fixed geometry is the published one.

## Traps

- `radius-km` is from the body's centre, not an altitude. Mars' equatorial radius is 3 396 km and
  Titan's is 2 575 km; a radius below the equatorial radius is refused rather than clipped.
- Cases are refused rather than guessed: a radius inside the body, an unknown body name, a step of
  zero, an epoch on a scale the rotation model does not take. Read the error and correct the
  parameter.
- The default masks the body's own limb only: `mask-deg` of zero is the geometric horizon, so say so
  when quoting a contact time instead of implying a station's real horizon.
