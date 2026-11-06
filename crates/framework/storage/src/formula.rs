//! The formula store: a formula as a record with provenance.
//!
//! `docs/design/mathematics.md` is the decision record. The short version: a formula is not prose, so it
//! is not a line range in a text file — it is a **box on a page**, with what the text layer said, what a
//! recogniser made of the image, what the sentence around it asks, and the digests that let a re-run
//! prove it read the same pixels.
//!
//! Two things are deliberate:
//!
//! - **The key is the region.** `document, page, x0, y0, x1, y1` is unique, and a re-record replaces the
//!   row: two different formulas cannot share a box, while two formulas can easily share a LaTeX
//!   spelling. Re-classifying a formula therefore updates it instead of duplicating it.
//! - **Everything that could be wrong is nullable, and nothing is invented.** `latex` is `NULL` when the
//!   recogniser produced nothing, `mathlib_name` is `NULL` when no concept matched, and `confidence` is
//!   `NULL` when the engine does not report one. An empty string would be a claim; absence is a fact.

use rusqlite::{Connection, params};
use stkcraft_framework_core::{Error, Result};

/// One formula, as the store holds it.
#[derive(Debug, Clone, PartialEq)]
pub struct FormulaRecord {
    /// The document, relative to the library root.
    pub document: String,
    /// The one-based page it is on.
    pub page: u32,
    /// The box, in page points: left, top, right, bottom.
    pub x0: f64,
    /// The top of the box.
    pub y0: f64,
    /// The right of the box.
    pub x1: f64,
    /// The bottom of the box.
    pub y1: f64,
    /// What the text layer says, always.
    pub unicode: String,
    /// What a recogniser produced, when one ran.
    pub latex: Option<String>,
    /// The sentence that introduces it: what it solves.
    pub problem: Option<String>,
    /// Where that sentence is, as a line range.
    pub problem_lines: Option<String>,
    /// Which engine read it, as `source`.
    pub source: String,
    /// How sure the engine is, when it says.
    pub confidence: Option<f64>,
    /// The Mathlib declaration it matched.
    pub mathlib_name: Option<String>,
    /// How strongly.
    pub mathlib_score: Option<f64>,
    /// The taxonomy the match implies.
    pub mathlib_domain: Option<String>,
    /// The Mathlib revision the index was built at.
    pub mathlib_rev: Option<String>,
    /// The scoring engine and version.
    pub classifier: Option<String>,
    /// SHA-256 of the PDF it was read from.
    pub paper_digest: String,
    /// SHA-256 of the region image.
    pub region_digest: String,
}

impl FormulaRecord {
    /// A formula found by the text layer, with nothing recognised and nothing classified.
    ///
    /// `place` is the region: document, page, box, and the two digests that make it checkable. It is a
    /// struct because nine positional arguments is a call nobody can read, and the box is a thing.
    #[must_use]
    pub fn from_text_layer(place: &Place, unicode: &str) -> Self {
        Self {
            document: place.document.clone(),
            page: place.page,
            x0: place.x0,
            y0: place.y0,
            x1: place.x1,
            y1: place.y1,
            unicode: unicode.to_owned(),
            latex: None,
            problem: None,
            problem_lines: None,
            source: "deterministic/text-layer".to_owned(),
            confidence: None,
            mathlib_name: None,
            mathlib_score: None,
            mathlib_domain: None,
            mathlib_rev: None,
            classifier: None,
            paper_digest: place.paper_digest.clone(),
            region_digest: place.region_digest.clone(),
        }
    }
}

/// Where a formula is: the region a citation names, and the two digests of the chain.
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    /// The document, relative to the library root.
    pub document: String,
    /// The one-based page.
    pub page: u32,
    /// The box, in page points.
    pub x0: f64,
    /// The top.
    pub y0: f64,
    /// The right.
    pub x1: f64,
    /// The bottom.
    pub y1: f64,
    /// SHA-256 of the PDF.
    pub paper_digest: String,
    /// SHA-256 of the region image.
    pub region_digest: String,
}

/// Store one formula, replacing any earlier reading of the same region.
///
/// # Errors
///
/// A message when the row cannot be written.
pub(crate) fn record(connection: &Connection, formula: &FormulaRecord) -> Result<()> {
    connection.execute(
        "INSERT INTO formula (
            document, page, x0, y0, x1, y1, unicode, latex, problem, problem_lines,
            source, confidence, mathlib_name, mathlib_score, mathlib_domain, mathlib_rev,
            classifier, paper_digest, region_digest, recorded_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
        params![
            formula.document,
            formula.page,
            formula.x0,
            formula.y0,
            formula.x1,
            formula.y1,
            formula.unicode,
            formula.latex,
            formula.problem,
            formula.problem_lines,
            formula.source,
            formula.confidence,
            formula.mathlib_name,
            formula.mathlib_score,
            formula.mathlib_domain,
            formula.mathlib_rev,
            formula.classifier,
            formula.paper_digest,
            formula.region_digest,
            crate::record::now_iso8601(),
        ],
    )
    .map_err(|error| {
        Error::invalid(
            "a formula record",
            format_args!(
                "page {} of {}: {error}",
                formula.page, formula.document
            ),
        )
    })?;
    Ok(())
}

