//! Reading and validating the authored expert packages under `share/experts/`.
//!
//! A formula record is a claim: this expression comes from this source, this code computes it, and
//! this test proves it. The validator is what stops a claim from becoming a citation without its
//! evidence. It is deliberately offline and deterministic; it reads files and hashes them.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

/// What one validation pass found.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Validation {
    /// Expert directories seen.
    pub experts: usize,
    /// Source records seen.
    pub sources: usize,
    /// Formula records seen.
    pub formulas: usize,
    /// Formulas with status `verified`.
    pub verified: usize,
    /// Formulas with status `implemented`.
    pub implemented: usize,
}

/// Validate every expert package below `<root>/share/experts`.
///
/// # Errors
///
/// Returns every problem found, each naming the file and field, rather than stopping at the first:
/// a package author needs the whole list. An empty `share/experts` directory is valid.
pub fn validate(root: &Path) -> Result<Validation, Vec<String>> {
    let directory = root.join("share/experts");
    let mut errors = Vec::new();
    let mut report = Validation::default();
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(report),
        Err(error) => {
            return Err(vec![format!(
                "could not read {}: {error}",
                directory.display()
            )]);
        }
    };
    let mut formula_ids = BTreeSet::new();
    let mut expert_directories: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    expert_directories.sort();
    for expert in &expert_directories {
        report.experts += 1;
        validate_expert(root, expert, &mut report, &mut formula_ids, &mut errors);
    }
    if errors.is_empty() {
        Ok(report)
    } else {
        Err(errors)
    }
}

fn validate_expert(
    root: &Path,
    expert: &Path,
    report: &mut Validation,
    formula_ids: &mut BTreeSet<String>,
    errors: &mut Vec<String>,
) {
    let Some(name) = expert.file_name().and_then(std::ffi::OsStr::to_str) else {
        errors.push(format!(
            "{}: expert directory has no UTF-8 name",
            expert.display()
        ));
        return;
    };
    let sources_path = expert.join("sources.toml");
    let sources = match fs::read_to_string(&sources_path) {
        Ok(text) => match toml::from_str::<SourceFile>(&text) {
            Ok(file) => file,
            Err(error) => {
                errors.push(format!("{}: {error}", sources_path.display()));
                return;
            }
        },
        Err(error) => {
            errors.push(format!(
                "could not read {}: {error}",
                sources_path.display()
            ));
            return;
        }
    };
    if sources.schema != 1 {
        errors.push(format!(
            "{}: unsupported schema {} (expected 1)",
            sources_path.display(),
            sources.schema
        ));
    }
    if sources.expert != name {
        errors.push(format!(
            "{}: expert is `{}`, directory is `{name}`",
            sources_path.display(),
            sources.expert
        ));
    }
    let mut source_ids = BTreeSet::new();
    for source in &sources.source {
        report.sources += 1;
        if !source_ids.insert(source.id.clone()) {
            errors.push(format!(
                "{}: duplicate source id `{}`",
                sources_path.display(),
                source.id
            ));
        }
        if source.title.trim().is_empty() || source.edition.trim().is_empty() {
            errors.push(format!(
                "{}: source `{}` needs a title and edition",
                sources_path.display(),
                source.id
            ));
        }
        if source.adopted_by.is_empty() {
            errors.push(format!(
                "{}: source `{}` needs at least one adopting body",
                sources_path.display(),
                source.id
            ));
        }
        validate_source_path(root, &sources_path, source, errors);
    }

    let formulas_dir = expert.join("formulas");
    let entries = match fs::read_dir(&formulas_dir) {
        Ok(entries) => entries,
        Err(error) => {
            errors.push(format!(
                "could not read {}: {error}",
                formulas_dir.display()
            ));
            return;
        }
    };
    let mut formula_files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "toml")
        })
        .collect();
    formula_files.sort();
    for file in formula_files {
        let text = match fs::read_to_string(&file) {
            Ok(text) => text,
            Err(error) => {
                errors.push(format!("could not read {}: {error}", file.display()));
                continue;
            }
        };
        let parsed = match toml::from_str::<FormulaFile>(&text) {
            Ok(parsed) => parsed,
            Err(error) => {
                errors.push(format!("{}: {error}", file.display()));
                continue;
            }
        };
        if parsed.schema != 1 {
            errors.push(format!(
                "{}: unsupported schema {} (expected 1)",
                file.display(),
                parsed.schema
            ));
        }
        if parsed.expert != name {
            errors.push(format!(
                "{}: expert is `{}`, directory is `{name}`",
                file.display(),
                parsed.expert
            ));
        }
        for formula in &parsed.formula {
            report.formulas += 1;
            if !formula_ids.insert(formula.id.clone()) {
                errors.push(format!(
                    "{}: duplicate formula id `{}`",
                    file.display(),
                    formula.id
                ));
            }
            validate_formula(root, &file, formula, &source_ids, report, errors);
        }
    }
}

