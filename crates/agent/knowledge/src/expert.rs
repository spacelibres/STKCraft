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

/// What one evaluation pass found.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EvaluationReport {
    /// Expert directories evaluated.
    pub experts: usize,
    /// Evaluation cases parsed.
    pub cases: usize,
    /// Formula records in those packages.
    pub formulas: usize,
    /// Formula records referenced by at least one case.
    pub covered: usize,
}

/// A runtime role: the expert id and the tools it may use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpertRole {
    /// The expert package id.
    pub id: String,
    /// The role version.
    pub version: String,
    tools: Vec<String>,
    objects: Vec<String>,
}

impl ExpertRole {
    /// Build a role, rejecting an empty id, version or tool list.
    ///
    /// # Errors
    ///
    /// Returns a message naming the invalid field.
    pub fn new(
        id: impl Into<String>,
        version: impl Into<String>,
        tools: Vec<String>,
    ) -> Result<Self, String> {
        Self::with_objects(id, version, tools, Vec::new())
    }

    /// Build a role restricted to object types, rejecting empty ids or tools.
    ///
    /// # Errors
    ///
    /// Returns a message naming the invalid field.
    pub fn with_objects(
        id: impl Into<String>,
        version: impl Into<String>,
        tools: Vec<String>,
        objects: Vec<String>,
    ) -> Result<Self, String> {
        let id = id.into();
        let version = version.into();
        if id.trim().is_empty() || version.trim().is_empty() {
            return Err("a role needs an id and a version".to_owned());
        }
        if tools.is_empty() || tools.iter().any(|tool| tool.trim().is_empty()) {
            return Err(format!("role `{id}` needs at least one non-empty tool"));
        }
        if objects.iter().any(|object| object.trim().is_empty()) {
            return Err(format!("role `{id}` has an empty object id"));
        }
        let mut tools = tools;
        tools.sort();
        tools.dedup();
        let mut objects = objects;
        objects.sort();
        objects.dedup();
        Ok(Self {
            id,
            version,
            tools,
            objects,
        })
    }

    /// Whether this role may call `tool`.
    #[must_use]
    pub fn allows(&self, tool: &str) -> bool {
        self.tools.iter().any(|allowed| allowed == tool)
    }

    /// The allowed tools, sorted.
    #[must_use]
    pub fn tools(&self) -> &[String] {
        &self.tools
    }

    /// The object types this role is restricted to; empty means no object restriction.
    #[must_use]
    pub fn objects(&self) -> &[String] {
        &self.objects
    }
}

/// Load one expert's `role.toml`.
///
/// # Errors
///
/// Returns a message naming the file for an unreadable or malformed role, an id that does not match
/// the directory, or an empty tool list.
pub fn load_role(root: &Path, id: &str) -> Result<ExpertRole, String> {
    let path = root.join("share/experts").join(id).join("role.toml");
    let text = fs::read_to_string(&path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    let file: RoleFile =
        toml::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))?;
    if file.id != id {
        return Err(format!(
            "{}: role id is `{}`, package is `{id}`",
            path.display(),
            file.id
        ));
    }
    if file.version.trim().is_empty() {
        return Err(format!("{}: role has no version", path.display()));
    }
    if file.tools.is_empty() || file.tools.iter().any(|tool| tool.trim().is_empty()) {
        return Err(format!(
            "{}: role needs at least one non-empty tool",
            path.display()
        ));
    }
    ExpertRole::with_objects(file.id, file.version, file.tools, file.objects)
}

#[derive(Debug, Deserialize)]
struct RoleFile {
    id: String,
    version: String,
    tools: Vec<String>,
    #[serde(default)]
    objects: Vec<String>,
}

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

