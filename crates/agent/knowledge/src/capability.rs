//! Object capabilities: what a satellite, station, target or scenario may do.
//!
//! A manifest is data, not a second command registry. Its command ids must exist in the engine's
//! registry; the engine's parameter schema remains the only schema. The manifest adds the object's
//! evidence and limitations, which the registry alone cannot express.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// One object type's declared capabilities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityManifest {
    /// Stable object id, e.g. `satellite`.
    pub id: String,
    /// Human-readable label.
    pub label: String,
    /// Command ids this object may use.
    pub commands: Vec<String>,
    /// What the object's result must carry as evidence.
    pub evidence: Vec<String>,
    /// What the object's result does not model.
    pub limitations: Vec<String>,
}

/// Load every `share/capabilities/*.toml`.
///
/// # Errors
///
/// Returns every malformed manifest, duplicate id or duplicate command, not only the first.
pub fn load_all(root: &Path) -> Result<Vec<CapabilityManifest>, Vec<String>> {
    let directory = root.join("share/capabilities");
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) => {
            return Err(vec![format!(
                "could not read {}: {error}",
                directory.display()
            )]);
        }
    };
    let mut errors = Vec::new();
    let mut manifests = Vec::new();
    let mut ids = BTreeSet::new();
    let mut files: Vec<_> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "toml")
        })
        .collect();
    files.sort();
    for path in files {
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) => {
                errors.push(format!("could not read {}: {error}", path.display()));
                continue;
            }
        };
        let manifest: CapabilityManifest = match toml::from_str(&text) {
            Ok(manifest) => manifest,
            Err(error) => {
                errors.push(format!("{}: {error}", path.display()));
                continue;
            }
        };
        validate(&path, &manifest, &mut errors);
        if !ids.insert(manifest.id.clone()) {
            errors.push(format!(
                "{}: duplicate capability id `{}`",
                path.display(),
                manifest.id
            ));
        }
        manifests.push(manifest);
    }
    if errors.is_empty() {
        manifests.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(manifests)
    } else {
        Err(errors)
    }
}

fn validate(path: &Path, manifest: &CapabilityManifest, errors: &mut Vec<String>) {
    if manifest.id.trim().is_empty()
        || !manifest.id.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
    {
        errors.push(format!(
            "{}: id must be lowercase letters, digits or `-`",
            path.display()
        ));
    }
    if manifest.label.trim().is_empty() {
        errors.push(format!("{}: label is empty", path.display()));
    }
    if manifest.commands.is_empty() {
        errors.push(format!("{}: commands is empty", path.display()));
    }
    let mut seen = BTreeSet::new();
    for command in &manifest.commands {
        if command.trim().is_empty() {
            errors.push(format!("{}: a command id is empty", path.display()));
        } else if !seen.insert(command) {
            errors.push(format!("{}: duplicate command `{command}`", path.display()));
        }
    }
    for (field, values) in [
        ("evidence", &manifest.evidence),
        ("limitations", &manifest.limitations),
    ] {
        if values.is_empty() || values.iter().any(|value| value.trim().is_empty()) {
            errors.push(format!(
                "{}: {field} must be a non-empty list of non-empty strings",
                path.display()
            ));
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn fixture(name: &str) -> PathBuf {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/capability-tests")
            .join(name);
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("share/capabilities")).expect("fixture");
        root
    }

    fn write(root: &Path, id: &str, commands: &str, evidence: &str, limitations: &str) {
        fs::write(
            root.join("share/capabilities").join(format!("{id}.toml")),
            format!(
                "id = \"{id}\"\nlabel = \"{id}\"\ncommands = [{commands}]\nevidence = [{evidence}]\nlimitations = [{limitations}]\n"
            ),
        )
        .expect("manifest");
    }

    #[test]
    fn a_valid_manifest_loads() {
        let root = fixture("valid");
        write(
            &root,
            "satellite",
            "\"satellite.add\"",
            "\"id\"",
            "\"tle epoch\"",
        );
        let manifests = load_all(&root).expect("valid");
        assert_eq!(manifests.len(), 1);
        assert_eq!(manifests[0].id, "satellite");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_duplicate_command_is_refused() {
        let root = fixture("duplicate-command");
        write(
            &root,
            "satellite",
            "\"satellite.add\", \"satellite.add\"",
            "\"id\"",
            "\"tle epoch\"",
        );
        let errors = load_all(&root).expect_err("duplicate");
        assert!(
            errors
                .iter()
                .any(|error| error.contains("duplicate command")),
            "{errors:?}"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn an_empty_limitation_is_refused() {
        let root = fixture("empty-limit");
        write(&root, "satellite", "\"satellite.add\"", "\"id\"", "\"\"");
        let errors = load_all(&root).expect_err("empty limitation");
        assert!(
            errors.iter().any(|error| error.contains("limitations")),
            "{errors:?}"
        );
        let _ = fs::remove_dir_all(root);
    }
}
