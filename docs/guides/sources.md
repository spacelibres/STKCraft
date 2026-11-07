# Sources: who says what, and what this project takes from each

The project follows **CCSDS**, **NASA** (through NAIF and its technical reports) and **ESA/ECSS**, and the point of following three authorities is to take what they agree on and to say where they differ. This page is the register: for each area, which body is the authority, which document, and where the implementation cites it.

A rule that cannot name its source here is a rule this project is not entitled to assert.

| Area | Authority | Document | Where the code says so |
|---|---|---|---|
| Time systems and their conversions | CCSDS + NAIF | CCSDS 301.0-B-4 (time code formats); NAIF `time.html` | `crates/framework/time` — the leap-second table and the scale conversions |
| Ephemeris archives | NAIF | `spk.html`, `daf.html`, `das.html` | `crates/astro/ephemeris` — segment types, the DAF reader |
| Body orientation | NAIF + IAU | `pck.html`, IAU WG reports | `crates/astro/body` — rotation models and their constants |
| Reference frames | IAU + CCSDS | `frames.html`; CCSDS 502.0-B-3 §5 | `crates/astro/frames` — TEME, GCRF, ITRF and the rotations between them |
| Orbit data exchange | CCSDS | 502.0-B-3 (ODM: OPM, OMM, OEM, OCM) | not implemented: the scenario document is serde JSON, and no ODM message is parsed or emitted |
| Tracking, attitude, conjunction data | CCSDS | 503.0-B-2 (TDM), 504.0-B-2 (ADM), 508.0-B-1 (CDM) | planned; the formats are read from the corpus in `docs/guides/ccsds-standards.md` |
| Propagation | NASA (public literature) | SGP4 as published in AIAA 2006-6753, and its verification vectors | `crates/astro/sgp4` — every formula cites its source and the vectors are transcribed |
| Access geometry | this project + published formulas | spherical and ellipsoidal geometry, hand-computable cases | `crates/astro/access` — the horizon and cone tests, with the derivation in comments |
| Space debris mitigation and traffic coordination | ESA/ECSS + CCSDS + IADC | ECSS-U-AS-10, the CCSDS 9xx orange books, the IADC guidelines | not implemented; the corpus holds the documents |
| Software engineering requirements | ESA/ECSS | ECSS-E-ST-40C, ECSS-Q-ST-80C | the development rules in `AGENTS.md` |

## Where they disagree, and what this project does

- **Units.** CCSDS writes kilometres and seconds in its messages; NAIF state vectors are kilometres and kilometres per second, and its time arguments are seconds past an epoch in TDB. The engine's typed quantities (`Distance`, `Duration`) make the conversion explicit at the boundary and the scenario document states its units, so a reader never has to guess.
- **Epoch scale.** NAIF's kernel interface is TDB; CCSDS messages carry UTC by default. This project keeps the time scale as part of the epoch type (`stkcraft-framework-time`) and refuses a silent conversion — the caller states one.
- **Naming.** NAIF's body and frame ids are integers with a registry (`naif_ids.html`); CCSDS names frames in strings. The two meet at the scenario document, which carries the NAIF id and the name.
- **Anything not decided here is not decided.** A disagreement that this page does not name is a gap to fill, not a licence to choose quietly.

## Acquisition status, with the evidence

Each authority has to be *fetchable* before its documents can be a corpus with checksums, and two of the three
are not, in the way a program can use:

| Authority | Status | Evidence |
|---|---|---|
| CCSDS | **fetched** — 495 publications, 548 MB | `cargo xtask standards`, `docs/guides/ccsds-standards.md` |
| NASA / NAIF | **fetched** — 27 documents, 2.2 MB | `cargo xtask naif`, `docs/guides/naif-corpus.md` |
| ESA / ECSS | **not fetched** | `ecss.nl` serves its catalogue as pages; the documents themselves are distributed through DOORS (`/doors-download/`), and the `wp-content/uploads/standards/*.pdf` paths answer 404. A program cannot download them without the registration the site asks for, and this repository does not scrape past a registration. |

So the ESA alignment in this release is by **reference, not by corpus**: the rules this project follows from
ECSS — software assurance (ECSS-E-ST-40C, ECSS-Q-ST-80C) and debris mitigation (ECSS-U-AS-10) — are cited in
`AGENTS.md` and in the review rules, and the CCSDS 9xx orange books that the corpus *does* hold carry much of
the same material, because they are joint CCSDS/ESA work. A person who has the ECSS PDFs can drop them into
`~/.config/stkcraft/knowledge/` and the search will read them like any other document; the repository does not
carry them and does not pretend to.

**What would change this**: a licence-and-source check that establishes a stable, program-fetchable URL for the
ECSS set (ESA's own download service, or a mirror with the publisher's permission). Until then the register
below stays honest about the gap rather than filling it with a screenshot of a login page.

## The corpora

- `docs/guides/ccsds-standards.md` — every CCSDS publication, 495 documents, hashed and searchable.
- `docs/guides/naif-corpus.md` — NAIF's required reading, 27 documents, hashed and searchable.
- `docs/guides/paper-corpus.md` — the papers the PDF reader is measured on.
