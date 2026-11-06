# Compute satellite passes over a station

Answer "when can this satellite see this station, and for how long" with the engine's own commands —
never by reasoning about orbits yourself.

## Procedure

1. `satellite.add` with `name`, `line1`, `line2` (a two-line element set), to put the satellite in
   the scenario.
2. `station.add` with `name`, `latitude-deg`, `longitude-deg`, `mask-deg`, to put the station on the
   Earth's ellipsoid with a horizon mask.
3. `access.compute` with `step-seconds`. A shorter step is more thorough and slower; the default is
   one minute and is right for a first answer.
4. Read `stkcraft://scenario` if you need the document itself.

## What the answer means

Pass windows are quoted in seconds from the scenario epoch, and the summary line gives the count
and the total time in view. A pass that is absent is a real answer: the satellite never rose above
the mask in the window. Do not invent a pass to fill the gap.

## Traps

- The scenario epoch is the element set's own epoch for anything built from a TLE, because
  propagating years-old elements against "now" produces numbers that look right and are not.
- A mask of zero is the geometric horizon. Real stations use five to ten degrees.
- The step must be shorter than the shortest pass you care about; nothing warns you if it is not.
