//! A directory of the user's documents, and the search over it.
//!
//! This is the whole point of the crate: a person drops the documents they trust into one folder —
//! mission requirements, a sensor data sheet, a thermal memo, a colleagues' email saved as text —
//! and every answer the platform gives can be checked against them.
//!
//! # Ranking
//!
//! BM25 over the chunk tokens. It rewards a passage that holds the query's rare terms and discounts
//! one that repeats a common word, which is what makes "maximum off-nadir angle" find the
//! specification row rather than every page that says "angle". The parameters are the classic ones
//! (`k1 = 1.2`, `b = 0.75`); there is nothing to tune for a library this size and a wrong tuning
//! would only hide a ranking bug.
//!
//! # Honesty
//!
//! [`Search::missed`] is set when the best hit scores below [`Library::threshold`], or when there is
//! nothing to search. The agent's prompt turns that into a sentence — "the knowledge base does not
//! cover this" — instead of letting a model fill the gap from memory. A search that always returns
//! something is a search that always lies about its evidence.

use std::path::{Path, PathBuf};

use crate::chunk::{Chunk, chunk};

/// The score below which a hit is not evidence.
///
/// A query that matches only a common word ("the") scores a fraction of one that matches two rare
/// terms; below this a model is better off told the library has nothing.
pub const DEFAULT_THRESHOLD: f64 = 0.6;

/// The extensions the library reads.
///
/// Text only, and deliberately so: a PDF needs a parser and a page model, and a citation into a PDF
/// is a page and a paragraph rather than a line. Extract the text beside the file and it is indexed;
/// the limit is stated rather than hidden.
pub const READABLE_EXTENSIONS: [&str; 12] = [
    "txt", "md", "markdown", "csv", "tsv", "json", "log", "rst", "tex", "org", "adoc", "text",
];

/// Limits applied before anything is read, so pointing the library at `/` is slow rather than fatal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The most documents to read.
    pub max_documents: usize,
    /// The largest single file to read.
    pub max_bytes: usize,
    /// The deepest directory nesting to follow.
    pub max_depth: usize,
    /// Whether to follow symbolic links below the root.
    pub follow_links: bool,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_documents: 2_000,
            max_bytes: 8 << 20,
            max_depth: 8,
            follow_links: false,
        }
    }
}

/// One document the library holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    /// The path, relative to the library root, with `/` separators so a citation reads the same on
    /// every platform.
    pub path: String,
    /// The file's size in bytes, for the listing.
    pub bytes: usize,
    /// The chunked text.
    pub chunks: Vec<Chunk>,
}

/// One passage that matched, with what a citation needs.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    /// The document's path, relative to the library root.
    pub path: String,
    /// One-based first line of the passage.
    pub line_start: usize,
    /// One-based last line of the passage, inclusive.
    pub line_end: usize,
    /// The BM25 score, for ordering and for the threshold.
    pub score: f64,
    /// The passage itself.
    pub text: String,
}

impl Hit {
    /// `path:lines 12-18`, the form a citation is printed in.
    #[must_use]
    pub fn cite(&self) -> String {
        format!("{}:lines {}-{}", self.path, self.line_start, self.line_end)
    }
}

/// The result of one search.
#[derive(Debug, Clone, PartialEq)]
pub struct Search {
    /// The passages that matched, best first.
    pub hits: Vec<Hit>,
    /// Whether the library had nothing to say about this query.
    ///
    /// The single most important field in this crate: a model that is told "the knowledge base does
    /// not cover this" behaves differently from one that is handed three weak passages and asked to
    /// be helpful.
    pub missed: bool,
    /// How many documents and passages were searched.
    pub documents: usize,
    /// How many passages were searched.
    pub passages: usize,
}

/// A library and what opening it cost.
#[derive(Debug, Clone)]
pub struct Opened {
    /// The documents, indexed.
    pub library: Library,
    /// How many were parsed and how many came from the cache.
    pub reuse: crate::cache::Reuse,
}

/// The user's documents, indexed for search.
#[derive(Debug, Clone, Default)]
pub struct Library {
    root: PathBuf,
    documents: Vec<Document>,
    /// The interned index over every passage of every document.
    index: crate::index::Index,
    /// The score a hit needs to count as evidence.
    threshold: f64,
}

