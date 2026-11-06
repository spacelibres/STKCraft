//! The key-value cache: values that are derived, disposable and safe to lose.
//!
//! A cache entry is written for something the program can recompute. That property is what allows the
//! format to be simple: one file per key, a header that says how long the value is and what its
//! checksum is, and the value itself. There is no index to corrupt, no compaction, and deleting the
//! directory loses nothing but time.
//!
//! ```text
//! cache/
//! └── 3f2a…9c            one file per key; the file name is the key's SHA-256
//! ```
//!
//! # Hostile input
//!
//! A key comes from a caller — a library path, a locale, a document root — so it is never used as a
//! file name: the name is the **hash** of the key, and the key itself is stored in the header and
//! compared on read. A key containing `../` is therefore not a path traversal but a hash, and two
//! different keys cannot collide into one entry without colliding in SHA-256.
//!
//! # Corruption
//!
//! Every value carries a CRC-32. A truncated or bit-flipped file is a **miss**, not an error and not a
//! panic: the caller recomputes the value, which is the whole premise of a cache.

use std::path::{Path, PathBuf};

/// The magic number at the start of every cache file, so a foreign file is a miss rather than a
/// confusing decode.
const MAGIC: &[u8; 8] = b"STKCACH1";

/// The header: magic, checksum, payload length, key length.
const HEADER: usize = 8 + 4 + 8 + 4;

/// The key-value cache.
#[derive(Debug, Clone)]
pub struct Cache {
    directory: PathBuf,
}

impl Cache {
    /// A cache rooted at a directory, created on first use.
    #[must_use]
    pub fn at(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }

    /// The directory entries are written to.
    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Store a value under a key, replacing any previous value.
    ///
    /// # Errors
    ///
    /// A message when the directory cannot be created or the file cannot be written. A caller that
    /// ignores it loses a cache write, not an answer.
    pub fn put(&self, key: &str, value: &[u8]) -> Result<(), String> {
        std::fs::create_dir_all(&self.directory)
            .map_err(|error| format!("could not create {}: {error}", self.directory.display()))?;
        let path = self.path_for(key);
        let key_bytes = key.as_bytes();
        let key_length = u32::try_from(key_bytes.len())
            .map_err(|_| "the cache key is longer than 4 GiB".to_owned())?;
        let payload_length = u64::try_from(value.len())
            .map_err(|_| "the cache value is longer than 8 EiB".to_owned())?;
        let mut file = Vec::with_capacity(HEADER + key_bytes.len() + value.len());
        file.extend_from_slice(MAGIC);
        file.extend_from_slice(&crc32(value).to_le_bytes());
        file.extend_from_slice(&payload_length.to_le_bytes());
        file.extend_from_slice(&key_length.to_le_bytes());
        file.extend_from_slice(key_bytes);
        file.extend_from_slice(value);
        // Write beside the target and rename, so a crash mid-write leaves the old entry rather than a
        // half-written one that reads as a corrupt miss.
        let temporary = path.with_extension("tmp");
        std::fs::write(&temporary, &file)
            .map_err(|error| format!("could not write {}: {error}", temporary.display()))?;
        std::fs::rename(&temporary, &path)
            .map_err(|error| format!("could not rename into {}: {error}", path.display()))
    }

    /// Read a value back, or `None` when it is absent or unusable.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<Vec<u8>> {
        let bytes = std::fs::read(self.path_for(key)).ok()?;
        let header = bytes.get(..HEADER)?;
        if header.get(..8)? != MAGIC {
            return None;
        }
        let checksum = u32::from_le_bytes(header.get(8..12)?.try_into().ok()?);
        let payload_length = u64::from_le_bytes(header.get(12..20)?.try_into().ok()?);
        let key_length =
            usize::try_from(u32::from_le_bytes(header.get(20..24)?.try_into().ok()?)).ok()?;
        let stored_key = bytes.get(HEADER..HEADER + key_length)?;
        if stored_key != key.as_bytes() {
            return None;
        }
        let payload = bytes.get(HEADER + key_length..)?;
        if u64::try_from(payload.len()).ok()? != payload_length {
            return None;
        }
        if crc32(payload) != checksum {
            return None;
        }
        Some(payload.to_vec())
    }

    /// Whether a key has a usable value.
    #[must_use]
    pub fn contains(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// Forget a key.
    pub fn remove(&self, key: &str) {
        let _ = std::fs::remove_file(self.path_for(key));
    }

    /// Forget everything.
    ///
    /// # Errors
    ///
    /// A message when the directory exists and cannot be removed.
    pub fn clear(&self) -> Result<(), String> {
        match std::fs::remove_dir_all(&self.directory) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!(
                "could not clear {}: {error}",
                self.directory.display()
            )),
        }
    }

    /// How many entries the cache holds.
    #[must_use]
    pub fn len(&self) -> usize {
        let Ok(entries) = std::fs::read_dir(&self.directory) else {
            return 0;
        };
        entries
            .flatten()
            // A `.tmp` file is a write in flight, not an entry.
            .filter(|entry| {
                entry
                    .path()
                    .extension()
                    .is_none_or(|extension| extension != "tmp")
            })
            .count()
    }

    /// Whether the cache holds nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Where a key's file lives: the hash of the key, so the key can never be a path.
    fn path_for(&self, key: &str) -> PathBuf {
        self.directory.join(crate::digest(key.as_bytes()))
    }
}

