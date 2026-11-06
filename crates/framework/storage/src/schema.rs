//! The schema, and the forward-only migrations that reach it.
//!
//! One table set, one version number, one migration per bump. The rules that make it safe to keep
//! someone's record across releases:
//!
//! - **Forward only.** A migration runs once and never reverses. Rolling back a release means
//!   restoring a copy of the file, which is why the file is a single portable artifact.
//! - **`STRICT` tables.** SQLite otherwise stores whatever type arrives in whatever column; a
//!   `REAL` that became `TEXT` is an analysis that cannot be reproduced. `STRICT` makes the type an
//!   error instead of a surprise.
//! - **Every migration is tested from the oldest supported version**, because the version a user has
//!   is the version nobody develops on.

/// The schema version this build writes.
pub const SCHEMA_VERSION: i64 = 2;

/// The statements that create version 1.
///
/// Kept as one string so a reader can see the whole shape at once, and so a migration is a function
/// rather than a diff to reverse-engineer.
pub const V1: &str = r"
CREATE TABLE scenario (
    id               INTEGER PRIMARY KEY,
    name             TEXT NOT NULL,
    central_body     TEXT NOT NULL,
    epoch            TEXT NOT NULL,
    duration_seconds REAL NOT NULL,
    created_at       TEXT NOT NULL
) STRICT;

-- Append-only: a revision is never edited, and a re-save is a new row.
CREATE TABLE scenario_revision (
    scenario_id INTEGER NOT NULL REFERENCES scenario(id) ON DELETE CASCADE,
    revision    INTEGER NOT NULL,
    recorded_at TEXT NOT NULL,
    author      TEXT NOT NULL,
    digest      TEXT NOT NULL,
    document    TEXT NOT NULL,
    PRIMARY KEY (scenario_id, revision)
) STRICT;

CREATE TABLE satellite (
    id          INTEGER PRIMARY KEY,
    scenario_id INTEGER NOT NULL REFERENCES scenario(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    line1       TEXT NOT NULL,
    line2       TEXT NOT NULL,
    color       TEXT NOT NULL
) STRICT;

CREATE TABLE station (
    id                    INTEGER PRIMARY KEY,
    scenario_id           INTEGER NOT NULL REFERENCES scenario(id) ON DELETE CASCADE,
    name                  TEXT NOT NULL,
    latitude_deg          REAL NOT NULL,
    longitude_deg         REAL NOT NULL,
    minimum_elevation_deg REAL NOT NULL
) STRICT;

-- One row per command run: the provenance that makes a stored number checkable.
CREATE TABLE computation (
    id               INTEGER PRIMARY KEY,
    scenario_name    TEXT NOT NULL,
    scenario_revision INTEGER,
    command          TEXT NOT NULL,
    parameters       TEXT NOT NULL,
    engine_version   TEXT NOT NULL,
    inputs_digest    TEXT NOT NULL,
    started_at       TEXT NOT NULL,
    duration_ms      INTEGER NOT NULL,
    summary          TEXT NOT NULL
) STRICT;

CREATE INDEX computation_by_command ON computation (command, started_at);

CREATE TABLE access_window (
    id                    INTEGER PRIMARY KEY,
    computation_id        INTEGER NOT NULL REFERENCES computation(id) ON DELETE CASCADE,
    satellite             TEXT NOT NULL,
    station               TEXT NOT NULL,
    start_utc             TEXT NOT NULL,
    end_utc               TEXT NOT NULL,
    start_seconds         REAL NOT NULL,
    duration_seconds      REAL NOT NULL,
    minimum_elevation_deg REAL NOT NULL
) STRICT;

CREATE INDEX access_window_by_start ON access_window (start_utc);

CREATE TABLE knowledge_document (
    id         INTEGER PRIMARY KEY,
    path       TEXT NOT NULL UNIQUE,
    digest     TEXT NOT NULL,
    bytes      INTEGER NOT NULL,
    passages   INTEGER NOT NULL,
    source_url TEXT,
    added_at   TEXT NOT NULL
) STRICT;

CREATE TABLE citation (
    id             INTEGER PRIMARY KEY,
    computation_id INTEGER REFERENCES computation(id) ON DELETE SET NULL,
    document_id    INTEGER NOT NULL REFERENCES knowledge_document(id) ON DELETE CASCADE,
    locator        TEXT NOT NULL,
    text           TEXT NOT NULL,
    recorded_at    TEXT NOT NULL
) STRICT;

CREATE TABLE translation_row (
    locale      TEXT NOT NULL,
    context     TEXT NOT NULL DEFAULT '',
    source      TEXT NOT NULL,
    translation TEXT NOT NULL,
    origin      TEXT NOT NULL,
    recorded_at TEXT NOT NULL,
    PRIMARY KEY (locale, context, source)
) STRICT;

CREATE TABLE extension_package (
    id              TEXT PRIMARY KEY,
    version         TEXT NOT NULL,
    kind            TEXT NOT NULL,
    trust           TEXT NOT NULL,
    installed_at    TEXT NOT NULL
) STRICT;
";

/// The statements that reach `version` from the one before it.
///
/// `None` when the version is not one this build knows how to reach.
#[must_use]
pub fn migration(version: i64) -> Option<&'static str> {
    match version {
        1 => Some(V1),
        2 => Some(V2),
        _ => None,
    }
}

