//! Reading a paper out of a PDF, page by page, so a citation can name the page.
//!
//! Papers arrive as PDFs, and a knowledge base that cannot open a PDF is a knowledge base that
//! cannot hold a paper. The family already owns the reader — `craft-pdf` over the patched `hayro`
//! backends, the same pair `pdfcraft` reads with — so this module is not a PDF parser. It is the one
//! thing a parser cannot give: **text placed where a person can find it**.
//!
//! # Why a device, and not a text dump
//!
//! A PDF has no lines. It has glyphs, each painted at a position with a font and a size, and the
//! order they were painted in is the order the *generator* wrote them — which for a two-column paper
//! is column-interleaved or worse. Reading the content stream in order therefore produces text no
//! human wrote, and a citation into it points at a paragraph that does not exist.
//!
//! So this module implements the interpreter's [`Device`] trait and receives every glyph the page
//! paints, with the matrix that put it there. That matrix gives the glyph's origin and its scale, and
//! from those two numbers the glyphs group into lines the way a reader sees them:
//!
//! - **a line** is a run of glyphs whose baselines agree to within a fraction of the type size;
//! - **a word break** is a horizontal gap wider than a fraction of the type size, which is how a
//!   space the generator expressed as *positioning* rather than as a space character is put back;
//! - **a paragraph break** is a vertical gap wider than the line pitch, which is what lets the
//!   chunker keep a paragraph together instead of cutting the page into fixed slices.
//!
//! Those three heuristics are the whole of it, and they are stated here because they are the part
//! that can be wrong: a page set in a font with unusual metrics may put a line break in the wrong
//! place. The citation still names the right page, and the text is still the text the page draws —
//! which is more than an image-only page can offer, and why such a page is reported rather than
//! silently indexed as nothing.

use craft_pdf::interpret::{
    BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache,
    InterpreterSettings, Paint, PathDrawMode, RectExt, SoftMask, TransformExt, font::Glyph,
    hayro_cmap::BfString,
};
use craft_pdf::{OpenOptions, open_slice_with_options};
use kurbo::{Affine, BezPath};

/// How much of a PDF to read, checked before the reader is handed the file.
///
/// Every field bounds work a file from outside this repository can ask for, and each is separately
/// reachable so a caller — and a test — can hold one of them down without the others.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    /// The largest file accepted, in bytes.
    pub max_bytes: usize,
    /// The most pages read.
    pub max_pages: usize,
    /// The most glyphs placed.
    pub max_glyphs: usize,
    /// The most characters kept.
    pub max_characters: usize,
}

impl Default for Bounds {
    /// The published defaults: a paper-sized file, and room for a proceedings volume.
    fn default() -> Self {
        Self {
            // A paper is a megabyte; a scanned one can be fifty. Beyond that a person is pointing the
            // knowledge base at an archive rather than at a document.
            max_bytes: 64 << 20,
            // Four thousand pages is a book, a thesis, or a whole conference proceedings.
            max_pages: 4_000,
            // A page of body text places two to four thousand glyphs.
            max_glyphs: 4_000_000,
            // Thirty-two million characters is the text of every page above, with room to spare.
            max_characters: 32 << 20,
        }
    }
}

/// One page's text, with the page number it is cited by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    /// The one-based page number, as a reader counts pages.
    pub number: usize,
    /// The lines found on the page, in reading order, with blank lines between paragraphs.
    pub text: String,
}

/// What a PDF yielded.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Extracted {
    /// The pages read, in order. A page with no text is here too, with empty text.
    pub pages: Vec<Page>,
    /// How many glyphs were placed.
    pub glyphs: usize,
    /// How many glyphs had no Unicode mapping, so their text could not be recovered.
    pub unnamed: usize,
    /// How many pages were left unread because a bound was reached.
    pub skipped: usize,
}

impl Extracted {
    /// The characters of text recovered, which is how a caller tells a readable paper from a scan.
    #[must_use]
    pub fn characters(&self) -> usize {
        self.pages.iter().map(|page| page.text.len()).sum()
    }

    /// The page texts as `(page number, text)`, the form [`crate::chunk::chunk_pages`] takes.
    #[must_use]
    pub fn passages(&self) -> Vec<(usize, String)> {
        self.pages
            .iter()
            .map(|page| (page.number, page.text.clone()))
            .collect()
    }
}

