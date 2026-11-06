//! The vector layer: recall candidates, never the authority.
//!
//! A vector answers one question — *which items are near this one* — and it exists for a corpus too
//! large and too paraphrased for lexical search alone. What it returns is a **candidate**: the answer
//! still cites a document and a page, and a candidate with no citation behind it is not evidence.
//!
//! # What is stored
//!
//! Each vector carries the **model** that produced it and its **dimension**, so a query embedded by a
//! different model is refused instead of being compared against incomparable numbers. That is the one
//! mistake this store must make impossible: cosine similarity between two different embedding spaces
//! is a number with no meaning that looks exactly like one with.
//!
//! # Why brute force
//!
//! At paper-corpus scale — tens of thousands of chunks — a linear scan in `f32` is milliseconds, and an
//! approximate index (HNSW) is a structure to build, tune and debug for a speed nobody is waiting on.
//! When a corpus outgrows that, the index is the change, not the format.
//!
//! # Persistence
//!
//! [`Vectors::to_bytes`] and [`Vectors::from_bytes`] are a flat little-endian format, so a set can be
//! stored in the [`Cache`](crate::Cache) under a key derived from the index revision. A truncated or
//! foreign file is rejected rather than misread: the count and dimensions are checked against the
//! bytes that follow.

use std::collections::BTreeMap;

/// The magic at the start of a serialised set.
const MAGIC: &[u8; 8] = b"STKVEC01";

/// One stored vector: an identity, the numbers, and whatever a citation needs.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// The caller's identity for the item — a passage id, a document and line range.
    pub id: String,
    /// The embedding.
    pub vector: Vec<f32>,
    /// Metadata that travels with it, so a hit can be cited without a second lookup.
    pub metadata: String,
}

/// A hit: what matched, and how closely.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    /// The item's identity.
    pub id: String,
    /// Cosine similarity in `[-1, 1]`; higher is closer.
    pub score: f32,
    /// The item's metadata.
    pub metadata: String,
}

/// A set of vectors from one model.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Vectors {
    /// The model that produced every vector in this set, e.g. `deepseek-embed`.
    model: String,
    /// How many components each vector has.
    dimensions: usize,
    /// The entries, by id, so re-inserting an id replaces it.
    entries: BTreeMap<String, Entry>,
}

impl Vectors {
    /// An empty set for vectors of `dimensions` components from `model`.
    #[must_use]
    pub fn new(model: impl Into<String>, dimensions: usize) -> Self {
        Self {
            model: model.into(),
            dimensions,
            entries: BTreeMap::new(),
        }
    }

    /// The model every vector in this set came from.
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// How many components a vector has.
    #[must_use]
    pub fn dimensions(&self) -> usize {
        self.dimensions
    }

    /// How many vectors the set holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the set holds nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Add a vector, replacing one with the same id.
    ///
    /// # Errors
    ///
    /// A message when the vector's length is not this set's dimension, or when it contains a value
    /// that is not finite. A `NaN` in an embedding makes every distance against it a `NaN`, which
    /// sorts unpredictably and silently poisons a ranking.
    pub fn insert(
        &mut self,
        id: impl Into<String>,
        vector: Vec<f32>,
        metadata: impl Into<String>,
    ) -> Result<(), String> {
        if vector.len() != self.dimensions {
            return Err(format!(
                "a vector of {} component(s) cannot go in a set of {} from `{}`",
                vector.len(),
                self.dimensions,
                self.model
            ));
        }
        if vector.iter().any(|component| !component.is_finite()) {
            return Err("a vector contains a value that is not finite".to_owned());
        }
        let id = id.into();
        self.entries.insert(
            id.clone(),
            Entry {
                id,
                vector,
                metadata: metadata.into(),
            },
        );
        Ok(())
    }