impl Library {
    /// Open a directory as a knowledge base.
    ///
    /// # Errors
    ///
    /// A message when the directory cannot be read. A missing directory is an *empty* library rather
    /// than an error: a person who has not built one yet should still be able to run the app.
    pub fn open(directory: &Path) -> Result<Self, String> {
        Self::open_with(directory, Limits::default())
    }

    /// Open a directory with explicit limits.
    ///
    /// # Errors
    ///
    /// As [`Self::open`].
    pub fn open_with(directory: &Path, limits: Limits) -> Result<Self, String> {
        Ok(Self::open_cached(directory, limits, None)?.library)
    }

    /// Open a directory, reusing the derived index of every file whose size and modification time
    /// have not changed.
    ///
    /// This is what makes a paper corpus practical: the first open parses everything, and every open
    /// after it is a directory walk plus a cache read. `None` parses everything, which is what a test
    /// and a one-off run want.
    ///
    /// # Errors
    ///
    /// As [`Self::open`].
    pub fn open_cached(
        directory: &Path,
        limits: Limits,
        cache: Option<&stkcraft_storage::Cache>,
    ) -> Result<Opened, String> {
        let files = scan(directory, limits)?;
        let manifest = files
            .iter()
            .map(|file| format!("{}\t{}\t{}", file.relative, file.bytes, file.modified))
            .collect::<Vec<_>>()
            .join("\n");
        let key = crate::cache::library_key(&manifest);
        // Warm: the whole library — documents and index — in one read.
        if let Some(cache) = cache
            && let Some((documents, index)) = crate::cache::load_library(cache, &key)
        {
            let reuse = crate::cache::Reuse {
                parsed: 0,
                reused: documents.len(),
            };
            return Ok(Opened {
                library: Self {
                    root: directory.to_path_buf(),
                    documents,
                    index,
                    threshold: DEFAULT_THRESHOLD,
                },
                reuse,
            });
        }
        // Cold: read and index every file, then remember the whole thing.
        let mut documents = Vec::with_capacity(files.len());
        for file in &files {
            let Ok(text) = std::fs::read_to_string(&file.absolute) else {
                // Not UTF-8: a binary dressed as text, skipped rather than decoded on a guess.
                continue;
            };
            documents.push(Document {
                path: file.relative.clone(),
                bytes: text.len(),
                chunks: chunk(&text),
            });
        }
        let passaged = documents.len();
        let library = Self::from_documents(directory, documents);
        if let Some(cache) = cache {
            crate::cache::store_library(cache, &key, library.documents(), library.index());
            // One assembled library at a time: the previous revision of an edited corpus is not an
            // index anybody will read again.
            crate::cache::prune_libraries(cache, &key);
        }
        Ok(Opened {
            library,
            reuse: crate::cache::Reuse {
                parsed: passaged,
                reused: 0,
            },
        })
    }

    /// Build a library from documents already in memory, which is what a test does.
    #[must_use]
    pub fn from_documents(root: &Path, documents: Vec<Document>) -> Self {
        // The passages are handed to the index with the document each belongs to, so a hit resolves
        // back to a document and a line range without a second structure.
        let index = crate::index::Index::build(documents.iter().enumerate().flat_map(
            |(document, entry)| {
                entry
                    .chunks
                    .iter()
                    .enumerate()
                    .map(move |(chunk, passage)| (document, chunk, passage.text.as_str()))
            },
        ));
        Self {
            root: root.to_path_buf(),
            documents,
            index,
            threshold: DEFAULT_THRESHOLD,
        }
    }

    /// The directory this library was opened from.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The documents, in path order.
    #[must_use]
    pub fn documents(&self) -> &[Document] {
        &self.documents
    }

    /// The index, for a caller that wants its size or its terms.
    #[must_use]
    pub fn index(&self) -> &crate::index::Index {
        &self.index
    }

    /// The score a passage needs to count as evidence.
    #[must_use]
    pub fn threshold(&self) -> f64 {
        self.threshold
    }

    /// Use a different evidence threshold.
    pub fn set_threshold(&mut self, threshold: f64) {
        if threshold.is_finite() && threshold >= 0.0 {
            self.threshold = threshold;
        }
    }

    /// How many passages the library holds.
    #[must_use]
    pub fn passages(&self) -> usize {
        self.index.passages()
    }

