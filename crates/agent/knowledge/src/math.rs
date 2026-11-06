//! The atom of formula detection: deciding whether a run of text is mathematics.
//!
//! `docs/design/mathematics.md` sets the pipeline: segmentation from the text layer, layout, then an
//! optional formula model. This module is the first layer's *evidence* — the test that is applied to
//! every glyph run before anything is stored — and it is deliberately small and deterministic, because
//! everything downstream inherits its mistakes.
//!
//! Three kinds of evidence, in the order a reader uses them:
//!
//! 1. **The characters.** A run that contains `∫`, `∑`, `∂`, `√`, `≤`, `≠` or a Greek letter is
//!    mathematics or it is a physicist's prose; either way it is not ordinary text, and the Unicode
//!    block it came from is named so a citation can say why.
//! 2. **The font.** Papers set mathematics in a different face — `CMMI`, `MathematicalPi`, `Symbol`,
//!    `STIXMath`, `CambriaMath`, a `-Italic` variant of the body font. The font name travels with the
//!    glyph, so this is evidence a scan cannot give and a text layer can.
//! 3. **The baseline.** Sub- and superscripts sit off the line by a fraction of the type size. A run
//!    with three baseline shifts is a formula even when every character in it is Latin.
//!
//! Nothing here decides *where* a formula starts and ends; that is the segmenter's job. This decides
//! what counts as evidence, and it is a pure function so a test can pin it character by character.

/// The Unicode blocks mathematics is written in, with the name a citation uses.
///
/// The ranges are the published block boundaries (Unicode 15.1, chapter 22 and the code-chart names),
/// not guesses: a wrong range would silently classify a symbol as prose, which is the failure this
/// module exists to prevent.
pub const BLOCKS: [(&str, char, char); 7] = [
    ("Greek and Coptic", '\u{0370}', '\u{03FF}'),
    ("Letterlike Symbols", '\u{2100}', '\u{214F}'),
    ("Number Forms", '\u{2150}', '\u{218F}'),
    ("Arrows", '\u{2190}', '\u{21FF}'),
    ("Mathematical Operators", '\u{2200}', '\u{22FF}'),
    ("Miscellaneous Technical", '\u{2300}', '\u{23FF}'),
    (
        "Supplemental Mathematical Operators",
        '\u{2A00}',
        '\u{2AFF}',
    ),
];

/// The characters that make a run mathematics even outside those blocks.
///
/// A digit string is not mathematics, and `=` appears in prose ("the answer = 4"), so neither is here.
/// What is here cannot be read as a sentence: an operator, a relation that is not `=`, or a symbol.
const OPERATORS: &str = "∑∏∫∮∂∇√∛∜∞≈≠≤≥≪≫∝∈∉⊂⊃⊆⊇∪∩∅⊕⊗⊥∥∠°′″¯^±−×·";

/// Font-name fragments that say "this face is for mathematics".
///
/// Compared case-insensitively against the run's font names. `cm` alone is not included: it would match
/// `Cmr10` (which *is* Computer Modern) but also any font with those two letters, and a false positive
/// here turns prose into a formula.
const MATH_FONTS: [&str; 15] = [
    "cmmi",
    "cmsy",
    "cmex",
    "cmr",
    "cmb",
    "symbol",
    "mathpi",
    "stixmath",
    "stixtwomath",
    "cambriamath",
    "asana",
    "xitsmath",
    "latinmodern-math",
    "dejavusansmath",
    "mtex",
];

/// One glyph run's evidence, before anything is decided.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Evidence {
    /// How many characters came from a mathematical block or are operators.
    pub math_characters: usize,
    /// The blocks the run used, by name, for the citation's "why".
    pub blocks: Vec<&'static str>,
    /// How many glyphs were drawn in a face named for mathematics.
    pub math_font_glyphs: usize,
    /// The distinct baselines in the run, which is what a sub- or superscript adds.
    pub baselines: usize,
    /// How many characters the run holds at all.
    pub characters: usize,
}

impl Evidence {
    /// Gather evidence from a run's text and the font names its glyphs were drawn with.
    #[must_use]
    pub fn of(text: &str, fonts: &[&str], baselines: usize) -> Self {
        let mut evidence = Self {
            baselines,
            characters: text.chars().count(),
            ..Self::default()
        };
        for character in text.chars() {
            if OPERATORS.contains(character) {
                evidence.math_characters += 1;
                if !evidence.blocks.contains(&"Operators") {
                    evidence.blocks.push("Operators");
                }
                continue;
            }
            if let Some((name, _, _)) = BLOCKS
                .iter()
                .find(|(_, low, high)| character >= *low && character <= *high)
            {
                evidence.math_characters += 1;
                if !evidence.blocks.contains(name) {
                    evidence.blocks.push(name);
                }
            }
        }
        evidence.math_font_glyphs = fonts.iter().filter(|font| is_math_font(font)).count();
        evidence
    }

