//! `cargo xtask i18n`: check the interface catalogs, report coverage, and translate them.
//!
//! Three jobs, one task, because they are the same job seen three ways:
//!
//! - **`i18n` / `i18n check`** — parse every catalog under `locales/` and refuse a row the format
//!   forbids. This is the family's CI check: the same rules the shared catalog project enforces.
//! - **`i18n report`** — what each language still lacks, against the English strings the code uses.
//!   A missing row is not an error (it shows in English); it is a to-do list.
//! - **`i18n translate --language <code>`** — fill the missing rows of one language from the
//!   English source, using `DeepSeek` and the key in `DEEPSEEK_API_KEY`.
//!
//! # Why translation is a task and not a feature
//!
//! The interface ships one English source and a catalog per language. A translation is therefore an
//! *input the project does not author*: either a person writes it, or a model proposes it and a
//! person reviews the pull request. Keeping the generator here — outside the app, in a developer
//! tool, behind the same key the assistant uses — means the kernel never gains a network call and a
//! release never depends on a provider being reachable.
//!
//! # The source strings
//!
//! The code calls one of three macros, and this task reads them out of the source text:
//!
//! ```text
//! tr!(translations, locale, "Save")                  → source "Save", no context
//! trc!(translations, locale, "menu", "Open Table…")  → context "menu", source "Open Table…"
//! trn!(translations, locale, "{n} page", "{n} pages") → plural source "{n} page|{n} pages"
//! ```
//!
//! A literal in one of those calls is a string a translator will see, and a string anywhere else is
//! not translated — which is why the convention is "every user-visible string goes through a
//! macro": it makes the extractor exact instead of heuristic.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use stkcraft_i18n::Source as Extracted;
use stkcraft_i18n::catalog::{self, Catalog, Row};

/// The directory the catalogs live in, relative to the workspace root.
pub const LOCALES: &str = "locales";

/// The catalog file an app's rows go in; one per app, as the shared format names it.
pub const APP_CATALOG: &str = "stkcraft.tsv";

/// What `cargo xtask i18n` was asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Parse every catalog and refuse what the format forbids.
    Check,
    /// Check, then report what each language lacks.
    Report,
    /// Fill one language's missing rows (or every language's, with `--all`).
    Translate {
        /// The languages to translate; empty means every locale that has a directory.
        languages: Vec<String>,
        /// Print the proposed rows instead of writing them.
        dry_run: bool,
        /// Include rows that already exist, so a bad translation can be redone.
        refresh: bool,
    },
}

/// How this task was invoked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// What to do.
    pub action: Action,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            action: Action::Check,
        }
    }
}

/// Parse the task's arguments.
///
/// # Errors
///
/// A message naming the argument that could not be understood.
pub fn parse_options(args: &[String]) -> Result<Options, String> {
    let mut action = Action::Check;
    let mut transfer = false;
    let mut languages = Vec::new();
    let mut dry_run = false;
    let mut refresh = false;
    let mut index = 0;
    while index < args.len() {
        let argument = args.get(index).map(String::as_str).unwrap_or_default();
        match argument {
            "check" => action = Action::Check,
            "report" => action = Action::Report,
            "translate" => transfer = true,
            "--language" | "--locale" => {
                index += 1;
                let language = args.get(index).ok_or_else(|| {
                    "`--language` needs a locale code, e.g. `--language de`".to_owned()
                })?;
                languages.push(language.clone());
            }
            "--dry-run" => dry_run = true,
            "--refresh" => refresh = true,
            other => {
                return Err(format!(
                    "unknown `i18n` option `{other}`; try `cargo xtask help`"
                ));
            }
        }
        index += 1;
    }
    if transfer {
        action = Action::Translate {
            languages,
            dry_run,
            refresh,
        };
    } else if dry_run || refresh || !languages.is_empty() {
        return Err(
            "`--language`, `--dry-run` and `--refresh` belong to `i18n translate`".to_owned(),
        );
    }
    Ok(Options { action })
}