/// Why a PDF could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PdfError {
    /// The file is larger than the caller's limit.
    TooLarge {
        /// The limit, in bytes.
        limit: usize,
    },
    /// The reader rejected the file: not a PDF, truncated, or encrypted with a password.
    Open(String),
    /// The file parsed, but a page could not be interpreted.
    Interpret,
    /// The file has no pages at all.
    NoPages,
}

impl std::fmt::Display for PdfError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge { limit } => write!(formatter, "larger than the {limit}-byte limit"),
            Self::Open(reason) => formatter.write_str(reason),
            Self::Interpret => formatter.write_str("a page could not be interpreted"),
            Self::NoPages => formatter.write_str("the document has no pages"),
        }
    }
}

impl std::error::Error for PdfError {}

/// Whether `bytes` starts with a PDF header.
///
/// Content decides, not the extension: a `.pdf` that is not one and a PDF named `.txt` both exist,
/// and the reader is what should find out.
#[must_use]
pub fn looks_like_pdf(bytes: &[u8]) -> bool {
    bytes.starts_with(b"%PDF-")
}

/// Extract the text of every page, with `max_bytes` as the only lowered bound.
///
/// # Errors
///
/// [`PdfError`] when the file is too large, is not a PDF, is encrypted, has no pages, or a page cannot
/// be interpreted. A page that draws no text is not an error: it is a page without text, and
/// [`Extracted::characters`] is how a caller tells a scan from a readable paper.
pub fn extract(bytes: &[u8], max_bytes: usize) -> Result<Extracted, PdfError> {
    extract_with(
        bytes,
        &Bounds {
            max_bytes,
            ..Bounds::default()
        },
    )
}

/// Extract the text of every page, with explicit bounds.
///
/// # Errors
///
/// As [`extract`].
pub fn extract_with(bytes: &[u8], bounds: &Bounds) -> Result<Extracted, PdfError> {
    if bytes.len() > bounds.max_bytes {
        return Err(PdfError::TooLarge {
            limit: bounds.max_bytes,
        });
    }
    let options = OpenOptions {
        max_input_bytes: bounds.max_bytes,
        ..OpenOptions::default()
    };
    let pdf = open_slice_with_options(bytes, None, &options)
        .map_err(|error| PdfError::Open(error.to_string()))?;
    let pages = pdf.pages();
    if pages.is_empty() {
        return Err(PdfError::NoPages);
    }
    // One cache for the whole document: the font one page uses is usually the font the page before it
    // used, and re-parsing it per page is what makes a naive extractor quadratic.
    let cache = InterpreterCache::new();
    let mut extracted = Extracted {
        skipped: pages.len().saturating_sub(bounds.max_pages),
        ..Extracted::default()
    };
    for (index, page) in pages.iter().take(bounds.max_pages).enumerate() {
        let room = Budget {
            glyphs: bounds.max_glyphs.saturating_sub(extracted.glyphs),
            characters: bounds.max_characters.saturating_sub(extracted.characters()),
        };
        if room.is_exhausted() {
            // Out of budget before this page: the rest of the document is unread, and the count says
            // so rather than pretending the document ended here.
            extracted.skipped = pages.len() - index;
            break;
        }
        let mut context = Context::new(
            page.initial_transform(true).to_kurbo(),
            page.media_box().to_kurbo(),
            &cache,
            page.xref(),
            InterpreterSettings::default(),
        );
        let mut device = TextDevice::new(room);
        craft_pdf::interpret_page(page, &mut context, &mut device)
            .map_err(|_| PdfError::Interpret)?;
        extracted.glyphs += device.glyphs;
        extracted.unnamed += device.unnamed;
        let exhausted = device.exhausted;
        extracted.pages.push(Page {
            number: index + 1,
            text: device.into_text(),
        });
        if exhausted {
            // The page was cut short, so everything after it is unread.
            extracted.skipped = pages.len() - index - 1;
            break;
        }
    }
    Ok(extracted)
}

/// What a device may still accept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Budget {
    /// Glyphs left.
    glyphs: usize,
    /// Characters left.
    characters: usize,
}

