//! Reading the interface strings out of Rust source.
//!
//! The interface writes English and calls one of three macros, and this is what finds them:
//!
//! ```text
//! tr!(translations, locale, "Save")                  → no context
//! trc!(translations, locale, "menu", "Open Table…")  → context `menu`
//! trn!(translations, locale, "{n} page", "{n} pages") → `@plural`, the two forms joined by `|`
//! ```
//!
//! It lives here, below every consumer, because there are three: `cargo xtask i18n` (what a
//! translator works from), the application's build script (the strings a *running* window can show,
//! which is what lets it translate itself), and the tests of both. Three copies of a scanner would
//! be three answers to "which strings exist".
//!
//! A literal is the only thing it can see, which is why the macros take literals: a string built at
//! run time is a string no catalog can be checked against.

/// One string the interface can show, as the extractor found it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Source {
    /// The context column: empty, a plain word, or `@plural`.
    pub context: String,
    /// The English source text; a plural row holds `one|other`.
    pub text: String,
}

impl Source {
    /// The catalog key this string translates: its context and its English text.
    ///
    /// A string is translated per (context, text) pair, so that is the identity a catalog and a
    /// report both use.
    #[must_use]
    pub fn key(&self) -> (String, String) {
        (self.context.clone(), self.text.clone())
    }
}

/// The translation macro calls in one source file.
///
/// Returns `(context, literals)` per call: `tr!` has no context, `trc!` takes a context literal
/// first, and `trn!` is a plural whose two English forms are joined into one source row.
///
/// A call is found only outside a string, a character literal and a comment. The scanner that looks
/// for the needles is also the one that reads their arguments, so `("tr!(", None)` — the text this
/// very crate matches on — is data and not a call. Getting that wrong put a row named `, Some(|)`
/// into a catalog, which is what a naive search does the first time a file mentions the macro it
/// looks for.
#[must_use]
pub fn macro_calls(text: &str) -> Vec<(Option<String>, Vec<String>)> {
    // Calls are returned in source order: an extractor whose report reorders the file is a bad
    // diff to review, and a test that asserts the order is asserting the contract.
    let mut calls: Vec<(usize, Option<String>, Vec<String>)> = Vec::new();
    // The scan walks characters, not bytes: an index in the middle of `Ö` is not a place a string
    // can be sliced, and a scanner that steps by byte panics on the first non-ASCII translation.
    let mut characters = text.char_indices().peekable();
    // `None` is code; `Some(quote)` is inside a string or character literal of that quote.
    let mut in_literal: Option<char> = None;
    let mut in_block_comment = false;
    while let Some((index, character)) = characters.next() {
        if in_block_comment {
            if character == '*' && characters.peek().is_some_and(|(_, next)| *next == '/') {
                in_block_comment = false;
                characters.next();
            }
            continue;
        }
        if let Some(quote) = in_literal {
            if character == '\\' {
                characters.next();
            } else if character == quote {
                in_literal = None;
            }
            continue;
        }
        match character {
            '"' => {
                in_literal = Some('"');
                continue;
            }
            '\'' => {
                // A lifetime is not a character literal. `&'static str` used to put the scanner
                // into a "character literal" that never closed on that line, so every `tr!` after
                // it was invisible to the extractor — the string was in the source and not in any
                // catalog. A character literal closes one character later; a lifetime continues
                // with an identifier.
                let mut look = characters.clone();
                let next = look.next().map(|(_, next)| next);
                let after = look.next().map(|(_, next)| next);
                let lifetime = next.is_some_and(|next| next.is_alphanumeric() || next == '_')
                    && after != Some('\'');
                if !lifetime {
                    in_literal = Some('\'');
                }
                continue;
            }
            '/' if characters.peek().is_some_and(|(_, next)| *next == '/') => {
                while characters.peek().is_some_and(|(_, next)| *next != '\n') {
                    characters.next();
                }
                continue;
            }
            '/' if characters.peek().is_some_and(|(_, next)| *next == '*') => {
                in_block_comment = true;
                characters.next();
                continue;
            }
            _ => {}
        }
        // A raw string is a string: `r"…"`, `r#"…"#`, `br#"…"#`. It has no escapes at all, so the
        // ordinary literal tracking above would end it at the first backslash-followed quote and
        // then read the rest of the file as code — which is how a test that asserts on this very
        // extractor ended up in a catalog.
        if let Some((consumed, hash_count)) =
            raw_string_start(text.get(index..).unwrap_or_default())
        {
            let terminator: String = std::iter::once('"')
                .chain(std::iter::repeat_n('#', hash_count))
                .collect();
            let body_start = index + consumed;
            let rest = text.get(body_start..).unwrap_or_default();
            let end = rest
                .find(&terminator)
                .map_or(rest.len(), |at| at + terminator.len());
            let mut skipped = 0;
            while characters
                .peek()
                .is_some_and(|(at, _)| *at < body_start + end)
            {
                characters.next();
                skipped += 1;
            }
            let _ = skipped;
            continue;
        }
        // In code: does a macro call start here?
        let Some(rest) = text.get(index..) else {
            continue;
        };
        let matched = [
            ("trc!(", Some("context")),
            ("trn!(", Some("plural")),
            ("tr!(", None),
        ]
        .into_iter()
        .find(|(needle, _)| rest.starts_with(needle));
        let Some((needle, context)) = matched else {
            continue;
        };
        let after_start = index + needle.len();
        let Some(after) = text.get(after_start..) else {
            continue;
        };
        let Some(end) = matching_paren(after) else {
            break;
        };
        let literals = string_literals(after.get(..end).unwrap_or_default());
        // Skip the iterator past the closing parenthesis of the call.
        let call_end = after_start + end;
        while characters.peek().is_some_and(|(at, _)| *at <= call_end) {
            characters.next();
        }
        let mut literals = literals;
        match context {
            Some("context") => {
                let context = if literals.len() >= 2 {
                    literals.remove(0)
                } else {
                    String::new()
                };
                calls.push((index, Some(context), literals));
            }
            Some(other) => calls.push((index, Some(other.to_owned()), literals)),
            None => calls.push((index, None, literals)),
        }
    }
    calls.sort_by_key(|(at, _, _)| *at);
    calls
        .into_iter()
        .map(|(_, context, literals)| (context, literals))
        .collect()
}

