//! The user's own documents, and the search that runs before a model answers.
//!
//! `STKCraft` computes physics, and a plausible-looking number is worse than no number. A model asked
//! "what is the maximum off-nadir angle for this sensor" will answer from its training data, which
//! may be wrong, may be about another vehicle, and cannot be checked. **The person's own documents
//! are the authority**, and this crate is how the platform reads them before the model speaks.
//!
//! ```text
//! knowledge/
//!   mission-requirements.md
//!   sensor-spec.csv
//!   thermal-notes.txt
//!   smad-chapter-7.pdf
//! ```
//!
//! Point the library at a directory, and every readable document in it becomes searchable — text by
//! line, a PDF by page:
//!
//! ```text
//! let library = Library::open(directory)?;            // the documents, chunked, indexed
//! for hit in library.search("maximum off-nadir angle", 3) {
//!     println!("{}", hit.cite());                    // `spec.md:lines 12-18`, or `paper.pdf:page 7, lines 3-9`
//!     println!("{}", hit.text);
//! }
//! ```
//!
//! # What makes this different from "ask the model"
//!
//! - **Every answer can be traced to a place a person can open.** A hit names the document and the
//!   line range it came from, or the page and line range when the document is a PDF, so the interface
//!   can print the citation and a person can go and look.
//! - **Nothing is inferred.** Ranking is lexical (BM25 over words and CJK character pairs): the same
//!   library and the same query give the same order on every machine, which is what makes a
//!   regression test possible at all.
//! - **The model is told what it does not know.** When the search finds nothing above the threshold,
//!   [`Search::missed`] says so, and the agent's prompt requires the model to say that rather than
//!   invent a number.
//! - **Hostile input is bounded before it is read**: file count, file size, a directory depth and a
//!   skip list, so pointing the library at `/` is slow rather than fatal.
//! - **A re-open costs only what changed.** The derived index is cached in the key-value store, keyed
//!   by each file's path, size and modification time, so a paper corpus is parsed once and a second
//!   run is a directory walk.
//! - **A file that cannot be read is reported, not swallowed.** [`Opened::skipped`] names it and says
//!   why — a scan with no text layer, a PDF that will not open, a binary dressed as text — because a
//!   person who added a paper and cannot search it needs the reason.
//!
//! # What it deliberately is not
//!
//! No embeddings and no network. A vector index would need a model to build, which would make the
//! retrieval non-deterministic and untestable, and the numbers this project cares about are *exact
//! tokens* — a frequency, an angle, a file name — which lexical search matches better than a vector
//! does. PDF and plain text are read; other binary formats (DOCX, XLSX, a scanned PDF with no text
//! layer) are not, and the second of those is named in [`Opened::skipped`] rather than indexed as
//! nothing. That is a stated limit, not a hidden one.

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
pub mod chunk;
pub mod index;
pub mod library;
pub mod pdf;
pub mod tokens;

pub use cache::Reuse;
pub use chunk::{Chunk, chunk, chunk_pages};
pub use index::Index;
pub use library::{Document, Hit, Library, Limits, Opened, Search, load_directory};
pub use pdf::{Bounds, Extracted, Page, PdfError, extract, looks_like_pdf};
pub use tokens::{query_tokens, tokenize, tokens};