fn validate_source_path(root: &Path, file: &Path, source: &Source, errors: &mut Vec<String>) {
    if source.path.is_empty() {
        if source.url.trim().is_empty() {
            errors.push(format!(
                "{}: source `{}` has no path and no URL",
                file.display(),
                source.id
            ));
        }
        return;
    }
    if source.sha256.len() != 64 {
        errors.push(format!(
            "{}: source `{}` has a path but no 64-hex SHA-256",
            file.display(),
            source.id
        ));
        return;
    }
    let path = root.join(&source.path);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) => {
            errors.push(format!(
                "{}: source `{}` path {} could not be read: {error}",
                file.display(),
                source.id,
                path.display()
            ));
            return;
        }
    };
    let digest = Sha256::digest(&bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        let _ = write!(&mut hex, "{byte:02x}");
    }
    if hex != source.sha256.to_ascii_lowercase() {
        errors.push(format!(
            "{}: source `{}` SHA-256 does not match {}",
            file.display(),
            source.id,
            path.display()
        ));
    }
}

fn validate_formula(
    root: &Path,
    file: &Path,
    formula: &Formula,
    source_ids: &BTreeSet<String>,
    report: &mut Validation,
    errors: &mut Vec<String>,
) {
    let where_ = format!("{}: formula `{}`", file.display(), formula.id);
    if formula.name.trim().is_empty()
        || formula.domain.trim().is_empty()
        || formula.expression_latex.trim().is_empty()
        || formula.expression_unicode.trim().is_empty()
        || formula.validity.trim().is_empty()
        || formula.source_locator.trim().is_empty()
    {
        errors.push(format!(
            "{where_}: name, domain, expressions, validity and source_locator are required"
        ));
    }
    if !source_ids.contains(&formula.source) {
        errors.push(format!(
            "{where_}: source `{}` is not in sources.toml",
            formula.source
        ));
    }
    if formula.symbols.is_empty() {
        errors.push(format!("{where_}: at least one symbol is required"));
    }
    for symbol in &formula.symbols {
        if symbol.name.trim().is_empty()
            || symbol.unit.trim().is_empty()
            || symbol.dimension.trim().is_empty()
            || symbol.meaning.trim().is_empty()
        {
            errors.push(format!(
                "{where_}: every symbol needs symbol, unit, dimension and meaning"
            ));
        }
    }
    if formula.assumptions.is_empty() {
        errors.push(format!("{where_}: at least one assumption is required"));
    }
    if formula
        .limitations
        .iter()
        .any(|line| line.trim().is_empty())
    {
        errors.push(format!("{where_}: a limitation may not be empty"));
    }
    let status_ok = matches!(
        formula.status.as_str(),
        "extracted" | "classified" | "implemented" | "verified" | "rejected"
    );
    if !status_ok {
        errors.push(format!("{where_}: unknown status `{}`", formula.status));
    }
    match formula.status.as_str() {
        "implemented" | "verified" => {
            validate_implementation(root, &where_, formula, errors);
        }
        _ => {}
    }
    if formula.status == "verified" {
        report.verified += 1;
        if formula.verification.reference.trim().is_empty()
            || formula.verification.case.trim().is_empty()
            || formula.verification.tolerance.trim().is_empty()
            || formula.verification.test.trim().is_empty()
        {
            errors.push(format!(
                "{where_}: a verified formula needs reference, case, tolerance and test"
            ));
        }
    } else if formula.status == "implemented" {
        report.implemented += 1;
    }
}

fn validate_implementation(root: &Path, where_: &str, formula: &Formula, errors: &mut Vec<String>) {
    if formula.implementation.crate_name.trim().is_empty()
        || formula.implementation.path.trim().is_empty()
        || formula.implementation.function.trim().is_empty()
    {
        errors.push(format!(
            "{where_}: an implemented formula needs crate, path and function"
        ));
        return;
    }
    let path = root.join(&formula.implementation.path);
    let body = match fs::read_to_string(&path) {
        Ok(body) => body,
        Err(error) => {
            errors.push(format!(
                "{where_}: implementation {} could not be read: {error}",
                path.display()
            ));
            return;
        }
    };
    if !body.contains(&formula.implementation.function) {
        errors.push(format!(
            "{where_}: implementation {} does not contain `{}`",
            path.display(),
            formula.implementation.function
        ));
    }
    if formula.status == "verified" && !formula.verification.test.trim().is_empty() {
        let test = formula
            .verification
            .test
            .rsplit("::")
            .next()
            .unwrap_or_default();
        let test_path = if formula.verification.test_path.trim().is_empty() {
            path.clone()
        } else {
            root.join(&formula.verification.test_path)
        };
        let test_body = match fs::read_to_string(&test_path) {
            Ok(test_body) => test_body,
            Err(error) => {
                errors.push(format!(
                    "{where_}: test file {} could not be read: {error}",
                    test_path.display()
                ));
                return;
            }
        };
        if !test_body.contains(test) {
            errors.push(format!(
                "{where_}: test `{}` is not in {}",
                formula.verification.test,
                test_path.display()
            ));
        }
    }
}

