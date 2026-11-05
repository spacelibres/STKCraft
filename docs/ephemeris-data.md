# Ephemeris data

This repository does not contain the ephemeris kernels it reads, and will not: they are large
binaries published by NAIF, they are not source, and a copy in the repository would age silently
against the file every deployment actually uses. The rule in `AGENTS.md` is that bulk data lives
outside the repository; this page is how that rule is kept for ephemerides.

## What is fetched, and from where

| File | Source | SHA-256 | Used for |
|---|---|---|---|
| `de440s.bsp` | `https://naif.jpl.nasa.gov/pub/naif/generic_kernels/spk/planets/de440s.bsp` | `c1c7feeab882263fc493a9d5a5b2ddd71b54826cdf65d8d17a76126b260a49f2` | Planet and Moon positions, 1849–2150 |

Satellite-system kernels (Mars, Jupiter, Saturn, Uranus, Neptune) are published the same way under
`https://naif.jpl.nasa.gov/pub/naif/generic_kernels/spk/satellites/`. They are fetched by the same
procedure; each one is added to the table above with its own checksum at the moment it is first
used, and never before — a row whose checksum was guessed is worse than no row.

## Fetching

```sh
cargo xtask fetch-data            # into data/, which .gitignore excludes
cargo xtask fetch-data <dir>      # or somewhere else
```

The task downloads each file once, checks it against the recorded SHA-256, and refuses a file that
does not match. It refuses to reuse a file that hashes differently, because a truncated kernel is
exactly the kind of input that produces a plausible wrong trajectory instead of an error — this
repository has already had one download end early, and the checksum is what caught it.

## Verifying the reader against a kernel

The tests that compare the SPK reader with CSPICE are skipped when no kernel is present rather than
passing as though they had run. To run them:

```sh
cargo xtask fetch-data
STKCRAFT_EPHEMERIS_KERNEL=data/de440s.bsp cargo test -p stkcraft-ephemeris
```

Reference states in that test are transcriptions of CSPICE's `spkezr` output and name the kernel's
checksum in their comment, so a test vector can always be traced back to the file that produced it.

## At deployment

A deployment fetches the same files from the same URLs with the same checksums. Nothing in the build
needs them: the crate compiles without data, and an ephemeris that is asked for a body no loaded
kernel holds says so by name.
