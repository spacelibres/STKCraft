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

/// The characters that say "this line is an equation" when the line is set as display layout.
///
/// These are the ASCII relations a paper sets an equation in when it does not use mathematical Unicode —
/// `r = a (1 - e cos E)` in Times is the case that made them necessary. They are not evidence *alone*:
/// prose contains `=`, so they count only when the layout already says display.
///
/// `-`, `/` and `*` are deliberately **not** here. They appear in every header, every date and every
/// range, and including them turned each standard's page header into a "formula" (three different
/// documents reported the same centred header line). A relation that cannot be prose is the only useful
/// one.
const RELATIONS: &str = "=+<>^±×÷≠≈≤≥";

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
    /// How many ASCII relations and operators it holds: evidence only beside a display layout.
    pub relations: usize,
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
            // A character can be both: `±` is a relation *and* an operator, and counting it only as a
            // relation would lose the layer-1 evidence it carries.
            if RELATIONS.contains(character) {
                evidence.relations += 1;
            }
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

/// One glyph run as the segmenter sees it: a piece of a line with its box and its evidence.
///
/// The PDF layer already produces these — `crates/agent/knowledge/src/pdf.rs` groups glyphs into
/// fragments with an `x` range, a baseline and an em size — and this type is the seam that keeps the
/// segmenter testable without a PDF: a test builds runs by hand and asserts the regions.
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    /// The one-based page it is on.
    pub page: usize,
    /// Its text, with the word breaks the generator positioned put back.
    pub text: String,
    /// Its left and right edge, in page points.
    pub x0: f64,
    /// Its right edge.
    pub x1: f64,
    /// Its baseline, in page points (y grows downwards).
    pub y: f64,
    /// The type size, in points.
    pub em: f64,
    /// The faces its glyphs were drawn with.
    pub fonts: Vec<String>,
    /// How many distinct baselines its glyphs used.
    pub baselines: usize,
    /// Whether the page lays this run out as display mathematics: centred, or indented off the body
    /// margin, and spaced away from the text around it.
    ///
    /// This is layer 2 of `docs/design/mathematics.md`: a property of the page rather than of the glyphs,
    /// and the only thing that can see an equation set in the body face.
    pub display: bool,
}

impl Run {
    /// The evidence this run carries.
    #[must_use]
    pub fn evidence(&self) -> Evidence {
        let fonts: Vec<&str> = self.fonts.iter().map(String::as_str).collect();
        Evidence::of(&self.text, &fonts, self.baselines)
    }
}

/// A region of page that the segmenter believes is mathematics.
#[derive(Debug, Clone, PartialEq)]
pub struct Region {
    /// The page it is on.
    pub page: usize,
    /// The box, in page points: the union of the runs it holds.
    pub x0: f64,
    /// The top of the box.
    pub y0: f64,
    /// The right of the box.
    pub x1: f64,
    /// The bottom of the box.
    pub y1: f64,
    /// The text the text layer gives for the region, runs joined in reading order.
    pub unicode: String,
    /// Why it was called mathematics: the evidence of its strongest run.
    pub evidence: Evidence,
    /// How many runs it holds.
    pub runs: usize,
}

/// How far apart two runs on one baseline may be and still belong to one region, in ems.
///
/// A space is a quarter of an em and a relation is set off by about half; a gap wider than this is a
/// column break or a different table cell, not one formula.
pub const JOIN_GAP_EM: f64 = 2.0;

/// How far a line may sit from the region above it and still continue it, in ems.
///
/// A display equation is often two lines — a fraction, or a matrix — and the gap between them is a
/// line's worth of leading, not a paragraph's.
pub const CONTINUE_GAP_EM: f64 = 2.2;

