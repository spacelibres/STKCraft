//! Parsing and checking one locale's catalog.
//!
//! A catalog is text: one row per line, three TAB-separated columns, `#` comments and blank lines
//! ignored. Parsing reports the line number of everything it refuses, because a translator editing
//! a thousand-row file needs to know which row is wrong, not that the file is.

use std::fmt::Write as _;

/// One translated string.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Row {
    /// The context column: empty, a plain word, or one of `@plural`, `@id`, `@msg`.
    pub context: String,
    /// The English source text. For an `@id` row this is a command id instead.
    pub source: String,
    /// The translation, in the locale's own language.
    pub translation: String,
}

impl Row {
    /// Whether the row is a plural row, whose source is `one|other` and whose translation lists the
    /// locale's forms.
    #[must_use]
    pub fn is_plural(&self) -> bool {
        self.context == "@plural"
    }

    /// Whether the row names the language, which every picker reads.
    #[must_use]
    pub fn is_language_name(&self) -> bool {
        self.context == "@id" && self.source == "language.name"
    }
}

/// Something wrong with a catalog, with the line that shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// 1-based line number in the catalog the problem was found in.
    pub line: usize,
    /// What is wrong, in words a translator can act on.
    pub message: String,
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

/// A whole locale's catalog.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Catalog {
    /// The locale code this catalog belongs to, as its directory names it.
    pub locale: String,
    /// Every row, in file order.
    pub rows: Vec<Row>,
}

impl Catalog {
    /// The language's own name, from its `@id language.name` row.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.rows
            .iter()
            .find(|row| row.is_language_name())
            .map(|row| row.translation.as_str())
    }

    /// How many rows the catalog holds, which is its translation count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether the catalog is empty, which a locale with no translations yet is.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// Parse a catalog, returning the rows and every problem found.
///
/// Both are returned rather than one or the other: the rows that are well-formed are useful — a
/// half-translated catalog is the normal state of a new language — and the problems are what stops
/// it being merged. A caller that wants a hard failure checks `problems.is_empty()`.
#[must_use]
pub fn parse(locale: &str, text: &str) -> (Catalog, Vec<Problem>) {
    let mut catalog = Catalog {
        locale: locale.to_owned(),
        rows: Vec::new(),
    };
    let mut problems = Vec::new();
    // A byte-order mark is legal and invisible; a Windows line ending is legal and visible in a
    // diff. Both are the editor's fault, not the translator's.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    for (index, line) in text.lines().enumerate() {
        let number = index + 1;
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let mut columns = line.split('\t');
        let (Some(context), Some(source), Some(translation), None) = (
            columns.next(),
            columns.next(),
            columns.next(),
            columns.next(),
        ) else {
            problems.push(Problem {
                line: number,
                message: "a row is `context <TAB> English <TAB> translation`".to_owned(),
            });
            continue;
        };
        let row = Row {
            context: unescape(context),
            source: unescape(source),
            translation: unescape(translation),
        };
        if row.translation.trim().is_empty() {
            problems.push(Problem {
                line: number,
                message: format!(
                    "the translation of `{}` is empty; leave the row out instead",
                    row.source
                ),
            });
            continue;
        }
        if row.source.trim().is_empty() {
            problems.push(Problem {
                line: number,
                message: "the English source column is empty".to_owned(),
            });
            continue;
        }
        if let Some(message) = row_problem(&row) {
            problems.push(Problem {
                line: number,
                message,
            });
            continue;
        }
        if catalog
            .rows
            .iter()
            .any(|existing| existing.context == row.context && existing.source == row.source)
        {
            problems.push(Problem {
                line: number,
                message: format!(
                    "`{}` is already translated in this catalog (context `{}`)",
                    row.source, row.context
                ),
            });
            continue;
        }
        catalog.rows.push(row);
    }
    (catalog, problems)
}

