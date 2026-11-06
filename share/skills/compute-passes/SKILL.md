# Compute satellite passes over a station

Answer "when can this satellite see this station, and for how long" with the engine's own commands —
never by reasoning about orbits yourself.

## Procedure

1. `satellite.add` with `name`, `line1`, `line2` (a two-line element set), to put the satellite in
   the scenario.
2. `station.add` with `name`, `latitude`, `longitude`, `minimum-elevation`, to put the station on the
   Earth's ellipsoid with a horizon mask. All three angles are degrees, and the parameter names are
   exactly those four — there is no `latitude-deg` on this command.
3. `scenario.set-duration` if the timeline should not be the default day: `seconds`.
4. `access.compute` with `step-seconds`. A shorter step is more thorough and slower; the default is
   one minute and is right for a first answer.
5. Read `stkcraft://scenario` if you need the document itself.

## What the answer means

`access.compute` returns one line per satellite/station pair — the pass count and the total time in
view — and then one line per pass:

```text
ISS over London: 4 pass(es), 22m 31s in view
  2024-01-01 09:12:04 → 2024-01-01 09:18:16 UTC (6m 12s)
```

Quote those instants as they are printed. Every window is also in the call's structured content, with
`start_utc`, `end_utc`, `start_seconds` (seconds from the scenario epoch) and `duration_seconds` —
which is where to look when the text summarises a long list.

A pass that is absent is a real answer: the satellite never rose above the mask in the window. Do not
invent a pass to fill the gap.

## Traps

- The scenario epoch is the element set's own epoch for anything built from a TLE, because
  propagating years-old elements against "now" produces numbers that look right and are not.
- A minimum elevation of zero is the geometric horizon. Real stations use five to ten degrees.
- The step must be shorter than the shortest pass you care about; nothing warns you if it is not.
- `access.compute` needs at least one satellite and one station. Calling it before both exist is an
  error, not an empty list.
