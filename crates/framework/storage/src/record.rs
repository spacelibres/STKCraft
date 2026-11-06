//! The record: what was computed, on what, and with which engine.
//!
//! [`Store`] is the only place in the project that issues SQL. It holds the scenario document's
//! revisions, the computations run against them, the access windows those computations produced, the
//! knowledge documents a person added and the citations an answer made. Everything is append-only:
//! nothing edits a computation, and a re-run is a new row with a new digest, so "what did this number
//! come from" always has an answer.

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension as _, params};
use stkcraft_core::{Error, Result};

use crate::schema::{SCHEMA_VERSION, migration};

/// One command run, and everything needed to reproduce it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Computation {
    /// The scenario the run was about, by name.
    pub scenario_name: String,
    /// The saved revision it ran on, when the scenario had been saved.
    pub scenario_revision: Option<i64>,
    /// The command id, e.g. `access.compute`.
    pub command: String,
    /// The parameters as JSON, verbatim, so a reader can re-run the same call.
    pub parameters: String,
    /// The engine's version, which decides the algorithm.
    pub engine_version: String,
    /// [`crate::digest`] of the inputs: the scenario document, the elements, the kernels.
    pub inputs_digest: String,
    /// What the command reported, in one line.
    pub summary: String,
}

/// A computation as stored, with what the store added.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredComputation {
    /// The row id.
    pub id: i64,
    /// The record as it was written.
    pub record: Computation,
    /// When the run finished, ISO-8601.
    pub started_at: String,
    /// How long it took.
    pub duration: Duration,
    /// How many access windows it stored.
    pub windows: usize,
}

/// One interval of access, as stored against the run that found it.
#[derive(Debug, Clone, PartialEq)]
pub struct AccessWindowRecord {
    /// The satellite's name at the time of the run.
    pub satellite: String,
    /// The station's name at the time of the run.
    pub station: String,
    /// Start, ISO-8601 in UTC.
    pub start_utc: String,
    /// End, ISO-8601 in UTC.
    pub end_utc: String,
    /// Start, seconds from the scenario epoch.
    pub start_seconds: f64,
    /// Duration, seconds.
    pub duration_seconds: f64,
    /// The horizon mask the window was found against.
    pub minimum_elevation_deg: f64,
}

/// One saved revision of a scenario document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revision {
    /// Which save this is, from 1.
    pub revision: i64,
    /// When it was written, ISO-8601.
    pub recorded_at: String,
    /// Who wrote it: a person, or the tool that did.
    pub author: String,
    /// [`crate::digest`] of `document`.
    pub digest: String,
    /// The document itself, as JSON.
    pub document: String,
}

/// One ingested knowledge document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentRecord {
    /// Its path relative to the library root.
    pub path: String,
    /// [`crate::digest`] of the text read.
    pub digest: String,
    /// How many bytes were read.
    pub bytes: u64,
    /// How many passages it was split into.
    pub passages: u64,
    /// Where it came from, for a fetched paper.
    pub source_url: Option<String>,
}

/// The relational store.
pub struct Store {
    connection: Connection,
}

