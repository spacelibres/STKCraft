//! The index cache: a library re-opened without re-reading what has not changed.
//!
//! Tokenizing a document is cheap. Tokenizing ten thousand papers is not, and a paper corpus is what
//! this library exists to hold — so the derived index goes in the key-value store, keyed by what the
//! file *was* when it was read: its path, its size and its modification time.
//!
//! # Why a key and not a content digest
//!
//! A content digest means reading the file to decide whether to read the file, which is the cost being
//! avoided. Size and modification time are the same pair every build tool uses for the same reason.
//! The risk — a file changed without its timestamp moving — is real and accepted: the cache is derived
//! data, `stkcraft knowledge --refresh` rebuilds it, and an answer never depends on the cache being
//! right, only on it being fast.
//!
//! # What a stored document holds
//!
//! The passages, with their line ranges and their text: everything that came out of the parse. The
//! format is written and read here, with a bounds-checked reader, because a cache file is input like
//! any other and a truncated one must be a miss rather than a panic.
//!
//! Tokens are not stored: the index interns them into a dictionary at build time, which is a few
//! milliseconds over a cached corpus and would be hundreds of megabytes in the cache otherwise.
//!
//! # Bounding the cache
//!
//! A manifest pins which document entries the current library uses, so re-indexing removes the entries
//! of files that were deleted or renamed instead of accumulating a copy of every revision a file has
//! ever had.

use stkcraft_framework_storage::Cache;

use crate::chunk::Chunk;
use crate::index::{Reader, write_string};

/// The magic at the start of a stored library.
///
/// The version moved to `02` when a passage gained its page: a cache written by the release before
/// PDFs has no page numbers in it, and a magic that does not match is a miss rather than an error,
/// because a miss costs one re-parse and a mis-read costs a wrong citation.
const LIBRARY_MAGIC: &[u8; 8] = b"STKLIB02";

/// What one open cost and what it saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Reuse {
    /// Documents read and parsed because they were new or changed.
    pub parsed: usize,
    /// Documents taken from the cache.
    pub reused: usize,
}

impl Reuse {
    /// How many documents the library holds.
    #[must_use]
    pub fn total(&self) -> usize {
        self.parsed + self.reused
    }
}

/// The cache key of an assembled library, from the manifest of what it was built from.
///
/// The manifest is one line per file — path, size, modification time — so the key changes when any
/// file does. A digest of the *contents* would mean reading every file to decide whether to read
/// every file, which is the cost the cache exists to avoid; `stkcraft knowledge` rebuilds when the
/// metadata lies.
#[must_use]
pub fn library_key(manifest: &str) -> String {
    format!(
        "knowledge/library/{}",
        stkcraft_framework_storage::digest(manifest.as_bytes())
    )
}

/// Read a whole library from the cache, or `None` when it is absent or unusable.
#[must_use]
pub(crate) fn load_library(
    cache: &Cache,
    key: &str,
) -> Option<(Vec<crate::library::Document>, crate::index::Index)> {
    let bytes = cache.get(key)?;
    decode_library(&bytes).ok()
}

/// Store a whole library: its documents and its index, so a re-open is one read.
pub(crate) fn store_library(
    cache: &Cache,
    key: &str,
    documents: &[crate::library::Document],
    index: &crate::index::Index,
) {
    let mut bytes = encode_documents(documents);
    bytes.extend_from_slice(&index.encode());
    // A cache write that fails costs a rebuild next time and nothing else.
    let _ = cache.put(key, &bytes);
}

/// The key that names every assembled library the cache holds, so an edited corpus does not leave a
/// copy of every revision behind.
const LIBRARIES: &str = "knowledge/libraries";