    /// The nearest `limit` entries to `query`, closest first.
    ///
    /// # Errors
    ///
    /// A message when the query is not this set's dimension — the one comparison that must never
    /// happen quietly.
    pub fn search(&self, query: &[f32], limit: usize) -> Result<Vec<Hit>, String> {
        if query.len() != self.dimensions {
            return Err(format!(
                "a query of {} component(s) cannot be compared with {} from `{}`",
                query.len(),
                self.dimensions,
                self.model
            ));
        }
        if query.iter().any(|component| !component.is_finite()) {
            return Err("the query contains a value that is not finite".to_owned());
        }
        let mut hits: Vec<Hit> = self
            .entries
            .values()
            .map(|entry| Hit {
                id: entry.id.clone(),
                score: cosine(query, &entry.vector),
                metadata: entry.metadata.clone(),
            })
            .collect();
        // Closest first, then by id so the order is total and a test can assert it.
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.id.cmp(&b.id))
        });
        hits.truncate(limit);
        Ok(hits)
    }

    /// Serialise the set for the cache.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&u32::try_from(self.dimensions).unwrap_or(0).to_le_bytes());
        out.extend_from_slice(&u32::try_from(self.entries.len()).unwrap_or(0).to_le_bytes());
        write_string(&mut out, &self.model);
        for entry in self.entries.values() {
            write_string(&mut out, &entry.id);
            write_string(&mut out, &entry.metadata);
            for component in &entry.vector {
                out.extend_from_slice(&component.to_le_bytes());
            }
        }
        out
    }

    /// Read a set back, refusing anything that does not match its own header.
    ///
    /// # Errors
    ///
    /// A message when the bytes are not a set, when the header and the payload disagree, or when a
    /// stored value is not finite. A cache that hands back a half-read index would rank garbage.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let mut reader = Reader::new(bytes);
        if reader.take(8)? != MAGIC {
            return Err("not a vector set".to_owned());
        }
        let dimensions =
            usize::try_from(reader.take_u32()?).map_err(|_| "dimension overflow".to_owned())?;
        let count = usize::try_from(reader.take_u32()?).map_err(|_| "count overflow".to_owned())?;
        let model = reader.take_string()?;
        let mut set = Self::new(model, dimensions);
        for _ in 0..count {
            let id = reader.take_string()?;
            let metadata = reader.take_string()?;
            let mut vector = Vec::with_capacity(dimensions);
            for _ in 0..dimensions {
                let bytes = reader.take(4)?;
                let component = f32::from_le_bytes(
                    bytes
                        .try_into()
                        .map_err(|_| "truncated vector".to_owned())?,
                );
                if !component.is_finite() {
                    return Err(format!("`{id}` holds a value that is not finite"));
                }
                vector.push(component);
            }
            set.entries.insert(
                id.clone(),
                Entry {
                    id,
                    vector,
                    metadata,
                },
            );
        }
        if !reader.is_empty() {
            return Err("the set holds bytes past its last entry".to_owned());
        }
        Ok(set)
    }
}

/// Cosine similarity between two equal-length vectors, `0.0` when either has no magnitude.
///
/// Zero magnitude is a real case — an embedding of an empty string can be all zeros — and returning
/// `0.0` ("no similarity") is honest, where dividing would be a `NaN` that sorts unpredictably.
#[must_use]
fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let mut dot = 0.0_f64;
    let mut norm_a = 0.0_f64;
    let mut norm_b = 0.0_f64;
    for (x, y) in a.iter().zip(b) {
        let x = f64::from(*x);
        let y = f64::from(*y);
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    let similarity = dot / (norm_a.sqrt() * norm_b.sqrt());
    // Rounding can leave a value infinitesimally outside the range; clamp rather than report ±1.0001.
    similarity.clamp(-1.0, 1.0) as f32
}

/// Write a length-prefixed string.
fn write_string(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(&u32::try_from(text.len()).unwrap_or(0).to_le_bytes());
    out.extend_from_slice(text.as_bytes());
}