impl Store {
    /// Open the record, creating it if it is not there and migrating it if it is older.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] when the file cannot be opened or the schema cannot be brought up to date,
    /// with the reason a reader needs.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(directory) = path.parent() {
            std::fs::create_dir_all(directory).map_err(|error| {
                Error::invalid(
                    "storage directory",
                    format!("{}: {error}", directory.display()),
                )
            })?;
        }
        let connection = Connection::open(path).map_err(|error| {
            Error::invalid("storage record", format!("{}: {error}", path.display()))
        })?;
        Self::prepare(connection)
    }

    /// Open a record in memory, which is what a test uses.
    ///
    /// # Errors
    ///
    /// As [`Self::open`].
    pub fn open_in_memory() -> Result<Self> {
        let connection = Connection::open_in_memory()
            .map_err(|error| Error::invalid("storage record", error.to_string()))?;
        Self::prepare(connection)
    }

    /// Set the pragmas, then migrate.
    fn prepare(connection: Connection) -> Result<Self> {
        // WAL so a reader does not block a writer; foreign keys on, because a row that points at a
        // scenario that does not exist is a broken record; `NORMAL` sync, because a lost analysis is
        // re-runnable and a lost commit-time fsync per window is not worth the wait.
        connection
            .execute_batch(
                "PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON; PRAGMA synchronous = NORMAL;",
            )
            .map_err(|error| Error::invalid("storage pragmas", error.to_string()))?;
        let mut store = Self { connection };
        store.migrate()?;
        Ok(store)
    }

    /// Bring the schema up to [`SCHEMA_VERSION`], one version at a time.
    fn migrate(&mut self) -> Result<()> {
        let current: i64 = self
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(|error| Error::invalid("storage version", error.to_string()))?;
        if current > SCHEMA_VERSION {
            return Err(Error::invalid(
                "storage version",
                format!(
                    "the record is version {current} and this build understands {SCHEMA_VERSION}; \
                     it was written by a newer STKCraft"
                ),
            ));
        }
        for version in (current + 1)..=SCHEMA_VERSION {
            let statements = migration(version).ok_or_else(|| {
                Error::invalid(
                    "storage version",
                    format!("no migration to version {version}"),
                )
            })?;
            self.connection
                .execute_batch(statements)
                .map_err(|error| Error::invalid("storage migration", error.to_string()))?;
            self.connection
                .pragma_update(None, "user_version", version)
                .map_err(|error| Error::invalid("storage version", error.to_string()))?;
        }
        Ok(())
    }

    /// The schema version of the open record.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] when the version cannot be read.
    pub fn schema_version(&self) -> Result<i64> {
        self.connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(|error| Error::invalid("storage version", error.to_string()))
    }

    /// Record a computation, and the moment it finished.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] when the row cannot be written.
    pub fn record_computation(&mut self, record: &Computation) -> Result<i64> {
        let started_at = now_iso8601();
        self.connection
            .execute(
                "INSERT INTO computation (
                    scenario_name, scenario_revision, command, parameters, engine_version,
                    inputs_digest, started_at, duration_ms, summary
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    record.scenario_name,
                    record.scenario_revision,
                    record.command,
                    record.parameters,
                    record.engine_version,
                    record.inputs_digest,
                    started_at,
                    0_i64,
                    record.summary,
                ],
            )
            .map_err(|error| Error::invalid("computation row", error.to_string()))?;
        Ok(self.connection.last_insert_rowid())
    }

    /// Fill in how long a computation took, once it is known.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] when there is no such computation.
    pub fn finish_computation(&mut self, id: i64, duration: Duration) -> Result<()> {
        let milliseconds = i64::try_from(duration.as_millis()).unwrap_or(i64::MAX);
        let changed = self
            .connection
            .execute(
                "UPDATE computation SET duration_ms = ?1 WHERE id = ?2",
                params![milliseconds, id],
            )
            .map_err(|error| Error::invalid("computation row", error.to_string()))?;
        if changed == 0 {
            return Err(Error::NotFound {
                what: format!("computation {id}"),
            });
        }
        Ok(())
    }

    /// Store one access window against a computation.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] when the window cannot be written — including when the computation does not
    /// exist, which the foreign key refuses rather than storing an orphan.
    pub fn record_window(&mut self, computation: i64, window: &AccessWindowRecord) -> Result<()> {
        self.connection
            .execute(
                "INSERT INTO access_window (
                    computation_id, satellite, station, start_utc, end_utc,
                    start_seconds, duration_seconds, minimum_elevation_deg
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    computation,
                    window.satellite,
                    window.station,
                    window.start_utc,
                    window.end_utc,
                    window.start_seconds,
                    window.duration_seconds,
                    window.minimum_elevation_deg,
                ],
            )
            .map_err(|error| Error::invalid("access window row", error.to_string()))?;
        Ok(())
    }

    /// The most recent computations, newest first.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] when the rows cannot be read.
    pub fn computations(&self, limit: usize) -> Result<Vec<StoredComputation>> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let mut statement = self
            .connection
            .prepare(
                "SELECT c.id, c.scenario_name, c.scenario_revision, c.command, c.parameters,
                        c.engine_version, c.inputs_digest, c.summary, c.started_at, c.duration_ms,
                        (SELECT COUNT(*) FROM access_window w WHERE w.computation_id = c.id)
                 FROM computation c ORDER BY c.id DESC LIMIT ?1",
            )
            .map_err(|error| Error::invalid("computation query", error.to_string()))?;
        let rows = statement
            .query_map(params![limit], |row| {
                let duration_ms: i64 = row.get(9)?;
                Ok(StoredComputation {
                    id: row.get(0)?,
                    record: Computation {
                        scenario_name: row.get(1)?,
                        scenario_revision: row.get(2)?,
                        command: row.get(3)?,
                        parameters: row.get(4)?,
                        engine_version: row.get(5)?,
                        inputs_digest: row.get(6)?,
                        summary: row.get(7)?,
                    },
                    started_at: row.get(8)?,
                    duration: Duration::from_millis(u64::try_from(duration_ms).unwrap_or(0)),
                    windows: row.get::<_, i64>(10)? as usize,
                })
            })
            .map_err(|error| Error::invalid("computation query", error.to_string()))?;
        let mut stored = Vec::new();
        for row in rows {
            stored.push(row.map_err(|error| Error::invalid("computation row", error.to_string()))?);
        }
        Ok(stored)
    }

    /// Every window a computation stored, in time order.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] when the rows cannot be read.
    pub fn windows(&self, computation: i64) -> Result<Vec<AccessWindowRecord>> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT satellite, station, start_utc, end_utc, start_seconds, duration_seconds,
                        minimum_elevation_deg
                 FROM access_window WHERE computation_id = ?1 ORDER BY start_seconds",
            )
            .map_err(|error| Error::invalid("access window query", error.to_string()))?;
        let rows = statement
            .query_map(params![computation], |row| {
                Ok(AccessWindowRecord {
                    satellite: row.get(0)?,
                    station: row.get(1)?,
                    start_utc: row.get(2)?,
                    end_utc: row.get(3)?,
                    start_seconds: row.get(4)?,
                    duration_seconds: row.get(5)?,
                    minimum_elevation_deg: row.get(6)?,
                })
            })
            .map_err(|error| Error::invalid("access window query", error.to_string()))?;
        let mut windows = Vec::new();
        for row in rows {
            windows
                .push(row.map_err(|error| Error::invalid("access window row", error.to_string()))?);
        }
        Ok(windows)
    }

    /// Save a scenario revision. Revisions are append-only and numbered from one.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] when the rows cannot be written.
    pub fn save_revision(
        &mut self,
        scenario_id: i64,
        author: &str,
        document: &str,
    ) -> Result<Revision> {
        let next: i64 = self
            .connection
            .query_row(
                "SELECT COALESCE(MAX(revision), 0) + 1 FROM scenario_revision WHERE scenario_id = ?1",
                params![scenario_id],
                |row| row.get(0),
            )
            .map_err(|error| Error::invalid("scenario revision", error.to_string()))?;
        let revision = Revision {
            revision: next,
            recorded_at: now_iso8601(),
            author: author.to_owned(),
            digest: crate::digest(document.as_bytes()),
            document: document.to_owned(),
        };
        self.connection
            .execute(
                "INSERT INTO scenario_revision (scenario_id, revision, recorded_at, author, digest, document)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    scenario_id,
                    revision.revision,
                    revision.recorded_at,
                    revision.author,
                    revision.digest,
                    revision.document,
                ],
            )
            .map_err(|error| Error::invalid("scenario revision", error.to_string()))?;
        Ok(revision)
    }

    /// Every revision of a scenario, oldest first.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] when the rows cannot be read.
    pub fn revisions(&self, scenario_id: i64) -> Result<Vec<Revision>> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT revision, recorded_at, author, digest, document
                 FROM scenario_revision WHERE scenario_id = ?1 ORDER BY revision",
            )
            .map_err(|error| Error::invalid("scenario revision query", error.to_string()))?;
        let rows = statement
            .query_map(params![scenario_id], |row| {
                Ok(Revision {
                    revision: row.get(0)?,
                    recorded_at: row.get(1)?,
                    author: row.get(2)?,
                    digest: row.get(3)?,
                    document: row.get(4)?,
                })
            })
            .map_err(|error| Error::invalid("scenario revision query", error.to_string()))?;
        let mut revisions = Vec::new();
        for row in rows {
            revisions
                .push(row.map_err(|error| Error::invalid("scenario revision", error.to_string()))?);
        }
        Ok(revisions)
    }

    /// Remember a knowledge document, replacing the row of the same path.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] when the row cannot be written.
    pub fn upsert_document(&mut self, document: &DocumentRecord) -> Result<i64> {
        // One path is one document: re-ingesting a file updates its digest and counts rather than
        // accumulating a row per run.
        self.connection
            .execute(
                "INSERT INTO knowledge_document (path, digest, bytes, passages, source_url, added_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(path) DO UPDATE SET
                    digest = excluded.digest,
                    bytes = excluded.bytes,
                    passages = excluded.passages,
                    source_url = excluded.source_url",
                params![
                    document.path,
                    document.digest,
                    i64::try_from(document.bytes).unwrap_or(i64::MAX),
                    i64::try_from(document.passages).unwrap_or(i64::MAX),
                    document.source_url,
                    now_iso8601(),
                ],
            )
            .map_err(|error| Error::invalid("knowledge document", error.to_string()))?;
        let id: i64 = self
            .connection
            .query_row(
                "SELECT id FROM knowledge_document WHERE path = ?1",
                params![document.path],
                |row| row.get(0),
            )
            .map_err(|error| Error::invalid("knowledge document", error.to_string()))?;
        Ok(id)
    }

    /// Record that an answer cited a document.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] when the citation cannot be written.
    pub fn record_citation(
        &mut self,
        document: i64,
        computation: Option<i64>,
        locator: &str,
        text: &str,
    ) -> Result<i64> {
        self.connection
            .execute(
                "INSERT INTO citation (document_id, computation_id, locator, text, recorded_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![document, computation, locator, text, now_iso8601()],
            )
            .map_err(|error| Error::invalid("citation row", error.to_string()))?;
        Ok(self.connection.last_insert_rowid())
    }

    /// How many documents the record holds.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] when the count cannot be read.
    pub fn documents(&self) -> Result<Vec<DocumentRecord>> {
        let mut statement = self
            .connection
            .prepare("SELECT path, digest, bytes, passages, source_url FROM knowledge_document ORDER BY path")
            .map_err(|error| Error::invalid("knowledge document query", error.to_string()))?;
        let rows = statement
            .query_map([], |row| {
                Ok(DocumentRecord {
                    path: row.get(0)?,
                    digest: row.get(1)?,
                    bytes: u64::try_from(row.get::<_, i64>(2)?).unwrap_or(0),
                    passages: u64::try_from(row.get::<_, i64>(3)?).unwrap_or(0),
                    source_url: row.get(4)?,
                })
            })
            .map_err(|error| Error::invalid("knowledge document query", error.to_string()))?;
        let mut documents = Vec::new();
        for row in rows {
            documents.push(
                row.map_err(|error| Error::invalid("knowledge document", error.to_string()))?,
            );
        }
        Ok(documents)
    }

    /// Whether a path is already recorded with the given digest, which is what makes an ingest
    /// incremental instead of a re-read of every file.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] when the row cannot be read.
    pub fn document_digest(&self, path: &str) -> Result<Option<String>> {
        self.connection
            .query_row(
                "SELECT digest FROM knowledge_document WHERE path = ?1",
                params![path],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| Error::invalid("knowledge document query", error.to_string()))
    }
}