impl Budget {
    /// Whether there is nothing left to accept.
    fn is_exhausted(&self) -> bool {
        self.glyphs == 0 || self.characters == 0
    }
}

/// One glyph the page painted, with where it was and how large it was.
#[derive(Debug, Clone)]
struct Placed {
    /// The glyph's origin, left to right, in the page's device space.
    x: f64,
    /// The glyph's origin, top to bottom: the page matrix arrives y-inverted, so y grows downwards.
    y: f64,
    /// The type size in device units — one em — so distances in x and y can be compared with it.
    ///
    /// The matrix the interpreter hands over maps *glyph* units (a font's em, which the outline
    /// interface normalises to 1000) into device units, so its scale is the type size over a thousand.
    em: f64,
    /// How far the pen moves after this glyph, in the same thousand-units-per-em, when the font says.
    advance: Option<f64>,
    /// The text the glyph stands for.
    text: String,
}

/// A run of glyphs on one baseline: a fragment of a text line, narrower than a column.
///
/// A page is not a sequence of lines, it is a rectangle of ink. Splitting a baseline at the wide
/// gaps turns it into fragments that belong to one column or one table cell, and those can be ordered
/// the way the page is read — left column to the bottom, then the right column — instead of the way
/// the baselines happen to line up.
#[derive(Debug)]
struct Span {
    /// The leftmost x.
    x0: f64,
    /// The rightmost x, being the end of the last glyph's advance.
    x1: f64,
    /// The baseline's y, for ordering and for spotting a paragraph break.
    y: f64,
    /// The largest type size in the fragment, which the other heuristics are relative to.
    em: f64,
    /// The fragment's text, with the word breaks the generator positioned put back.
    text: String,
}

/// How wide a horizontal gap has to be, in ems, before a baseline is two fragments rather than one.
///
/// A word space is a quarter of an em and a justified line widens it by at most another quarter, so
/// anything past a whole em is not a space inside a line: it is the gutter beside it.
const FRAGMENT_GAP_EM: f64 = 1.2;

/// The narrowest gutter, in ems, a page must have before it is read in columns.
const MIN_GUTTER_EM: f64 = 0.8;

/// The share of a page's fragments each side of a gutter must hold for it to be a column break.
///
/// A table row can leave a wide gap too, and a gutter that splits a page fourteen to two is a table,
/// not two columns.
const MIN_COLUMN_SHARE: f64 = 0.25;

/// How deep the column split recurses, so a three- or four-column page still reads column by column
/// while a pathological page cannot recurse for ever.
const MAX_COLUMN_DEPTH: usize = 3;

/// The interpreter device: it paints nothing and remembers every glyph it was asked to paint.
struct TextDevice {
    /// The glyphs, in the order the page painted them.
    placed: Vec<Placed>,
    /// How many glyphs had no Unicode mapping.
    unnamed: usize,
    /// What it may still accept.
    budget: Budget,
    /// Whether it stopped early because the budget ran out.
    exhausted: bool,
    /// How many glyphs it placed, kept because `placed` is consumed when the text is assembled.
    glyphs: usize,
}

impl TextDevice {
    /// A device that accepts what `budget` allows.
    fn new(budget: Budget) -> Self {
        Self {
            placed: Vec::new(),
            unnamed: 0,
            budget,
            exhausted: false,
            glyphs: 0,
        }
    }

    /// The page's text: lines top to bottom, paragraph breaks as blank lines.
    fn into_text(self) -> String {
        let spans = order(fragments(self.placed), MAX_COLUMN_DEPTH);
        let mut text = String::new();
        let mut current: Option<(f64, f64, String)> = None;
        let mut last: Option<(f64, f64)> = None;
        for span in spans {
            let same_line = current
                .as_ref()
                .is_some_and(|(y, em, _)| (span.y - *y).abs() <= 0.4 * em.max(span.em));
            if same_line {
                if let Some((_, em, assembled)) = current.as_mut() {
                    // The wide gap that made this a fragment was a space: the two runs are one line.
                    if !assembled.ends_with(' ') && !span.text.starts_with(' ') {
                        assembled.push(' ');
                    }
                    assembled.push_str(&span.text);
                    *em = em.max(span.em);
                }
            } else {
                if let Some((y, em, assembled)) = current.take() {
                    emit(&mut text, &mut last, y, em, &assembled);
                }
                current = Some((span.y, span.em, span.text));
            }
        }
        if let Some((y, em, assembled)) = current {
            emit(&mut text, &mut last, y, em, &assembled);
        }
        text
    }
}

