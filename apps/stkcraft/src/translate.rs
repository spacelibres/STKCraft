//! Translating the interface inside the window, with the person's own key.
//!
//! `cargo xtask i18n translate` is for a checkout: it writes a catalog a project ships. A person
//! running the released binary has no checkout, and the language they want may not have a catalog at
//! all. This is that path — pick a language, and `DeepSeek`, on the key in `DEEPSEEK_API_KEY`,
//! translates the strings the window still shows in English.
//!
//! # The rules it inherits
//!
//! The batch, the prompt and the acceptance rules are the shared ones in `stkcraft_agent_runtime::translate`,
//! so a catalog made here and one made by the developer task cannot disagree about what a valid row
//! is. What is new is only where the result goes and who waits for it:
//!
//! - the catalog is written to `locales/<locale>.tsv` **in this person's configuration directory**,
//!   never into the checkout, so an unsigned machine translation does not become the project's;
//! - the work happens on a worker thread, because a batch is a network round trip and a window that
//!   blocks on one looks broken;
//! - the window reloads its catalogs when the answer lands, so the interface changes language in
//!   place.
//!
//! # When it does not run
//!
//! No key: the panel says so, and the interface stays English. A language the build already carries:
//! nothing to do. A translation the validator refuses: reported, and that row keeps showing English.

use std::sync::mpsc::{Receiver, Sender, channel};

use stkcraft_framework_i18n::catalog::{self, Row};
use stkcraft_framework_i18n::{Source, Translations};

/// What the worker reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A batch is being translated, with how many strings it holds.
    Progress {
        /// The language being translated.
        locale: String,
        /// How many strings this run will ask about.
        count: usize,
    },
    /// The catalog was written and is ready to load.
    Done {
        /// The language.
        locale: String,
        /// How many rows the catalog now holds.
        rows: usize,
        /// How many the model proposed but the format refused.
        refused: usize,
    },
    /// Nothing happened, and why.
    Failed(String),
}

/// A translation in flight.
pub struct Run {
    events: Receiver<Event>,
}

impl Run {
    /// Drain what the worker has reported. Never blocks.
    pub fn poll(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        while let Ok(event) = self.events.try_recv() {
            events.push(event);
        }
        events
    }
}

/// Every interface string this build can show, as the build script extracted them.
#[must_use]
pub fn sources() -> Vec<Source> {
    crate::catalogs::SOURCES
        .iter()
        .map(|(context, text)| Source {
            context: (*context).to_owned(),
            text: (*text).to_owned(),
        })
        .collect()
}

/// How many strings a locale still shows in English.
#[must_use]
pub fn missing(translations: &Translations, locale: &str) -> usize {
    let rows = crate::prefs::Prefs::locales_dir()
        .map(|directory| existing_rows(&directory, locale))
        .unwrap_or_default();
    sources()
        .iter()
        .filter(|source| {
            !rows
                .iter()
                .any(|row| row.context == source.context && row.source == source.text)
                && translations.tr_context(locale, &source.context, &source.text) == source.text
        })
        .count()
}

/// The rows already on disk for a locale, so a second run only asks about what is new.
fn existing_rows(directory: &std::path::Path, locale: &str) -> Vec<Row> {
    let Ok(text) = std::fs::read_to_string(directory.join(format!("{locale}.tsv"))) else {
        return Vec::new();
    };
    catalog::parse(locale, &text).0.rows
}

/// Start translating `locale` in the background.
///
/// # Errors
///
/// A message when there is nothing to do or no model to do it with — reported before a thread
/// starts, because a person who picked a language needs the reason now rather than a spinner.
pub fn start(locale: &str) -> Result<Run, String> {
    let model = stkcraft_agent_runtime::DeepSeek::from_environment()?;
    let locale = locale.to_owned();
    let directory = crate::prefs::Prefs::locales_dir().ok_or_else(|| {
        "this platform has no configuration directory to write a catalog to".to_owned()
    })?;
    let (sender, events) = channel();
    let worker_locale = locale.clone();
    std::thread::spawn(move || {
        let outcome = translate(&model, &worker_locale, &directory, &sender);
        let _ = sender.send(match outcome {
            Ok((rows, refused)) => Event::Done {
                locale: worker_locale,
                rows,
                refused,
            },
            Err(message) => Event::Failed(message),
        });
    });
    Ok(Run { events })
}

/// The worker: translate, validate, write.
fn translate<M: stkcraft_agent_runtime::ChatModel>(
    model: &M,
    locale: &str,
    directory: &std::path::Path,
    events: &Sender<Event>,
) -> Result<(usize, usize), String> {
    let wanted = sources();
    let existing = existing_rows(directory, locale);
    let _ = events.send(Event::Progress {
        locale: locale.to_owned(),
        count: wanted.len(),
    });
    let language_name = crate::prefs::language_name_for(locale);
    let catalogue = stkcraft_agent_runtime::translate::translate_catalogue(
        model,
        locale,
        &language_name,
        &wanted,
        &existing,
    )?;
    if catalogue.is_empty() && catalogue.language_name.is_empty() {
        return Err(format!("nothing to translate for `{locale}`"));
    }
    let mut rows = existing;
    rows.extend(catalogue.rows.iter().cloned());
    rows.sort_by(|a, b| {
        (a.context.as_str(), a.source.as_str()).cmp(&(b.context.as_str(), b.source.as_str()))
    });
    rows.dedup_by(|a, b| a.context == b.context && a.source == b.source);
    write_catalog(directory, locale, &catalogue.language_name, &rows)?;
    Ok((rows.len(), catalogue.refused.len()))
}

