# The paper corpus

The knowledge base reads papers, and a reader is only as good as the documents it is measured on.
This page records the corpus `cargo xtask papers` downloads: what each file is, where it came from,
what it hashes to, and what it is for. Like the ephemeris data it is **not committed** — `data/` is
`.gitignore`d — because the point is that every deployment fetches the same bytes from the same
place and verifies them.

## Why a corpus at all

Three questions need real documents rather than fixtures:

- **Does the PDF reader work on a paper, not just on a PDF this repository wrote?** A hand-built test
  PDF proves the assembly, not the parser: real papers bring two-column layouts, embedded fonts,
  Type 3 glyphs, tables and running headers.
- **Is a citation the place a person would look?** `paper.pdf:page 65, lines 34-35` is only useful if
  page 65 of that paper really holds the sentence.
- **Is the index practical?** Six documents and 580 pages is where the measurement of the cache
  starts to mean something (`docs/design/knowledge.md` records the numbers).

## What is fetched

The CCSDS blue books are the formats this project parses and emits, so a search over them is the
documentation a person would actually consult. The two papers are openly licensed work on the two
subjects the engine is built around: SGP4 propagation and TLE accuracy.

| File | What it is | Source | SHA-256 | Licence |
|---|---|---|---|---|
| `ccsds-502.0-b-3-odm.pdf` | CCSDS 502.0-B-3, *Orbit Data Messages* | `https://public.ccsds.org/Pubs/502x0b3e1.pdf` | `97d1687b4a461ffdf965b9b65923820cc6aa9d1d1c4d4df77a598a53d4a49d81` | CCSDS publication, freely distributed |
| `ccsds-503.0-b-2-tdm.pdf` | CCSDS 503.0-B-2, *Tracking Data Message* | `https://public.ccsds.org/Pubs/503x0b2c1.pdf` | `6bbe6169eccb3c6b744ba002f02785e1861f8531d3cd4dcf6fa9056efe515677` | CCSDS publication, freely distributed |
| `ccsds-504.0-b-2-adm.pdf` | CCSDS 504.0-B-2, *Attitude Data Messages* | `https://public.ccsds.org/Pubs/504x0b2.pdf` | `0869187cb326dbf009e6f17a1fd6e920b7b4a4f8f6446706b314e2d9a6121214` | CCSDS publication, freely distributed |
| `ccsds-508.0-b-1-cdm.pdf` | CCSDS 508.0-B-1, *Conjunction Data Messages* | `https://public.ccsds.org/Pubs/508x0b1e2c2.pdf` | `6afd6dad4b5f6c2ce17c3b310fdbda62bbfa6f248c32decea7de6f9116276978` | CCSDS publication, freely distributed |
| `arxiv-2402.04830-differentiable-sgp4.pdf` | *Closing the Gap Between SGP4 and High-Precision Propagation via Differentiable Programming* | `https://arxiv.org/pdf/2402.04830v5` | `391fbf4e8cc97e510c86a113e2d171c83992a67e4da422068bf752d3eb1b9dcb` | CC BY 4.0 |
| `arxiv-2605.19850-starlink-tle-trust.pdf` | *How long can you trust a Starlink TLE? An empirical comparison of SGP4 and high-fidelity propagation* | `https://arxiv.org/pdf/2605.19850v1` | `610d3caab852ce224eb41c1b1b3a25574271006414990ddacf51453f8de26420` | CC BY 4.0 |

Each checksum above was taken from a file that was downloaded, hashed, extracted and searched on the
day this page was written. None was guessed in advance, and a row is added only by running the
fetcher and copying the hash it verified.

The arXiv entries pin a *versioned* PDF (`v5`, `v1`): an arXiv document can be revised, and the
checksum would then be a lie about the file. A newer version is a new row, not an edit.

## Fetching

```sh
cargo xtask papers              # into data/papers/, which .gitignore excludes
cargo xtask papers <dir>        # or somewhere else
cargo xtask papers --force      # re-download even when the file is present and correct
cargo xtask papers --jobs 4     # how many ranges a large download is split into
```

The task downloads each file once, checks it against the recorded SHA-256, and refuses a file that
does not match — the same rule and the same code as `cargo xtask fetch-data`, because a document that
is not the document the release documented is a citation nobody can check.

## Using it

```sh
STKCRAFT_KNOWLEDGE=data/papers cargo run --release -p stkcraft -- knowledge
STKCRAFT_KNOWLEDGE=data/papers cargo run --release -p stkcraft -- knowledge "mean elements"
STKCRAFT_KNOWLEDGE=data/papers cargo run --release -p stkcraft -- ask "what does the ODM say about mean elements?"
```

The listing is the reader's own report: a paper with no text layer, a file the reader could not open,
and a document a bound cut short all say so there. On the six files above a cold open extracts 580
pages, and a second open is a cache read.

## Adding a document

1. Check that the source is stable (a versioned URL) and that the licence permits distributing the
   document, or at least reading it — a fetched corpus is downloaded by the person running it.
2. Add the row to `PAPER_FILES` in `xtask/src/main.rs` with an empty checksum.
3. Run `cargo xtask papers --force`, and copy the hash it verifies into the table.
4. Run the extraction over it and read a page: `STKCRAFT_KNOWLEDGE=data/papers cargo run -p stkcraft
   -- knowledge "<a phrase from that page>"`. A citation that points at the wrong page means the file
   changed, not that the reader is wrong.
5. Record the row here, with the same hash.

`cargo xtask assets` does not see these files because they are never in the repository; the document
that needs updating when one is added is this page and the table in `xtask/src/main.rs`, together.
