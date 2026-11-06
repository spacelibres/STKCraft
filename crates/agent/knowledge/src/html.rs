//! Turning a page into the text a reader sees.
//!
//! The standards are published as HTML — NAIF's required reading, the CCSDS publications index, the ECSS
//! pages — and a knowledge base that indexed markup would answer questions about `<table cellpadding>` while
//! missing "the SPK segment type 2 record contains a Chebyshev polynomial".
//!
//! This is not an HTML parser and does not pretend to be one: it drops tags, drops the elements whose content
//! is not prose (`script`, `style`, `head`), turns block boundaries into line breaks, and decodes the handful
//! of entities that actually appear in a technical document. What it guarantees is what a citation needs: a
//! line number that points at text a person would recognise on the page.

/// Elements whose content is not prose and must not reach the index.
const SILENT: [&str; 5] = ["script", "style", "head", "title", "noscript"];

/// Tags that end a line, so a paragraph stays a paragraph and a table row stays a row.
const BLOCKS: [&str; 12] = [
    "p", "div", "br", "tr", "li", "h1", "h2", "h3", "h4", "h5", "h6", "pre",
];

/// The text of a page, one line per block, with tags and entities resolved.
#[must_use]
pub fn to_text(page: &str) -> String {
    let mut text = String::with_capacity(page.len() / 2);
    let mut rest = page;
    while let Some(start) = rest.find('<') {
        // The text before the tag.
        let prose = rest.get(..start).unwrap_or_default();
        push_prose(&mut text, prose);
        // A tag opens with a name, a close, or a declaration. Anything else is the comparison operator in a
        // sentence like `run when a < b`, and swallowing it would delete the sentence.
        let opens = rest
            .get((start + 1)..)
            .and_then(|tail| tail.chars().next())
            .is_some_and(|first| {
                first.is_alphanumeric() || first == '/' || first == '!' || first == '?'
            });
        if !opens {
            text.push('<');
            rest = rest.get((start + 1)..).unwrap_or_default();
            continue;
        }
        let Some(end) = rest.get(start..).and_then(|tail| tail.find('>')) else {
            // A `<` with no `>` is text, not a tag: a comparison in a formula is not markup.
            push_prose(&mut text, rest.get(start..).unwrap_or_default());
            return finish(&text);
        };
        let tag = rest.get(start + 1..start + end).unwrap_or_default();
        let closing = tag.starts_with('/');
        let name = tag
            .trim_start_matches('/')
            .split(|character: char| character.is_whitespace() || character == '/')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !closing && SILENT.contains(&name.as_str()) {
            // Skip to the matching close tag, or to the end: a page whose `</script>` is missing must not
            // swallow the rest of the document, so the skip is bounded by the tag's own end.
            let close = format!("</{name}");
            let skip_from = start + end + 1;
            let skipped = rest
                .get(skip_from..)
                .and_then(|tail| tail.to_ascii_lowercase().find(&close))
                .map_or(rest.len(), |offset| skip_from + offset);
            let resume = rest
                .get(skipped..)
                .and_then(|tail| tail.find('>'))
                .map_or(skipped, |offset| skipped + offset + 1);
            rest = rest.get(resume..).unwrap_or_default();
            continue;
        }
        if BLOCKS.contains(&name.as_str()) {
            text.push('\n');
        }
        rest = rest.get((start + end + 1)..).unwrap_or_default();
    }
    push_prose(&mut text, rest);
    finish(&text)
}

/// Append prose, decoding the entities a technical page uses.
fn push_prose(text: &mut String, prose: &str) {
    let mut characters = prose.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '&' => {
                let mut entity = String::new();
                while let Some(next) = characters.peek().copied() {
                    if next == ';' || entity.len() > 8 {
                        break;
                    }
                    entity.push(next);
                    characters.next();
                }
                if characters.peek() == Some(&';') {
                    characters.next();
                }
                text.push(decode(&entity));
            }
            '\n' | '\r' => text.push(' '),
            other => text.push(other),
        }
    }
}

/// One entity, by name or by number.
fn decode(entity: &str) -> char {
    match entity {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => ' ',
        "ndash" => '–',
        "mdash" => '—',
        "times" => '×',
        "deg" => '°',
        "plusmn" => '±',
        "le" => '≤',
        "ge" => '≥',
        "ne" => '≠',
        "infin" => '∞',
        "sum" => '∑',
        "int" => '∫',
        "radic" => '√',
        "part" => '∂',
        "alpha" => 'α',
        "beta" => 'β',
        "gamma" => 'γ',
        "delta" => 'δ',
        "theta" => 'θ',
        "lambda" => 'λ',
        "mu" => 'μ',
        "pi" => 'π',
        "sigma" => 'σ',
        "tau" => 'τ',
        "phi" => 'φ',
        "omega" => 'ω',
        other => {
            let code = other.strip_prefix('#').and_then(|rest| {
                rest.strip_prefix('x')
                    .or_else(|| rest.strip_prefix('X'))
                    .map_or_else(
                        || rest.parse::<u32>().ok(),
                        |hex| u32::from_str_radix(hex, 16).ok(),
                    )
            });
            code.and_then(char::from_u32).unwrap_or(' ')
        }
    }
}

/// Tidy: collapse runs of blank lines, trim each line, end with one newline.
fn finish(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank = 0_usize;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            blank += 1;
            continue;
        }
        if blank > 0 && !out.is_empty() {
            out.push('\n');
        }
        blank = 0;
        out.push_str(line);
        out.push('\n');
    }
    out
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn tags_are_dropped_and_blocks_keep_their_lines() {
        let page = "<html><head><title>SPK</title></head><body><h1>Segment type 2</h1>\
                    <p>The record contains a <b>Chebyshev</b> polynomial.</p><p>Second.</p></body></html>";
        let text = to_text(page);
        assert!(text.contains("Segment type 2"), "{text:?}");
        assert!(
            text.contains("The record contains a Chebyshev polynomial."),
            "{text:?}"
        );
        assert!(!text.contains('<'), "{text:?}");
        // The title lives in `head`, whose content is not prose.
        assert!(!text.contains("SPK"), "{text:?}");
    }

    #[test]
    fn entities_become_the_characters_a_reader_sees() {
        let text =
            to_text("<p>x &le; 5 &amp;&amp; y &ne; 0, so &radic;2 &asymp; 1.414&nbsp;km</p>");
        assert!(text.contains("x ≤ 5 && y ≠ 0"), "{text:?}");
        assert!(text.contains("√2"), "{text:?}");
        // An entity this reader does not know is a space, never the raw bytes.
        assert!(!text.contains("&asymp;"), "{text:?}");
    }

    #[test]
    fn script_content_never_reaches_the_text() {
        let text = to_text("<p>visible</p><script>var hidden = 1;</script><p>also visible</p>");
        assert!(text.contains("visible"), "{text:?}");
        assert!(!text.contains("hidden"), "{text:?}");
        assert!(text.contains("also visible"), "{text:?}");
    }

    #[test]
    fn a_comparison_is_not_a_tag() {
        // A page that writes `if a < b` in prose must keep it: an unmatched `<` is text.
        let text = to_text("<p>run when a < b</p>");
        assert!(text.contains("a < b"), "{text:?}");
    }
}
