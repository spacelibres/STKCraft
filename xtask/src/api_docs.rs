//! `cargo xtask api-docs`: the API reference, generated from the code and translated.
//!
//! The reference is not written by hand, because a hand-written list of commands is a list that
//! drifts from the registry the first time a command lands. It is built from three things the code
//! already says:
//!
//! 1. **The command registry** — every command's id, label, menu path and parameters, including the
//!    [`ParameterKind`](stkcraft_astro_engine::commands::ParameterKind) that decides how a caller's value
//!    is converted. This is the surface an agent, an MCP client and the CLI all use.
//! 2. **The crate manifests and module documentation** — each crate's name, layer and description,
//!    and the `//!` documentation at its root.
//! 3. **The version**, so a reader can tell which release a page describes.
//!
//! # Translation
//!
//! One English reference is maintained. `api-docs translate --language <code>` produces the same
//! document in another language by translating its *prose* — headings, paragraphs, labels and
//! parameter descriptions — while leaving code, ids and numbers alone, and reusing every string it
//! translated last time from a sidecar file. The translated page carries the English revision it was
//! made from, so a stale translation is visible rather than trusted.
//!
//! The two halves are the same pipeline as the interface catalogs: English source text, a batch of
//! `DeepSeek` proposals, a check that nothing was invented, a file a person reviews.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use stkcraft_astro_engine::commands::{Parameter, ParameterKind, Registry};
use stkcraft_framework_i18n::catalog::{self, Catalog, Row};

use crate::layers::{APP_LAYER, LAYOUT};

/// Where the generated reference lives, relative to the workspace root.
pub const API_DIR: &str = "docs/api";

/// The English page's file name.
pub const PAGE: &str = "index.md";

/// What `cargo xtask api-docs` was asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Generate the English reference.
    Generate,
    /// Check that the committed reference matches the code.
    ///
    /// The gate runs this, not `Generate`: a check that rewrites the file it is checking passes on
    /// the first run and hides a stale reference forever after.
    Check,
    /// Translate it into one or more languages.
    Translate {
        /// The locales to write.
        languages: Vec<String>,
        /// Print the proposals instead of writing them.
        dry_run: bool,
    },
}

/// One table column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Column {
    /// A column of prose: a label or a description a reader reads.
    Text,
    /// A column of identifiers and numbers: never translated.
    Code,
}

/// One cell of a table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cell {
    /// Translatable prose.
    Text(String),
    /// Something that is not translated: an id, a number, a type.
    Code(String),
    /// Markdown that is neither wrapped in backticks nor translated.
    ///
    /// A command's parameter documentation is ids and prose in one cell — `name` (text, required):
    /// a display name — and wrapping the whole thing in backticks produces a cell a Markdown reader
    /// sees as broken code. The English parameter text stays English here for now; the same text is
    /// translated in the interface, where the catalogs cover it.
    Raw(String),
}

/// A table in the reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    /// The column headings, and whether each is translated.
    pub columns: Vec<(Column, String)>,
    /// The rows.
    pub rows: Vec<Vec<Cell>>,
}

/// A section of the reference: a heading, prose, and at most one table.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Section {
    /// The heading text, translated.
    pub heading: String,
    /// Paragraphs, translated.
    pub prose: Vec<String>,
    /// The section's table, if it has one.
    pub table: Option<Table>,
}

/// The whole reference, before it is rendered or translated.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Document {
    /// The title, translated.
    pub title: String,
    /// A line naming the version and the tool, not translated.
    pub banner: String,
    /// The sections, in order.
    pub sections: Vec<Section>,
}

