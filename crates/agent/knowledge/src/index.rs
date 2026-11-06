//! The inverted index: one interned dictionary, postings of `u32`, and the scoring over them.
//!
//! Searching means looking a term up in every passage, and the naive representation — a `BTreeMap`
//! from `String` to a count per passage, plus a `Vec<String>` of tokens — costs a heap allocation per
//! word. Measured on a 27 MB corpus of 2 000 documents that was 400 MB of memory and two seconds of
//! index building *per open*, and no cache can help with either.
//!
//! So tokens are interned: every distinct word becomes one `u32` in a dictionary, each passage stores
//! its term ids, and the scoring looks them up by id. The corpus's own text is the only thing that
//! scales with the corpus; the index is a few numbers per passage.

use std::collections::HashMap;

use crate::tokens;

/// The BM25 parameters, from the original formulation (Robertson & Zaragoza 2009). Left at the
/// published values: this library is small enough that tuning would be superstition.
pub const K1: f64 = 1.2;
/// See [`K1`].
pub const B: f64 = 0.75;

/// One passage's postings: the term ids it contains, with their counts, and how long it is.
#[derive(Debug, Clone, Default)]
pub struct Posting {
    /// `(term id, occurrences)`, in first-appearance order.
    pub terms: Vec<(u32, u32)>,
    /// How many tokens the passage holds.
    pub length: u32,
}

/// The index: a dictionary, one posting per passage, and the document frequency of every term.
#[derive(Debug, Clone, Default)]
pub struct Index {
    /// Term id → the term.
    terms: Vec<String>,
    /// Term → its id.
    ids: HashMap<String, u32>,
    /// passage → its postings.
    postings: Vec<Posting>,
    /// term id → how many passages contain it.
    frequencies: Vec<u32>,
    /// passage → `(document, chunk)`.
    owners: Vec<(u32, u32)>,
    /// The mean passage length in tokens.
    mean_length: f64,
}

