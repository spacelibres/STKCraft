//! Persistence: the record, the cache and the vector store.
//!
//! Every stored thing belongs to one of three stores, and which one is decided by **the question a
//! reader asks of the data** rather than by convenience (`docs/design/storage.md`):
//!
//! | Question | Store | In this crate |
//! |---|---|---|
//! | "What was the scenario, and what exactly did it compute?" | relational | [`Store`] — SQLite, `STRICT`, versioned |
//! | "Give me this value again, it will not have changed" | key-value | [`Cache`] — one file per key, CRC per value |
//! | "Which vectors are near this one?" | vector | [`Vectors`] — brute-force cosine, candidates only |
//!
//! # One crate owns persistence
//!
//! No other crate opens a database, writes a key-value file or issues SQL. A crate that needs to
//! remember something asks this one. That rule is what keeps a stray JSON file from appearing beside
//! the executable, which is the failure the storage design exists to prevent.
//!
//! # What makes a row reproducible
//!
//! A record that came from outside — a document, a kernel, a scenario document — carries its
//! **SHA-256 digest**. A computation row carries the digest of the inputs it ran on. Without them a
//! row says what happened but not what it happened to, and an analysis that cannot be reproduced is
//! the thing this project refuses to ship.
//!
//! ```text
//! let mut store = Store::open(&Paths::for_user()?.record())?;
//! let id = store.record_computation(&Computation {
//!     command: "access.compute".to_owned(),
//!     parameters: "{\"step-seconds\":30}".to_owned(),
//!     inputs_digest: digest(b"the scenario document"),
//!     ..
//! })?;
//! for window in passes { store.record_window(id, &window)?; }
//! ```

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
#![forbid(unsafe_code)]

pub mod cache;
pub mod record;
pub mod schema;
pub mod vector;

pub use cache::Cache;
pub use record::{
    AccessWindowRecord, Computation, DocumentRecord, Revision, Store, StoredComputation,
};
pub use schema::SCHEMA_VERSION;
pub use vector::Vectors;

use std::path::{Path, PathBuf};

/// A SHA-256 digest, as lowercase hexadecimal.
///
/// The one digest function of the storage layer: a document, a kernel, a scenario document and a
/// computation's inputs are all identified the same way, so a reader never has to ask which hash a
/// column holds.
#[must_use]
pub fn digest(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut text = String::with_capacity(64);
    for byte in digest {
        let _ = std::fmt::Write::write_fmt(&mut text, format_args!("{byte:02x}"));
    }
    text
}

/// Where this installation keeps its data.
///
/// Configuration is what a person decided (`~/.config/stkcraft`); data is what the program produced
/// (`~/.local/share/stkcraft`). Keeping them apart means a copy of the configuration reproduces
/// someone's setup without copying a byte of derived data, and deleting the cache never loses a
/// decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    root: PathBuf,
}

impl Paths {
    /// The paths for the current user, honouring `STKCRAFT_DATA` and then the platform's data
    /// directory.
    ///
    /// # Errors
    ///
    /// [`stkcraft_framework_core::Error::NotFound`] when the platform has no data directory to use.
    pub fn for_user() -> Result<Self, stkcraft_framework_core::Error> {
        if let Some(explicit) = std::env::var_os("STKCRAFT_DATA") {
            return Ok(Self {
                root: PathBuf::from(explicit),
            });
        }
        let base = if cfg!(target_os = "windows") {
            std::env::var_os("APPDATA").map(PathBuf::from)
        } else if cfg!(target_os = "macos") {
            std::env::var_os("HOME")
                .map(|home| PathBuf::from(home).join("Library/Application Support"))
        } else {
            std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .or_else(|| {
                    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
                })
        };
        let base = base.ok_or_else(|| stkcraft_framework_core::Error::NotFound {
            what: "a platform data directory (set STKCRAFT_DATA to choose one)".to_owned(),
        })?;
        Ok(Self {
            root: base.join("stkcraft"),
        })
    }

    /// Paths rooted at an explicit directory, which is what a test and a `--data` option use.
    #[must_use]
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The directory everything lives under.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The record: one SQLite file.
    #[must_use]
    pub fn record(&self) -> PathBuf {
        self.root.join("stkcraft.db")
    }

    /// The derived cache.
    #[must_use]
    pub fn cache(&self) -> PathBuf {
        self.root.join("cache")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_digest_is_sha256_in_hex() {
        // The published SHA-256 of "abc", so a wrong hash function cannot pass quietly.
        assert_eq!(
            digest(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(digest(b"abc").len(), 64);
        assert_ne!(digest(b"abc"), digest(b"abd"));
    }

    #[test]
    fn the_paths_separate_the_record_from_the_cache() {
        let paths = Paths::at("/tmp/stkcraft-data");
        assert_eq!(
            paths.record(),
            PathBuf::from("/tmp/stkcraft-data/stkcraft.db")
        );
        assert_eq!(paths.cache(), PathBuf::from("/tmp/stkcraft-data/cache"));
        assert_eq!(paths.root(), Path::new("/tmp/stkcraft-data"));
    }
}