impl Document {
    /// Build the document from the code.
    ///
    /// # Errors
    ///
    /// A message when a crate manifest cannot be read: a reference that silently omits a crate is
    /// worse than one that refuses to be generated.
    pub fn from_code(root: &Path) -> Result<Self, String> {
        let version = crate::workspace_version(root)?;
        let registry = Registry::builtin();
        let mut sections = vec![
            Section {
                heading: "About this reference".to_owned(),
                prose: vec![
                    "This page is generated from STKCraft's source: the command registry, the crate \
                     manifests and the module documentation. Editing it by hand is pointless — the \
                     next generation overwrites it. Edit the code, or the document that describes \
                     the design."
                        .to_owned(),
                    "Every command is reachable by every transport. The menus, the shortcuts, the \
                     command line, the control channel, the Model Context Protocol server and the \
                     assistant all dispatch a command by its id, so the table below is not an \
                     integration guide for one client: it is the surface."
                        .to_owned(),
                ],
                table: None,
            },
            Section {
                heading: "Commands".to_owned(),
                prose: vec![
                    "A command is the unit of user-visible behaviour. Its id never changes once \
                     released; its parameters declare their kind, so a caller sends a number, text \
                     or degrees and the engine receives the typed value."
                        .to_owned(),
                ],
                table: Some(command_table(&registry)),
            },
            Section {
                heading: "Crates".to_owned(),
                prose: vec![
                    "The workspace is layered: a crate may depend only on a strictly lower layer, so \
                     the analysis can run in a window, in a terminal and in a browser tab without \
                     knowing which it is in."
                        .to_owned(),
                ],
                table: Some(crate_table(root)?),
            },
        ];
        sections.extend(crate_sections(root)?);
        Ok(Self {
            title: "STKCraft API reference".to_owned(),
            banner: format!(
                "Generated by `cargo xtask api-docs` from STKCraft {version} — do not edit by hand."
            ),
            sections,
        })
    }

    /// Every string a translator has to handle, with where it lives.
    fn strings(&self) -> Vec<String> {
        let mut strings = vec![self.title.clone()];
        for section in &self.sections {
            strings.push(section.heading.clone());
            strings.extend(section.prose.iter().cloned());
            if let Some(table) = &section.table {
                for (column, heading) in &table.columns {
                    if *column == Column::Text {
                        strings.push(heading.clone());
                    }
                }
                for row in &table.rows {
                    for cell in row {
                        if let Cell::Text(text) = cell {
                            strings.push(text.clone());
                        }
                    }
                }
            }
        }
        strings.sort();
        strings.dedup();
        strings
    }

    /// The document with `translations` applied; an untranslated string stays English.
    fn translated(&self, translations: &BTreeMap<String, String>) -> Self {
        let translate = |text: &str| -> String {
            translations
                .get(text)
                .cloned()
                .unwrap_or_else(|| text.to_owned())
        };
        Self {
            title: translate(&self.title),
            banner: self.banner.clone(),
            sections: self
                .sections
                .iter()
                .map(|section| Section {
                    heading: translate(&section.heading),
                    prose: section.prose.iter().map(|line| translate(line)).collect(),
                    table: section.table.as_ref().map(|table| Table {
                        columns: table
                            .columns
                            .iter()
                            .map(|(column, heading)| {
                                let heading = match column {
                                    Column::Text => translate(heading),
                                    Column::Code => heading.clone(),
                                };
                                (*column, heading)
                            })
                            .collect(),
                        rows: table
                            .rows
                            .iter()
                            .map(|row| {
                                row.iter()
                                    .map(|cell| match cell {
                                        Cell::Text(text) => Cell::Text(translate(text)),
                                        Cell::Code(text) => Cell::Code(text.clone()),
                                        Cell::Raw(text) => Cell::Raw(text.clone()),
                                    })
                                    .collect()
                            })
                            .collect(),
                    }),
                })
                .collect(),
        }
    }

    /// Render the document as Markdown.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "# {}\n", self.title);
        let _ = writeln!(out, "*{}*\n", self.banner);
        for section in &self.sections {
            let _ = writeln!(out, "## {}\n", section.heading);
            for paragraph in &section.prose {
                let _ = writeln!(out, "{paragraph}\n");
            }
            if let Some(table) = &section.table {
                let headings: Vec<&str> = table
                    .columns
                    .iter()
                    .map(|(_, heading)| heading.as_str())
                    .collect();
                let _ = writeln!(out, "| {} |", headings.join(" | "));
                let _ = writeln!(
                    out,
                    "|{}|",
                    table
                        .columns
                        .iter()
                        .map(|(column, _)| match column {
                            Column::Text => "---",
                            Column::Code => ":---",
                        })
                        .collect::<Vec<_>>()
                        .join("|")
                );
                for row in &table.rows {
                    let cells: Vec<String> = row
                        .iter()
                        .map(|cell| match cell {
                            Cell::Code(text) => format!("`{}`", text.replace('|', "\\|")),
                            // Prose and pre-formatted Markdown differ only in who wrote the
                            // backticks; both are escaped so a pipe cannot end the cell early.
                            Cell::Text(text) | Cell::Raw(text) => escape_cell(text),
                        })
                        .collect();
                    let _ = writeln!(out, "| {} |", cells.join(" | "));
                }
                out.push('\n');
            }
        }
        let _ = writeln!(
            out,
            "---\n\nGenerated from the source; see `docs/standards.md` §4 for why it is not hand-written."
        );
        out
    }
}