    /// Whether the library has no documents at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.documents.is_empty()
    }

    /// Search for `query`, best passage first.
    ///
    /// Returns at most `limit` hits, and never a hit below the threshold: the caller's `missed` flag
    /// is set instead, because "no evidence" is a fact the interface has to be able to state.
    #[must_use]
    pub fn search(&self, query: &str, limit: usize) -> Search {
        let passages = self.passages();
        let mut hits: Vec<Hit> = self
            .index
            .score(query)
            .into_iter()
            .filter_map(|(passage, score)| {
                let (document, chunk) = self.index.owner(passage)?;
                let document = self.documents.get(document)?;
                let chunk = document.chunks.get(chunk)?;
                Some(Hit {
                    path: document.path.clone(),
                    line_start: chunk.line_start,
                    line_end: chunk.line_end,
                    score,
                    text: chunk.text.clone(),
                })
            })
            .collect();
        // The index already ordered by score; re-sort for a total order across equal scores, so a
        // library renders the same list on every machine.
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.path.cmp(&b.path))
                .then_with(|| a.line_start.cmp(&b.line_start))
        });
        let best = hits.first().map_or(0.0, |hit| hit.score);
        hits.truncate(limit);
        Search {
            hits,
            missed: best < self.threshold,
            documents: self.documents.len(),
            passages,
        }
    }
}

/// Read every readable document under `directory`.
///
/// # Errors
///
/// A message when the directory cannot be listed. Unreadable files are skipped rather than failing the
/// whole library: one broken file must not make the other nine hundred unusable.
pub fn load_directory(directory: &Path, limits: Limits) -> Result<Vec<Document>, String> {
    let mut documents = Vec::new();
    for file in scan(directory, limits)? {
        let Ok(text) = std::fs::read_to_string(&file.absolute) else {
            continue;
        };
        documents.push(Document {
            path: file.relative,
            bytes: text.len(),
            chunks: chunk(&text),
        });
    }
    Ok(documents)
}

/// One file the library would index: where it is, what it is called inside the library, and the
/// metadata that decides whether a cached index still describes it.
#[derive(Debug, Clone)]
struct File {
    /// The path on disk.
    absolute: PathBuf,
    /// The path relative to the library root, with `/` separators.
    relative: String,
    /// Its size in bytes.
    bytes: u64,
    /// Its modification time in nanoseconds since the Unix epoch; `0` when the system does not say.
    modified: u128,
}

/// List the files a library would index, in a stable order, applying the limits before reading.
fn scan(directory: &Path, limits: Limits) -> Result<Vec<File>, String> {
    let mut files = Vec::new();
    if limits.max_documents == 0 || limits.max_bytes == 0 {
        return Ok(files);
    }
    scan_into(directory, directory, 0, limits, &mut files)?;
    files.sort_by(|a, b| a.relative.cmp(&b.relative));
    files.truncate(limits.max_documents);
    Ok(files)
}

/// The recursive half of [`scan`].
fn scan_into(
    root: &Path,
    directory: &Path,
    depth: usize,
    limits: Limits,
    files: &mut Vec<File>,
) -> Result<(), String> {
    if depth > limits.max_depth || files.len() >= limits.max_documents {
        return Ok(());
    }
    let entries = std::fs::read_dir(directory)
        .map_err(|error| format!("could not read {}: {error}", directory.display()))?;
    let mut paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect();
    paths.sort();
    for path in paths {
        if files.len() >= limits.max_documents {
            return Ok(());
        }
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.is_symlink() && !limits.follow_links {
            continue;
        }
        let resolved = if metadata.is_symlink() {
            match std::fs::metadata(&path) {
                Ok(metadata) => metadata,
                Err(_) => continue,
            }
        } else {
            metadata
        };
        if resolved.is_dir() {
            scan_into(root, &path, depth + 1, limits, files)?;
            continue;
        }
        if !resolved.is_file() || !is_readable(&path) {
            continue;
        }
        // Skip a directory's own bookkeeping: a `.git` or an editor's swap file is not a document.
        if path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with('.'))
        {
            continue;
        }
        if resolved.len() > limits.max_bytes as u64 {
            continue;
        }
        let modified = resolved
            .modified()
            .ok()
            .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |since| since.as_nanos());
        files.push(File {
            absolute: path.clone(),
            relative: path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/"),
            bytes: resolved.len(),
            modified,
        });
    }
    Ok(())
}