#[derive(Debug, Deserialize)]
struct SourceFile {
    schema: u32,
    expert: String,
    #[serde(default)]
    source: Vec<Source>,
}

#[derive(Debug, Deserialize)]
struct Source {
    id: String,
    title: String,
    edition: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    sha256: String,
    #[serde(default)]
    adopted_by: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct FormulaFile {
    schema: u32,
    expert: String,
    #[serde(default)]
    formula: Vec<Formula>,
}

#[derive(Debug, Deserialize)]
struct Formula {
    id: String,
    name: String,
    domain: String,
    expression_latex: String,
    expression_unicode: String,
    source: String,
    source_locator: String,
    assumptions: Vec<String>,
    validity: String,
    status: String,
    #[serde(default)]
    limitations: Vec<String>,
    #[serde(default)]
    symbols: Vec<Symbol>,
    implementation: Implementation,
    verification: Verification,
}

#[derive(Debug, Deserialize)]
struct Symbol {
    #[serde(rename = "symbol")]
    name: String,
    unit: String,
    dimension: String,
    meaning: String,
}

#[derive(Debug, Deserialize)]
struct Implementation {
    #[serde(rename = "crate")]
    crate_name: String,
    path: String,
    function: String,
}

#[derive(Debug, Deserialize)]
struct Verification {
    reference: String,
    case: String,
    tolerance: String,
    test: String,
    /// Optional crate-relative test file when the test is not in the implementation file.
    #[serde(default)]
    test_path: String,
}

#[cfg(test)]
#[expect(clippy::expect_used)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos());
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../target/expert-tests");
        let root = base.join(format!(
            "stkcraft-expert-{name}-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("share/experts/test/formulas")).expect("fixture");
        fs::write(
            root.join("share/experts/test/sources.toml"),
            r#"
schema = 1
expert = "test"
[[source]]
id = "s"
title = "A source"
edition = "1"
url = "https://example.invalid/source"
path = ""
sha256 = ""
adopted_by = ["test"]
"#,
        )
        .expect("sources");
        fs::write(
            root.join("impl.rs"),
            "pub fn compute() {}\n#[test]\nfn proves_it() {}\n",
        )
        .expect("implementation");
        root
    }

    fn write_formula(root: &Path, test: &str) {
        fs::write(
            root.join("share/experts/test/formulas/test.toml"),
            format!(
                r#"
schema = 1
expert = "test"
[[formula]]
id = "test.a"
name = "A formula"
domain = "test"
expression_latex = "x = 1"
expression_unicode = "x = 1"
source = "s"
source_locator = "eq. 1"
mathlib = "none"
assumptions = ["none"]
validity = "always"
status = "verified"
limitations = []
[[formula.symbols]]
symbol = "x"
unit = "1"
dimension = "number"
meaning = "a value"
[formula.implementation]
crate = "test"
path = "impl.rs"
function = "compute"
[formula.verification]
reference = "reference"
case = "case"
tolerance = "1e-12"
test = "{test}"
"#
            ),
        )
        .expect("formula");
    }

    #[test]
    fn a_complete_package_validates() {
        let root = fixture("ok");
        write_formula(&root, "proves_it");
        let report = validate(&root).expect("valid package");
        assert_eq!(report.experts, 1);
        assert_eq!(report.sources, 1);
        assert_eq!(report.formulas, 1);
        assert_eq!(report.verified, 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_verified_formula_without_its_test_is_refused() {
        let root = fixture("missing-test");
        write_formula(&root, "does_not_exist");
        let errors = validate(&root).expect_err("missing test must fail");
        assert!(
            errors.iter().any(|error| error.contains("does_not_exist")),
            "{errors:?}"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_missing_source_is_refused() {
        let root = fixture("missing-source");
        write_formula(&root, "proves_it");
        let text = fs::read_to_string(root.join("share/experts/test/sources.toml"))
            .expect("sources")
            .replace("id = \"s\"", "id = \"other\"");
        fs::write(root.join("share/experts/test/sources.toml"), text).expect("sources");
        let errors = validate(&root).expect_err("missing source must fail");
        assert!(
            errors
                .iter()
                .any(|error| error.contains("not in sources.toml")),
            "{errors:?}"
        );
        let _ = fs::remove_dir_all(root);
    }
}