impl Index {
    /// Index the passages of a library.
    ///
    /// `passages` is every passage in document order, as `(document, passage within the document,
    /// text)`. The two indices are kept separately: the passage's own position in the index is a third
    /// number, and confusing them resolves a hit to the wrong passage — which looks like a ranking bug
    /// and is an off-by-one.
    pub fn build<'a>(passages: impl IntoIterator<Item = (usize, usize, &'a str)>) -> Self {
        let mut index = Self::default();
        for (document, chunk, text) in passages {
            let mut posting = Posting::default();
            let mut counts: HashMap<u32, u32> = HashMap::new();
            for token in tokens::tokenize(text) {
                let id = index.intern(&token);
                posting.length = posting.length.saturating_add(1);
                *counts.entry(id).or_insert(0) += 1;
            }
            for (id, count) in counts {
                posting.terms.push((id, count));
            }
            // A stable order inside a passage, so two builds of the same corpus are identical.
            posting.terms.sort_unstable();
            for (id, _) in &posting.terms {
                if let Some(frequency) = index.frequencies.get_mut(*id as usize) {
                    *frequency = frequency.saturating_add(1);
                }
            }
            index.postings.push(posting);
            index.owners.push((
                u32::try_from(document).unwrap_or(u32::MAX),
                u32::try_from(chunk).unwrap_or(u32::MAX),
            ));
        }
        let total: u64 = index
            .postings
            .iter()
            .map(|posting| u64::from(posting.length))
            .sum();
        index.mean_length = if index.postings.is_empty() {
            0.0
        } else {
            total as f64 / index.postings.len() as f64
        };
        index
    }

    /// The term's id, adding it to the dictionary the first time it is seen.
    fn intern(&mut self, term: &str) -> u32 {
        if let Some(id) = self.ids.get(term) {
            return *id;
        }
        let id = u32::try_from(self.terms.len()).unwrap_or(u32::MAX);
        self.terms.push(term.to_owned());
        self.ids.insert(term.to_owned(), id);
        self.frequencies.push(0);
        id
    }

    /// How many passages the index holds.
    #[must_use]
    pub fn passages(&self) -> usize {
        self.postings.len()
    }

    /// How many distinct terms it holds.
    #[must_use]
    pub fn terms(&self) -> usize {
        self.terms.len()
    }

    /// The document and passage a passage index belongs to.
    #[must_use]
    pub fn owner(&self, passage: usize) -> Option<(usize, usize)> {
        self.owners
            .get(passage)
            .map(|(document, chunk)| (*document as usize, *chunk as usize))
    }

    /// Serialise the index, so a cached library does not rebuild it.
    ///
    /// The dictionary and the postings are what cost: interning three million tokens over a paper
    /// corpus is a second of work and the point of the cache is to pay it once.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&u32::try_from(self.terms.len()).unwrap_or(0).to_le_bytes());
        for term in &self.terms {
            write_string(&mut out, term);
        }
        out.extend_from_slice(
            &u32::try_from(self.postings.len())
                .unwrap_or(0)
                .to_le_bytes(),
        );
        for posting in &self.postings {
            out.extend_from_slice(&posting.length.to_le_bytes());
            out.extend_from_slice(
                &u32::try_from(posting.terms.len())
                    .unwrap_or(0)
                    .to_le_bytes(),
            );
            for (term, count) in &posting.terms {
                out.extend_from_slice(&term.to_le_bytes());
                out.extend_from_slice(&count.to_le_bytes());
            }
        }
        out.extend_from_slice(&u32::try_from(self.owners.len()).unwrap_or(0).to_le_bytes());
        for (document, chunk) in &self.owners {
            out.extend_from_slice(&document.to_le_bytes());
            out.extend_from_slice(&chunk.to_le_bytes());
        }
        out
    }

    /// Read an index back, refusing anything that does not match its own header.
    ///
    /// # Errors
    ///
    /// A message when the bytes are not an index or are truncated. A half-read index would rank
    /// against postings that point nowhere, which is worse than a rebuild.
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        let mut reader = Reader::new(bytes);
        if reader.take(8)? != MAGIC {
            return Err("not an index".to_owned());
        }
        let term_count =
            usize::try_from(reader.take_u32()?).map_err(|_| "count overflow".to_owned())?;
        if term_count > bytes.len() {
            return Err("the term count is larger than the file".to_owned());
        }
        let mut index = Self::default();
        for _ in 0..term_count {
            let term = reader.take_string()?;
            let id = u32::try_from(index.terms.len()).unwrap_or(u32::MAX);
            index.ids.insert(term.clone(), id);
            index.terms.push(term);
            index.frequencies.push(0);
        }
        let passage_count =
            usize::try_from(reader.take_u32()?).map_err(|_| "count overflow".to_owned())?;
        if passage_count > bytes.len() {
            return Err("the passage count is larger than the file".to_owned());
        }
        for _ in 0..passage_count {
            let length = reader.take_u32()?;
            let pair_count =
                usize::try_from(reader.take_u32()?).map_err(|_| "count overflow".to_owned())?;
            if pair_count > term_count {
                return Err("a passage holds more terms than the dictionary".to_owned());
            }
            let mut terms = Vec::with_capacity(pair_count);
            for _ in 0..pair_count {
                let id = reader.take_u32()?;
                let count = reader.take_u32()?;
                if let Some(frequency) = index.frequencies.get_mut(id as usize) {
                    *frequency = frequency.saturating_add(1);
                }
                terms.push((id, count));
            }
            index.postings.push(Posting { terms, length });
        }
        let owner_count =
            usize::try_from(reader.take_u32()?).map_err(|_| "count overflow".to_owned())?;
        for _ in 0..owner_count {
            let document = reader.take_u32()?;
            let chunk = reader.take_u32()?;
            index.owners.push((document, chunk));
        }
        if !reader.is_empty() {
            return Err("the index holds bytes past its last passage".to_owned());
        }
        if index.owners.len() != index.postings.len() {
            return Err("the index's passages and owners disagree".to_owned());
        }
        let total: u64 = index
            .postings
            .iter()
            .map(|posting| u64::from(posting.length))
            .sum();
        index.mean_length = if index.postings.is_empty() {
            0.0
        } else {
            total as f64 / index.postings.len() as f64
        };
        Ok(index)
    }

    /// Every passage that matches the query, as `(passage, score)`, above zero.
    ///
    /// A single pass over the postings of the query's terms: the passages a term does not appear in
    /// are never touched, which is what makes this cheaper than scanning passages for every query.
    #[must_use]
    pub fn score(&self, query: &str) -> Vec<(usize, f64)> {
        let wanted: Vec<u32> = tokens::query_tokens(query)
            .iter()
            .filter_map(|token| self.ids.get(token).copied())
            .collect();
        if wanted.is_empty() {
            return Vec::new();
        }
        let mut scores: Vec<f64> = vec![0.0; self.postings.len()];
        let mut touched: Vec<usize> = Vec::new();
        // Which passages hold any of the query's terms: a flag per passage, gathered once, so the
        // scoring pass below never visits the passages the query cannot match.
        let mut candidate = vec![false; self.postings.len()];
        for id in &wanted {
            for (passage, posting) in self.postings.iter().enumerate() {
                let holds = posting
                    .terms
                    .binary_search_by_key(id, |(term, _)| *term)
                    .is_ok();
                if holds && let Some(slot) = candidate.get_mut(passage) {
                    *slot = true;
                }
            }
        }
        let candidate_list: Vec<usize> = candidate
            .iter()
            .enumerate()
            .filter_map(|(passage, is_candidate)| is_candidate.then_some(passage))
            .collect();
        for id in &wanted {
            let frequency = self.frequencies.get(*id as usize).copied().unwrap_or(0);
            if frequency == 0 {
                continue;
            }
            let idf = inverse_document_frequency(frequency as usize, self.postings.len());
            for passage in &candidate_list {
                let Some(posting) = self.postings.get(*passage) else {
                    continue;
                };
                let Ok(at) = posting.terms.binary_search_by_key(id, |(term, _)| *term) else {
                    continue;
                };
                let Some((_, occurrences)) = posting.terms.get(at) else {
                    continue;
                };
                let occurrences = f64::from(*occurrences);
                let normalised = if self.mean_length > 0.0 {
                    f64::from(posting.length) / self.mean_length
                } else {
                    1.0
                };
                let contribution = idf * (occurrences * (K1 + 1.0))
                    / (occurrences + K1 * (1.0 - B + B * normalised));
                if let Some(slot) = scores.get_mut(*passage) {
                    *slot += contribution;
                }
                if !touched.contains(passage) {
                    touched.push(*passage);
                }
            }
        }
        let mut ranked: Vec<(usize, f64)> = touched
            .into_iter()
            .filter_map(|passage| {
                let score = scores.get(passage).copied().unwrap_or(0.0);
                (score > 0.0).then_some((passage, score))
            })
            .collect();
        ranked.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        ranked
    }
}