/// Run the task.
///
/// # Errors
///
/// A message for the developer: a catalog the format refuses, a missing key, a model that answered
/// something unusable, or a file that could not be written.
pub fn run(root: &Path, options: &Options) -> Result<(), String> {
    let catalogs = read_catalogs(root)?;
    let mut problems: Vec<String> = Vec::new();
    for (locale, catalog, catalog_problems) in &catalogs {
        for problem in catalog_problems {
            problems.push(format!("{LOCALES}/{locale}/{APP_CATALOG}: {problem}"));
        }
        if catalog.name().is_none() {
            problems.push(format!(
                "{LOCALES}/{locale}/{APP_CATALOG}: no `@id\tlanguage.name` row, so the language \
                 picker cannot name this language"
            ));
        }
    }
    if !problems.is_empty() {
        for problem in &problems {
            eprintln!("i18n: {problem}");
        }
        return Err(format!(
            "{} catalog problem(s); the format is described in docs/localization.md",
            problems.len()
        ));
    }

    let sources = extract_sources(root)?;
    println!(
        "i18n: {} English string(s) in the source, {} catalog(s)",
        sources.len(),
        catalogs.len()
    );
    match &options.action {
        Action::Check => {
            println!("i18n: every catalog is well-formed");
            Ok(())
        }
        Action::Report => {
            print!("{}", coverage(&sources, &catalogs));
            Ok(())
        }
        Action::Translate {
            languages,
            dry_run,
            refresh,
        } => translate(root, &sources, &catalogs, languages, *dry_run, *refresh),
    }
}

/// One locale's catalog as read from disk.
type Loaded = (String, Catalog, Vec<catalog::Problem>);

/// Read every catalog under `locales/<locale>/stkcraft.tsv`, in locale order.
#[allow(clippy::unnecessary_wraps)]
fn read_catalogs(root: &Path) -> Result<Vec<Loaded>, String> {
    let directory = root.join(LOCALES);
    let mut loaded = Vec::new();
    // No catalogs at all is a valid state: the app is English-only until one lands.
    let Ok(entries) = std::fs::read_dir(&directory) else {
        return Ok(loaded);
    };
    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let locale = entry.file_name().to_string_lossy().into_owned();
        if locale.starts_with('.') {
            continue;
        }
        let file = entry.path().join(APP_CATALOG);
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        let (catalog, problems) = catalog::parse(&locale, &text);
        loaded.push((locale, catalog, problems));
    }
    loaded.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(loaded)
}

/// A string the interface shows, as the source extraction found it: the type the shared extractor
/// produces, so this task and a running window answer "which strings exist" the same way.
type Source = Extracted;

/// Every English string the code translates, in a stable order.
///
/// A string used twice appears once: a catalog translates a string, not an occurrence. The count is
/// kept for the report, because "used in four places, untranslated" is a different to-do from
/// "used once".
fn extract_sources(root: &Path) -> Result<BTreeMap<Source, usize>, String> {
    let mut sources: BTreeMap<Source, usize> = BTreeMap::new();
    let mut files = Vec::new();
    for directory in ["crates", "apps"] {
        collect_rust_files(&root.join(directory), &mut files)?;
    }
    for file in files {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        for source in stkcraft_i18n::extract::sources(&text) {
            *sources.entry(source).or_insert(0) += 1;
        }
    }
    Ok(sources)
}

/// Collect `.rs` files under a directory, skipping build output.
fn collect_rust_files(directory: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name == "target") {
                continue;
            }
            collect_rust_files(&path, files)?;
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
    Ok(())
}

/// One line per language: how many source strings it lacks, and the first few of them.
fn coverage(sources: &BTreeMap<Source, usize>, catalogs: &[Loaded]) -> String {
    /// How many missing strings to name before summarising the rest.
    const LISTED: usize = 12;
    let reference: BTreeSet<(String, String)> = sources.keys().map(Source::key).collect();
    let translated: BTreeSet<(String, String)> = catalogs
        .iter()
        .flat_map(|(_, catalog, _)| {
            catalog
                .rows
                .iter()
                .map(|row| (row.context.clone(), row.source.clone()))
        })
        .filter(|key| reference.contains(key))
        .collect();
    let _ = &translated;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "i18n-report: {} English string(s); a string a catalog lacks shows in English",
        sources.len()
    );
    if catalogs.is_empty() {
        let _ = writeln!(out, "  no catalogs: the interface is English-only");
        return out;
    }
    for (locale, catalog, _) in catalogs {
        let own: BTreeSet<(String, String)> = catalog
            .rows
            .iter()
            .map(|row| (row.context.clone(), row.source.clone()))
            .collect();
        let missing: Vec<&Source> = sources
            .keys()
            .filter(|source| !own.contains(&source.key()))
            .collect();
        let stale = catalog
            .rows
            .iter()
            .filter(|row| !row.is_language_name() && !sources.contains_key(&source_of(row)))
            .count();
        let translated_here = own.iter().filter(|key| reference.contains(*key)).count();
        let _ = writeln!(
            out,
            "  {locale} ({}): {translated_here}/{}, {missing} missing, {stale} unused",
            catalog.name().unwrap_or("unnamed"),
            sources.len(),
            missing = missing.len()
        );
        for source in missing.iter().take(LISTED) {
            let _ = writeln!(
                out,
                "     {}{}",
                if source.context.is_empty() {
                    String::new()
                } else {
                    format!("[{}] ", source.context)
                },
                source.text
            );
        }
        if missing.len() > LISTED {
            let _ = writeln!(out, "     … and {} more", missing.len() - LISTED);
        }
    }
    let _ = translated;
    out
}

