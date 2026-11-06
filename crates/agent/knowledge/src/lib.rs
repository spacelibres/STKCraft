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
//! ```
//!
//! Point the library at a directory, and every readable text document in it becomes searchable:
//!
//! ```text
//! let library = Library::open(directory)?;            // the documents, chunked, indexed
//! for hit in library.search("maximum off-nadir angle", 3) {
//!     println!("{}:{}-{}", hit.path, hit.line_start, hit.line_end);   // a citation a reader can check
//!     println!("{}", hit.text);
//! }
//! ```
//!
//! # What makes this different from "ask the model"
//!
//! - **Every answer can be traced to a line.** A hit names the document and the line range it came
//!   from, so the interface can print the citation and a person can open the file and look.
//! - **Nothing is inferred.** Ranking is lexical (BM25 over words and CJK character pairs): the same
//!   library and the same query give the same order on every machine, which is what makes a
//!   regression test possible at all.
//! - **The model is told what it does not know.** When the search finds nothing above the threshold,
//!   [`Search::missed`] says so, and the agent's prompt requires the model to say that rather than
//!   invent a number.
//! - **Hostile input is bounded before it is read**: file count, file size, a directory depth and a
//!   skip list, so pointing the library at `/` is slow rather than fatal.
//!
//! # What it deliberately is not
//!
//! No embeddings and no network. A vector index would need a model to build, which would make the
//! retrieval non-deterministic and untestable, and the numbers this project cares about are *exact
//! tokens* — a frequency, an angle, a file name — which lexical search matches better than a vector
//! does. Binary formats (PDF, DOCX, XLSX) are not read: extract the text beside them, and the
//! library will index it. That is a stated limit, not a hidden one.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
#![forbid(unsafe_code)]

pub mod chunk;
pub mod library;
pub mod tokens;

pub use chunk::{Chunk, chunk};
pub use library::{Document, Hit, Library, Limits, Search, load_directory};
pub use tokens::{query_tokens, tokenize, tokens};