impl<'a> Device<'a> for TextDevice {
    fn set_soft_mask(&mut self, _: Option<SoftMask<'a>>) {}
    fn set_blend_mode(&mut self, _: BlendMode) {}
    fn draw_path(&mut self, _: &BezPath, _: Affine, _: &Paint<'a>, _: &PathDrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn pop_clip_path(&mut self) {}
    fn pop_transparency_group(&mut self) {}
    fn draw_image(&mut self, _: Image<'a, '_>, _: Affine) {}

    fn draw_glyph(
        &mut self,
        glyph: &Glyph<'a>,
        transform: Affine,
        glyph_transform: Affine,
        _: &Paint<'a>,
        _: &GlyphDrawMode,
    ) {
        // Every drawing mode is text: `Invisible` is how a scanned page carries its OCR layer, and
        // `Clip` is text used as a clip path. Both are text a reader wants extracted.
        let Some(text) = glyph.as_unicode().map(|unicode| match unicode {
            BfString::Char(character) => character.to_string(),
            BfString::String(text) => text,
        }) else {
            self.unnamed += 1;
            return;
        };
        if self.budget.is_exhausted() {
            self.exhausted = true;
            return;
        }
        // The matrix that put this glyph here: its translation is the origin and its scale is the
        // type size. A matrix that gives neither (degenerate, or a non-finite number from a hostile
        // file) is dropped rather than turned into a NaN position, which would sort unpredictably and
        // split every line around it.
        let matrix = transform * glyph_transform;
        let [a, b, c, d, tx, ty] = matrix.as_coeffs();
        // The mean of the two axis lengths: a text matrix scales x and y alike, and a rotation keeps
        // the length, so this is the type size in device units per glyph unit.
        let scale = f64::midpoint(a.hypot(b), c.hypot(d));
        let em = 1000.0 * scale;
        if !em.is_finite() || em <= 0.0 || !tx.is_finite() || !ty.is_finite() {
            return;
        }
        // Vertical text advances along y, so a horizontal advance would put a space after every glyph;
        // a font that does not report one gets none, and no space is invented for it.
        let advance = match glyph {
            Glyph::Outline(outline) if !outline.is_vertical() => outline
                .advance_width()
                .map(f64::from)
                .filter(|advance| advance.is_finite()),
            _ => None,
        };
        let characters = text.chars().count();
        if characters > self.budget.characters {
            self.exhausted = true;
            return;
        }
        self.budget.characters -= characters;
        self.budget.glyphs -= 1;
        self.glyphs += 1;
        self.placed.push(Placed {
            x: tx,
            y: ty,
            em,
            advance,
            text,
        });
    }
}

/// Turn the glyphs of a page into fragments: runs on one baseline, split at the wide gaps.
fn fragments(mut placed: Vec<Placed>) -> Vec<Span> {
    placed.sort_by(|a, b| {
        a.y.partial_cmp(&b.y)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal))
    });
    let mut spans: Vec<Span> = Vec::new();
    let mut run: Vec<Placed> = Vec::new();
    for glyph in placed {
        if let Some(previous) = run.last() {
            // A glyph belongs to the baseline it says it does when its baseline agrees with the run's
            // to a fraction of the type size: superscripts and a wobbling generator sit inside that
            // tolerance, the next line down does not.
            let same_baseline = (glyph.y - previous.y).abs() <= 0.4 * glyph.em.max(previous.em);
            let wide_gap = unaccounted(previous, &glyph) > FRAGMENT_GAP_EM * glyph.em;
            if !same_baseline || wide_gap {
                spans.push(span(&std::mem::take(&mut run)));
            }
        }
        run.push(glyph);
    }
    if !run.is_empty() {
        spans.push(span(&run));
    }
    spans
}

