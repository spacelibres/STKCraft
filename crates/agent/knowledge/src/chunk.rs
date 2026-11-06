//! Splitting a document into passages that can be cited.
//!
//! A citation has to be **checkable**: a person opens the file at the line the answer named and
//! reads the sentence the model used. So a chunk is a run of lines with a line range, not a bag of
//! characters: chunking on blank lines keeps a table row with its heading, and chunking on a
//! sentence would be three citations for one paragraph.
//!
//! Two limits shape it:
//!
//! - **Small enough to be a citation.** A chunk that holds a whole document is a citation nobody
//!   will follow, and it costs the model context for no precision.
//! - **Big enough to be readable.** A chunk of one line loses the paragraph it belongs to, so a
//!   short paragraph stays whole even when it is under the target size.

/// The largest number of lines a chunk may hold before it is split at a line boundary.
pub const MAX_CHUNK_LINES: usize = 24;

/// The smallest number of lines worth returning on its own.
pub const MIN_CHUNK_LINES: usize = 2;

/// One passage of a document, with the lines it occupies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// Zero-based index of the chunk within its document.
    pub index: usize,
    /// The one-based page the passage starts on, for a document that has pages.
    ///
    /// A text file has none, and its citation is a line range in the file. A PDF does: the lines are
    /// numbered within the page, because that is what a reader can turn to — "page 7" is a place in a
    /// paper, and "line 812" is a place in a file nobody can open.
    pub page: Option<usize>,
    /// One-based first line, as an editor numbers them.
    pub line_start: usize,
    /// One-based last line, inclusive.
    pub line_end: usize,
    /// The lines themselves, joined with `\n`.
    ///
    /// The passage holds no tokens of its own: a `String` per token costs 24 bytes plus its heap for
    /// every word in the corpus, which on a paper library is hundreds of megabytes for text that is
    /// already here. The index interns each token once and stores `u32` ids instead
    /// ([`crate::index::Index`]).
    pub text: String,
}

impl Chunk {
    /// A short label for a citation: `lines 12-18`, or `page 7, lines 3-9` in a paged document.
    #[must_use]
    pub fn lines_label(&self) -> String {
        label(self.page, self.line_start, self.line_end)
    }
}

/// The label a citation uses: `lines 12-18`, or `page 7, lines 3-9` in a paged document.
///
/// One function so a passage and a hit cannot print the same place two ways.
#[must_use]
pub fn label(page: Option<usize>, line_start: usize, line_end: usize) -> String {
    match page {
        Some(page) => format!("page {page}, lines {line_start}-{line_end}"),
        None => format!("lines {line_start}-{line_end}"),
    }
}

/// Split `text` into chunks, each with the line range it occupies.
///
/// Paragraphs are the unit — a run of lines between blank lines — and a paragraph longer than
/// [`MAX_CHUNK_LINES`] is cut on a line boundary. A paragraph shorter than [`MIN_CHUNK_LINES`] is
/// extended with the following lines rather than returned alone, because a one-line citation is
/// usually a heading whose meaning is in the paragraph below it.
#[must_use]
pub fn chunk(text: &str) -> Vec<Chunk> {
    let lines: Vec<&str> = text.lines().collect();
    // `get` rather than `lines[i]`: an index that walks a document is exactly the kind of index that
    // one day walks past its end, and a library that panics while reading someone's documents is
    // worse than one that returns no citation.
    let line = |index: usize| lines.get(index).copied().unwrap_or_default();
    // Paragraphs first: a citation that splits a paragraph mid-sentence is worse than a long one,
    // and a specification's table rows are usually one paragraph.
    let mut paragraphs: Vec<(usize, usize)> = Vec::new();
    let mut position = 0;
    while position < lines.len() {
        if line(position).trim().is_empty() {
            position += 1;
            continue;
        }
        let start = position;
        while position < lines.len() && !line(position).trim().is_empty() {
            position += 1;
        }
        // A one-line paragraph is a heading more often than not; it takes the paragraph after it so
        // the citation has something to say. The two are still one range, so nothing is lost.
        if position - start < MIN_CHUNK_LINES {
            let mut look = position;
            while look < lines.len() && line(look).trim().is_empty() {
                look += 1;
            }
            if look < lines.len() {
                let mut end = look;
                while end < lines.len() && !line(end).trim().is_empty() {
                    end += 1;
                }
                if end - start <= MAX_CHUNK_LINES {
                    position = end;
                }
            }
        }
        paragraphs.push((start, position));
    }
    // Then cut each paragraph into slices of at most `MAX_CHUNK_LINES`, and fold a remainder that
    // would be shorter than `MIN_CHUNK_LINES` into the slice before it.
    let mut chunks = Vec::new();
    for (start, end) in paragraphs {
        let mut slice_start = start;
        while slice_start < end {
            let mut slice_end = (slice_start + MAX_CHUNK_LINES).min(end);
            if end - slice_end < MIN_CHUNK_LINES && slice_end < end {
                // The leftover is too short to stand alone: take it into this slice.
                slice_end = end;
            }
            chunks.push(build(chunks.len(), slice_start, slice_end, &lines));
            slice_start = slice_end;
        }
    }
    chunks
}