/// Validate every package and its evaluation cases.
///
/// A case names a formula id and a value, or a refusal and a citation. Every formula in a package
/// must be named by at least one case: a record no evaluation exercises is a claim nobody checks.
///
/// # Errors
///
/// Returns the validation errors first, then the evaluation errors, each naming the file and case.
pub fn evaluate(root: &Path) -> Result<EvaluationReport, Vec<String>> {
    validate(root)?;
    let directory = root.join("share/experts");
    let mut errors = Vec::new();
    let mut report = EvaluationReport::default();
    let mut case_ids = BTreeSet::new();
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
    let mut expert_directories: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    expert_directories.sort();
    for expert in &expert_directories {
        report.experts += 1;
        let Some(name) = expert.file_name().and_then(std::ffi::OsStr::to_str) else {
            errors.push(format!(
                "{}: expert directory has no UTF-8 name",
                expert.display()
            ));
            continue;
        };
        let formulas = match load_formula_statuses(expert) {
            Ok(formulas) => formulas,
            Err(error) => {
                errors.push(error);
                continue;
            }
        };
        report.formulas += formulas.len();
        let evaluations = expert.join("evaluations");
        let entries = match fs::read_dir(&evaluations) {
            Ok(entries) => entries,
            Err(error) => {
                errors.push(format!("could not read {}: {error}", evaluations.display()));
                continue;
            }
        };
        let mut files: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "jsonl")
            })
            .collect();
        files.sort();
        let mut referenced = BTreeSet::new();
        for file in files {
            let text = match fs::read_to_string(&file) {
                Ok(text) => text,
                Err(error) => {
                    errors.push(format!("could not read {}: {error}", file.display()));
                    continue;
                }
            };
            for (line_number, line) in text.lines().enumerate() {
                if line.trim().is_empty() {
                    continue;
                }
                let case: serde_json::Value = match serde_json::from_str(line) {
                    Ok(case) => case,
                    Err(error) => {
                        errors.push(format!(
                            "{}:{}: not a JSON object: {error}",
                            file.display(),
                            line_number + 1
                        ));
                        continue;
                    }
                };
                let id = case.get("id").and_then(serde_json::Value::as_str);
                let Some(id) = id.filter(|id| !id.trim().is_empty()) else {
                    errors.push(format!(
                        "{}:{}: case has no id",
                        file.display(),
                        line_number + 1
                    ));
                    continue;
                };
                if !case_ids.insert(id.to_owned()) {
                    errors.push(format!("duplicate evaluation id `{id}`"));
                }
                let question = case
                    .get("question")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                if question.trim().is_empty() {
                    errors.push(format!("{}: case `{id}` has no question", file.display()));
                }
                let expect = case.get("expect").and_then(serde_json::Value::as_object);
                let Some(expect) = expect else {
                    errors.push(format!(
                        "{}: case `{id}` has no expect object",
                        file.display()
                    ));
                    continue;
                };
                let citation = expect
                    .get("citation")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                if citation.trim().is_empty() {
                    errors.push(format!("{}: case `{id}` has no citation", file.display()));
                }
                let formula = expect
                    .get("formula")
                    .and_then(serde_json::Value::as_str)
                    .filter(|formula| !formula.trim().is_empty());
                let refusal = expect
                    .get("refusal")
                    .and_then(serde_json::Value::as_str)
                    .filter(|refusal| !refusal.trim().is_empty());
                match (formula, refusal) {
                    (Some(formula), None) => {
                        let Some(status) = formulas.get(formula) else {
                            errors.push(format!(
                                "{}: case `{id}` names formula `{formula}`, which is not in `{name}`",
                                file.display()
                            ));
                            report.cases += 1;
                            continue;
                        };
                        if !matches!(status.as_str(), "verified" | "implemented") {
                            errors.push(format!(
                                "{}: case `{id}` names formula `{formula}` with status `{status}`; \
                                 only verified or implemented formulas may be cited",
                                file.display()
                            ));
                        }
                        let value = expect
                            .get("value")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default();
                        if value.trim().is_empty() {
                            errors.push(format!(
                                "{}: case `{id}` names a formula but has no value",
                                file.display()
                            ));
                        }
                        referenced.insert(formula.to_owned());
                    }
                    (None, Some(refusal)) => {
                        if refusal.trim().is_empty() {
                            errors.push(format!(
                                "{}: case `{id}` has an empty refusal",
                                file.display()
                            ));
                        }
                    }
                    (Some(_), Some(_)) => errors.push(format!(
                        "{}: case `{id}` names both a formula and a refusal",
                        file.display()
                    )),
                    (None, None) => errors.push(format!(
                        "{}: case `{id}` names neither a formula nor a refusal",
                        file.display()
                    )),
                }
                report.cases += 1;
            }
        }
        for formula in formulas.keys() {
            if !referenced.contains(formula) {
                errors.push(format!(
                    "{}: formula `{formula}` has no evaluation case",
                    expert.display()
                ));
            }
        }
        report.covered += referenced.len();
    }
    if errors.is_empty() {
        Ok(report)
    } else {
        Err(errors)
    }
}

fn load_formula_statuses(
    expert: &Path,
) -> Result<std::collections::BTreeMap<String, String>, String> {
    let formulas_dir = expert.join("formulas");
    let entries = fs::read_dir(&formulas_dir)
        .map_err(|error| format!("could not read {}: {error}", formulas_dir.display()))?;
    let mut statuses = std::collections::BTreeMap::new();
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "toml") {
            continue;
        }
        let text = fs::read_to_string(&path)
            .map_err(|error| format!("could not read {}: {error}", path.display()))?;
        let file: FormulaFile =
            toml::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))?;
        for formula in file.formula {
            statuses.insert(formula.id, formula.status);
        }
    }
    Ok(statuses)
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
    if let Err(error) = load_role(root, name) {
        errors.push(error);
    }
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