/// What is wrong with a well-formed row, if anything.
///
/// These are the checks the shared catalog project runs on every pull request. A translation that
/// drops a `{name}` cannot be filled in, and one that drops a trailing ellipsis is a sentence whose
/// ending went missing — both produce a worse interface than showing English.
fn row_problem(row: &Row) -> Option<String> {
    let (source_forms, translation_forms) = if row.is_plural() {
        (
            row.source.split('|').collect::<Vec<_>>(),
            row.translation.split('|').collect::<Vec<_>>(),
        )
    } else {
        (vec![row.source.as_str()], vec![row.translation.as_str()])
    };
    if row.is_plural() && translation_forms.len() < source_forms.len().min(2) {
        return Some(format!(
            "`{}` is a plural row with {} form(s) in the source and {} in the translation",
            row.source,
            source_forms.len(),
            translation_forms.len()
        ));
    }
    // Every form of a plural says the same thing with a different shape, so the placeholders of the
    // first form are the placeholders of them all.
    for (source, translation) in source_forms.iter().zip(translation_forms.iter()) {
        let source_placeholders = placeholders(source);
        let translation_placeholders = placeholders(translation);
        for placeholder in &source_placeholders {
            if !translation_placeholders.contains(placeholder) {
                return Some(format!(
                    "`{{{} }}` is in the English but not in the translation of `{}`",
                    placeholder, row.source
                ));
            }
        }
        for placeholder in &translation_placeholders {
            if !source_placeholders.contains(placeholder) {
                return Some(format!(
                    "`{{{} }}` is in the translation of `{}` but not in the English",
                    placeholder, row.source
                ));
            }
        }
        if source.ends_with('…') != translation.ends_with('…') {
            return Some(format!(
                "the English of `{}` {} with an ellipsis and the translation {}",
                row.source,
                if source.ends_with('…') {
                    "ends"
                } else {
                    "does not end"
                },
                if translation.ends_with('…') {
                    "does"
                } else {
                    "does not"
                }
            ));
        }
    }
    None
}

/// The `{name}` placeholders in a string, in order of first appearance.
///
/// A `{` with no `}` is not a placeholder: it is text, and treating it as one would make a stray
/// brace a translation failure.
#[must_use]
pub fn placeholders(text: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('}') else {
            break;
        };
        let name = &after[..end];
        if !name.is_empty() && !found.contains(&name) {
            found.push(name);
        }
        rest = &after[end + 1..];
    }
    found
}

/// Turn the three escapes a catalog allows back into the characters they stand for.
///
/// Only `\n`, `\t` and `\\`: a catalog has no other reason to hold a backslash, and inventing more
/// escapes would make a translated string mean something its English does not.
#[allow(clippy::match_same_arms)]
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut characters = text.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            out.push(character);
            continue;
        }
        match characters.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            // `\\` is itself, and an unknown escape or a trailing backslash is kept as written:
            // losing it would change a translated string silently.
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Write one row as catalog text, escaping what the format reserves.
#[must_use]
pub fn row_text(row: &Row) -> String {
    let mut out = String::new();
    let _ = write!(
        out,
        "{}\t{}\t{}",
        escape(&row.context),
        escape(&row.source),
        escape(&row.translation)
    );
    out
}

/// Escape the characters a column cannot hold.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\\' => out.push_str("\\\\"),
            other => out.push(other),
        }
    }
    out
}

/// The header every catalog starts with, as the shared format specifies it.
#[must_use]
pub fn header(language: &str, locale: &str) -> String {
    format!(
        "# {language} ({locale}) interface catalog for STKCraft.\n\
         # Columns, separated by a TAB:  context <TAB> English source text <TAB> translation\n\
         #   context  usually empty. `@plural`, `@id` and `@msg` mean something to the apps; any other\n\
         #            word is a label that tells apart two meanings of the same English.\n\
         #   escapes  \\n  \\t  \\\\  inside a column. `{{name}}` placeholders must appear in both columns (in\n\
         #            any order), and a trailing \"…\" in one needs one in the other.\n\
         #   @plural  English `one|other`; the translation lists this language's plural forms, `|`-separated.\n\
         #   Lines starting with # and blank lines are ignored. A missing entry shows in English.\n\
         #\n\
         # Clean room: every entry is written from the plain meaning of the English text, never copied\n\
         # from another product's translated interface.\n"
    )
}

