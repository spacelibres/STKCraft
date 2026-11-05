# Contributing and committing

This document states how work lands in STKCraft. It follows the Craft suite: one task per
commit, a quality gate before each one, and a message that explains *why* the change exists
rather than restating the diff.

## Before every commit

```sh
cargo xtask ci
```

That runs, in order: `fmt --check`, clippy with `-D warnings`, the workspace tests, the asset
attribution check, the layering check, and the WebAssembly check. A commit that fails any of
them does not land.

If you cannot run a step — no network for the wasm target, for instance — say so in the commit
message. Do not silently skip it.

## One change per commit

A commit is one coherent change, named by a task id in the same form the sibling apps use:

```
M1.3: leap-second table with confidence reporting
```

- `M<major>.<n>` is the milestone and task number from the roadmap.
- The summary is imperative, lowercase after the colon, and describes the behaviour change.
- Prefer several small commits over one large one. A reviewer should be able to read each in
  isolation.

Do not mix an unrelated fix into a feature commit. If you find a bug while working on
something else, commit it separately.

## Commit body

The subject says what changed; the body says why, and what it costs.

State, in order:

1. **Why** — the problem or requirement, not the solution.
2. **What changed** — only where it is not obvious from the diff.
3. **Verification** — which checks ran, and any that did not and why.
4. **Known gaps** — what a reader might expect that is deliberately absent.

Example:

```
M1.2: two-part Julian dates

A Julian Date near the present is about 2.46e6, so a single f64 resolves it to
roughly 40 us — coarser than a one-second scenario step and far coarser than
ephemeris interpolation needs. JulianDate now keeps a whole-day part and a
fractional part, following the IAU SOFA two-part convention.

Every interval is computed part by part. An earlier version subtracted two
summed Julian Dates and produced TT - TAI = 32.184013724 s instead of 32.184 s;
a regression test locks the exact value in.

Verification: cargo xtask ci (fmt, clippy -D warnings, 63 tests, assets,
layers; wasm could not fetch cfg-if on this machine — no network).

Gap: TDB/TCG/TCB currently reuse the TT offset; the periodic TDB term is not
applied yet and is documented at the conversion site.
```

## Numerical changes

A change to a numerical layer carries one extra burden: it names the reference it is checked
against. Put the citation in the commit body when the reference is not already cited in the
code.

If a change makes a layer *less* certain — a new approximation, an untested branch, a wider
tolerance — say so explicitly under **Known gaps**. A plausible-but-unverified number is a
defect, not a shortcut.

## Versions and the changelog

Versions live in exactly one place: `[workspace.package] version` in the root `Cargo.toml`.
Change it with:

```sh
cargo xtask version set 0.0.2
```

The changelog is written *with* the change, not afterwards. Each released version has a
section that states:

- **Added** — capabilities that now exist.
- **Correctness** — properties the numerical layers guarantee, each with its test.
- **Known limitations** — what the version deliberately cannot do yet.

The last of those matters most. An early-alpha release that overstates its coverage wastes a
user's afternoon; one that is honest about a gap saves it.

## Assets

An asset is a file with an image, font or audio extension anywhere outside `target/`. It must
have a row in `ATTRIBUTION.md` **in the same commit that adds it**. `cargo xtask assets`
enforces both directions: an unattributed file and an orphaned row both fail.

No asset from any commercial mission-analysis, CAD, graphics or office product may be added,
ever. Prefer drawing in code.