/// Every interface string in one source file.
///
/// A call whose shape the macros do not produce — no literals, or a `trn!` with one form — yields
/// nothing rather than a guess: an extractor that invents a string adds a row to every catalog that
/// no interface shows.
#[must_use]
pub fn sources(text: &str) -> Vec<Source> {
    let mut found = Vec::new();
    for (context, literals) in macro_calls(text) {
        let source = match (context.as_deref(), literals.as_slice()) {
            // `trn!`: the two English forms are one plural row.
            (Some("plural"), [one, other]) => Source {
                context: "@plural".to_owned(),
                text: format!("{one}|{other}"),
            },
            // `trc!`: the macro moved the context into the call, so one literal remains.
            (Some(context), [text]) => Source {
                context: context.to_owned(),
                text: text.clone(),
            },
            // `tr!`: no context.
            (None, [text]) => Source {
                context: String::new(),
                text: text.clone(),
            },
            // Anything else (a call with no literal, or a shape the macros do not produce) is not a
            // translatable string.
            _ => continue,
        };
        found.push(source);
    }
    found
}

/// How long a raw string's opening is, and how many `#` delimit its end.
///
/// `r"…"` is `(2, 0)`; `r#"…"#` is `(4, 1)`; `br##"…"##` is `(5, 2)`. `None` when the text at the
/// start of `text` is not a raw string.
fn raw_string_start(text: &str) -> Option<(usize, usize)> {
    let mut characters = text.char_indices().peekable();
    let mut consumed = 0;
    // An optional `b` makes it a byte string; the rest is the same.
    if characters
        .peek()
        .is_some_and(|(_, character)| *character == 'b')
    {
        characters.next();
        consumed += 1;
    }
    if characters
        .peek()
        .is_none_or(|(_, character)| *character != 'r')
    {
        return None;
    }
    characters.next();
    consumed += 1;
    let mut hash_count = 0;
    while characters
        .peek()
        .is_some_and(|(_, character)| *character == '#')
    {
        characters.next();
        consumed += 1;
        hash_count += 1;
    }
    if characters
        .peek()
        .is_none_or(|(_, character)| *character != '"')
    {
        return None;
    }
    consumed += 1;
    Some((consumed, hash_count))
}