/// Now, as ISO-8601 in UTC, to the second.
///
/// Written here rather than through `chrono`: a record's timestamp is a label on a row, and this
/// crate keeps its dependency list to the SQLite driver and the hash.
#[must_use]
pub fn now_iso8601() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    iso8601_from_unix(seconds)
}

/// A Unix timestamp as ISO-8601 UTC, by the civil-from-days algorithm.
///
/// Howard Hinnant's `civil_from_days` (public domain, `chrono`-compatible results): days from the
/// epoch, then the Gregorian date, so a stored timestamp sorts as text and reads as a date.
#[must_use]
pub fn iso8601_from_unix(seconds: u64) -> String {
    let days = i64::try_from(seconds / 86_400).unwrap_or(0);
    let of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        of_day / 3_600,
        (of_day % 3_600) / 60,
        of_day % 60
    )
}

/// Days since 1970-01-01 to a Gregorian date, after Hinnant's `civil_from_days`.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_piece = (5 * day_of_year + 2) / 153;
    let day = u32::try_from(day_of_year - (153 * month_piece + 2) / 5 + 1).unwrap_or(1);
    let month = u32::try_from(if month_piece < 10 {
        month_piece + 3
    } else {
        month_piece - 9
    })
    .unwrap_or(1);
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn store() -> Store {
        Store::open_in_memory().expect("an in-memory record")
    }

    fn computation(command: &str) -> Computation {
        Computation {
            scenario_name: "Demo".to_owned(),
            scenario_revision: None,
            command: command.to_owned(),
            parameters: "{}".to_owned(),
            engine_version: "0.1.33".to_owned(),
            inputs_digest: crate::digest(b"demo"),
            summary: "done".to_owned(),
        }
    }

    #[test]
    fn opening_a_record_creates_the_schema_and_reports_its_version() {
        let store = store();
        assert_eq!(store.schema_version().expect("a version"), SCHEMA_VERSION);
        // Opening the same file twice must not try to create the tables again.
        let directory = std::env::temp_dir().join("stkcraft-storage-open");
        let _ = std::fs::remove_dir_all(&directory);
        let path = directory.join("stkcraft.db");
        drop(Store::open(&path).expect("created"));
        let reopened = Store::open(&path).expect("reopened");
        assert_eq!(
            reopened.schema_version().expect("a version"),
            SCHEMA_VERSION
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_computation_is_stored_with_its_provenance_and_read_back() {
        let mut store = store();
        let id = store
            .record_computation(&computation("access.compute"))
            .expect("recorded");
        store
            .finish_computation(id, Duration::from_millis(1234))
            .expect("finished");
        let stored = store.computations(10).expect("read");
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].id, id);
        assert_eq!(stored[0].record.command, "access.compute");
        assert_eq!(stored[0].record.inputs_digest, crate::digest(b"demo"));
        assert_eq!(stored[0].duration, Duration::from_millis(1234));
        assert!(
            stored[0].started_at.contains('T'),
            "{}",
            stored[0].started_at
        );
        assert_eq!(stored[0].windows, 0);
    }

    #[test]
    fn the_newest_computation_is_first_and_the_limit_is_respected() {
        let mut store = store();
        for index in 0..5 {
            store
                .record_computation(&Computation {
                    summary: format!("run {index}"),
                    ..computation("access.compute")
                })
                .expect("recorded");
        }
        let stored = store.computations(2).expect("read");
        assert_eq!(stored.len(), 2);
        assert_eq!(stored[0].record.summary, "run 4");
        assert_eq!(stored[1].record.summary, "run 3");
    }

    #[test]
    fn a_window_belongs_to_the_run_that_found_it() {
        let mut store = store();
        let id = store
            .record_computation(&computation("access.compute"))
            .expect("recorded");
        store
            .record_window(
                id,
                &AccessWindowRecord {
                    satellite: "ISS".to_owned(),
                    station: "London".to_owned(),
                    start_utc: "2024-01-01T09:12:04Z".to_owned(),
                    end_utc: "2024-01-01T09:18:16Z".to_owned(),
                    start_seconds: 372.0,
                    duration_seconds: 372.0,
                    minimum_elevation_deg: 5.0,
                },
            )
            .expect("recorded");
        let windows = store.windows(id).expect("read");
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].satellite, "ISS");
        assert_eq!(windows[0].start_seconds, 372.0);
        let listed = store.computations(1).expect("read");
        assert_eq!(listed[0].windows, 1);
    }

    #[test]
    fn a_window_for_a_computation_that_does_not_exist_is_refused() {
        // The foreign key is the point: a window that belongs to nothing is a number with no run
        // behind it, which is exactly what this store exists to prevent.
        let mut store = store();
        let error = store
            .record_window(
                999,
                &AccessWindowRecord {
                    satellite: "ISS".to_owned(),
                    station: "London".to_owned(),
                    start_utc: "2024-01-01T09:12:04Z".to_owned(),
                    end_utc: "2024-01-01T09:18:16Z".to_owned(),
                    start_seconds: 0.0,
                    duration_seconds: 372.0,
                    minimum_elevation_deg: 5.0,
                },
            )
            .expect_err("an orphan window");
        assert!(error.to_string().contains("FOREIGN KEY"), "{error}");
    }

    #[test]
    fn revisions_append_and_are_never_edited() {
        let mut store = store();
        store
            .connection
            .execute(
                "INSERT INTO scenario (id, name, central_body, epoch, duration_seconds, created_at)
                 VALUES (1, 'Demo', 'Earth', '2024-01-01T00:00:00Z', 86400.0, '2024-01-01T00:00:00Z')",
                [],
            )
            .expect("a scenario");
        let first = store
            .save_revision(1, "person", "{\"name\":\"Demo\"}")
            .expect("saved");
        let second = store
            .save_revision(1, "person", "{\"name\":\"Demo\",\"duration\":3600}")
            .expect("saved");
        assert_eq!(first.revision, 1);
        assert_eq!(second.revision, 2);
        assert_ne!(first.digest, second.digest);
        let revisions = store.revisions(1).expect("read");
        assert_eq!(revisions.len(), 2);
        // The first document is untouched by the second save: undo is a pointer, not a rewrite.
        assert_eq!(revisions[0].document, "{\"name\":\"Demo\"}");
        assert_eq!(revisions[0].digest, crate::digest(b"{\"name\":\"Demo\"}"));
    }

    #[test]
    fn ingesting_the_same_path_twice_updates_the_row() {
        let mut store = store();
        let first = store
            .upsert_document(&DocumentRecord {
                path: "spec.md".to_owned(),
                digest: crate::digest(b"v1"),
                bytes: 2,
                passages: 1,
                source_url: None,
            })
            .expect("recorded");
        let second = store
            .upsert_document(&DocumentRecord {
                path: "spec.md".to_owned(),
                digest: crate::digest(b"v2"),
                bytes: 2,
                passages: 2,
                source_url: Some("https://example.org/spec".to_owned()),
            })
            .expect("recorded");
        assert_eq!(first, second, "a re-ingest grew a second row");
        let documents = store.documents().expect("read");
        assert_eq!(documents.len(), 1);
        assert_eq!(documents[0].digest, crate::digest(b"v2"));
        assert_eq!(
            store.document_digest("spec.md").expect("read"),
            Some(crate::digest(b"v2"))
        );
        assert_eq!(store.document_digest("absent.md").expect("read"), None);
    }

    #[test]
    fn a_citation_points_at_a_document_and_a_run() {
        let mut store = store();
        let id = store
            .record_computation(&computation("ask"))
            .expect("recorded");
        let document = store
            .upsert_document(&DocumentRecord {
                path: "spec.md".to_owned(),
                digest: crate::digest(b"spec"),
                bytes: 4,
                passages: 1,
                source_url: None,
            })
            .expect("recorded");
        let citation = store
            .record_citation(document, Some(id), "lines 1-5", "Maximum angle 45 degrees")
            .expect("recorded");
        assert!(citation > 0);
    }

    #[test]
    fn timestamps_are_iso8601_and_sort_as_text() {
        assert_eq!(iso8601_from_unix(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso8601_from_unix(1_704_067_200), "2024-01-01T00:00:00Z");
        assert_eq!(iso8601_from_unix(1_704_412_800), "2024-01-05T00:00:00Z");
        // A leap day, because the calendar arithmetic is the part that goes wrong.
        assert_eq!(iso8601_from_unix(1_709_164_800), "2024-02-29T00:00:00Z");
        let early = iso8601_from_unix(1_000_000_000);
        let late = iso8601_from_unix(1_700_000_000);
        assert!(early < late, "ISO timestamps must sort chronologically");
    }
}