/// The source a catalog row translates.
fn source_of(row: &Row) -> Source {
    Source {
        context: row.context.clone(),
        text: row.source.clone(),
    }
}

/// Fill the missing rows of one or more languages.
fn translate(
    root: &Path,
    sources: &BTreeMap<Source, usize>,
    catalogs: &[Loaded],
    languages: &[String],
    dry_run: bool,
    refresh: bool,
) -> Result<(), String> {
    let locales: Vec<String> = if languages.is_empty() {
        catalogs
            .iter()
            .map(|(locale, _, _)| locale.clone())
            .collect()
    } else {
        languages.to_vec()
    };
    if locales.is_empty() {
        return Err(
            "no catalog to translate: pass `--language <code>` or create \
             locales/<code>/stkcraft.tsv first"
                .to_owned(),
        );
    }
    // Fail on a missing key before doing anything else, with the sentence a reader needs. The model
    // itself is built per language below.
    std::env::var("DEEPSEEK_API_KEY").map_err(|_| {
        "DEEPSEEK_API_KEY is not set: translation needs a model, and the key is yours to supply"
            .to_owned()
    })?;
    for locale in &locales {
        let existing = catalogs
            .iter()
            .find(|(code, _, _)| code == locale)
            .map_or_else(
                || Catalog {
                    locale: locale.clone(),
                    rows: Vec::new(),
                },
                |(_, catalog, _)| catalog.clone(),
            );
        let language_name = existing
            .name()
            .map_or_else(|| suggested_name(locale), str::to_owned);
        let want: Vec<&Source> = sources
            .keys()
            .filter(|source| {
                // A `@plural` row is translated as one row with every form the language uses, so it
                // is requested like any other source.
                let translated = existing
                    .rows
                    .iter()
                    .any(|row| row.context == source.context && row.source == source.text);
                refresh || !translated
            })
            .collect();
        if want.is_empty() {
            println!("i18n: {locale} is complete");
            continue;
        }
        println!(
            "i18n: {locale}: {} row(s) to translate{}",
            want.len(),
            if dry_run { " (dry run)" } else { "" }
        );
        let (proposed, translated_name) =
            translate_rows(locale, &language_name, &want, &existing.rows)?;
        let mut rows: BTreeMap<(String, String), String> = existing
            .rows
            .iter()
            .map(|row| {
                (
                    (row.context.clone(), row.source.clone()),
                    row.translation.clone(),
                )
            })
            .collect();
        for (source, translation) in &proposed {
            rows.insert(source.key(), translation.clone());
        }
        // The language names itself, as every catalog must: a picker that shows "de" is a picker
        // nobody can read.
        rows.insert(
            (
                stkcraft_i18n::LANGUAGE_NAME_CONTEXT.to_owned(),
                stkcraft_i18n::LANGUAGE_NAME_SOURCE.to_owned(),
            ),
            translated_name.clone(),
        );
        let language_name = translated_name;
        if dry_run {
            for source in &want {
                if let Some(translation) = rows.get(&source.key()) {
                    println!("  [{}] {} → {translation}", source.context, source.text);
                }
            }
            continue;
        }
        let path = root.join(LOCALES).join(locale).join(APP_CATALOG);
        write_catalog(&path, locale, &language_name, &rows)?;
        println!("i18n: {locale}: wrote {}", path.display());
    }
    Ok(())
}

/// A language's name, for a catalog that does not name itself yet.
///
/// The model is asked for the name too, so this is only the placeholder that goes into the header
/// before the translation arrives; it is replaced by the model's answer when it gives one.
fn suggested_name(locale: &str) -> String {
    locale.to_owned()
}

/// Ask the model for the translations of one language's missing rows.
///
/// The batch size, the prompt and the acceptance rules are the shared ones in `stkcraft-agent`, so a
/// catalog generated here and one a window generates for itself cannot disagree about what a valid
/// row is. What stays here is the part that is this task's job: reading the answer into the catalog
/// representation a file is written from.
fn translate_rows(
    locale: &str,
    language_name: &str,
    sources: &[&Source],
    existing: &[Row],
) -> Result<(Vec<(Source, String)>, String), String> {
    let model = stkcraft_agent::DeepSeek::from_environment()?;
    let wanted: Vec<Source> = sources.iter().map(|source| (*source).clone()).collect();
    let catalogue = stkcraft_agent::translate::translate_catalogue(
        &model,
        locale,
        language_name,
        &wanted,
        existing,
    )?;
    for (text, reason) in &catalogue.refused {
        eprintln!("i18n: refused `{text}`: {reason}");
    }
    let rows = catalogue
        .rows
        .into_iter()
        .map(|row| {
            (
                Source {
                    context: row.context,
                    text: row.source,
                },
                row.translation,
            )
        })
        .collect();
    Ok((rows, catalogue.language_name))
}