/// Escape the characters that would break a Markdown table cell.
fn escape_cell(text: &str) -> String {
    text.replace('|', "\\|").replace('\n', " ")
}

/// One row per command, with its parameters.
fn command_table(registry: &Registry) -> Table {
    let mut rows = Vec::new();
    for command in registry.iter() {
        let descriptor = &command.descriptor;
        let parameters: Vec<String> = descriptor
            .parameters
            .iter()
            .map(|parameter: &Parameter| {
                format!(
                    "`{}` ({}{}): {}",
                    parameter.name,
                    kind_name(parameter.kind),
                    if parameter.required { ", required" } else { "" },
                    parameter.description.trim_end_matches('.')
                )
            })
            .collect();
        rows.push(vec![
            Cell::Code(descriptor.id.to_owned()),
            Cell::Text(descriptor.label.to_owned()),
            Cell::Text(descriptor.menu_path.to_owned()),
            Cell::Raw(if parameters.is_empty() {
                "—".to_owned()
            } else {
                parameters.join("<br>")
            }),
        ]);
    }
    Table {
        columns: vec![
            (Column::Code, "Id".to_owned()),
            (Column::Text, "Label".to_owned()),
            (Column::Text, "Menu".to_owned()),
            (Column::Code, "Parameters".to_owned()),
        ],
        rows,
    }
}

/// The kind of a parameter, as a reader needs it: is the number degrees or plain?
fn kind_name(kind: ParameterKind) -> &'static str {
    match kind {
        ParameterKind::Number => "number",
        ParameterKind::Text => "text",
        ParameterKind::Degrees => "degrees",
    }
}

/// One row per crate: name, layer, description.
fn crate_table(root: &Path) -> Result<Table, String> {
    let mut rows = Vec::new();
    for spec in LAYOUT.iter().filter(|spec| spec.layer < APP_LAYER) {
        let manifest = read_manifest(root, spec.path)?;
        rows.push(vec![
            Cell::Code(spec.name.to_owned()),
            Cell::Code(format!("L{}", spec.layer)),
            Cell::Text(manifest.description),
        ]);
    }
    Ok(Table {
        columns: vec![
            (Column::Code, "Crate".to_owned()),
            (Column::Code, "Layer".to_owned()),
            (Column::Text, "What it is".to_owned()),
        ],
        rows,
    })
}

/// One section per crate, from the module documentation at its root.
fn crate_sections(root: &Path) -> Result<Vec<Section>, String> {
    let mut sections = Vec::new();
    for spec in LAYOUT.iter().filter(|spec| spec.layer < APP_LAYER) {
        let manifest = read_manifest(root, spec.path)?;
        let prose = module_paragraphs(&root.join(spec.path).join("src/lib.rs"))?;
        if prose.is_empty() {
            continue;
        }
        sections.push(Section {
            heading: spec.name.to_owned(),
            prose,
            table: None,
        });
        let _ = &manifest;
    }
    Ok(sections)
}

/// What a crate manifest says about itself.
struct Manifest {
    /// The `description` field, in one line.
    description: String,
}

/// Read a crate's manifest description.
fn read_manifest(root: &Path, crate_path: &str) -> Result<Manifest, String> {
    let path = root.join(crate_path).join("Cargo.toml");
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    let parsed: toml::Value = text
        .parse()
        .map_err(|error| format!("could not parse {}: {error}", path.display()))?;
    let description = parsed
        .get("package")
        .and_then(|package| package.get("description"))
        .and_then(toml::Value::as_str)
        .unwrap_or_default()
        .to_owned();
    Ok(Manifest { description })
}