/// Group runs into the regions that are mathematics.
///
/// The rules, in order of application:
///
/// 1. A run that **is** mathematics starts a region, or joins the open one when it is close enough —
///    same baseline within half an em and no further than [`JOIN_GAP_EM`] away, or on a line within
///    [`CONTINUE_GAP_EM`] of the region's last line, which is how a formula split across two lines is
///    kept whole.
/// 2. A run that **is not** mathematics joins an open region only when it is adjacent on the same
///    baseline — `=` and a variable are set in the body face in many papers, and dropping them would
///    cut a formula in half.
/// 3. A region closes when nothing has joined it and the next run starts a new one, or when the page
///    changes.
///
/// A region always holds at least one run that *is* mathematics: prose with a passing symbol in it is
/// prose, and that is decided by [`Evidence::is_mathematics`], not here.
#[must_use]
pub fn segment(runs: &[Run]) -> Vec<Region> {
    let mut regions: Vec<Region> = Vec::new();
    let mut current: Option<Region> = None;
    let mut current_page = 0_usize;
    for run in runs {
        let evidence = run.evidence();
        // Layer 1 says "these glyphs are mathematics"; layer 2 says "the page sets this line apart and
        // the line holds relations". Either is enough to open a region, and the second is what finds an
        // equation typed in the body face.
        let mathematical = evidence.is_mathematics() || (run.display && evidence.relations > 0);
        let joins = current.as_ref().is_some_and(|open| {
            if open.page != run.page {
                return false;
            }
            let em = run.em.max(1.0);
            let overlap =
                run.x0 <= open.x1 + JOIN_GAP_EM * em && run.x1 >= open.x0 - JOIN_GAP_EM * em;
            if !overlap {
                return false;
            }
            // Same baseline, close enough to be one line of one formula.
            let on_line =
                (run.y - open.y1).abs() <= 0.5 * em && run.x0 - open.x1 <= JOIN_GAP_EM * em;
            // Or the next line of a display formula: a fraction, a matrix, a continued equation.
            let next_line = run.y - open.y1 > 0.0 && run.y - open.y1 <= CONTINUE_GAP_EM * em;
            on_line || next_line
        });
        if let Some(region) = current.take() {
            if mathematical && !joins || current_page != run.page {
                regions.push(region);
            } else if joins {
                let mut region = region;
                region.x0 = region.x0.min(run.x0);
                region.x1 = region.x1.max(run.x1);
                region.y1 = region.y1.max(run.y);
                region.y0 = region.y0.min(run.y);
                if !region.unicode.is_empty() {
                    region.unicode.push(' ');
                }
                region.unicode.push_str(&run.text);
                region.runs += 1;
                let evidence = run.evidence();
                if evidence.math_characters > region.evidence.math_characters {
                    region.evidence = evidence;
                }
                current = Some(region);
                current_page = run.page;
                continue;
            } else {
                regions.push(region);
            }
        }
        if mathematical {
            current = Some(Region {
                page: run.page,
                x0: run.x0,
                x1: run.x1,
                y0: run.y,
                y1: run.y,
                unicode: run.text.clone(),
                evidence: run.evidence(),
                runs: 1,
            });
            current_page = run.page;
        }
    }
    if let Some(region) = current {
        regions.push(region);
    }
    regions
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

    /// A run at a stated place, with the faces a test wants to claim.
    fn run(page: usize, text: &str, x0: f64, x1: f64, y: f64, em: f64, font: &str) -> Run {
        Run {
            page,
            text: text.to_owned(),
            x0,
            x1,
            y,
            em,
            fonts: vec![font.to_owned()],
            baselines: 1,
            display: false,
        }
    }

    #[test]
    fn an_equation_in_the_body_face_is_found_by_its_layout() {
        // The ODM book's case: `r = a (1 - e cos E)` in Times, no operator, no Greek, no baseline shift.
        // The characters alone cannot see it; the page's layout and its relations can.
        let mut equation = run(
            1,
            "r = a (1 - e cos E)",
            220.0,
            380.0,
            400.0,
            12.0,
            "TimesNewRomanPSMT",
        );
        equation.display = true;
        let regions = segment(&[equation]);
        assert_eq!(regions.len(), 1, "the display layout found nothing");

        // The control: the same text not set as display is prose, and a display line with no relation
        // (`Range and geometry`, a heading) is prose too.
        let prose = run(
            1,
            "r = a (1 - e cos E)",
            220.0,
            380.0,
            400.0,
            12.0,
            "TimesNewRomanPSMT",
        );
        assert_eq!(segment(&[prose]), Vec::new());
        let mut heading = run(
            1,
            "Range and geometry",
            220.0,
            380.0,
            400.0,
            12.0,
            "Times-Bold",
        );
        heading.display = true;
        assert_eq!(segment(&[heading]), Vec::new());
    }

    #[test]
    fn a_display_equation_becomes_one_region_with_its_box() {
        // `∫ f dx = F` set with a maths face on the operator and the body face elsewhere: the equals
        // sign and the prose face must not cut the formula in half.
        let runs = [
            run(1, "∫", 120.0, 128.0, 340.0, 12.0, "CMMI10"),
            run(1, "f dx", 130.0, 176.0, 340.0, 12.0, "CMMI10"),
            run(1, "=", 180.0, 190.0, 340.0, 12.0, "Times-Roman"),
            run(1, "F", 194.0, 202.0, 340.0, 12.0, "CMMI10"),
        ];
        let regions = segment(&runs);
        assert_eq!(regions.len(), 1, "{regions:#?}");
        let region = &regions[0];
        assert_eq!(region.page, 1);
        assert_eq!(region.runs, 4);
        assert_eq!(region.unicode, "∫ f dx = F");
        assert!((region.x0 - 120.0).abs() < f64::EPSILON);
        assert!((region.x1 - 202.0).abs() < f64::EPSILON);
        assert_eq!(region.evidence.blocks, vec!["Operators"]);
    }

    #[test]
    fn prose_with_one_greek_letter_makes_no_region() {
        // The rule the whole design turns on: a sentence mentioning `α` is a sentence.
        let runs = [
            run(1, "the angle", 120.0, 180.0, 200.0, 12.0, "Times-Roman"),
            run(1, "α", 184.0, 192.0, 200.0, 12.0, "Times-Roman"),
            run(1, "is small", 196.0, 250.0, 200.0, 12.0, "Times-Roman"),
        ];
        assert_eq!(segment(&runs), Vec::new());
    }

    #[test]
    fn two_equations_on_one_page_stay_two_regions() {
        // Vertical distance ends a region: two display equations 40 points apart are two formulas.
        let runs = [
            run(1, "∑", 120.0, 130.0, 200.0, 12.0, "CMMI10"),
            run(1, "x", 134.0, 142.0, 200.0, 12.0, "CMMI10"),
            run(1, "∫", 120.0, 130.0, 240.0, 12.0, "CMMI10"),
            run(1, "y", 134.0, 142.0, 240.0, 12.0, "CMMI10"),
        ];
        let regions = segment(&runs);
        assert_eq!(regions.len(), 2, "{regions:#?}");
        assert_eq!(regions[0].unicode, "∑ x");
        assert_eq!(regions[1].unicode, "∫ y");
    }

    #[test]
    fn a_formula_continued_on_the_next_line_is_one_region() {
        // A fraction or a matrix is set above and below one baseline: the continuation gap keeps it.
        let runs = [
            run(1, "∑", 120.0, 130.0, 200.0, 12.0, "CMMI10"),
            run(1, "n", 134.0, 140.0, 214.0, 12.0, "CMMI10"),
            run(1, "2", 120.0, 126.0, 228.0, 12.0, "CMMI10"),
        ];
        let regions = segment(&runs);
        assert_eq!(regions.len(), 1, "{regions:#?}");
        assert_eq!(regions[0].runs, 3);
        assert!((regions[0].y0 - 200.0).abs() < f64::EPSILON);
        assert!((regions[0].y1 - 228.0).abs() < f64::EPSILON);
    }

    #[test]
    fn a_region_never_crosses_a_page() {
        let runs = [
            run(1, "∫", 120.0, 130.0, 700.0, 12.0, "CMMI10"),
            run(2, "dx", 120.0, 140.0, 40.0, 12.0, "CMMI10"),
        ];
        let regions = segment(&runs);
        assert_eq!(regions.len(), 2, "{regions:#?}");
        assert_eq!(regions[0].page, 1);
        assert_eq!(regions[1].page, 2);
    }
}
