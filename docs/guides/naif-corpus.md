# The NAIF corpus

A number in this project is either traced to a published standard and a reference implementation, or it is not an answer. NAIF's required reading is the **algorithmic** half of that pair: the CCSDS books say what a message contains, and these pages say what the numbers in it mean, what a kernel file is made of, and which algorithm produces a state from it.

That is why they are a corpus rather than a footnote: NASA has spent decades writing down, in these pages, how an ephemeris is archived and how it is turned back into a position — the same operations this engine performs. `spk.html` establishes segment type 2 as Chebyshev polynomials for position, `pck.html` the orientation models, `time.html` the time systems and their conversions, `frames.html` the frame chain, `daf.html` and `das.html` the file structures underneath.

## Fetching

```sh
cargo xtask naif          # into data/naif/, which .gitignore excludes
cargo xtask naif --force  # re-download even when present and correct

STKCRAFT_KNOWLEDGE=data/naif cargo run --release -p stkcraft -- knowledge
STKCRAFT_KNOWLEDGE=data/naif cargo run --release -p stkcraft -- knowledge "SPK segment type 2 Chebyshev"
```

The task downloads every page from `https://naif.jpl.nasa.gov/pub/naif/toolkit_docs/C/req/` and verifies it against the SHA-256 below. A page is read as **text**, not markup: `crates/agent/knowledge/src/html.rs` drops the tags and keeps the lines, so a citation names a line a person can read on the page.

Measured on the first run: **27 documents, 2.2 MB, 4 661 passages**, and a search for `SPK segment type 2 Chebyshev polynomial` lands on `spk.html:lines 717-719`, which describes exactly that record.

## The table

| File | Bytes | SHA-256 |
|---|---|---|
| `abcorr.html` | 28058 | `c801451d5480c4875e54bc76e9658b74b3decce75eca0576b4a665fdd6869dab` |
| `cells.html` | 18982 | `2a81da6a56a142332ae35801519f477867d21053f42b57bcf966711432248830` |
| `ck.html` | 195638 | `a447ec34b113f0400a8315dabd4e338b44192264bb8c20fe3d5e2e282015469f` |
| `cspice.html` | 39631 | `9c1be8b3afe45a852a82f1274e5daad51440e074429faabceb39ca43055e9dd7` |
| `daf.html` | 90490 | `f2481e3d121df056037090e6433fd12c9fbfbf9b92bf9eb99da34216ba96338c` |
| `das.html` | 79847 | `039e49caede4ac1ff36ea4011de46c3e2e6891a15c01ab2872c26272e7ba3bca` |
| `dla.html` | 22217 | `7a964569c724b4a6c4a4a85cb820d945b693c200fec0002b26ec83a0367fcf9a` |
| `dsk.html` | 123147 | `f4fac7a104c942a12ae1cd8b6aa14a7b22c7b77b5f3c7200de741c3110ebc763` |
| `ek.html` | 117881 | `e7784f586127a667be04b9e1a7ad9fbdc6114d54ccd1ced64230c057a48ab08c` |
| `ellipses.html` | 28800 | `33037d9cf96f6aad7aed1a0f8bf60b18813e126f59c7a8945849dcda4c416a47` |
| `error.html` | 78112 | `8f1ecc9d0816e2158591836a5b86544f7675d90c58d11c7ee5315a82257c6591` |
| `frames.html` | 203675 | `c9a03998b4d1c652d7dba8154ae0c90a4c5bf9f4654ac8624c59a8687937df36` |
| `gf.html` | 161428 | `c96147b444c0c86c508d7017e741573f1553a500f95be30e7d012e62ee6871d3` |
| `kernel.html` | 84870 | `98ee9e4f6b32cf7b6f87f0127be915299bc6a93f2a29f9954b22ccbf0747c0a5` |
| `naif_ids.html` | 54644 | `4b647f8cc5f3101dc16011a51fe22c6beb8e5c2db49c7619cd11787a4eaf36c3` |
| `pck.html` | 89169 | `771177b1c21a448bd81b5b5f872840797d30e0c1400af363aa0fde730bf1585f` |
| `planes.html` | 24721 | `5f331c64983b88251b9b19089fec85b9ec72fd44fb844dcbc3455f85cb309eb5` |
| `problems.html` | 64507 | `e18412a0ab82ef7d3066af2c122186487c17739a0a31ed1453c5cc1a06526ae8` |
| `rotation.html` | 118680 | `729046b42faa732c0f3b0dedb9ca99fbff2a302ee930a288754c7785eacfdcf2` |
| `scanning.html` | 10292 | `f511c09346e65106d19c93e6a877f2b6a293b242d5e30ac2b8ec0745702462f2` |
| `sclk.html` | 73244 | `ffe5ad1440bd92822508e1565cae432fc5cc33ba7ab8939b96d5f541766d3ff5` |
| `sets.html` | 21455 | `f76bd5c7e8b5d12fb1673db156d069e49fa5b28fb3fd4f3b887542673a8ca79c` |
| `spc.html` | 32687 | `9ebf57fdc6b2af9cf921a65be908d4593ebcf47528bbef5e91a9511b1239190c` |
| `spk.html` | 216199 | `8aa1f05a4fe60e8ad705f24a840acb4b696d2160ded176bbd585c28440905c88` |
| `symbols.html` | 26002 | `7fb9ddb395a118a030883779242d860f820106905179e14793560168a06e52e5` |
| `time.html` | 122505 | `025bd684a9997feca2f51a54550a52898e1d7686d500fee0e7b0ff3c9ebc54a1` |
| `windows.html` | 25220 | `14e032225ef9b30350872b989f03748a05cd7fb84fd6f5b7ac1315b37af73660` |

## What each page is

| Page | What it establishes |
|---|---|
| `spk.html` | SPK segment types and their record layouts: Chebyshev polynomials, Lagrange interpolation, the mid-point and radius a type 2 record carries |
| `pck.html` | orientation models: Euler angles, RA/DEC/prime meridian, the constants a body kernel publishes |
| `frames.html` | the frame chain and how a transformation is composed |
| `time.html` | the time systems and the conversions between them |
| `daf.html`, `das.html`, `dla.html` | the file structures under a kernel: arrays, summaries, character data |
| `dsk.html`, `ck.html`, `sclk.html` | shape models, pointing, spacecraft clock correlation |
| `rotation.html`, `planes.html`, `ellipses.html`, `scanning.html`, `gf.html`, `windows.html` | the geometry and the search primitives |
| `naif_ids.html` | the naming conventions for bodies, frames and instruments |

## How this corpus is used

- **A rule cites the page.** A comment that states how a type 2 record is evaluated names `spk.html`; a conversion between time systems names `time.html`. See `docs/guides/sources.md` for the register of authorities.

- **A question is answered from it.** `knowledge.search` over this corpus is how the assistant quotes the definition instead of remembering it, and the citation is a line on the page.

- **A discrepancy is a finding.** Where NAIF and a CCSDS book say different things — units, a keyword's scope, an epoch's scale — the difference is recorded in `sources.md` rather than smoothed over, because a merged standard that hides a disagreement is worse than two standards.