#[cfg(test)]
#[allow(clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn a_catalog_parses_its_three_columns_and_ignores_comments() {
        let text = "# a comment\n\n\tSave\tSpeichern\nmenu\tOpen\tÖffnen\n";
        let (catalog, problems) = parse("de", text);
        assert_eq!(problems, Vec::new());
        assert_eq!(catalog.locale, "de");
        assert_eq!(catalog.len(), 2);
        assert_eq!(
            catalog.rows[0],
            Row {
                context: String::new(),
                source: "Save".to_owned(),
                translation: "Speichern".to_owned(),
            }
        );
        assert_eq!(catalog.rows[1].context, "menu");
    }

    #[test]
    fn the_language_name_comes_from_its_id_row() {
        let (catalog, _) = parse("de", "@id\tlanguage.name\tDeutsch\n\tSave\tSpeichern\n");
        assert_eq!(catalog.name(), Some("Deutsch"));
    }

    #[test]
    fn a_row_without_three_columns_is_reported_by_line() {
        let (catalog, problems) = parse("de", "\tSave\tSpeichern\n\tBroken\n");
        assert_eq!(catalog.len(), 1);
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].line, 2);
        assert!(problems[0].message.contains("context"), "{}", problems[0]);
    }

    #[test]
    fn an_empty_translation_is_refused_rather_than_merged() {
        // An empty cell would show an empty label; leaving the row out shows English instead.
        let (_, problems) = parse("de", "\tSave\t\n");
        assert_eq!(problems.len(), 1);
        assert!(problems[0].message.contains("empty"), "{}", problems[0]);
    }

    #[test]
    fn a_duplicate_row_is_refused() {
        let (catalog, problems) = parse("de", "\tSave\tSpeichern\n\tSave\tSichern\n");
        assert_eq!(catalog.len(), 1);
        assert_eq!(problems.len(), 1);
        assert!(problems[0].message.contains("already"), "{}", problems[0]);
    }

    #[test]
    fn the_same_english_in_two_contexts_is_two_rows() {
        let (catalog, problems) =
            parse("de", "\tOpen\tÖffnen\nmenu\tOpen Table…\tTabelle öffnen…\n");
        assert_eq!(problems, Vec::new());
        assert_eq!(catalog.len(), 2);
    }

    #[test]
    fn placeholders_are_extracted_and_must_match() {
        assert_eq!(placeholders("{n} page"), vec!["n"]);
        assert_eq!(placeholders("{a} and {b}, then {a}"), vec!["a", "b"]);
        assert_eq!(placeholders("no placeholders"), Vec::<&str>::new());
        assert_eq!(placeholders("unclosed {brace"), Vec::<&str>::new());
        assert_eq!(placeholders("{}"), Vec::<&str>::new());

        let (_, problems) = parse("de", "@msg\tAdded {name}\tHinzugefügt\n");
        assert_eq!(problems.len(), 1);
        assert!(problems[0].message.contains("{name}"), "{}", problems[0]);

        let (_, problems) = parse("de", "@msg\tAdded {name}\t{name} hinzugefügt\n");
        assert_eq!(problems, Vec::new());
    }

    #[test]
    fn the_ellipsis_has_to_match() {
        let (_, problems) = parse("de", "menu\tMore…\tMehr\n");
        assert_eq!(problems.len(), 1);
        assert!(problems[0].message.contains("ellipsis"), "{}", problems[0]);
        let (_, problems) = parse("de", "menu\tMore…\tMehr…\n");
        assert_eq!(problems, Vec::new());
    }

    #[test]
    fn a_plural_row_needs_every_form_the_language_uses() {
        // English has two forms and Japanese one, so a Japanese row with two is legal and one is
        // what a language with three would need more of.
        let (catalog, problems) = parse(
            "ru",
            "@plural\t{n} page|{n} pages\t{n} страница|{n} страницы|{n} страниц\n",
        );
        assert_eq!(problems, Vec::new());
        assert!(catalog.rows[0].is_plural());

        let (_, problems) = parse("ru", "@plural\t{n} page|{n} pages\t{n} страница\n");
        assert_eq!(
            problems.len(),
            1,
            "a one-form Russian plural must be refused"
        );
    }

    #[test]
    fn escapes_round_trip_through_the_file_format() {
        let (catalog, problems) = parse("de", "\tFirst\\nSecond\tErste\\nZweite\n");
        assert_eq!(problems, Vec::new());
        assert_eq!(catalog.rows[0].source, "First\nSecond");
        assert_eq!(
            row_text(&catalog.rows[0]),
            "\tFirst\\nSecond\tErste\\nZweite"
        );
        let (again, problems) = parse("de", &row_text(&catalog.rows[0]));
        assert_eq!(problems, Vec::new());
        assert_eq!(again.rows, catalog.rows);
    }

    #[test]
    fn a_byte_order_mark_and_windows_line_endings_are_tolerated() {
        let (catalog, problems) = parse("de", "\u{feff}\tSave\tSpeichern\r\n");
        assert_eq!(problems, Vec::new());
        assert_eq!(catalog.len(), 1);
    }

    #[test]
    fn the_header_names_the_language_and_the_clean_room_rule() {
        let header = header("Deutsch", "de");
        assert!(header.contains("Deutsch (de)"), "{header}");
        assert!(header.contains("Clean room"), "{header}");
        assert!(header.contains("context <TAB> English source text <TAB> translation"));
    }
}