/// The magic at the start of a serialised index.
const MAGIC: &[u8; 8] = b"STKIDX01";

/// Write a length-prefixed string.
pub(crate) fn write_string(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(&u32::try_from(text.len()).unwrap_or(0).to_le_bytes());
    out.extend_from_slice(text.as_bytes());
}

/// A bounds-checked reader, so a truncated cache file is an error rather than a panic.
pub(crate) struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    pub(crate) fn take(&mut self, length: usize) -> Result<&'a [u8], String> {
        let end = self.position.saturating_add(length);
        let slice = self
            .bytes
            .get(self.position..end)
            .ok_or_else(|| "the index ends early".to_owned())?;
        self.position = end;
        Ok(slice)
    }

    pub(crate) fn take_u32(&mut self) -> Result<u32, String> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes(
            bytes
                .try_into()
                .map_err(|_| "truncated number".to_owned())?,
        ))
    }

    pub(crate) fn take_byte(&mut self) -> Result<u8, String> {
        let bytes = self.take(1)?;
        bytes
            .first()
            .copied()
            .ok_or_else(|| "the index ends early".to_owned())
    }

    pub(crate) fn take_string(&mut self) -> Result<String, String> {
        let length = usize::try_from(self.take_u32()?).map_err(|_| "length overflow".to_owned())?;
        let bytes = self.take(length)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| "a stored string is not UTF-8".to_owned())
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.position >= self.bytes.len()
    }

    /// Everything from the current position on, which is where a following record begins.
    pub(crate) fn rest(&self) -> &'a [u8] {
        self.bytes.get(self.position..).unwrap_or_default()
    }
}

