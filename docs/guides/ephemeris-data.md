# Bulk data

This repository does not contain the bulk data it reads, and will not: ephemeris kernels, Earth
orientation parameters, leap-second tables, planetary constants and star catalogues are large
published files rather than source, and a copy in the repository would age silently against the file
every deployment actually uses. The rule in `AGENTS.md` is that bulk data lives
outside the repository; this page is how that rule is kept for ephemerides.

## What is fetched, and from where

| File | Source | SHA-256 | Used for |
|---|---|---|---|
| `de440s.bsp` | `https://naif.jpl.nasa.gov/pub/naif/generic_kernels/spk/planets/de440s.bsp` | `c1c7feeab882263fc493a9d5a5b2ddd71b54826cdf65d8d17a76126b260a49f2` | Planet and Moon barycentres, 1849–2150 |

Earth orientation (`finals2000A.all`, IERS Bulletin A), the leap-second kernel
(`naif0012.tls`), the planetary constants and mass kernels (`pck00011.tpc`, `gm_de440.tpc`) and the
HYG star catalogue are fetched and verified the same way.

The satellite-system kernels come from
`https://naif.jpl.nasa.gov/pub/naif/generic_kernels/spk/satellites/` and carry the planets
themselves along with their moons. Each checksum below was taken from a file that was downloaded,
verified against CSPICE, and used in the regression table; none was guessed in advance.

| `mar099.bsp` | `.../satellites/mar099.bsp` | `9991e57b196bae1a096acc6e2afc6718102ee420d684695de0f0333064c046bc` | Mars, Phobos, Deimos |
| `jup365.bsp` | `.../satellites/jup365.bsp` | `dbf016c01ba4d022154838000cf3f06962cf958ddc503a366f7fe8f81495c5cb` | Jupiter and its moons |
| `sat441.bsp` | `.../satellites/sat441.bsp` | `d7e444a9ba7a52b8f448ff0747030789524d2f0c1212e4e2bd7f5fc3c96444d5` | Saturn and its moons |
| `ura111xl-799.bsp` | `.../satellites/ura111xl-799.bsp` | `fa55665eb129613574a5d029bf2cc7b521d3f0d2edc7de5790add3cf950714bf` | Uranus and its moons |
| `nep097xl-899.bsp` | `.../satellites/nep097xl-899.bsp` | `95e815cb8584e530d3c83f35e39af4bf67d0874f01b65538cbe9dbf279818df8` | Neptune and its moons |
| `nep097xl-801.bsp` | `.../satellites/nep097xl-801.bsp` | `3b4ec5b6f23a3b10da9574bc8fb35cfd93ebf9d7951bdcba0c7e26309304bfb1` | Triton (2.1 GB) |


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
STKCRAFT_EPHEMERIS_KERNEL=data/de440s.bsp cargo test -p stkcraft-astro-ephemeris
```

Reference states in that test are transcriptions of CSPICE's `spkezr` output and name the kernel's
checksum in their comment, so a test vector can always be traced back to the file that produced it.

## At deployment

A deployment fetches the same files from the same URLs with the same checksums. Nothing in the build
needs them: the crate compiles without data, and an ephemeris that is asked for a body no loaded
kernel holds says so by name.