/// The paragraphs of a module's `//!` documentation.
///
/// Only the prose: a line that starts with `//! #` is a heading in the source and is left out of the
/// reference, because the reference has its own headings, and a code block is left out for the same
/// reason — this page describes an interface, it does not re-print the crate's documentation.
#[allow(clippy::unnecessary_wraps)]
fn module_paragraphs(path: &Path) -> Result<Vec<String>, String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Ok(Vec::new());
    };
    let mut paragraphs = Vec::new();
    let mut current = String::new();
    let mut in_code = false;
    for line in text.lines() {
        let Some(doc) = line.strip_prefix("//!") else {
            // The module documentation is a prefix of the file; the first item ends it.
            if !line.trim().is_empty() && !line.starts_with("#![") && !current.is_empty() {
                paragraphs.push(std::mem::take(&mut current));
            }
            continue;
        };
        let doc = doc.trim();
        if doc.starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            continue;
        }
        if doc.is_empty() {
            if !current.is_empty() {
                paragraphs.push(std::mem::take(&mut current));
            }
            continue;
        }
        if doc.starts_with('#') {
            continue;
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(doc);
    }
    if !current.is_empty() {
        paragraphs.push(current);
    }
    Ok(paragraphs)
}

/// Parse the task's arguments.
///
/// # Errors
///
/// A message naming the argument that could not be understood.
pub fn parse_options(args: &[String]) -> Result<Action, String> {
    let mut action = Action::Generate;
    let mut languages = Vec::new();
    let mut dry_run = false;
    let mut translating = false;
    let mut index = 0;
    while index < args.len() {
        match args.get(index).map(String::as_str).unwrap_or_default() {
            "generate" => action = Action::Generate,
            "check" => action = Action::Check,
            "translate" => translating = true,
            "--language" | "--locale" => {
                index += 1;
                let language = args.get(index).ok_or_else(|| {
                    "`--language` needs a locale code, e.g. `--language de`".to_owned()
                })?;
                languages.push(language.clone());
            }
            "--dry-run" => dry_run = true,
            other => {
                return Err(format!(
                    "unknown `api-docs` option `{other}`; try `cargo xtask help`"
                ));
            }
        }
        index += 1;
    }
    if translating {
        if languages.is_empty() {
            return Err("`api-docs translate` needs `--language <code>`".to_owned());
        }
        action = Action::Translate { languages, dry_run };
    } else if dry_run || !languages.is_empty() {
        return Err("`--language` and `--dry-run` belong to `api-docs translate`".to_owned());
    }
    Ok(action)
}

/// Run the task.
///
/// # Errors
///
/// A message for the developer: an unreadable manifest, a missing key, or a model answer that
/// cannot be used.
pub fn run(root: &Path, action: &Action) -> Result<(), String> {
    let document = Document::from_code(root)?;
    let english = root.join(API_DIR).join("en").join(PAGE);
    let rendered = document.render();
    if *action == Action::Check {
        let committed = std::fs::read_to_string(&english).unwrap_or_default();
        if committed != rendered {
            return Err(format!(
                "{} is stale: run `cargo xtask api-docs` and commit the result",
                english.display()
            ));
        }
        println!(
            "api-docs: {} matches the code ({} string(s) to translate)",
            english.display(),
            document.strings().len()
        );
        return Ok(());
    }
    write(&english, &rendered)?;
    println!(
        "api-docs: wrote {} ({} string(s) to translate)",
        english.display(),
        document.strings().len()
    );
    match action {
        Action::Generate | Action::Check => Ok(()),
        Action::Translate { languages, dry_run } => {
            let key = std::env::var("DEEPSEEK_API_KEY").map_err(|_| {
                "DEEPSEEK_API_KEY is not set: translating the reference needs a model, and the key \
                 is yours to supply"
                    .to_owned()
            })?;
            for language in languages {
                if *dry_run {
                    println!(
                        "api-docs: {language}: {} string(s) (dry run)",
                        document.strings().len()
                    );
                    continue;
                }
                let translations = translate_document(root, language, &document, &key)?;
                let translated = document.translated(&translations);
                let path = root.join(API_DIR).join(language).join(PAGE);
                let mut text = translated.render();
                // A translated page names the English revision it came from, so a stale
                // translation is visible rather than believed.
                let _ = writeln!(
                    text,
                    "\n*Translated from the English reference of STKCraft {}; the English page is \
                     authoritative and this one may lag behind it.*",
                    crate::workspace_version(root)?
                );
                write(&path, &text)?;
                println!("api-docs: wrote {}", path.display());
            }
            Ok(())
        }
    }
}