/// A base dimension for the unit/dimension pair check.
///
/// This is not full dimensional analysis. It checks that a symbol's unit and its declared dimension
/// label describe the same base quantity; expression-level balance is a later step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dimension {
    One,
    Length,
    Time,
    Angle,
    AngularRate,
}

fn unit_dimension(unit: &str) -> Option<Dimension> {
    match unit.trim().to_lowercase().as_str() {
        "1" | "" => Some(Dimension::One),
        "km" | "m" | "cm" | "mm" => Some(Dimension::Length),
        "s" | "min" | "h" | "hour" | "day" | "century" => Some(Dimension::Time),
        "rad" | "deg" | "degree" => Some(Dimension::Angle),
        "rad/min" | "rad/s" => Some(Dimension::AngularRate),
        _ => None,
    }
}

fn label_dimension(label: &str) -> Option<Dimension> {
    match label.trim().to_lowercase().as_str() {
        "ratio" | "integer" | "number" | "dimensionless" => Some(Dimension::One),
        "length" => Some(Dimension::Length),
        "time" | "time offset" => Some(Dimension::Time),
        "angle" => Some(Dimension::Angle),
        "angular rate" | "frequency" => Some(Dimension::AngularRate),
        _ => None,
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
            continue;
        }
        if let (Some(unit), Some(label)) = (
            unit_dimension(&symbol.unit),
            label_dimension(&symbol.dimension),
        ) && unit != label
        {
            errors.push(format!(
                "{where_}: symbol `{}` has unit `{}` and dimension `{}` that do not agree",
                symbol.name, symbol.unit, symbol.dimension
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
            root.join("share/experts/test/role.toml"),
            "id = \"test\"\nversion = \"0.1.0\"\ntools = [\"knowledge.search\", \"access.compute\"]\n",
        )
        .expect("role");
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
        write_formula_units(root, test, "1", "number");
    }

    fn write_formula_units(root: &Path, test: &str, unit: &str, dimension: &str) {
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
unit = "{unit}"
dimension = "{dimension}"
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

    fn write_evaluation(root: &Path, formula: &str) {
        let directory = root.join("share/experts/test/evaluations");
        fs::create_dir_all(&directory).expect("evaluation directory");
        fs::write(
            directory.join("test.jsonl"),
            format!(
                "{{\"id\":\"e1\",\"question\":\"q\",\"expect\":{{\"formula\":\"{formula}\",\"value\":\"v\",\"citation\":\"c\"}}}}\n"
            ),
        )
        .expect("evaluation");
    }

    #[test]
    fn a_unit_and_dimension_that_disagree_are_refused() {
        let root = fixture("unit-dimension");
        write_formula_units(&root, "proves_it", "km", "time");
        let errors = validate(&root).expect_err("a mismatch");
        assert!(
            errors.iter().any(|error| error.contains("do not agree")),
            "{errors:?}"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_role_loads_and_allows_only_its_tools() {
        let root = fixture("role");
        let role = load_role(&root, "test").expect("valid role");
        assert_eq!(role.id, "test");
        assert!(role.allows("knowledge.search"));
        assert!(role.allows("access.compute"));
        assert!(!role.allows("satellite.add"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_role_id_mismatch_is_refused() {
        let root = fixture("role-id");
        fs::write(
            root.join("share/experts/test/role.toml"),
            "id = \"other\"\nversion = \"0.1.0\"\ntools = [\"knowledge.search\"]\n",
        )
        .expect("role");
        let error = load_role(&root, "test").expect_err("mismatch must fail");
        assert!(error.contains("role id"), "{error}");
        let _ = fs::remove_dir_all(root);
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
    fn a_complete_package_evaluates() {
        let root = fixture("eval-ok");
        write_formula(&root, "proves_it");
        write_evaluation(&root, "test.a");
        let report = evaluate(&root).expect("valid evaluation");
        assert_eq!(report.experts, 1);
        assert_eq!(report.cases, 1);
        assert_eq!(report.formulas, 1);
        assert_eq!(report.covered, 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_formula_without_an_evaluation_is_refused() {
        let root = fixture("eval-missing");
        write_formula(&root, "proves_it");
        fs::create_dir_all(root.join("share/experts/test/evaluations")).expect("directory");
        let errors = evaluate(&root).expect_err("missing evaluation must fail");
        assert!(
            errors
                .iter()
                .any(|error| error.contains("no evaluation case")),
            "{errors:?}"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn an_evaluation_naming_an_unknown_formula_is_refused() {
        let root = fixture("eval-unknown");
        write_formula(&root, "proves_it");
        write_evaluation(&root, "test.missing");
        let errors = evaluate(&root).expect_err("unknown formula must fail");
        assert!(
            errors.iter().any(|error| error.contains("which is not in")),
            "{errors:?}"
        );
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