/// Split a paged document — a PDF, whose text came out of [`crate::pdf`] — into citable chunks.
///
/// `pages` are one-based page numbers with the text of each page, in reading order. Each page is
/// chunked on its own: a paragraph does not continue across a page turn in a way a reader can follow,
/// and a citation that spanned one would send a person to two places at once. Every chunk remembers
/// its page, so the citation reads `page 7, lines 3-9` instead of a line number in a file that has no
/// lines.
#[must_use]
pub fn chunk_pages(pages: &[(usize, String)]) -> Vec<Chunk> {
    let mut chunks: Vec<Chunk> = Vec::new();
    for (number, text) in pages {
        for chunk in chunk(text) {
            chunks.push(Chunk {
                index: chunks.len(),
                page: Some(*number),
                ..chunk
            });
        }
    }
    chunks
}

/// Build one chunk from a half-open line range.
fn build(index: usize, start: usize, end: usize, lines: &[&str]) -> Chunk {
    let slice = lines.get(start..end).unwrap_or_default();
    let text = slice.join("\n");
    Chunk {
        index,
        page: None,
        line_start: start + 1,
        line_end: end,
        text,
    }
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn a_heading_takes_the_paragraph_under_it_and_the_citation_covers_both() {
        let text = "Heading\n\nFirst paragraph line one.\nFirst paragraph line two.\n\nSecond paragraph.\nAnother line.\n";
        let chunks = chunk(text);
        assert_eq!(chunks.len(), 2, "{chunks:#?}");
        // The heading is one line, so it merges with the paragraph below: a citation of "Heading"
        // alone says nothing, and the blank line it spans is part of the range.
        assert_eq!((chunks[0].line_start, chunks[0].line_end), (1, 4));
        assert!(
            chunks[0].text.starts_with("Heading\n\nFirst paragraph"),
            "{:?}",
            chunks[0].text
        );
        assert_eq!((chunks[1].line_start, chunks[1].line_end), (6, 7));
        assert_eq!(chunks[1].text, "Second paragraph.\nAnother line.");
        assert_eq!(chunks[1].lines_label(), "lines 6-7");
    }

    #[test]
    fn a_short_paragraph_is_not_returned_alone() {
        // A heading with no body is a citation nobody can act on, so it takes its neighbour.
        let chunks = chunk("Sensor\nmaximum off-nadir 45 degrees\n");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, "Sensor\nmaximum off-nadir 45 degrees");
    }

    #[test]
    fn a_long_paragraph_is_cut_on_line_boundaries() {
        let text: String = (0..60).fold(String::new(), |mut text, index| {
            let _ = std::fmt::Write::write_fmt(
                &mut text,
                format_args!("line {index} of an over-long specification table\n"),
            );
            text
        });
        let chunks = chunk(&text);
        assert!(chunks.len() >= 3, "a 60-line paragraph stayed whole");
        for pair in chunks.windows(2) {
            assert_eq!(
                pair[1].line_start,
                pair[0].line_end + 1,
                "the chunks leave a gap or overlap"
            );
        }
        assert!(
            chunks
                .iter()
                .all(|chunk| chunk.line_end - chunk.line_start < MAX_CHUNK_LINES)
        );
    }

    #[test]
    fn every_line_of_the_document_is_reachable_exactly_once() {
        // The property that makes a citation trustworthy: every non-blank line appears in one chunk,
        // and the ranges never overlap or leave a gap.
        let text = "one\n\ntwo\nthree\n\n\nfour\nfive\nsix\n";
        let chunks = chunk(text);
        let mut covered: Vec<&str> = chunks
            .iter()
            .flat_map(|chunk| chunk.text.lines())
            .filter(|line| !line.trim().is_empty())
            .collect();
        covered.sort_unstable();
        assert_eq!(covered, vec!["five", "four", "one", "six", "three", "two"]);
        for pair in chunks.windows(2) {
            assert!(
                pair[1].line_start > pair[0].line_end,
                "chunks overlap: {:?} then {:?}",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn a_paged_document_cites_the_page_it_came_from() {
        let pages = vec![
            (
                1_usize,
                "Sensor specification\n\nMaximum off-nadir angle is 45 degrees.\n".to_owned(),
            ),
            (
                2,
                "The swath is 1200 km.\nIt is measured at that angle.\n".to_owned(),
            ),
        ];
        let chunks = chunk_pages(&pages);
        assert_eq!(chunks.len(), 2, "{chunks:#?}");
        // The heading takes the paragraph under it, on the page it is on.
        assert_eq!(chunks[0].page, Some(1));
        assert_eq!(chunks[0].lines_label(), "page 1, lines 1-3");
        // The second page starts again at line 1, and says which page it is on.
        assert_eq!(chunks[1].page, Some(2));
        assert_eq!((chunks[1].line_start, chunks[1].line_end), (1, 2));
        assert_eq!(chunks[1].lines_label(), "page 2, lines 1-2");
        // The chunk indices are the document's, not the page's.
        assert_eq!(
            chunks.iter().map(|chunk| chunk.index).collect::<Vec<_>>(),
            vec![0, 1]
        );
    }

    #[test]
    fn a_text_document_has_no_page_and_says_so_in_its_label() {
        let chunks = chunk("one\ntwo\n");
        assert_eq!(chunks[0].page, None);
        assert_eq!(chunks[0].lines_label(), "lines 1-2");
        // An empty page adds nothing, so a PDF whose pages hold no text yields no passages.
        assert_eq!(
            chunk_pages(&[(1, String::new()), (2, "\n\n".to_owned())]),
            vec![]
        );
    }

    #[test]
    fn an_empty_document_has_no_chunks() {
        assert_eq!(chunk(""), Vec::new());
        assert_eq!(chunk("\n\n\n"), Vec::new());
    }
}