/// Every formula of a document, in page and then top-to-bottom order.
///
/// # Errors
///
/// A message when the rows cannot be read.
pub(crate) fn of_document(connection: &Connection, document: &str) -> Result<Vec<FormulaRecord>> {
    let mut statement = connection
        .prepare(
            "SELECT document, page, x0, y0, x1, y1, unicode, latex, problem, problem_lines,
                source, confidence, mathlib_name, mathlib_score, mathlib_domain, mathlib_rev,
                classifier, paper_digest, region_digest
         FROM formula WHERE document = ?1 ORDER BY page, y0, x0",
        )
        .map_err(|error| Error::invalid("the formula query", error))?;
    let rows = statement
        .query_map(params![document], |row| {
            Ok(FormulaRecord {
                document: row.get(0)?,
                page: row.get(1)?,
                x0: row.get(2)?,
                y0: row.get(3)?,
                x1: row.get(4)?,
                y1: row.get(5)?,
                unicode: row.get(6)?,
                latex: row.get(7)?,
                problem: row.get(8)?,
                problem_lines: row.get(9)?,
                source: row.get(10)?,
                confidence: row.get(11)?,
                mathlib_name: row.get(12)?,
                mathlib_score: row.get(13)?,
                mathlib_domain: row.get(14)?,
                mathlib_rev: row.get(15)?,
                classifier: row.get(16)?,
                paper_digest: row.get(17)?,
                region_digest: row.get(18)?,
            })
        })
        .map_err(|error| Error::invalid("the formula rows", error))?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(|error| {
        Error::invalid(
            "the formulas of a document",
            format_args!("{document}: {error}"),
        )
    })
}

/// How a formula is cited: `paper.pdf:page 12, formula at (120,340)-(300,388)`.
///
/// A box is what a reader can find on a page that has no lines, and rounding to whole points is what a
/// viewer shows when a person selects the region.
#[must_use]
pub fn cite(formula: &FormulaRecord) -> String {
    format!(
        "{}:page {}, formula at ({:.0},{:.0})-({:.0},{:.0})",
        formula.document, formula.page, formula.x0, formula.y0, formula.x1, formula.y1
    )
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::Store;

    fn formula(unicode: &str, x0: f64) -> FormulaRecord {
        let place = Place {
            document: "paper.pdf".to_owned(),
            page: 12,
            x0,
            y0: 340.0,
            x1: x0 + 80.0,
            y1: 352.0,
            paper_digest: "aa11".to_owned(),
            region_digest: "bb22".to_owned(),
        };
        FormulaRecord::from_text_layer(&place, unicode)
    }

    #[test]
    fn a_formula_round_trips_with_its_provenance() {
        let mut store = Store::open_in_memory().expect("a store");
        store
            .record_formula(&formula("∫ f dx = F", 120.0))
            .expect("recorded");
        let rows = store.formulas("paper.pdf").expect("read");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].unicode, "∫ f dx = F");
        assert_eq!(rows[0].source, "deterministic/text-layer");
        // Nothing was invented: no LaTeX, no classification, no confidence.
        assert_eq!(rows[0].latex, None);
        assert_eq!(rows[0].mathlib_name, None);
        assert_eq!(rows[0].confidence, None);
        assert_eq!(rows[0].paper_digest, "aa11");
        assert_eq!(rows[0].region_digest, "bb22");
        // The citation a hit prints: a box on a page, rounded to whole points.
        assert_eq!(
            cite(&rows[0]),
            "paper.pdf:page 12, formula at (120,340)-(200,352)"
        );
    }

    #[test]
    fn the_region_is_the_key_so_a_rereading_replaces_rather_than_duplicates() {
        // A classifier run must update the row, not add a second reading of the same box.
        let mut store = Store::open_in_memory().expect("a store");
        store
            .record_formula(&formula("∫ f dx", 120.0))
            .expect("recorded");
        let mut classified = formula("∫ f dx", 120.0);
        classified.latex = Some("\\int f\\,dx".to_owned());
        classified.mathlib_name = Some("MeasureTheory.integral".to_owned());
        classified.mathlib_score = Some(0.9);
        classified.mathlib_domain = Some("Analysis".to_owned());
        classified.source = "model/texify/0.2".to_owned();
        store.record_formula(&classified).expect("recorded again");
        let rows = store.formulas("paper.pdf").expect("read");
        assert_eq!(rows.len(), 1, "the same box became two rows");
        assert_eq!(rows[0].latex.as_deref(), Some("\\int f\\,dx"));
        assert_eq!(rows[0].mathlib_domain.as_deref(), Some("Analysis"));

        // A different box is a different formula.
        store
            .record_formula(&formula("∑ x", 400.0))
            .expect("recorded");
        assert_eq!(store.formulas("paper.pdf").expect("read").len(), 2);
        // And an unrelated document is not in this one's list.
        assert_eq!(store.formulas("other.pdf").expect("read").len(), 0);
    }
}
