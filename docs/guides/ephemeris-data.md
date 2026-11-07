# Bulk data

This repository does not contain the bulk data it reads, and will not: ephemeris kernels, Earth
orientation parameters, leap-second tables, planetary constants and star catalogues are large
published files rather than source, and a copy in the repository would age silently against the file
every deployment actually uses. The rule in `AGENTS.md` is that bulk data lives
outside the repository; this page is how that rule is kept for ephemerides.

## What is fetched, and from where

This table is the list `cargo xtask fetch-data` carries in `xtask/src/main.rs`. Each checksum is the one
that task verifies, and each matches the file a fetch places in `data/`.

| File | Source | SHA-256 | Used for |
|---|---|---|---|
| `de440s.bsp` | `https://naif.jpl.nasa.gov/pub/naif/generic_kernels/spk/planets/de440s.bsp` | `c1c7feeab882263fc493a9d5a5b2ddd71b54826cdf65d8d17a76126b260a49f2` | Planet and Moon barycentres, 1849–2150 |
| `mar099.bsp` | `https://naif.jpl.nasa.gov/pub/naif/generic_kernels/spk/satellites/mar099.bsp` | `9991e57b196bae1a096acc6e2afc6718102ee420d684695de0f0333064c046bc` | Mars, Phobos, Deimos |
| `jup365.bsp` | `https://naif.jpl.nasa.gov/pub/naif/generic_kernels/spk/satellites/jup365.bsp` | `dbf016c01ba4d022154838000cf3f06962cf958ddc503a366f7fe8f81495c5cb` | Jupiter and its moons |
| `sat441.bsp` | `https://naif.jpl.nasa.gov/pub/naif/generic_kernels/spk/satellites/sat441.bsp` | `d7e444a9ba7a52b8f448ff0747030789524d2f0c1212e4e2bd7f5fc3c96444d5` | Saturn and its moons |
| `ura111xl-799.bsp` | `https://naif.jpl.nasa.gov/pub/naif/generic_kernels/spk/satellites/ura111xl-799.bsp` | `fa55665eb129613574a5d029bf2cc7b521d3f0d2edc7de5790add3cf950714bf` | Uranus and its moons |
| `nep097xl-899.bsp` | `https://naif.jpl.nasa.gov/pub/naif/generic_kernels/spk/satellites/nep097xl-899.bsp` | `95e815cb8584e530d3c83f35e39af4bf67d0874f01b65538cbe9dbf279818df8` | Neptune and its moons |
| `nep097xl-801.bsp` | `https://naif.jpl.nasa.gov/pub/naif/generic_kernels/spk/satellites/nep097xl-801.bsp` | `3b4ec5b6f23a3b10da9574bc8fb35cfd93ebf9d7951bdcba0c7e26309304bfb1` | Triton (about 2.1 GB) |
| `finals2000A.all` | `https://datacenter.iers.org/data/9/finals2000A.all` | `4c2308a3a782f561a84f549ab0e074e745aa950facd9dff56e2e60a7eac8f042` | Earth orientation, IERS Bulletin A |
| `naif0012.tls` | `https://naif.jpl.nasa.gov/pub/naif/generic_kernels/lsk/naif0012.tls` | `678e32bdb5a744117a467cd9601cd6b373f0e9bc9bbde1371d5eee39600a039b` | Leap-second kernel |
| `pck00011.tpc` | `https://naif.jpl.nasa.gov/pub/naif/generic_kernels/pck/pck00011.tpc` | `3dff7b1dbeceaa01f25467767d3fa25816051c85d162d1edf04acb310ee28bb1` | Planetary constants and rotation models |
| `gm_de440.tpc` | `https://naif.jpl.nasa.gov/pub/naif/generic_kernels/pck/gm_de440.tpc` | `924ddf4fb9ead9fe8a1aa55780bcabde40b09d00065d58226e24b68d8092f140` | Planetary and satellite masses (DE440) |
| `hygdata_v40.csv.gz` | `https://raw.githubusercontent.com/astronexus/HYG-Database/main/hyg/CURRENT/hygdata_v40.csv.gz` | `8e3ff9e67445e558a759b117910850cff1b1d4d492f45f715c2ee2db3d869bac` | Star catalogue (HYG) |

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
kernel_root="$PWD/data"
STKCRAFT_EPHEMERIS_KERNEL="$kernel_root/de440s.bsp:$kernel_root/mar099.bsp:$kernel_root/jup365.bsp:$kernel_root/sat441.bsp:$kernel_root/ura111xl-799.bsp:$kernel_root/nep097xl-899.bsp:$kernel_root/nep097xl-801.bsp" cargo test -p stkcraft-astro-ephemeris
```

Use absolute paths: Cargo runs tests from the package directory, not necessarily the workspace
root. The full 54-state reference test needs all seven SPKs because `de440s.bsp` alone carries
planetary barycentres rather than every planet's body centre and moons. Supplying only the
planetary file is not the complete reference test.

The variable is a list joined by the platform's path separator: `:` on Unix and `;` on Windows.

In PowerShell, construct the path list from the workspace root:

```powershell
$kernelPaths = @('de440s.bsp', 'mar099.bsp', 'jup365.bsp', 'sat441.bsp', 'ura111xl-799.bsp', 'nep097xl-899.bsp', 'nep097xl-801.bsp') |
    ForEach-Object { (Resolve-Path (Join-Path 'data' $_)).Path }
$env:STKCRAFT_EPHEMERIS_KERNEL = $kernelPaths -join [IO.Path]::PathSeparator
cargo test -p stkcraft-astro-ephemeris
```

Reference states in that test are transcriptions of CSPICE's `spkezr` output and name the kernel's
checksum in their comment, so a test vector can always be traced back to the file that produced it.

## At deployment

A deployment fetches the same files from the same URLs with the same checksums. Nothing in the build
needs them: the crate compiles without data, and an ephemeris that is asked for a body no loaded
kernel holds says so by name.