/// One fragment from a run of glyphs already in x order.
fn span(run: &[Placed]) -> Span {
    let em = run.iter().map(|glyph| glyph.em).fold(0.0, f64::max);
    let x0 = run.first().map_or(0.0, |glyph| glyph.x);
    let y = run.first().map_or(0.0, |glyph| glyph.y);
    let x1 = run.last().map_or(x0, |glyph| {
        glyph.x
            + glyph
                .advance
                .map_or(0.0, |advance| advance * glyph.em / 1000.0)
    });
    let text = assemble(run);
    Span {
        x0,
        x1: x1.max(x0),
        y,
        em,
        text,
    }
}

/// How far the next glyph's origin is past where the previous glyph's own advance would put it.
///
/// A font that reports no advance gives no evidence of a gap, so nothing is inferred: a space invented
/// there would split a word, and a fragment split there would move a sentence into another column.
fn unaccounted(previous: &Placed, next: &Placed) -> f64 {
    match previous.advance {
        Some(advance) => next.x - (previous.x + advance * previous.em / 1000.0),
        None => 0.0,
    }
}

/// Order fragments the way the page is read: column by column, each top to bottom.
///
/// A page with a gutter is read one column at a time; a page without one is read line by line, which
/// is what a single-column page and a page of tables want.
fn order(spans: Vec<Span>, depth: usize) -> Vec<Span> {
    if depth == 0 {
        return by_line(spans);
    }
    let Some(gutter) = gutter(&spans) else {
        return by_line(spans);
    };
    let (left, right): (Vec<Span>, Vec<Span>) =
        spans.into_iter().partition(|span| span.x1 <= gutter);
    if left.is_empty() || right.is_empty() {
        let mut all = left;
        all.extend(right);
        return by_line(all);
    }
    // A three- or four-column page is split again inside each half, which is why this recurses.
    let mut ordered = order(left, depth - 1);
    ordered.extend(order(right, depth - 1));
    ordered
}

/// The x of the page's widest vertical strip that no fragment crosses, when it divides the page into
/// columns rather than into a body and a footnote.
fn gutter(spans: &[Span]) -> Option<f64> {
    let mut sorted: Vec<&Span> = spans.iter().collect();
    sorted.sort_by(|a, b| a.x0.partial_cmp(&b.x0).unwrap_or(std::cmp::Ordering::Equal));
    let mut edge = f64::NEG_INFINITY;
    let mut widest: Option<(f64, f64)> = None;
    for span in &sorted {
        if edge.is_finite() && span.x0 > edge {
            let width = span.x0 - edge;
            let mid = edge + width / 2.0;
            let wide_enough = width >= MIN_GUTTER_EM * span.em.max(1.0);
            let wider = widest.is_none_or(|(_, best)| width > best);
            if wide_enough && wider {
                let left = spans.iter().filter(|other| other.x1 <= mid).count();
                let share = left as f64 / spans.len() as f64;
                // Both sides have to hold a real share of the page: a table leaves wide gaps too, and
                // a gap that splits a page fourteen to two is a table, not two columns.
                if (MIN_COLUMN_SHARE..=1.0 - MIN_COLUMN_SHARE).contains(&share) {
                    widest = Some((mid, width));
                }
            }
        }
        edge = edge.max(span.x1);
    }
    widest.map(|(mid, _)| mid)
}

/// Order fragments line by line: top to bottom, then left to right.
fn by_line(mut spans: Vec<Span>) -> Vec<Span> {
    spans.sort_by(|a, b| {
        a.y.partial_cmp(&b.y)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.x0.partial_cmp(&b.x0).unwrap_or(std::cmp::Ordering::Equal))
    });
    spans
}

/// Append one assembled line, with the paragraph break the gap above it asked for.
fn emit(text: &mut String, last: &mut Option<(f64, f64)>, y: f64, em: f64, assembled: &str) {
    let assembled = assembled.trim_end();
    if assembled.trim().is_empty() {
        return;
    }
    if let Some((previous_y, previous_em)) = *last
        && y - previous_y > 1.7 * previous_em
    {
        // A vertical gap beyond the line pitch is a paragraph break, and that is what keeps a
        // paragraph whole when the page is cut into citable passages.
        text.push('\n');
    }
    text.push_str(assembled);
    text.push('\n');
    *last = Some((y, em));
}