    /// Whether the evidence is enough to call the run mathematics.
    ///
    /// The rule is deliberately conservative: **one** operator, or **two** characters from a
    /// mathematical block, or a mathematics face, or three baselines. A single Greek letter is how a
    /// paper writes a variable in prose, so one block character is not enough — a run of two is.
    #[must_use]
    pub fn is_mathematics(&self) -> bool {
        let operators = self.blocks.contains(&"Operators");
        operators || self.math_characters >= 2 || self.math_font_glyphs > 0 || self.baselines >= 3
    }

    /// A one-sentence reason, for the provenance chain: *why* this run was called mathematics.
    #[must_use]
    pub fn why(&self) -> String {
        let mut reasons: Vec<String> = Vec::new();
        if self.blocks.contains(&"Operators") {
            reasons.push("a mathematical operator".to_owned());
        } else if self.math_characters > 0 {
            reasons.push(format!(
                "{} character(s) from {}",
                self.math_characters,
                self.blocks.join(", ")
            ));
        }
        if self.math_font_glyphs > 0 {
            reasons.push(format!(
                "{} glyph(s) in a mathematical face",
                self.math_font_glyphs
            ));
        }
        if self.baselines >= 3 {
            reasons.push(format!("{} distinct baselines", self.baselines));
        }
        if reasons.is_empty() {
            return "no evidence".to_owned();
        }
        reasons.join("; ")
    }
}

/// Whether a font name says it is a mathematics face.
#[must_use]
pub fn is_math_font(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    MATH_FONTS.iter().any(|fragment| lower.contains(fragment))
        // A subscript or superscript face is mathematics wherever it appears.
        || lower.contains("sub") && lower.contains("mat")
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn an_operator_is_mathematics_on_its_own() {
        // `∫ f dx` is a formula however it is set: no sentence contains a contour integral by accident.
        let evidence = Evidence::of("∫ f dx", &["Times-Roman"], 1);
        assert!(evidence.is_mathematics(), "{}", evidence.why());
        assert_eq!(evidence.blocks, vec!["Operators"]);
        assert_eq!(evidence.why(), "a mathematical operator");
    }

    #[test]
    fn one_greek_letter_is_a_variable_and_not_yet_a_formula() {
        // The conservative rule that keeps a paper's prose out of the formula store: `α` alone is how a
        // sentence writes a variable. Two of them, or an operator, is a formula.
        let single = Evidence::of("the angle α", &["Times-Roman"], 1);
        assert!(!single.is_mathematics(), "{}", single.why());
        let pair = Evidence::of("α and β", &["Times-Roman"], 1);
        assert!(pair.is_mathematics(), "{}", pair.why());
        assert!(pair.why().contains("Greek and Coptic"), "{}", pair.why());
    }

    #[test]
    fn a_mathematical_face_is_evidence_a_citation_can_name() {
        let evidence = Evidence::of("f x", &["CMMI10"], 1);
        assert!(evidence.is_mathematics());
        assert_eq!(evidence.why(), "1 glyph(s) in a mathematical face");
        // The control: the same text in a body face is prose.
        assert!(!Evidence::of("f x", &["Times-Roman"], 1).is_mathematics());
        assert!(is_math_font("STIXTwoMath-Regular"));
        assert!(is_math_font("CambriaMath"));
        assert!(!is_math_font("Helvetica"));
        // `cm` alone is not a fragment: `Cmr10` matches, but so would any name containing those letters.
        assert!(is_math_font("CMR10"));
    }

    #[test]
    fn baseline_shifts_are_enough_for_all_latin_evidence() {
        // x² + y² has no operator and no Greek: its evidence is that three baselines were used.
        let evidence = Evidence::of("x2 y2", &["Times-Roman"], 3);
        assert!(evidence.is_mathematics());
        assert!(
            evidence.why().contains("3 distinct baselines"),
            "{}",
            evidence.why()
        );
        assert!(!Evidence::of("x2 y2", &["Times-Roman"], 2).is_mathematics());
    }

    #[test]
    fn the_blocks_cover_the_ranges_the_unicode_charts_name() {
        // A wrong range silently classifies a symbol as prose; these are the boundaries themselves.
        assert!(BLOCKS.contains(&("Mathematical Operators", '\u{2200}', '\u{22FF}')));
        for (name, low, high) in BLOCKS {
            let evidence = Evidence::of(&format!("{low}{high}"), &["Times-Roman"], 1);
            assert!(evidence.blocks.contains(&name), "{name} was not found");
        }
        // Prose with a digit and an equals sign is not mathematics.
        assert!(!Evidence::of("the answer = 4", &["Times-Roman"], 1).is_mathematics());
    }
}
