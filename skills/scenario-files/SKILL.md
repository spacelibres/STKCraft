# The scenario document

A scenario is data, not behaviour: satellites carry their element sets, stations their geodetic
position and sensor, and the document says which body the analysis is about.

## Shape

```json
{
  "name": "Demo",
  "central_body": "Earth",
  "epoch": "…",
  "duration_seconds": 86400.0,
  "satellites": [{"id": 1, "name": "ISS", "element_set": {…}, "color": [240, 90, 60]}],
  "stations": [{"id": 1, "name": "London", "site": {…}, "sensor": {…}}]
}
```

## Rules

- `central_body` is the IAU name of a body; it defaults to the Earth, so a document written before
  the field existed still loads and still means what it meant.
- Ids are assigned by the scenario and are stable for the life of the document; do not compute them.
- Read the current document from `stkcraft://scenario` rather than reconstructing it.