/// Whether the extension is one the library reads.
fn is_readable(path: &Path) -> bool {
    path.extension()
        .map(|extension| extension.to_string_lossy().to_ascii_lowercase())
        .is_some_and(|extension| READABLE_EXTENSIONS.contains(&extension.as_str()))
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn document(path: &str, text: &str) -> Document {
        Document {
            path: path.to_owned(),
            bytes: text.len(),
            chunks: chunk(text),
        }
    }

    fn sample_library() -> Library {
        Library::from_documents(
            Path::new("/knowledge"),
            vec![
                document(
                    "sensor-spec.md",
                    "Titan orbiter sensor\n\nMaximum off-nadir angle is 45 degrees.\nThe sensor swath is 1200 km at that angle.",
                ),
                document(
                    "mission-requirements.md",
                    "Requirements\n\nThe downlink window is at least 10 minutes per orbit.\nPasses over the pole station are excluded.",
                ),
                document(
                    "thermal.txt",
                    "Thermal\n\nRadiator area 2.4 square metres.\nMaximum temperature 320 K.",
                ),
            ],
        )
    }

    #[test]
    fn a_specification_value_is_found_and_cited_by_line() {
        let library = sample_library();
        let search = library.search("maximum off-nadir angle", 3);
        assert!(!search.missed);
        let best = search.hits.first().expect("a hit");
        assert_eq!(best.path, "sensor-spec.md");
        // The line under the heading, since a one-line heading merges with the paragraph below it.
        assert_eq!((best.line_start, best.line_end), (1, 4));
        assert!(best.text.contains("45 degrees"), "{}", best.text);
        assert_eq!(best.cite(), "sensor-spec.md:lines 1-4");
    }

    #[test]
    fn a_query_the_library_does_not_cover_says_so() {
        // The property the whole design exists for: no evidence must be reportable as no evidence.
        let library = sample_library();
        let search = library.search("hyperspectral methane detector sensitivity", 3);
        assert!(search.missed, "{:?}", search.hits);
    }

    #[test]
    fn an_empty_library_misses_rather_than_failing() {
        let library = Library::from_documents(Path::new("/empty"), Vec::new());
        let search = library.search("anything", 5);
        assert!(search.missed);
        assert_eq!(search.hits, Vec::new());
        assert_eq!(search.passages, 0);
        assert!(library.is_empty());
    }

    #[test]
    fn a_bare_query_is_not_answered_with_everything() {
        let library = sample_library();
        let search = library.search("the", 5);
        // "the" appears everywhere, so its weight is near zero and nothing passes the threshold.
        assert!(search.missed, "{:?}", search.hits);
    }

    #[test]
    fn the_same_query_ranks_the_same_way_every_time() {
        let library = sample_library();
        let first = library.search("maximum temperature radiator", 3);
        let second = library.search("maximum temperature radiator", 3);
        assert_eq!(first.hits, second.hits);
        assert_eq!(
            first.hits.first().map(|hit| hit.path.as_str()),
            Some("thermal.txt")
        );
    }

    #[test]
    fn chinese_documents_are_searchable() {
        let library = Library::from_documents(
            Path::new("/知识库"),
            vec![document(
                "传感器规格.txt",
                "传感器\n\n最大侧摆角为四十五度。\n辐宽一千二百公里。",
            )],
        );
        let search = library.search("最大侧摆角", 3);
        assert!(!search.missed, "a Chinese query found nothing");
        let best = search.hits.first().expect("a hit");
        assert!(best.text.contains("四十五度"), "{}", best.text);
    }

    #[test]
    fn the_limit_is_respected_and_results_are_ordered() {
        let library = sample_library();
        let search = library.search("angle sensor downlink radiator", 2);
        assert!(search.hits.len() <= 2);
        for pair in search.hits.windows(2) {
            assert!(
                pair[0].score >= pair[1].score,
                "the hits are not in score order"
            );
        }
    }

    #[test]
    fn a_second_open_reuses_what_did_not_change() {
        // The property a paper corpus depends on: the first open parses, the second is a walk.
        let root = std::env::temp_dir().join("stkcraft-knowledge-cache");
        let _ = std::fs::remove_dir_all(&root);
        let cache_directory = root.join("cache");
        std::fs::create_dir_all(&root).expect("a directory");
        std::fs::write(root.join("spec.md"), "Maximum off-nadir angle 45 degrees.")
            .expect("written");
        std::fs::write(root.join("thermal.txt"), "Radiator 2.4 square metres.").expect("written");
        let cache = stkcraft_storage::Cache::at(&cache_directory);

        let cold = Library::open_cached(&root, Limits::default(), Some(&cache)).expect("opened");
        assert_eq!(cold.reuse.parsed, 2, "the first open parsed nothing");
        assert_eq!(cold.reuse.reused, 0);
        assert!(!cold.library.is_empty());

        let warm = Library::open_cached(&root, Limits::default(), Some(&cache)).expect("opened");
        assert_eq!(warm.reuse.reused, 2, "the second open re-parsed");
        assert_eq!(warm.reuse.parsed, 0);
        // And the reused index answers the same way: a cache that changes an answer is a bug.
        let before = cold.library.search("maximum off-nadir angle", 3);
        let after = warm.library.search("maximum off-nadir angle", 3);
        assert_eq!(before.hits, after.hits);
        assert_eq!(before.missed, after.missed);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_edited_file_is_parsed_again_and_a_deleted_one_leaves_no_entry() {
        let root = std::env::temp_dir().join("stkcraft-knowledge-cache-edit");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("a directory");
        std::fs::write(root.join("a.md"), "first version").expect("written");
        std::fs::write(root.join("b.md"), "second document").expect("written");
        let cache = stkcraft_storage::Cache::at(root.join("cache"));
        Library::open_cached(&root, Limits::default(), Some(&cache)).expect("opened");
        let cached = cache.len();

        std::fs::write(root.join("a.md"), "first version, edited and longer").expect("written");
        std::fs::remove_file(root.join("b.md")).expect("removed");
        let again = Library::open_cached(&root, Limits::default(), Some(&cache)).expect("opened");
        assert_eq!(again.reuse.parsed, 1, "the edited file was not re-parsed");
        assert_eq!(again.reuse.reused, 0);
        // The deleted file's entry is gone: the cache does not accumulate a copy of every revision.
        assert!(
            cache.len() <= cached,
            "the cache grew by keeping a deleted document"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn opening_without_a_cache_behaves_as_before() {
        let root = std::env::temp_dir().join("stkcraft-knowledge-no-cache");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("a directory");
        std::fs::write(root.join("a.md"), "Maximum off-nadir angle 45 degrees.").expect("written");
        let opened = Library::open_cached(&root, Limits::default(), None).expect("opened");
        assert_eq!(opened.reuse.parsed, 1);
        assert_eq!(opened.reuse.total(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn only_text_extensions_are_read() {
        assert!(is_readable(Path::new("a.md")));
        assert!(is_readable(Path::new("b.CSV")));
        assert!(!is_readable(Path::new("c.pdf")));
        assert!(!is_readable(Path::new("d.bin")));
        assert!(!is_readable(Path::new("no-extension")));
    }

    #[test]
    fn a_directory_is_read_with_its_relative_paths() {
        let root = std::env::temp_dir().join("stkcraft-knowledge-load");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("notes")).expect("a directory");
        std::fs::write(root.join("spec.md"), "Maximum off-nadir angle 45 degrees.")
            .expect("written");
        std::fs::write(
            root.join("notes/thermal.txt"),
            "Radiator 2.4 square metres.",
        )
        .expect("written");
        std::fs::write(root.join("ignored.pdf"), "%PDF-1.7 binary").expect("written");
        std::fs::write(root.join(".hidden.md"), "hidden").expect("written");

        let documents = load_directory(&root, Limits::default()).expect("loaded");
        let paths: Vec<&str> = documents
            .iter()
            .map(|document| document.path.as_str())
            .collect();
        assert_eq!(paths, vec!["notes/thermal.txt", "spec.md"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_limits_stop_the_walk_before_it_reads_everything() {
        let root = std::env::temp_dir().join("stkcraft-knowledge-limits");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("a directory");
        for index in 0..5 {
            std::fs::write(root.join(format!("doc{index}.md")), "text").expect("written");
        }
        let limits = Limits {
            max_documents: 2,
            ..Limits::default()
        };
        let documents = load_directory(&root, limits).expect("loaded");
        assert_eq!(documents.len(), 2, "the document cap was not applied");
        let _ = std::fs::remove_dir_all(&root);
    }
}
