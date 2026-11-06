# Storage

STKCraft keeps state in three kinds of store, and which kind a thing belongs to is decided here once
rather than per feature. The rule that drives every row of the table below: **a store is chosen by the
question a reader asks of the data**, not by convenience.

| Question | Store | Why |
|---|---|---|
| "What was the scenario, and what exactly did it compute?" | Relational | Transactions, joins, and a history a mission reviewer can query |
| "Give me this value again, it will not have changed" | Key-value | One key, one value, no query language needed |
| "Which passages are near this question?" | Vector | Nearest-neighbour over embeddings is not a scan |

Anything that fits none of them is a file on disk with a checksum — an ephemeris kernel, a fetched
corpus, a font. Those are not records and they do not belong in a database.

## 1. Relational: the record

The relational store holds what a mission engineer would call *the record*: the things that were
decided, computed and signed off. It is written in transactions and read with joins, and it is the only
store in the system that is expected to be **queried in ways nobody anticipated** ("all passes under 5°
elevation that overlap a downlink window in Q3").

| Table | Holds | Why relational |
|---|---|---|
| `scenario` | one row per document: id, name, central body, epoch, duration, revision | it is the unit of work, and everything else points at it |
| `scenario_revision` | an append-only revision per save: scenario id, revision, timestamp, author, the serialised document | "who changed what, when" is a join, and an undo is a pointer |
| `satellite`, `station`, `sensor` | the scenario's contents, one row each, typed columns | a query like "every station with a 10° mask" is a `WHERE`, not a parse |
| `access_window` | satellite, station, start, end, minimum elevation, the run that produced it | the analysis result people quote; indexable by time range |
| `computation` | a run: id, scenario revision, command id, parameters, engine version, ephemeris digest, started/finished | **provenance**: which numbers came from which inputs |
| `knowledge_document` | one row per ingested document: path, digest, bytes, pages, added time, source URL | "which document says this" is a join to the citation below |
| `citation` | document id, page or line range, the text quoted, the answer that quoted it | an answer's evidence, queryable afterwards |
| `translation_row` | locale, context, English source, translation, provenance (a person or a model) | the catalogs are queried and reviewed, not just read |
| `extension_package` | id, version, kind, trust tier, enabled-for-app, installed time | the store `craft-extensions` describes |

**Engine of record:** SQLite, through one crate (`stkcraft-storage`), with the schema versioned and a
migration per version. It is embedded, it is a file a person can copy, it needs no server, and its
`STRICT` tables catch the type mistakes that make an analysis unreproducible. A deployment that needs
concurrency moves to PostgreSQL by changing the driver, not the schema — every column is portable.

**What is *not* relational:** ephemeris files, kernels, fonts, the papers a user fetches. They are
blobs on disk with a digest in the database, which is the only part of them the record needs.

## 2. Key-value: the cache

The key-value store holds values that are **derivable and disposable**: if the file is deleted, the
program recomputes them and nobody notices beyond the wait. That property is what makes it safe to keep
unversioned and to clear on upgrade.

| Key | Value | Recompute cost |
|---|---|---|
| `kernel/<sha256>/index` | the SPK segment directory: target, centre, start, end, offset | a few seconds of parsing |
| `knowledge/<root-digest>/index` | the inverted index: term → postings, chunk offsets | seconds to minutes on a paper corpus |
| `knowledge/<root-digest>/vectors` | the embeddings for that index, once an embedding model exists | hours — the reason the vector store is separate |
| `translation/<locale>/<catalog-digest>` | generated catalog text, so a second run does not pay for the model | one model call per row |
| `prefs` | the `ui.json` the window already writes | nothing |

**Engine:** one crate, one file format, no service — a directory of length-prefixed records with a
CRC per value, or `redb` if the format needs transactions. It is chosen for *durability of cheap data*,
not for queries: a key either hits or misses.

**What is *not* KV:** the scenario. It looks like "one key, one value" until someone asks for the
history, and then it is a schema.

## 3. Vector: the recall layer

A vector store answers one question — *which passages are near this text* — and it exists because
paper corpora are too large and too paraphrased for lexical search alone. It is **not** the authority:
a semantic hit is a candidate, and the answer still cites the passage and the page.

| What is stored | Field |
|---|---|
| embedding | a fixed-width float vector, with its model id and dimension |
| passage | document id, page or line range, the text |
| metadata | the model, the chunking parameters, the index revision |

**Engine:** the same key-value file, holding `(id, vector, metadata)` records, with a brute-force cosine
scan and an optional HNSW index per document set. At paper-corpus scale — tens of thousands of chunks —
a linear scan in `f32` is milliseconds, and a real vector database (Qdrant, LanceDB) is a decision to
make when the corpus outgrows that, not before.

**Embeddings are not built yet**, and this is a stated limit rather than a hidden one: the model that
would produce them is a dependency on a provider, and until it is chosen the lexical index in
`stkcraft-knowledge` is the retrieval, with page-level citations. The vector table above is the shape the
data will take, so the day an embedding exists it fills a store rather than forcing a redesign.

## 4. Where each store lives

```text
${XDG_DATA_HOME:-~/.local/share}/stkcraft/
├── stkcraft.db          the relational record (SQLite, STRICT, versioned)
├── cache/
│   ├── kernels/         derived kernel indexes
│   ├── knowledge/       derived inverted index and vectors
│   └── translations/    generated catalogs
└── log/
```

`${XDG_CONFIG_HOME:-~/.config}/stkcraft/` keeps only what a person chose: `ui.json`, the language
catalogs they generated, their knowledge documents, their extension packages. Data is what the program
produced; config is what the person decided. A copy of the config directory reproduces someone's setup
without copying a byte of derived data — and deleting `cache/` never loses a decision.

## 5. Rules

- **One crate owns persistence** (`stkcraft-storage`). No crate opens a database, writes a key-value
  file or issues SQL anywhere else; a crate that needs to remember something asks the storage crate.
- **The record is append-only.** Nothing edits a computation row; a re-run is a new row with a new
  digest. Undo is a pointer to an earlier revision, not a mutation.
- **Every stored value that came from outside carries its digest**: a kernel, a document, a fetched
  paper. A record without its input digest cannot be reproduced and is not written.
- **Derived data is disposable and never the source of truth.** If deleting a cache changes an answer,
  the cache was holding a record by mistake.
- **Migrations are forward-only and tested**: schema version in the file, one migration per bump, and a
  test that opens the oldest supported version and reads it.
- **Nothing is scattered.** A new feature that wants to remember something adds a table or a key to
  this document first; a stray JSON file beside the executable is the failure this section exists to
  prevent.

## 6. What is built, and what is not

`crates/framework/storage` implements this document: `Store` (SQLite, `STRICT`, WAL, foreign keys,
forward-only migrations from `PRAGMA user_version`), `Cache` (one file per key, the key never used as a
path, CRC-32 per value, a damaged value is a miss), and `Vectors` (model and dimension checked, cosine
by brute force, a flat format for the cache). The record carries the schema of §1 except the two tables
that have no writer yet: `translation_row` and `extension_package` exist and are not filled.

The first real consumer is provenance: `stkcraft passes` and `stkcraft planet` record a `computation`
row — command, parameters, engine version, the **digest of their inputs** — and the access windows they
found, and `stkcraft history` lists them. A number quoted from a report can therefore be traced to the
run that produced it and to what that run read.

- **No PostgreSQL path.** The schema is portable and the driver swap is deferred.
- **No embeddings and no vector index in use**: the store exists and is tested, and it holds nothing
  until an embedding model is chosen. The lexical index in `stkcraft-knowledge` is the retrieval.
- **The knowledge index is cached in the KV store**: the whole assembled library — documents and index —
  under a key derived from every file's path, size and modification time, so a re-open is one read and
  an edited corpus replaces its entry rather than accumulating one per save. That is measured in
  `CHANGELOG.md` 0.1.35: 1.43 s → 0.55 s and 173 MB → 124 MB on a 2 000-document corpus.
- **No corpus fetcher**: the papers the knowledge base should hold need a pinned, checksummed download
  like `cargo xtask fetch-data`.