/// Write a file, creating its directory.
fn write(path: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    std::fs::write(path, text)
        .map_err(|error| format!("could not write {}: {error}", path.display()))
}

/// The sidecar file holding the strings already translated for one language.
fn strings_file(root: &Path, language: &str) -> PathBuf {
    root.join(API_DIR).join(language).join("strings.tsv")
}

/// Translate a document's strings, reusing what a previous run translated.
fn translate_document(
    root: &Path,
    language: &str,
    document: &Document,
    key: &str,
) -> Result<BTreeMap<String, String>, String> {
    let path = strings_file(root, language);
    let known: BTreeMap<String, String> = std::fs::read_to_string(&path)
        .ok()
        .map(|text| catalog::parse(language, &text).0)
        .map(|catalog: Catalog| {
            catalog
                .rows
                .iter()
                .map(|row| (row.source.clone(), row.translation.clone()))
                .collect()
        })
        .unwrap_or_default();
    let wanted = document.strings();
    let missing: Vec<&String> = wanted
        .iter()
        .filter(|text| !known.contains_key(text.as_str()))
        .collect();
    let mut translations = known;
    if missing.is_empty() {
        println!("api-docs: {language}: every string is already translated");
        return Ok(translations);
    }
    println!(
        "api-docs: {language}: {} string(s) to translate ({} remembered)",
        missing.len(),
        translations.len()
    );
    let batch: Vec<serde_json::Value> = missing
        .iter()
        .map(|text| serde_json::json!({"english": text, "placeholders": catalog::placeholders(text)}))
        .collect();
    let prompt = format!(
        "Translate the prose of STKCraft's generated API reference into the language with the BCP \
         47 code `{language}`.\n\
         \n\
         The input is a JSON array of `english` strings and the `placeholders` they contain. These \
         are headings, paragraphs, menu labels and parameter descriptions of a technical document \
         about orbital analysis: keep identifiers, command ids, numbers and units exactly as they \
         are, keep every placeholder, and translate the meaning rather than the words. Leave a \
         trailing `…` in place if the English has one.\n\
         \n\
         Answer with JSON only, an object whose keys are the exact `english` strings and whose \
         values are the translations.\n\
         \n\
         Input:\n{}",
        serde_json::to_string_pretty(&batch).unwrap_or_else(|_| "[]".to_owned())
    );
    let answer = crate::translate::chat_completion(key, &prompt)?;
    let json = crate::translate::json_object_in(&answer)
        .ok_or_else(|| format!("the model's answer is not a JSON object: {answer}"))?;
    for text in missing {
        let Some(translation) = json.get(text).and_then(serde_json::Value::as_str) else {
            eprintln!("api-docs: no translation for `{text}`");
            continue;
        };
        if catalog::placeholders(text) != catalog::placeholders(translation) {
            eprintln!("api-docs: refused `{text}`: the placeholders do not match");
            continue;
        }
        translations.insert(text.clone(), translation.to_owned());
    }
    let rows: Vec<Row> = translations
        .iter()
        .map(|(source, translation)| Row {
            context: String::new(),
            source: source.clone(),
            translation: translation.clone(),
        })
        .collect();
    let mut text = catalog::header(&format!("API reference ({language})"), language);
    for row in rows {
        text.push_str(&catalog::row_text(&row));
        text.push('\n');
    }
    write(&path, &text)?;
    Ok(translations)
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn the_generated_document_names_the_commands_and_the_crates() {
        let root = crate::workspace_root();
        let document = Document::from_code(&root).expect("the reference generates");
        let commands = document
            .sections
            .iter()
            .find(|section| section.heading == "Commands")
            .and_then(|section| section.table.as_ref())
            .expect("a command table");
        assert!(
            commands
                .rows
                .iter()
                .any(|row| matches!(&row[0], Cell::Code(id) if id == "access.compute"))
        );
        let crates = document
            .sections
            .iter()
            .find(|section| section.heading == "Crates")
            .and_then(|section| section.table.as_ref())
            .expect("a crate table");
        assert!(
            crates
                .rows
                .iter()
                .any(|row| matches!(&row[0], Cell::Code(name) if name == "stkcraft-astro-engine"))
        );
        // The version is in the banner, so a reader knows which release the page describes.
        assert!(document.banner.contains("Generated"), "{}", document.banner);
    }

    #[test]
    fn rendering_keeps_ids_in_code_and_prose_in_text() {
        let document = Document {
            title: "Reference".to_owned(),
            banner: "generated".to_owned(),
            sections: vec![Section {
                heading: "Commands".to_owned(),
                prose: vec!["A paragraph.".to_owned()],
                table: Some(Table {
                    columns: vec![
                        (Column::Code, "Id".to_owned()),
                        (Column::Text, "Label".to_owned()),
                    ],
                    rows: vec![vec![
                        Cell::Code("access.compute".to_owned()),
                        Cell::Text("Compute Access".to_owned()),
                    ]],
                }),
            }],
        };
        let text = document.render();
        assert!(text.contains("`access.compute`"), "{text}");
        assert!(text.contains("| Compute Access |"), "{text}");
        assert!(text.contains("## Commands"), "{text}");
    }

    #[test]
    fn a_translation_replaces_prose_and_leaves_ids_alone() {
        let document = Document {
            title: "Reference".to_owned(),
            banner: "generated".to_owned(),
            sections: vec![Section {
                heading: "Commands".to_owned(),
                prose: vec!["A paragraph.".to_owned()],
                table: Some(Table {
                    columns: vec![
                        (Column::Code, "Id".to_owned()),
                        (Column::Text, "Label".to_owned()),
                    ],
                    rows: vec![vec![
                        Cell::Code("access.compute".to_owned()),
                        Cell::Text("Compute Access".to_owned()),
                    ]],
                }),
            }],
        };
        let mut translations = BTreeMap::new();
        translations.insert("Reference".to_owned(), "Referenz".to_owned());
        translations.insert("Commands".to_owned(), "Befehle".to_owned());
        translations.insert("A paragraph.".to_owned(), "Ein Absatz.".to_owned());
        translations.insert("Label".to_owned(), "Beschriftung".to_owned());
        translations.insert("Compute Access".to_owned(), "Zugriff berechnen".to_owned());
        let text = document.translated(&translations).render();
        assert!(text.contains("## Befehle"), "{text}");
        assert!(text.contains("Ein Absatz."), "{text}");
        assert!(text.contains("`access.compute`"), "{text}");
        assert!(text.contains("| Zugriff berechnen |"), "{text}");
        assert!(!text.contains("Compute Access"), "{text}");
    }

    #[test]
    fn every_translatable_string_is_offered_exactly_once() {
        let document = Document {
            title: "T".to_owned(),
            banner: "b".to_owned(),
            sections: vec![
                Section {
                    heading: "S".to_owned(),
                    prose: vec!["P".to_owned(), "P".to_owned()],
                    table: None,
                },
                Section {
                    heading: "S".to_owned(),
                    prose: Vec::new(),
                    table: None,
                },
            ],
        };
        assert_eq!(document.strings(), vec!["P", "S", "T"]);
    }

    #[test]
    fn the_module_paragraphs_skip_headings_and_code() {
        let directory = std::env::temp_dir().join("stkcraft-api-docs-test");
        std::fs::create_dir_all(&directory).expect("a directory");
        let path = directory.join("lib.rs");
        std::fs::write(
            &path,
            "//! First paragraph.\n//! Still first.\n//!\n//! # A heading\n//!\n//! Second paragraph.\n\
             //! ```text\n//! a code block\n//! ```\n//!\n//! Third.\n\npub fn f() {}\n",
        )
        .expect("written");
        let paragraphs = module_paragraphs(&path).expect("parsed");
        assert_eq!(
            paragraphs,
            vec![
                "First paragraph. Still first.".to_owned(),
                "Second paragraph.".to_owned(),
                "Third.".to_owned()
            ]
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn the_options_parse_both_actions() {
        assert_eq!(parse_options(&[]).expect("generate"), Action::Generate);
        assert_eq!(
            parse_options(&["check".to_owned()]).expect("check"),
            Action::Check
        );
        assert_eq!(
            parse_options(&[
                "translate".to_owned(),
                "--language".to_owned(),
                "de".to_owned()
            ])
            .expect("translate"),
            Action::Translate {
                languages: vec!["de".to_owned()],
                dry_run: false,
            }
        );
        assert!(parse_options(&["translate".to_owned()]).is_err());
        assert!(parse_options(&["--language".to_owned(), "de".to_owned()]).is_err());
    }
}