/// The statements that create version 2: the formula store.
///
/// A formula is a *record* with provenance (`docs/design/mathematics.md`), so it lives in the
/// relational store rather than in the index cache: the box, both renderings, the sentence that
/// introduces it — what it solves — the engine that read it, and the digests that make the chain
/// re-verifiable. Two strings are never the same formula, so the natural key is the region: a document,
/// a page and a box.
pub const V2: &str = r"
CREATE TABLE formula (
    id             INTEGER PRIMARY KEY,
    document       TEXT NOT NULL,
    page           INTEGER NOT NULL,
    x0             REAL NOT NULL,
    y0             REAL NOT NULL,
    x1             REAL NOT NULL,
    y1             REAL NOT NULL,
    -- What the text layer says, always, and what a recogniser made of the image when there is one.
    unicode        TEXT NOT NULL,
    latex          TEXT,
    -- What it solves: the sentence or caption that introduces it, and where that line is.
    problem        TEXT,
    problem_lines  TEXT,
    source         TEXT NOT NULL,
    confidence     REAL,
    mathlib_name   TEXT,
    mathlib_score  REAL,
    mathlib_domain TEXT,
    mathlib_rev    TEXT,
    classifier     TEXT,
    -- Provenance: the PDF it came from and the pixels of the region.
    paper_digest   TEXT NOT NULL,
    region_digest  TEXT NOT NULL,
    recorded_at    TEXT NOT NULL,
    UNIQUE (document, page, x0, y0, x1, y1) ON CONFLICT REPLACE
) STRICT;

-- The classification is a query: which formulas are about this concept, which are in this domain.
CREATE INDEX formula_by_mathlib ON formula(mathlib_name);
CREATE INDEX formula_by_domain ON formula(mathlib_domain);
CREATE INDEX formula_by_document ON formula(document, page);
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_build_knows_a_migration_for_its_own_schema_version() {
        // A schema version with no migration is a database nobody can create.
        assert!(migration(SCHEMA_VERSION).is_some());
        assert!(migration(0).is_none());
        assert!(migration(SCHEMA_VERSION + 1).is_none());
    }

    #[test]
    fn every_table_is_strict() {
        // The rule the schema exists for: an `INTEGER` column cannot quietly hold text.
        for statement in V1.split(';').chain(V2.split(';')) {
            let statement = statement.trim_start().to_ascii_uppercase();
            if statement.starts_with("CREATE TABLE") {
                assert!(
                    statement.ends_with("STRICT"),
                    "a table is not STRICT: {statement}"
                );
            }
        }
    }
}