/// Forget every assembled library except `keep`, and remember it.
///
/// A library is re-keyed whenever a file changes, so without this an actively edited corpus would
/// accumulate one entry — the whole corpus, every time — per save. Returns how many entries were
/// removed.
pub(crate) fn prune_libraries(cache: &Cache, keep: &str) -> usize {
    let previous: Vec<String> = cache
        .get(LIBRARIES)
        .map(|bytes| {
            String::from_utf8(bytes)
                .unwrap_or_default()
                .lines()
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let mut removed = 0;
    for key in &previous {
        if key != keep {
            cache.remove(key);
            removed += 1;
        }
    }
    let _ = cache.put(LIBRARIES, keep.as_bytes());
    removed
}

/// Encode the documents of a library.
fn encode_documents(documents: &[crate::library::Document]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(LIBRARY_MAGIC);
    out.extend_from_slice(&u32::try_from(documents.len()).unwrap_or(0).to_le_bytes());
    for document in documents {
        write_string(&mut out, &document.path);
        out.extend_from_slice(
            &u32::try_from(document.bytes)
                .unwrap_or(u32::MAX)
                .to_le_bytes(),
        );
        // The page count, then a flag for a document a bound cut short. A text file writes `0` pages.
        out.extend_from_slice(
            &u32::try_from(document.pages.unwrap_or(0))
                .unwrap_or(u32::MAX)
                .to_le_bytes(),
        );
        out.push(u8::from(document.limited));
        out.extend_from_slice(
            &u32::try_from(document.chunks.len())
                .unwrap_or(0)
                .to_le_bytes(),
        );
        for chunk in &document.chunks {
            out.extend_from_slice(&u32::try_from(chunk.index).unwrap_or(0).to_le_bytes());
            // The page, or `0` for a passage that has none.
            out.extend_from_slice(
                &u32::try_from(chunk.page.unwrap_or(0))
                    .unwrap_or(0)
                    .to_le_bytes(),
            );
            out.extend_from_slice(&u32::try_from(chunk.line_start).unwrap_or(0).to_le_bytes());
            out.extend_from_slice(&u32::try_from(chunk.line_end).unwrap_or(0).to_le_bytes());
            write_string(&mut out, &chunk.text);
        }
    }
    out
}

/// Decode a stored library: the documents, then the index.
fn decode_library(
    bytes: &[u8],
) -> Result<(Vec<crate::library::Document>, crate::index::Index), String> {
    let mut reader = Reader::new(bytes);
    if reader.take(8)? != LIBRARY_MAGIC {
        return Err("not a stored library of this version".to_owned());
    }
    let document_count =
        usize::try_from(reader.take_u32()?).map_err(|_| "count overflow".to_owned())?;
    if document_count > bytes.len() {
        return Err("the document count is larger than the file".to_owned());
    }
    let mut documents = Vec::with_capacity(document_count);
    for _ in 0..document_count {
        let path = reader.take_string()?;
        let file_bytes = reader.take_u32()? as usize;
        let page_count = reader.take_u32()? as usize;
        let limited = reader.take_byte()? != 0;
        let chunk_count =
            usize::try_from(reader.take_u32()?).map_err(|_| "count overflow".to_owned())?;
        if chunk_count > bytes.len() {
            return Err("the passage count is larger than the file".to_owned());
        }
        let mut chunks = Vec::with_capacity(chunk_count);
        for _ in 0..chunk_count {
            let index =
                usize::try_from(reader.take_u32()?).map_err(|_| "index overflow".to_owned())?;
            let page = reader.take_u32()?;
            let page = (page != 0).then_some(page as usize);
            let line_start =
                usize::try_from(reader.take_u32()?).map_err(|_| "line overflow".to_owned())?;
            let line_end =
                usize::try_from(reader.take_u32()?).map_err(|_| "line overflow".to_owned())?;
            let text = reader.take_string()?;
            chunks.push(Chunk {
                index,
                page,
                line_start,
                line_end,
                text,
            });
        }
        documents.push(crate::library::Document {
            path,
            bytes: file_bytes,
            pages: (page_count != 0).then_some(page_count),
            limited,
            chunks,
        });
    }
    let index = crate::index::Index::decode(reader.rest())?;
    Ok((documents, index))
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, clippy::expect_used)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let directory = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&directory);
        directory
    }

    fn documents() -> Vec<crate::library::Document> {
        vec![crate::library::Document {
            path: "spec.md".to_owned(),
            bytes: 42,
            pages: None,
            limited: false,
            chunks: crate::chunk::chunk(
                "Sensor\n\nMaximum off-nadir angle 45 degrees.\nSwath 1200 km.\n",
            ),
        }]
    }

    #[test]
    fn a_paged_document_keeps_its_pages_and_its_bound_through_the_cache() {
        // A cached library is read by the next run instead of the files, so anything the cache drops
        // is a citation that changes shape between runs: the page number and the "read in part" flag
        // have to survive the round trip.
        let documents = vec![crate::library::Document {
            path: "paper.pdf".to_owned(),
            bytes: 1_024,
            pages: Some(12),
            limited: true,
            chunks: crate::chunk::chunk_pages(&[
                (1, "Maximum off-nadir angle is 45 degrees.\n".to_owned()),
                (2, "The swath is 1200 km at that angle.\n".to_owned()),
            ]),
        }];
        let index = crate::index::Index::build(documents.iter().enumerate().flat_map(
            |(document, entry)| {
                entry
                    .chunks
                    .iter()
                    .enumerate()
                    .map(move |(chunk, passage)| (document, chunk, passage.text.as_str()))
            },
        ));
        let directory = scratch("stkcraft-knowledge-library-pages");
        let cache = Cache::at(&directory);
        let key = library_key("paper.pdf\t1024\t1");
        store_library(&cache, &key, &documents, &index);
        let (restored, _) = load_library(&cache, &key).expect("a stored library");
        assert_eq!(restored, documents);
        assert_eq!(restored[0].pages, Some(12));
        assert!(restored[0].limited);
        assert_eq!(restored[0].chunks[1].page, Some(2));
        assert_eq!(restored[0].chunks[1].lines_label(), "page 2, lines 1-1");
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_library_round_trips_through_the_cache() {
        let documents = documents();
        let index = crate::index::Index::build(documents.iter().enumerate().flat_map(
            |(document, entry)| {
                entry
                    .chunks
                    .iter()
                    .enumerate()
                    .map(move |(chunk, passage)| (document, chunk, passage.text.as_str()))
            },
        ));
        let directory = scratch("stkcraft-knowledge-library");
        let cache = Cache::at(&directory);
        let key = library_key("spec.md\t42\t1");
        store_library(&cache, &key, &documents, &index);
        assert_eq!(prune_libraries(&cache, &key), 0);
        let (restored_documents, restored_index) =
            load_library(&cache, &key).expect("a stored library");
        assert_eq!(restored_documents, documents);
        assert_eq!(restored_index.passages(), index.passages());
        assert_eq!(
            restored_index.score("off-nadir angle"),
            index.score("off-nadir angle")
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_key_changes_when_the_manifest_does() {
        // The property the cache rests on: a different corpus is a different key, so a changed
        // library is rebuilt rather than served stale.
        let a = library_key("spec.md\t42\t1");
        assert_eq!(a, library_key("spec.md\t42\t1"));
        assert_ne!(a, library_key("spec.md\t43\t1"));
        assert_ne!(a, library_key("spec.md\t42\t2"));
        assert_ne!(a, library_key("spec.md\t42\t1\nother.md\t7\t3"));
        // The tail of the key is a hash, not a path.
        let tail = a.rsplit('/').next().unwrap_or_default();
        assert_eq!(tail.len(), 64, "{tail}");
    }

    #[test]
    fn an_edited_corpus_does_not_leave_a_copy_of_every_revision() {
        let directory = scratch("stkcraft-knowledge-library-prune");
        let cache = Cache::at(&directory);
        let documents = documents();
        let index = crate::index::Index::build(documents.iter().enumerate().flat_map(
            |(document, entry)| {
                entry
                    .chunks
                    .iter()
                    .enumerate()
                    .map(move |(chunk, passage)| (document, chunk, passage.text.as_str()))
            },
        ));
        let first = library_key("a\t1\t1");
        store_library(&cache, &first, &documents, &index);
        prune_libraries(&cache, &first);
        let second = library_key("a\t2\t2");
        store_library(&cache, &second, &documents, &index);
        assert_eq!(
            prune_libraries(&cache, &second),
            1,
            "the old library was kept"
        );
        assert!(
            load_library(&cache, &first).is_none(),
            "a stale library survived"
        );
        assert!(load_library(&cache, &second).is_some());
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_truncated_library_is_refused() {
        let documents = documents();
        let index = crate::index::Index::build(documents.iter().enumerate().flat_map(
            |(document, entry)| {
                entry
                    .chunks
                    .iter()
                    .enumerate()
                    .map(move |(chunk, passage)| (document, chunk, passage.text.as_str()))
            },
        ));
        let directory = scratch("stkcraft-knowledge-library-truncated");
        let cache = Cache::at(&directory);
        let key = library_key("a");
        store_library(&cache, &key, &documents, &index);
        let bytes = cache.get(&key).expect("stored");
        cache.put(&key, &bytes[..bytes.len() / 2]).expect("written");
        assert!(
            load_library(&cache, &key).is_none(),
            "a half library was accepted"
        );
        let _ = std::fs::remove_dir_all(&directory);
    }
}