/// The index of the `)` that closes the argument list, allowing for nesting.
fn matching_paren(text: &str) -> Option<usize> {
    let mut depth = 1usize;
    for (index, character) in text.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

/// Every plain string literal in a piece of code, in order.
///
/// Only `"…"` with `\"` and `\\` escapes: the macros take literals, and a caller that builds a
/// string at run time is calling a string the extractor cannot see — which is why the convention is
/// literal-only.
fn string_literals(text: &str) -> Vec<String> {
    let mut literals = Vec::new();
    let mut characters = text.char_indices();
    while let Some((_, character)) = characters.next() {
        if character != '"' {
            continue;
        }
        let mut literal = String::new();
        while let Some((_, inner)) = characters.next() {
            match inner {
                '"' => break,
                '\\' => match characters.next() {
                    Some((_, 'n')) => literal.push('\n'),
                    Some((_, 't')) => literal.push('\t'),
                    // A backslash at the end of a line is Rust's line continuation: it removes the
                    // newline and the next line's leading whitespace, so a long literal split across
                    // two lines is one string and not one string with a hole in it. A source file
                    // checked out with CRLF ends the line with `\r\n`; treating that `\r` as text
                    // made every continued literal a different key from its catalog row.
                    Some((_, '\r'))
                        if characters
                            .clone()
                            .next()
                            .is_some_and(|(_, next)| next == '\n') =>
                    {
                        characters.next();
                        while matches!(characters.clone().next(), Some((_, ' ' | '\t'))) {
                            characters.next();
                        }
                    }
                    Some((_, '\n')) => {
                        while matches!(characters.clone().next(), Some((_, ' ' | '\t'))) {
                            characters.next();
                        }
                    }
                    Some((_, other)) => literal.push(other),
                    None => break,
                },
                other => literal.push(other),
            }
        }
        literals.push(literal);
    }
    literals
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn a_crlf_line_continuation_is_one_string() {
        // A Windows checkout has `\r\n`; the `\r` is not part of the string Rust builds, so it must
        // not become part of the catalog key either.
        let text = "let a = tr!(translations, locale, \"model \\\r\n  to choose\");";
        let sources = sources(text);
        assert_eq!(
            sources.first().map(|source| source.text.as_str()),
            Some("model to choose")
        );
    }

    #[test]
    fn a_lifetime_does_not_hide_the_calls_after_it() {
        let text = r#"
            fn label(value: &'static str) -> &'static str {
                let _ = tr!(translations, locale, "Clock");
                value
            }
        "#;
        let sources = sources(text);
        assert_eq!(
            sources.first().map(|source| source.text.as_str()),
            Some("Clock")
        );
    }

    #[test]
    fn the_three_macros_are_extracted_with_their_context() {
        let text = r#"
            let a = tr!(translations, locale, "Save");
            let b = trc!(translations, locale, "menu", "Open Table…");
            let c = trn!(translations, locale, "{n} page", "{n} pages");
            let d = something_else("not translated");
        "#;
        let calls = macro_calls(text);
        assert_eq!(calls.len(), 3, "{calls:?}");
        assert_eq!(
            calls[0],
            (None, vec!["Save".to_owned()]),
            "tr! has no context"
        );
        assert_eq!(
            calls[1],
            (Some("menu".to_owned()), vec!["Open Table…".to_owned()]),
            "trc! carries its context in the call and one literal remains"
        );
        assert_eq!(
            calls[2],
            (
                Some("plural".to_owned()),
                vec!["{n} page".to_owned(), "{n} pages".to_owned()]
            )
        );
    }

    #[test]
    fn sources_join_the_two_english_forms_of_a_plural() {
        let found = sources(r#"trn!(t, l, "{n} satellite", "{n} satellites"); tr!(t, l, "Save");"#);
        assert_eq!(
            found,
            vec![
                Source {
                    context: "@plural".to_owned(),
                    text: "{n} satellite|{n} satellites".to_owned()
                },
                Source {
                    context: String::new(),
                    text: "Save".to_owned()
                },
            ]
        );
    }

    #[test]
    fn nested_calls_and_escapes_survive_the_scan() {
        let text =
            r#"tr!(handle, locale, format!("nested {}", "x")); tr!(h, l, "Tab\there\nNew");"#;
        let calls = macro_calls(text);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[1].1, vec!["Tab\there\nNew".to_owned()]);
    }

    #[test]
    fn a_doc_comment_example_is_not_a_call() {
        let text =
            "/// `tr!(handle, locale, \"Save\")` is how it is used.\nlet a = tr!(h, l, \"Close\");";
        let calls = macro_calls(text);
        assert_eq!(calls, vec![(None, vec!["Close".to_owned()])], "{calls:?}");
    }

    #[test]
    fn a_non_ascii_source_is_scanned_without_panicking() {
        // The regression: the scanner stepped by byte and sliced in the middle of `Ö`.
        let text = "// Öffnen\nlet a = tr!(t, l, \"Öffnen …\");\nlet b = trc!(t, l, \"menu\", \"Öffnen\");";
        let found = sources(text);
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(found[0].text, "Öffnen …");
    }

    #[test]
    fn a_raw_string_is_data_and_its_content_is_never_a_call() {
        // The regression: a raw string has no escapes, so the scanner ended it at the first
        // backslash-quote and read the rest of the file as code. A test that asserts on this
        // extractor then put `Yes"` and `, Some(` into a real catalog.
        let text = r##"let a = r#"tr!(h, l, "No"); "#; let b = tr!(h, l, "Yes");"##;
        assert_eq!(
            sources(text),
            vec![Source {
                context: String::new(),
                text: "Yes".to_owned()
            }]
        );
        assert_eq!(raw_string_start(r#"r"x""#), Some((2, 0)));
        assert_eq!(raw_string_start(r##"r#"x"# "##), Some((3, 1)));
        assert_eq!(raw_string_start("plain"), None);
    }

    #[test]
    fn a_url_inside_a_string_does_not_start_a_comment() {
        let text = r#"let url = "https://example/x"; tr!(t, l, "Save");"#;
        assert_eq!(
            sources(text).len(),
            1,
            "a `//` in a string ate the rest of the line"
        );
    }

    #[test]
    fn the_needle_inside_a_string_is_data_and_not_a_call() {
        // The regression: `("tr!(", None)` — the text the scanner matches on, in its own source —
        // was read as a call, and a catalog grew a row named `, Some(|)`.
        let text = r#"for (needle, context) in [("trc!(", Some("context")), ("tr!(", None)] {}"#;
        assert_eq!(
            sources(text),
            Vec::new(),
            "a needle inside a literal became a string"
        );
        // A block comment is not a call either.
        let text = "/* tr!(h, l, \"Nope\"); */ tr!(h, l, \"Yes\");";
        assert_eq!(
            sources(text),
            vec![Source {
                context: String::new(),
                text: "Yes".to_owned()
            }]
        );
    }

    #[test]
    fn literals_are_read_without_their_quotes() {
        assert_eq!(
            string_literals(r#""One" "Two \"quoted\"" "#),
            vec!["One".to_owned(), "Two \"quoted\"".to_owned()]
        );
    }

    #[test]
    fn a_line_continuation_is_not_a_newline() {
        // Rust's `\` at the end of a line joins two source lines into one string, dropping the
        // newline and the next line's indentation. Treating it as a newline put a hole in a
        // translated sentence and made the catalog row unrecognisable to the extraction.
        let text = "tr!(h, l, \"one \\\n             two\");";
        let calls = macro_calls(text);
        assert_eq!(calls, vec![(None, vec!["one two".to_owned()])], "{calls:?}");
    }
}