/// Join a fragment's glyphs, putting back the spaces the generator expressed as positioning.
///
/// The test is against the *advance the font reports*, not against the raw distance between two
/// origins: an `M` advances three times as far as an `i`, and comparing distances alone would put a
/// space before every wide letter. What is left over after the previous glyph's own advance is the
/// space the generator did not draw — a tenth of an em is already generous, since a real space is a
/// quarter of one and kerning only removes width.
fn assemble(run: &[Placed]) -> String {
    let mut text = String::new();
    let mut previous: Option<&Placed> = None;
    for placed in run {
        if let Some(previous) = previous
            && unaccounted(previous, placed) > 0.1 * placed.em
            && !text.ends_with(' ')
            && !placed.text.starts_with(' ')
        {
            text.push(' ');
        }
        text.push_str(&placed.text);
        previous = Some(placed);
    }
    text
}

/// A PDF of `pages`, each drawn with its own content stream in 12 pt Helvetica.
///
/// Built here rather than committed as a fixture: a test that ships a binary attachment is a test
/// nobody can read, and this one says exactly what each page draws. Compiled only for tests, because
/// the shipping library reads PDFs and never writes them.
#[cfg(test)]
fn pdf(pages: &[String]) -> Vec<u8> {
    let page_count = pages.len();
    let mut objects: Vec<Vec<u8>> = Vec::new();
    // 1: catalog, 2: page tree, then (page, content) per page, then the font last.
    objects.push(b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());
    let kids: Vec<String> = (0..page_count)
        .map(|index| format!("{} 0 R", 3 + index * 2))
        .collect();
    objects.push(
        format!(
            "<< /Type /Pages /Kids [{}] /Count {page_count} >>",
            kids.join(" ")
        )
        .into_bytes(),
    );
    let font = format!("{} 0 R", 3 + page_count * 2);
    for content in pages {
        objects.push(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents {} 0 R \
                 /Resources << /Font << /F1 {font} >> >> >>",
                // The page dictionary is pushed now and its content stream next, so the stream's
                // object number — one-based — is this many plus two.
                objects.len() + 2
            )
            .into_bytes(),
        );
        let body = content.as_bytes();
        let mut stream = format!("<< /Length {} >>\nstream\n", body.len()).into_bytes();
        stream.extend_from_slice(body);
        stream.extend_from_slice(b"\nendstream");
        objects.push(stream);
    }
    objects.push(
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_vec(),
    );
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        bytes.extend_from_slice(object);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    bytes.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in &offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Root 1 0 R /Size {} >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    bytes
}

/// One page drawing `content`.
#[cfg(test)]
fn single(content: &str) -> Vec<u8> {
    pdf(&[content.to_owned()])
}

/// A content stream that draws `text` at `y` in 12 pt Helvetica.
#[cfg(test)]
fn line(text: &str, y: i32) -> String {
    format!("BT /F1 12 Tf 72 {y} Td ({text}) Tj ET\n")
}

/// A PDF drawn by hand for a test: one page per entry of `pages`, each a content stream.
///
/// Here rather than in the test module because the library's own tests need a real file to open, and
/// a committed binary fixture would be a file nobody can read.
#[cfg(test)]
pub(crate) fn test_pdf(pages: &[String]) -> Vec<u8> {
    pdf(pages)
}