/// CRC-32 (IEEE 802.3), the checksum a cache file carries.
///
/// Table-free on purpose: a cache reads its values once and a table would be 1 KiB of static data for
/// a few microseconds.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff_u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, clippy::panic, clippy::expect_used)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&directory);
        directory
    }

    #[test]
    fn a_value_round_trips_under_its_key() {
        let directory = scratch("stkcraft-cache-roundtrip");
        let cache = Cache::at(&directory);
        cache.put("knowledge/index", b"postings").expect("written");
        assert_eq!(
            cache.get("knowledge/index").as_deref(),
            Some(&b"postings"[..])
        );
        assert!(cache.contains("knowledge/index"));
        assert_eq!(cache.get("knowledge/other"), None);
        assert_eq!(cache.len(), 1);
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_key_is_never_a_path() {
        // Hostile input: a key that would escape the cache directory if it were used as a file name.
        let directory = scratch("stkcraft-cache-traversal");
        let cache = Cache::at(&directory);
        let key = "../../etc/passwd";
        cache.put(key, b"value").expect("written");
        assert_eq!(cache.get(key).as_deref(), Some(&b"value"[..]));
        // Everything the cache wrote is inside its own directory, with a hash for a name.
        let entries: Vec<String> = std::fs::read_dir(&directory)
            .expect("a directory")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(entries.len(), 1, "{entries:?}");
        assert_eq!(
            entries[0].len(),
            64,
            "the file name is not a SHA-256: {entries:?}"
        );
        assert!(!directory.join("../passwd").exists());
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_corrupt_value_is_a_miss_and_not_an_error() {
        // The premise of a cache: a damaged value is recomputed, not reported.
        let directory = scratch("stkcraft-cache-corrupt");
        let cache = Cache::at(&directory);
        cache.put("kernel/index", b"0123456789").expect("written");
        let path = cache.path_for("kernel/index");
        let mut bytes = std::fs::read(&path).expect("read");
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        std::fs::write(&path, &bytes).expect("write");
        assert_eq!(
            cache.get("kernel/index"),
            None,
            "a flipped bit was accepted"
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_truncated_value_is_a_miss() {
        let directory = scratch("stkcraft-cache-truncated");
        let cache = Cache::at(&directory);
        cache.put("a", b"0123456789").expect("written");
        let path = cache.path_for("a");
        let bytes = std::fs::read(&path).expect("read");
        std::fs::write(&path, &bytes[..bytes.len() - 2]).expect("write");
        assert_eq!(cache.get("a"), None);
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_foreign_file_is_a_miss() {
        let directory = scratch("stkcraft-cache-foreign");
        let cache = Cache::at(&directory);
        std::fs::create_dir_all(&directory).expect("a directory");
        std::fs::write(cache.path_for("x"), b"not a cache file").expect("written");
        assert_eq!(cache.get("x"), None);
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn removing_and_clearing_work_and_clearing_twice_is_fine() {
        let directory = scratch("stkcraft-cache-clear");
        let cache = Cache::at(&directory);
        cache.put("a", b"1").expect("written");
        cache.put("b", b"2").expect("written");
        cache.remove("a");
        assert_eq!(cache.get("a"), None);
        assert!(cache.contains("b"));
        cache.clear().expect("cleared");
        assert!(cache.is_empty());
        cache
            .clear()
            .expect("clearing an absent cache is not an error");
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn the_crc_matches_the_published_check_value() {
        // CRC-32/ISO-HDLC of "123456789" is 0xCBF43926: the standard check value, so a wrong table or
        // a wrong polynomial cannot pass.
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn overwriting_a_key_replaces_its_value() {
        let directory = scratch("stkcraft-cache-overwrite");
        let cache = Cache::at(&directory);
        cache.put("index", b"old").expect("written");
        cache.put("index", b"new").expect("written");
        assert_eq!(cache.get("index").as_deref(), Some(&b"new"[..]));
        assert_eq!(cache.len(), 1);
        let _ = std::fs::remove_dir_all(&directory);
    }
}