/// The BM25 inverse document frequency, with the `+0.5` smoothing that keeps a term in every passage
/// from having a negative weight.
fn inverse_document_frequency(document_frequency: usize, passages: usize) -> f64 {
    let n = passages as f64;
    let df = document_frequency as f64;
    ((n - df + 0.5) / (df + 0.5) + 1.0).ln()
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, clippy::expect_used)]
mod tests {
    use super::*;

    fn index() -> Index {
        Index::build(vec![
            (0, 0, "Maximum off-nadir angle is 45 degrees."),
            (0, 1, "The sensor swath is 1200 km at that angle."),
            (
                1,
                0,
                "Radiator area 2.4 square metres, maximum temperature 320 K.",
            ),
        ])
    }

    #[test]
    fn a_rare_term_finds_its_passage_and_a_common_one_finds_nothing() {
        let index = index();
        let scored = index.score("maximum off-nadir angle");
        assert_ne!(scored, Vec::new());
        assert_eq!(scored[0].0, 0, "the specification passage is not first");
        assert!(
            index.score("the and of").is_empty(),
            "function words matched"
        );
    }

    #[test]
    fn the_index_interns_a_term_once() {
        let index = index();
        // "angle" appears in two passages and "maximum" in two, each as one term id.
        assert!(
            index.terms() < 40,
            "the dictionary is not interned: {}",
            index.terms()
        );
        assert_eq!(index.passages(), 3);
        assert_eq!(index.owner(2), Some((1, 0)));
    }

    #[test]
    fn scores_are_ordered_and_deterministic() {
        let index = index();
        let first = index.score("angle sensor radiator");
        let second = index.score("angle sensor radiator");
        assert_eq!(first, second, "two searches of one index differ");
        for pair in first.windows(2) {
            assert!(pair[0].1 >= pair[1].1, "the scores are not in order");
        }
    }

    #[test]
    fn an_index_round_trips_and_still_ranks() {
        let original = index();
        let restored = Index::decode(&original.encode()).expect("decoded");
        assert_eq!(restored.passages(), original.passages());
        assert_eq!(restored.terms(), original.terms());
        assert_eq!(restored.owner(2), original.owner(2));
        // The ranking survives the round trip, scores and all.
        assert_eq!(
            restored.score("maximum off-nadir"),
            original.score("maximum off-nadir")
        );
    }

    #[test]
    fn a_truncated_index_is_refused() {
        let bytes = index().encode();
        assert!(Index::decode(&bytes[..bytes.len() - 4]).is_err());
        assert!(Index::decode(b"not an index").is_err());
    }

    #[test]
    fn an_index_over_nothing_scores_nothing() {
        let index = Index::build(Vec::<(usize, usize, &str)>::new());
        assert_eq!(index.passages(), 0);
        assert_eq!(index.score("anything"), Vec::new());
    }

    #[test]
    fn the_idf_is_the_published_form() {
        // One passage of three containing a term: ln((3 - 1 + 0.5)/(1 + 0.5) + 1) = ln(8/3).
        let value = inverse_document_frequency(1, 3);
        assert!((value - (8.0_f64 / 3.0).ln()).abs() < 1e-12, "{value}");
        // A term in every passage has no weight, never a negative one.
        assert!(inverse_document_frequency(3, 3) > 0.0);
    }
}