/// A content stream that draws `text` at `y` in 12 pt Helvetica, for a test PDF.
#[cfg(test)]
pub(crate) fn test_line(text: &str, y: i32) -> String {
    line(text, y)
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn a_page_of_text_reads_back_in_reading_order() {
        let content = format!(
            "{}{}",
            line("Maximum off-nadir angle is 45 degrees.", 720),
            line("The swath is 1200 km at that angle.", 704),
        );
        let extracted = extract(&single(&content), 1 << 20).expect("a readable page");
        assert_eq!(extracted.pages.len(), 1);
        assert_eq!(extracted.pages[0].number, 1);
        assert_eq!(
            extracted.pages[0].text,
            "Maximum off-nadir angle is 45 degrees.\nThe swath is 1200 km at that angle.\n"
        );
        assert_eq!(extracted.unnamed, 0);
        assert!(extracted.glyphs > 60, "{} glyphs", extracted.glyphs);
        assert_eq!(extracted.skipped, 0);
    }

    #[test]
    fn a_vertical_gap_is_a_paragraph_break_and_a_positioned_gap_is_a_space() {
        let content = format!(
            "{}{}BT /F1 12 Tf 72 624 Td (45) Tj 20 0 Td (degrees) Tj ET\n",
            line("Sensor specification", 720),
            line("Maximum off-nadir angle", 640),
        );
        let extracted = extract(&single(&content), 1 << 20).expect("a readable page");
        let text = &extracted.pages[0].text;
        assert!(text.contains("Sensor specification\n\n"), "{text:?}");
        // The two positioned runs keep a space between them even though the stream has none: the gap
        // between their origins is what carries the word break.
        assert!(text.contains("45 degrees"), "{text:?}");
    }

    #[test]
    fn the_citation_of_a_pdf_passage_names_its_page() {
        let content = line("Maximum off-nadir angle is 45 degrees.", 720);
        let extracted = extract(&single(&content), 1 << 20).expect("a readable page");
        let chunks = crate::chunk::chunk_pages(&extracted.passages());
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].page, Some(1));
        assert_eq!(chunks[0].lines_label(), "page 1, lines 1-1");
        assert_eq!(chunks[0].text, "Maximum off-nadir angle is 45 degrees.");
    }

    #[test]
    fn every_page_is_numbered_and_read_in_order() {
        let first = line("The first page says this.", 720);
        let second = line("The second page says that.", 720);
        let extracted = extract(&pdf(&[first, second]), 1 << 20).expect("two readable pages");
        assert_eq!(extracted.pages.len(), 2);
        assert_eq!(extracted.pages[0].number, 1);
        assert_eq!(extracted.pages[1].number, 2);
        assert!(extracted.pages[0].text.contains("first page"));
        assert!(extracted.pages[1].text.contains("second page"));
        let chunks = crate::chunk::chunk_pages(&extracted.passages());
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[1].lines_label(), "page 2, lines 1-1");
    }

    #[test]
    fn the_page_bound_stops_the_read_and_reports_what_it_left() {
        let first = line("The first page says this.", 720);
        let second = line("The second page says that.", 720);
        let bounds = Bounds {
            max_pages: 1,
            ..Bounds::default()
        };
        let extracted = extract_with(&pdf(&[first, second]), &bounds).expect("one page");
        assert_eq!(extracted.pages.len(), 1);
        assert_eq!(extracted.skipped, 1, "the unread page was not counted");
    }

    #[test]
    fn the_glyph_and_character_bounds_stop_a_page_that_never_ends() {
        let content = line("Maximum off-nadir angle is 45 degrees.", 720);
        let glyph_bound = Bounds {
            max_glyphs: 8,
            ..Bounds::default()
        };
        let extracted = extract_with(&single(&content), &glyph_bound).expect("a bounded page");
        assert!(extracted.glyphs <= 8, "{} glyphs", extracted.glyphs);
        let character_bound = Bounds {
            max_characters: 5,
            ..Bounds::default()
        };
        let extracted = extract_with(&single(&content), &character_bound).expect("a bounded page");
        assert!(
            extracted.characters() <= 6,
            "{} characters",
            extracted.characters()
        );
    }

    #[test]
    fn a_file_that_is_not_a_pdf_is_refused_rather_than_guessed_at() {
        let error = extract(b"this is a text file", 1 << 20).expect_err("not a PDF");
        assert!(matches!(error, PdfError::Open(_)), "{error:?}");
        assert!(!looks_like_pdf(b"this is a text file"));
        // The control, so the refusal above is about content and not about every file failing.
        assert!(looks_like_pdf(b"%PDF-1.7\n"));
    }

    #[test]
    fn a_truncated_pdf_is_an_error_and_not_a_panic() {
        let whole = single(&line("Maximum off-nadir angle is 45 degrees.", 720));
        for cut in [16, 64, whole.len() / 2, whole.len() - 32] {
            let cut = cut.min(whole.len());
            if let Ok(extracted) = extract(&whole[..cut], 1 << 20) {
                // A reader that repairs the document may return text; it must not crash, and it must
                // not return more text than the file could hold.
                assert!(extracted.characters() < 100, "text from a truncated file");
            }
        }
    }

    #[test]
    fn an_oversized_file_is_refused_before_it_is_parsed() {
        let whole = single(&line("Maximum off-nadir angle is 45 degrees.", 720));
        let error = extract(&whole, 64).expect_err("larger than the limit");
        assert_eq!(error, PdfError::TooLarge { limit: 64 });
        assert!(error.to_string().contains("64-byte limit"), "{error}");
    }

    #[test]
    fn a_page_with_no_text_layer_reads_as_no_characters() {
        // A path and nothing else: what a scanned page looks like once its images are ignored.
        let extracted = extract(&single("0 0 m 100 100 l S\n"), 1 << 20).expect("a page");
        assert_eq!(extracted.characters(), 0);
        assert_eq!(
            extracted.pages.len(),
            1,
            "a page without text is still a page"
        );
    }

    #[test]
    fn a_two_column_page_is_read_one_column_at_a_time() {
        // The layout every paper uses, and the layout that a line-by-line reading gets wrong: two
        // columns share every baseline, so reading by baseline interleaves them into text no human
        // wrote and a citation nobody can act on.
        let content = format!(
            "{}{}{}{}",
            "BT /F1 12 Tf 40 760 Td (Left column, first line) Tj ET\n",
            "BT /F1 12 Tf 40 744 Td (Left column, second line) Tj ET\n",
            "BT /F1 12 Tf 320 760 Td (Right column, first line) Tj ET\n",
            "BT /F1 12 Tf 320 744 Td (Right column, second line) Tj ET\n",
        );
        let extracted = extract(&single(&content), 1 << 20).expect("a readable page");
        assert_eq!(
            extracted.pages[0].text,
            "Left column, first line\nLeft column, second line\n\
             Right column, first line\nRight column, second line\n"
        );
    }

    #[test]
    fn a_gap_that_is_not_a_column_does_not_split_the_page() {
        // The control for the test above: one column, with the wide gap a table row leaves. A body
        // and a footnote on the same baseline are not two columns, and a page read column-wise would
        // silently reorder every paragraph under it.
        let mut spans = Vec::new();
        // y grows downwards, so the first line is the smallest.
        for (index, y) in [52.0_f64, 68.0, 84.0, 100.0].into_iter().enumerate() {
            spans.push(Span {
                x0: 40.0,
                x1: 300.0,
                y,
                em: 12.0,
                text: format!("body line {index}"),
            });
        }
        // One lone label far to the right on one baseline only: a page number, not a column.
        spans.push(Span {
            x0: 520.0,
            x1: 560.0,
            y: 52.0,
            em: 12.0,
            text: "page 3".to_owned(),
        });
        assert_eq!(gutter(&spans), None, "a lone label became a column break");
        let ordered = order(spans, MAX_COLUMN_DEPTH);
        assert_eq!(
            ordered
                .iter()
                .map(|span| span.text.as_str())
                .collect::<Vec<_>>(),
            vec![
                "body line 0",
                "page 3",
                "body line 1",
                "body line 2",
                "body line 3"
            ]
        );
    }

    #[test]
    fn a_gutter_is_found_when_the_two_sides_hold_the_page() {
        // Two columns of equal weight: the strip between them is the gutter, and it is the widest
        // strip no fragment crosses rather than the widest gap between two fragment starts.
        let mut spans = Vec::new();
        for y in [100.0_f64, 84.0, 68.0, 52.0] {
            spans.push(Span {
                x0: 40.0,
                x1: 300.0,
                y,
                em: 12.0,
                text: "left".to_owned(),
            });
            spans.push(Span {
                x0: 340.0,
                x1: 600.0,
                y,
                em: 12.0,
                text: "right".to_owned(),
            });
        }
        let found = gutter(&spans).expect("a gutter between two columns");
        assert!((320.0..=340.0).contains(&found), "{found}");
    }

    #[test]
    fn a_glyph_drawn_at_zero_size_is_dropped_rather_than_placed() {
        // A zero type size gives the matrix no scale, so the glyph's position says nothing. Placing it
        // would put a bogus baseline on the page and split the real lines around it.
        let content = format!(
            "BT /F1 0 Tf 72 400 Td (invisible) Tj ET\n{}",
            line("Maximum off-nadir angle is 45 degrees.", 720)
        );
        let extracted = extract(&single(&content), 1 << 20).expect("a readable page");
        assert_eq!(
            extracted.pages[0].text,
            "Maximum off-nadir angle is 45 degrees.\n"
        );
    }
}