/// Write a locale's catalog, keeping the format's header and naming the language.
fn write_catalog(
    directory: &std::path::Path,
    locale: &str,
    language_name: &str,
    rows: &[Row],
) -> Result<(), String> {
    std::fs::create_dir_all(directory)
        .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
    let mut text = catalog::header(language_name, locale);
    // The language names itself first, so a reader of the file sees what language it is in.
    let name_row = Row {
        context: stkcraft_framework_i18n::LANGUAGE_NAME_CONTEXT.to_owned(),
        source: stkcraft_framework_i18n::LANGUAGE_NAME_SOURCE.to_owned(),
        translation: language_name.to_owned(),
    };
    text.push_str(&catalog::row_text(&name_row));
    text.push('\n');
    for row in rows {
        if row.is_language_name() {
            continue;
        }
        text.push_str(&catalog::row_text(row));
        text.push('\n');
    }
    let path = directory.join(format!("{locale}.tsv"));
    std::fs::write(&path, text)
        .map_err(|error| format!("could not write {}: {error}", path.display()))
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let directory = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("a scratch directory");
        directory
    }

    #[test]
    fn the_build_script_extracted_the_strings_the_window_shows() {
        // If this is empty the window would translate nothing and never say why.
        let sources = sources();
        assert!(!sources.is_empty(), "no interface strings were embedded");
        assert!(sources.iter().any(|source| source.text == "Clock"));
        assert!(
            sources
                .iter()
                .any(|source| source.context == "@plural" && source.text.contains('|')),
            "no plural row was extracted"
        );
    }

    /// A model that answers from a script, so the worker is testable without a key.
    struct Scripted {
        answer: String,
    }

    impl stkcraft_agent_runtime::ChatModel for Scripted {
        fn complete(
            &self,
            _messages: &[stkcraft_agent_runtime::Message],
            _tools: &[serde_json::Value],
        ) -> Result<stkcraft_agent_runtime::Turn, String> {
            Ok(stkcraft_agent_runtime::Turn {
                text: self.answer.clone(),
                calls: Vec::new(),
            })
        }
    }

    #[test]
    fn the_worker_translates_writes_and_reports() {
        let directory = scratch("stkcraft-runtime-translate-worker");
        // Every string the build embedded, answered with a marked translation.
        let mut answer = serde_json::Map::new();
        for source in sources() {
            let translation = format!("DE {}", source.text).replace('|', "|DE ");
            answer.insert(source.text, serde_json::Value::String(translation));
        }
        answer.insert(
            "language.name".to_owned(),
            serde_json::Value::String("Deutsch".to_owned()),
        );
        let model = Scripted {
            answer: serde_json::Value::Object(answer).to_string(),
        };
        let (sender, events) = channel();
        let (rows, refused) =
            translate(&model, "de", &directory, &sender).expect("the worker runs");
        assert!(rows > 1, "{rows} row(s)");
        assert_eq!(refused, 0, "a marked translation should pass the format");
        let written = std::fs::read_to_string(directory.join("de.tsv")).expect("a catalog");
        let (catalogue, problems) = catalog::parse("de", &written);
        assert_eq!(problems, Vec::new(), "{problems:?}");
        assert_eq!(catalogue.name(), Some("Deutsch"));
        // The progress event reached the panel before the work finished.
        let reported = events.try_recv().expect("a progress event");
        assert!(matches!(reported, Event::Progress { .. }), "{reported:?}");
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_written_catalog_reads_back_and_names_its_language() {
        let directory = scratch("stkcraft-runtime-translate");
        let rows = vec![Row {
            context: String::new(),
            source: "Clock".to_owned(),
            translation: "Uhr".to_owned(),
        }];
        write_catalog(&directory, "de", "Deutsch", &rows).expect("written");
        let text = std::fs::read_to_string(directory.join("de.tsv")).expect("read");
        let (catalogue, problems) = catalog::parse("de", &text);
        assert_eq!(problems, Vec::new(), "{problems:?}");
        assert_eq!(catalogue.name(), Some("Deutsch"));
        assert_eq!(catalogue.len(), 2);
        // The header carries the clean-room line, as the format requires.
        assert!(text.contains("Clean room"), "the header is missing");
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn missing_counts_only_what_still_shows_in_english() {
        // With no catalog on disk, everything is missing; the count is what the panel offers to fix.
        let translations =
            stkcraft_framework_i18n::Translations::from_embedded(crate::catalogs::EMBEDDED);
        let english_only = missing(&translations, "zz");
        assert_eq!(english_only, sources().len());
    }
}