/// Write a catalog, keeping the format's header and sorting the rows.
fn write_catalog(
    path: &Path,
    locale: &str,
    language_name: &str,
    rows: &BTreeMap<(String, String), String>,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    let mut text = catalog::header(language_name, locale);
    let mut rows: Vec<(&(String, String), &String)> = rows.iter().collect();
    // The language's own name first, then the interface strings in a stable order, so a diff of a
    // translated catalog is about translations and not about row order.
    rows.sort_by(|a, b| (a.0).cmp(b.0));
    let name_row = (
        stkcraft_i18n::LANGUAGE_NAME_CONTEXT.to_owned(),
        stkcraft_i18n::LANGUAGE_NAME_SOURCE.to_owned(),
    );
    if let Some(translation) = rows
        .iter()
        .find(|(key, _)| **key == name_row)
        .map(|(_, translation)| (*translation).clone())
    {
        let row = Row {
            context: name_row.0.clone(),
            source: name_row.1.clone(),
            translation,
        };
        text.push_str(&catalog::row_text(&row));
        text.push('\n');
    }
    for (key, translation) in rows {
        if *key == name_row {
            continue;
        }
        let row = Row {
            context: key.0.clone(),
            source: key.1.clone(),
            translation: (*translation).clone(),
        };
        text.push_str(&catalog::row_text(&row));
        text.push('\n');
    }
    std::fs::write(path, text)
        .map_err(|error| format!("could not write {}: {error}", path.display()))
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn the_default_action_is_the_check() {
        let options = parse_options(&[]).expect("no arguments is the check");
        assert_eq!(options.action, Action::Check);
        let options = parse_options(&["report".to_owned()]).expect("report");
        assert_eq!(options.action, Action::Report);
    }

    #[test]
    fn translate_takes_every_language_it_is_given() {
        let options = parse_options(&[
            "translate".to_owned(),
            "--language".to_owned(),
            "de".to_owned(),
            "--language".to_owned(),
            "zh".to_owned(),
            "--dry-run".to_owned(),
        ])
        .expect("translate");
        assert_eq!(
            options.action,
            Action::Translate {
                languages: vec!["de".to_owned(), "zh".to_owned()],
                dry_run: true,
                refresh: false,
            }
        );
    }

    #[test]
    fn a_translate_option_outside_translate_is_refused() {
        let error = parse_options(&["--language".to_owned(), "de".to_owned()])
            .expect_err("`--language` needs `translate`");
        assert!(error.contains("translate"), "{error}");
        let error = parse_options(&["translate".to_owned(), "--language".to_owned()])
            .expect_err("`--language` needs a value");
        assert!(error.contains("locale code"), "{error}");
    }

    #[test]
    fn a_written_catalog_round_trips_through_the_parser() {
        let directory = std::env::temp_dir().join("stkcraft-i18n-test");
        let path = directory.join("stkcraft.tsv");
        let mut rows = BTreeMap::new();
        rows.insert(
            (
                stkcraft_i18n::LANGUAGE_NAME_CONTEXT.to_owned(),
                stkcraft_i18n::LANGUAGE_NAME_SOURCE.to_owned(),
            ),
            "Deutsch".to_owned(),
        );
        rows.insert((String::new(), "Save".to_owned()), "Speichern".to_owned());
        write_catalog(&path, "de", "Deutsch", &rows).expect("written");
        let text = std::fs::read_to_string(&path).expect("read");
        let (catalog, problems) = catalog::parse("de", &text);
        assert_eq!(problems, Vec::new());
        assert_eq!(catalog.name(), Some("Deutsch"));
        assert_eq!(catalog.len(), 2);
        // The name row comes first, so a reader sees what language the file is in.
        assert!(text.contains("@id\tlanguage.name\tDeutsch     ".trim()));
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn coverage_names_the_missing_strings() {
        let mut sources = BTreeMap::new();
        sources.insert(
            Source {
                context: String::new(),
                text: "Save".to_owned(),
            },
            1,
        );
        sources.insert(
            Source {
                context: String::new(),
                text: "Open".to_owned(),
            },
            2,
        );
        let (catalog, _) = catalog::parse("de", "@id\tlanguage.name\tDeutsch\n\tSave\tSpeichern\n");
        let report = coverage(&sources, &[("de".to_owned(), catalog, Vec::new())]);
        assert!(report.contains("de (Deutsch): 1/2"), "{report}");
        assert!(report.contains("Open"), "{report}");
    }
}
