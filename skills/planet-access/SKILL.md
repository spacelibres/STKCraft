# Access on a body other than the Earth

`planet.access` answers the same question as `access.compute` for a station standing on Mars, Titan
or any other body the kernel models, with that body's own ellipsoid and rotation.

## Procedure

1. Call `planet.access` with `body` (the IAU name: `Mars`, `Titan`, `Io`, …), `radius-km` for a
   circular orbit about that body's centre, `inclination-deg`, `latitude-deg`, `longitude-deg`, and
   optionally `mask-deg`, `hours`, `step-seconds`.
2. The answer counts passes over the window and the total time in view.

## What is and is not modelled

The orbit is two-body about the body's gravitational parameter: exact and checkable, with no J2, no
third body and no drag. For a low orbit that is a kilometre-a-minute along the track, so quote the
geometry, not a precise contact time. The station sits on the body's own ellipsoid and its visibility
uses the IAU 2015 rotation model, so the body-fixed geometry is the published one.

## Traps

- `radius-km` is from the body's centre, not an altitude. Mars' equatorial radius is 3 396 km.
- Cases are refused rather than guessed: a radius inside the body, an unknown body name, a step of
  zero. Read the error and correct the parameter.
