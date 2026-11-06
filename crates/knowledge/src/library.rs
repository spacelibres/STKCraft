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

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::chunk::{Chunk, chunk};

/// The BM25 parameters, from the original formulation (Robertson & Zaragoza 2009, "The Probabilistic
/// Relevance Framework"). Left at the published values: this library is small enough that tuning
/// would be superstition.
const K1: f64 = 1.2;
const B: f64 = 0.75;

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

/// The user's documents, indexed for search.
#[derive(Debug, Clone, Default)]
pub struct Library {
    root: PathBuf,
    documents: Vec<Document>,
    /// document index, chunk index, term → frequency
    term_frequencies: Vec<Vec<BTreeMap<String, usize>>>,
    /// term → number of passages containing it
    document_frequencies: BTreeMap<String, usize>,
    /// passage lengths in tokens, and their mean
    lengths: Vec<usize>,
    mean_length: f64,
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
        let documents = load_directory(directory, limits)?;
        Ok(Self::from_documents(directory, documents))
    }

    /// Build a library from documents already in memory, which is what a test does.
    #[must_use]
    pub fn from_documents(root: &Path, documents: Vec<Document>) -> Self {
        let mut term_frequencies = Vec::new();
        let mut document_frequencies: BTreeMap<String, usize> = BTreeMap::new();
        let mut lengths = Vec::new();
        for document in &documents {
            let mut per_chunk = Vec::new();
            for chunk in &document.chunks {
                let mut frequencies: BTreeMap<String, usize> = BTreeMap::new();
                for token in &chunk.tokens {
                    *frequencies.entry(token.clone()).or_insert(0) += 1;
                }
                for term in frequencies.keys() {
                    *document_frequencies.entry(term.clone()).or_insert(0) += 1;
                }
                lengths.push(chunk.tokens.len());
                per_chunk.push(frequencies);
            }
            term_frequencies.push(per_chunk);
        }
        let total: usize = lengths.iter().sum();
        let mean_length = if lengths.is_empty() {
            0.0
        } else {
            total as f64 / lengths.len() as f64
        };
        Self {
            root: root.to_path_buf(),
            documents,
            term_frequencies,
            document_frequencies,
            lengths,
            mean_length,
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
        self.documents
            .iter()
            .map(|document| document.chunks.len())
            .sum()
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
        let query_tokens = crate::tokens::query_tokens(query);
        let mut scored: Vec<Hit> = Vec::new();
        let passages = self.passages();
        if query_tokens.is_empty() || passages == 0 {
            return Search {
                hits: Vec::new(),
                missed: true,
                documents: self.documents.len(),
                passages,
            };
        }
        let unique: Vec<&String> = {
            let mut seen: Vec<&String> = Vec::new();
            for token in &query_tokens {
                if !seen.contains(&token) {
                    seen.push(token);
                }
            }
            seen
        };
        for (document_index, document) in self.documents.iter().enumerate() {
            let Some(per_chunk) = self.term_frequencies.get(document_index) else {
                continue;
            };
            for (chunk_index, chunk) in document.chunks.iter().enumerate() {
                let Some(frequencies) = per_chunk.get(chunk_index) else {
                    continue;
                };
                let length = self
                    .lengths
                    .get(chunk_index_global(self, document_index, chunk_index))
                    .copied()
                    .unwrap_or(chunk.tokens.len());
                let score = self.score(frequencies, &unique, length, passages);
                if score <= 0.0 {
                    continue;
                }
                scored.push(Hit {
                    path: document.path.clone(),
                    line_start: chunk.line_start,
                    line_end: chunk.line_end,
                    score,
                    text: chunk.text.clone(),
                });
            }
        }
        // Best first, then by path and line so the order is total and a test can assert it.
        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.path.cmp(&b.path))
                .then_with(|| a.line_start.cmp(&b.line_start))
        });
        let best = scored.first().map_or(0.0, |hit| hit.score);
        scored.truncate(limit);
        Search {
            hits: scored,
            missed: best < self.threshold,
            documents: self.documents.len(),
            passages,
        }
    }

    /// BM25 for one passage.
    fn score(
        &self,
        frequencies: &BTreeMap<String, usize>,
        query: &[&String],
        length: usize,
        passages: usize,
    ) -> f64 {
        let mut score = 0.0;
        for term in query {
            let Some(frequency) = frequencies.get(*term).copied() else {
                continue;
            };
            let document_frequency = self
                .document_frequencies
                .get(term.as_str())
                .copied()
                .unwrap_or(0);
            if document_frequency == 0 {
                continue;
            }
            let idf = inverse_document_frequency(document_frequency, passages);
            let frequency = frequency as f64;
            let normalised = if self.mean_length > 0.0 {
                length as f64 / self.mean_length
            } else {
                1.0
            };
            score += idf * (frequency * (K1 + 1.0)) / (frequency + K1 * (1.0 - B + B * normalised));
        }
        score
    }
}

/// The global passage index of a (document, chunk) pair.
///
/// The lengths vector is flat, so this walks the documents before it. It is a small library — a few
/// thousand passages — and an index map would be a second structure to keep in step for no gain.
fn chunk_index_global(library: &Library, document: usize, chunk: usize) -> usize {
    let before: usize = library
        .documents
        .iter()
        .take(document)
        .map(|document| document.chunks.len())
        .sum();
    before + chunk
}

/// The BM25 inverse document frequency, with the `+0.5` smoothing that keeps a term in every passage
/// from having a negative weight.
fn inverse_document_frequency(document_frequency: usize, passages: usize) -> f64 {
    let n = passages as f64;
    let df = document_frequency as f64;
    ((n - df + 0.5) / (df + 0.5) + 1.0).ln()
}

/// Read every readable document under `directory`.
///
/// # Errors
///
/// A message when the directory cannot be listed. Unreadable files are skipped and counted rather
/// than failing the whole library: one broken file must not make the other nine hundred unusable.
pub fn load_directory(directory: &Path, limits: Limits) -> Result<Vec<Document>, String> {
    if limits.max_documents == 0 || limits.max_bytes == 0 {
        return Ok(Vec::new());
    }
    let mut documents = Vec::new();
    walk(directory, directory, 0, limits, &mut documents)?;
    documents.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(documents)
}

/// The recursive half of [`load_directory`].
fn walk(
    root: &Path,
    directory: &Path,
    depth: usize,
    limits: Limits,
    documents: &mut Vec<Document>,
) -> Result<(), String> {
    if depth > limits.max_depth || documents.len() >= limits.max_documents {
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
        if documents.len() >= limits.max_documents {
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
            walk(root, &path, depth + 1, limits, documents)?;
            continue;
        }
        if !resolved.is_file() {
            continue;
        }
        // Skip a directory's own bookkeeping: a `.git` or an editor's swap file is not a document.
        if path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with('.'))
        {
            continue;
        }
        if !is_readable(&path) {
            continue;
        }
        let bytes = usize::try_from(resolved.len()).unwrap_or(usize::MAX);
        if bytes > limits.max_bytes {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            // Not UTF-8: a binary dressed as text, skipped rather than decoded on a guess.
            continue;
        };
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        documents.push(Document {
            path: relative,
            bytes: text.len(),
            chunks: chunk(&text),
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