/// A bounds-checked reader, so a truncated cache file is an error rather than a panic.
struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], String> {
        let start = self.position;
        let end = start + length;
        let slice = self
            .bytes
            .get(start..end)
            .ok_or_else(|| "the vector set ends early".to_owned())?;
        self.position = end;
        Ok(slice)
    }

    fn take_u32(&mut self) -> Result<u32, String> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes(
            bytes
                .try_into()
                .map_err(|_| "truncated number".to_owned())?,
        ))
    }

    fn take_string(&mut self) -> Result<String, String> {
        let length = usize::try_from(self.take_u32()?).map_err(|_| "length overflow".to_owned())?;
        let bytes = self.take(length)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| "a stored string is not UTF-8".to_owned())
    }

    fn is_empty(&self) -> bool {
        self.position >= self.bytes.len()
    }
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn set() -> Vectors {
        let mut set = Vectors::new("test-embed", 3);
        set.insert("north", vec![1.0, 0.0, 0.0], "doc a")
            .expect("inserted");
        set.insert("east", vec![0.0, 1.0, 0.0], "doc b")
            .expect("inserted");
        set.insert("northeast", vec![0.7, 0.7, 0.0], "doc c")
            .expect("inserted");
        set
    }

    #[test]
    fn the_nearest_vector_comes_first() {
        let hits = set().search(&[1.0, 0.0, 0.0], 3).expect("searched");
        assert_eq!(hits[0].id, "north");
        assert_eq!(hits[1].id, "northeast");
        assert_eq!(hits[2].id, "east");
        assert!((hits[0].score - 1.0).abs() < 1e-6, "{}", hits[0].score);
        assert_eq!(hits[0].metadata, "doc a");
    }

    #[test]
    fn the_limit_is_respected() {
        assert_eq!(
            set().search(&[1.0, 0.0, 0.0], 2).expect("searched").len(),
            2
        );
    }

    #[test]
    fn a_query_from_another_embedding_space_is_refused() {
        // The mistake this store must make impossible: two models' vectors are incomparable, and
        // comparing them produces a number that looks meaningful.
        let error = set()
            .search(&[1.0, 0.0], 3)
            .expect_err("a two-component query");
        assert!(error.contains("cannot be compared"), "{error}");
        let mut other = set();
        let error = other
            .insert("wrong", vec![1.0, 2.0], "doc d")
            .expect_err("a two-component vector");
        assert!(error.contains("cannot go in"), "{error}");
    }

    #[test]
    fn a_non_finite_value_is_refused() {
        let mut set = Vectors::new("test-embed", 2);
        assert!(set.insert("nan", vec![f32::NAN, 0.0], "x").is_err());
        assert!(set.insert("inf", vec![f32::INFINITY, 0.0], "x").is_err());
        assert!(set.search(&[f32::NAN, 0.0], 1).is_err());
    }

    #[test]
    fn a_zero_vector_scores_zero_rather_than_dividing() {
        let mut set = Vectors::new("test-embed", 2);
        set.insert("zero", vec![0.0, 0.0], "empty embedding")
            .expect("inserted");
        let hits = set.search(&[1.0, 0.0], 1).expect("searched");
        assert_eq!(hits[0].score, 0.0);
    }

    #[test]
    fn a_set_round_trips_through_bytes() {
        let original = set();
        let bytes = original.to_bytes();
        let restored = Vectors::from_bytes(&bytes).expect("read back");
        assert_eq!(restored, original);
        assert_eq!(
            restored.search(&[1.0, 0.0, 0.0], 1).expect("searched")[0].id,
            "north"
        );
    }

    #[test]
    fn a_truncated_or_foreign_set_is_refused() {
        let bytes = set().to_bytes();
        assert!(
            Vectors::from_bytes(&bytes[..bytes.len() - 3]).is_err(),
            "truncation accepted"
        );
        assert!(Vectors::from_bytes(b"not a vector set").is_err());
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(
            Vectors::from_bytes(&trailing).is_err(),
            "trailing bytes accepted"
        );
    }

    #[test]
    fn inserting_the_same_id_replaces_it() {
        let mut set = Vectors::new("test-embed", 2);
        set.insert("a", vec![1.0, 0.0], "first").expect("inserted");
        set.insert("a", vec![0.0, 1.0], "second").expect("inserted");
        assert_eq!(set.len(), 1);
        let hits = set.search(&[0.0, 1.0], 1).expect("searched");
        assert_eq!(hits[0].metadata, "second");
    }

    #[test]
    fn cosine_is_the_published_value_for_a_known_pair() {
        // 3-4-5: the angle between (1,0) and (3,4) has cosine 3/5.
        let similarity = cosine(&[1.0, 0.0], &[3.0, 4.0]);
        assert!((similarity - 0.6).abs() < 1e-6, "{similarity}");
        assert!((cosine(&[1.0, 0.0], &[-1.0, 0.0]) + 1.0).abs() < 1e-6);
    }
}
