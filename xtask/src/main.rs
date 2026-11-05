//! Build, lint and release tasks for `STKCraft`.
//!
//! Run through the `cargo xtask` alias in `.cargo/config.toml`. The task names match the
//! sibling Craft apps so the shared CI workflows and habits carry over.
//!
//! The two tasks that enforce a policy rather than run a tool are [`layers`] and
//! [`assets`]: a layering violation and an unattributed asset are both build failures
//! here, not review comments.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

mod layers;

use layers::{CrateSpec, LAYOUT};

/// Resolve the workspace root from the `xtask` crate's own manifest directory.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

fn main() -> ExitCode {
    // Cargo passes `--offline` and similar flags through to the xtask binary; they belong to
    // cargo, not to the task, so drop everything that starts with a dash before parsing.
    let args: Vec<String> = env::args()
        .skip(1)
        .filter(|a| !a.starts_with('-'))
        .collect();
    let task = args.first().map(String::as_str);

    let result = match task {
        Some("ci") => task_ci(),
        Some("fmt") => run_cargo(&["fmt", "--all", "--", "--check"]),
        Some("layers") => task_layers(),
        Some("assets") => task_assets(),
        Some("wasm") => task_wasm(),
        Some("version") => task_version(args.get(1..).unwrap_or_default()),
        Some("help" | "-h" | "--help") | None => {
            print_help();
            Ok(())
        }
        Some(other) => {
            eprintln!("xtask: unknown task `{other}`");
            print_help();
            return ExitCode::FAILURE;
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("xtask: {message}");
            ExitCode::FAILURE
        }
    }
}

fn print_help() {
    println!(
        "\
Usage: cargo xtask <task>

Tasks:
  ci        fmt check, clippy, tests, assets, layers, wasm (the gate before a commit)
  fmt       cargo fmt --check over the workspace
  layers    enforce crate layering (a crate may depend only on lower layers)
  assets    every asset in the repo has a row in ATTRIBUTION.md
  wasm      cargo check --target wasm32-unknown-unknown for the portable crates
  version   print or set the workspace version (`version set 0.2.0`)
"
    );
}

fn run_cargo(args: &[&str]) -> Result<(), String> {
    run_program("cargo", args)
}

fn run_program(program: &str, args: &[&str]) -> Result<(), String> {
    let status = Command::new(program)
        .args(args)
        .current_dir(workspace_root())
        .status()
        .map_err(|e| format!("could not run `{program}`: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`{program} {}` failed", args.join(" ")))
    }
}

/// One step of the `ci` gate: a name to print and the task to run.
type CiStep = (&'static str, fn() -> Result<(), String>);

fn task_ci() -> Result<(), String> {
    let steps: [CiStep; 7] = [
        ("fmt", || run_cargo(&["fmt", "--all", "--", "--check"])),
        ("clippy", || {
            run_cargo(&[
                "clippy",
                "--workspace",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ])
        }),
        ("test", || run_cargo(&["test", "--workspace"])),
        ("assets", task_assets),
        ("layers", task_layers),
        ("wasm", task_wasm),
        ("version", task_version_check),
    ];

    for (name, step) in steps {
        println!("==> {name}");
        step()?;
    }
    println!("==> ci: all gates passed");
    Ok(())
}

// ---------------------------------------------------------------------------
// layering
// ---------------------------------------------------------------------------

fn task_layers() -> Result<(), String> {
    let root = workspace_root();
    let mut violations = Vec::new();
    let seen: BTreeMap<&str, &CrateSpec> = LAYOUT.iter().map(|spec| (spec.name, spec)).collect();

    for spec in LAYOUT {
        let manifest = root.join(spec.path).join("Cargo.toml");
        let text = fs::read_to_string(&manifest)
            .map_err(|e| format!("could not read {}: {e}", manifest.display()))?;
        let parsed: toml::Value = text
            .parse()
            .map_err(|e| format!("could not parse {}: {e}", manifest.display()))?;

        for dependency in declared_dependencies(&parsed) {
            let Some(dep_spec) = seen.get(dependency.as_str()) else {
                // A third-party crate: nothing to check.
                continue;
            };
            if dep_spec.layer >= spec.layer {
                violations.push(format!(
                    "{} (L{}) depends on {} (L{}); a crate may depend only on a lower layer",
                    spec.name, spec.layer, dep_spec.name, dep_spec.layer
                ));
            }
        }

        // The isolation rule: nothing below the UI layer may touch a windowing or GUI crate.
        if spec.layer <= layers::LAST_NON_UI_LAYER {
            for dependency in declared_dependencies(&parsed) {
                if layers::GUI_CRATES.contains(&dependency.as_str()) {
                    violations.push(format!(
                        "{} (L{}) depends on `{dependency}`, which only the UI layer and the \
                         apps may use",
                        spec.name, spec.layer
                    ));
                }
            }
        }
    }

    // Every crate on disk must be declared, so a new crate cannot skip the layer table.
    let members = workspace_members(&root)?;
    for member in &members {
        let package = read_package_name(&root, member)?;
        if !seen.contains_key(package.as_str()) {
            violations.push(format!(
                "workspace member `{package}` ({member}) is missing from xtask/src/layers.rs; \\
                 add it with the layer it belongs to"
            ));
        }
    }

    if violations.is_empty() {
        println!(
            "layers: {} crates in {} layers, no violations",
            LAYOUT.len(),
            layers::LAYER_COUNT
        );
        Ok(())
    } else {
        for v in &violations {
            eprintln!("layers: {v}");
        }
        Err(format!("{} layering violation(s)", violations.len()))
    }
}

/// Names of the crates a manifest depends on, across all dependency tables.
fn declared_dependencies(manifest: &toml::Value) -> Vec<String> {
    let mut names = Vec::new();
    for table in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(deps) = manifest.get(table).and_then(toml::Value::as_table) {
            names.extend(deps.keys().cloned());
        }
        if let Some(deps) = manifest.get("target").and_then(toml::Value::as_table) {
            for (_, target) in deps {
                if let Some(inner) = target.get(table).and_then(toml::Value::as_table) {
                    names.extend(inner.keys().cloned());
                }
            }
        }
    }
    names
}

fn workspace_members(root: &Path) -> Result<Vec<String>, String> {
    let text = fs::read_to_string(root.join("Cargo.toml"))
        .map_err(|e| format!("could not read the workspace manifest: {e}"))?;
    let parsed: toml::Value = text
        .parse()
        .map_err(|e| format!("could not parse the workspace manifest: {e}"))?;
    let members = parsed
        .get("workspace")
        .and_then(|w| w.get("members"))
        .and_then(toml::Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(toml::Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    Ok(members)
}

fn read_package_name(root: &Path, member: &str) -> Result<String, String> {
    let manifest = root.join(member).join("Cargo.toml");
    let text = fs::read_to_string(&manifest)
        .map_err(|e| format!("could not read {}: {e}", manifest.display()))?;
    let parsed: toml::Value = text
        .parse()
        .map_err(|e| format!("could not parse {}: {e}", manifest.display()))?;
    parsed
        .get("package")
        .and_then(|p| p.get("name"))
        .and_then(toml::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("{} has no [package] name", manifest.display()))
}

// ---------------------------------------------------------------------------
// assets
// ---------------------------------------------------------------------------

/// Extensions treated as bundled assets that need an attribution row.
const ASSET_EXTENSIONS: [&str; 12] = [
    "png", "jpg", "jpeg", "webp", "svg", "gif", "ico", "ttf", "otf", "wav", "ogg", "mp3",
];

fn task_assets() -> Result<(), String> {
    let root = workspace_root();
    let attribution = fs::read_to_string(root.join("ATTRIBUTION.md"))
        .map_err(|e| format!("could not read ATTRIBUTION.md: {e}"))?;

    let mut missing = Vec::new();
    let mut counted = 0_usize;
    collect_assets(&root, &mut |path| {
        counted += 1;
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_owned();
        if !attribution.contains(&name) {
            let relative = path
                .strip_prefix(&root)
                .unwrap_or(path)
                .display()
                .to_string();
            missing.push(relative);
        }
    })?;

    // An attribution row must name a file that exists, or the table rots.
    let known: Vec<String> = collect_asset_names(&root)?;
    let mut orphaned = Vec::new();
    for line in attribution.lines() {
        let Some(cell) = line.trim().strip_prefix('|') else {
            continue;
        };
        // Asset rows start with a backticked path.
        let Some(start) = cell.find('`') else {
            continue;
        };
        let rest = cell.get(start + 1..).unwrap_or_default();
        let Some(end) = rest.find('`') else {
            continue;
        };
        let referenced = &rest[..end];
        if referenced.is_empty() || referenced.eq_ignore_ascii_case("path") {
            continue;
        }
        let file = referenced.rsplit('/').next().unwrap_or(referenced);
        if !known.iter().any(|k| k == file) {
            orphaned.push(referenced.to_owned());
        }
    }

    for path in &missing {
        eprintln!("assets: {path} has no row in ATTRIBUTION.md");
    }
    for path in &orphaned {
        eprintln!("assets: ATTRIBUTION.md lists {path}, which is not in the repository");
    }

    if missing.is_empty() && orphaned.is_empty() {
        println!("assets: {counted} asset(s), all attributed");
        Ok(())
    } else {
        Err(format!(
            "{} unattributed asset(s), {} orphaned row(s)",
            missing.len(),
            orphaned.len()
        ))
    }
}

fn collect_asset_names(root: &Path) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    collect_assets(root, &mut |path| {
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            names.push(name.to_owned());
        }
    })?;
    Ok(names)
}

fn collect_assets(dir: &Path, visit: &mut dyn FnMut(&Path)) -> Result<(), String> {
    let entries =
        fs::read_dir(dir).map_err(|e| format!("could not read {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("could not read an entry: {e}"))?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();

        // Generated build output, version control, and checkouts of *other* repositories are not
        // assets of this one. `craft-fonts` is cloned here by CI (see `AGENTS.md`: "Fonts live in
        // storytold/craft-fonts"), and its fonts carry their own licence in their own repository;
        // listing them here would be a table of somebody else's files that rots the moment
        // upstream adds a face. Before this skip, the asset gate passed locally and failed in CI
        // for exactly that reason.
        if matches!(
            name.as_ref(),
            "target" | ".git" | "node_modules" | "dist" | "craft-fonts"
        ) {
            continue;
        }

        if path.is_dir() {
            collect_assets(&path, visit)?;
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str())
            && ASSET_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str())
        {
            visit(&path);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// wasm
// ---------------------------------------------------------------------------

/// Crates that must build for the browser.
///
/// The analysis layers carry no platform code, so a WebAssembly build is a check that no
/// one has pulled in a file-system or threading dependency by accident. The app and xtask
/// are deliberately absent: the first is the native shell, the second is a build tool.
const PORTABLE_CRATES: [&str; 6] = [
    "stkcraft-core",
    "stkcraft-time",
    "stkcraft-frames",
    "stkcraft-sgp4",
    "stkcraft-access",
    "stkcraft-engine",
];

fn task_wasm() -> Result<(), String> {
    for crate_name in PORTABLE_CRATES {
        println!("==> wasm: {crate_name}");
        // No `--offline` here: the wasm target needs its own copy of some dependencies, and
        // a first run has to fetch them. The gate is meaningful only when it can resolve
        // them, so a network failure should be visible rather than silently skipped.
        run_cargo(&[
            "check",
            "--target",
            "wasm32-unknown-unknown",
            "-p",
            crate_name,
        ])?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// version
// ---------------------------------------------------------------------------

fn task_version(args: &[String]) -> Result<(), String> {
    let root = workspace_root();
    let manifest = root.join("Cargo.toml");
    let text =
        fs::read_to_string(&manifest).map_err(|e| format!("could not read Cargo.toml: {e}"))?;

    match args.first().map(String::as_str) {
        None => {
            for line in text.lines() {
                let trimmed = line.trim();
                if let Some(rest) = trimmed.strip_prefix("version = ") {
                    println!("{}", rest.trim_matches('"'));
                    return Ok(());
                }
            }
            Err("no version in [workspace.package]".to_owned())
        }
        Some("bump") => {
            let current = workspace_version(&root)?;
            let new = bumped(&current)?;
            validate_version(&new)?;
            write_version(&root, &text, &new)?;
            println!("version {current} -> {new}");
            Ok(())
        }
        Some("set") => {
            let Some(new) = args.get(1) else {
                return Err("usage: cargo xtask version set <X.Y.Z>".to_owned());
            };
            validate_version(new)?;
            write_version(&root, &text, new)
        }
        Some(other) => Err(format!("unknown version subcommand `{other}`")),
    }
}

/// The version in `[workspace.package]`.
fn workspace_version(root: &Path) -> Result<String, String> {
    let text = fs::read_to_string(root.join("Cargo.toml"))
        .map_err(|error| format!("could not read Cargo.toml: {error}"))?;
    text.lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix("version = "))
        .map(|rest| rest.trim_matches('"').to_owned())
        .ok_or_else(|| "no version in [workspace.package]".to_owned())
}

/// The newest `## [x.y.z]` heading in `CHANGELOG.md`, ignoring `[Unreleased]`.
///
/// The heading is the version the changelog claims to describe, and the gate compares it with the
/// workspace version; anything that is not three dot-separated numbers is skipped rather than
/// parsed leniently, so `[Unreleased]` and a malformed heading both fall through to the error.
fn newest_changelog_version(changelog: &str) -> Option<String> {
    changelog.lines().find_map(|line| {
        let candidate = line.trim().strip_prefix("## [")?;
        let candidate = &candidate[..candidate.find(']')?];
        let parts: Vec<&str> = candidate.split('.').collect();
        let well_formed = parts.len() == 3 && parts.iter().all(|part| part.parse::<u32>().is_ok());
        well_formed.then(|| candidate.to_owned())
    })
}

/// The workspace version bumped in its smallest component: `0.1.7` becomes `0.1.8`.
fn bumped(version: &str) -> Result<String, String> {
    let core = version.split(['-', '+']).next().unwrap_or_default();
    let mut parts: Vec<u32> = core
        .split('.')
        .map(|part| {
            part.parse::<u32>()
                .map_err(|_| format!("`{version}` is not a semantic version"))
        })
        .collect::<Result<_, _>>()?;
    let [major, minor, patch] = parts.as_mut_slice() else {
        return Err(format!("`{version}` is not a semantic version"));
    };
    *patch = patch.saturating_add(1);
    Ok(format!("{major}.{minor}.{patch}"))
}

/// Fail unless `CHANGELOG.md`'s newest section is the version the workspace declares.
///
/// This is the mechanical half of the version-control rule in `AGENTS.md`: a commit bumps the
/// smallest version component and writes the changelog section that names it, so the two can never
/// drift. An `[Unreleased]` heading is rejected outright, because an unreleased section is exactly
/// where entries go to be forgotten.
fn task_version_check() -> Result<(), String> {
    let root = workspace_root();
    let version = workspace_version(&root)?;
    let changelog = fs::read_to_string(root.join("CHANGELOG.md"))
        .map_err(|error| format!("could not read CHANGELOG.md: {error}"))?;
    if changelog.contains("## [Unreleased]") {
        return Err(
            "CHANGELOG.md has an [Unreleased] section; each commit writes its own version section"
                .to_owned(),
        );
    }
    let Some(newest) = newest_changelog_version(&changelog) else {
        return Err("CHANGELOG.md has no `## [x.y.z]` section".to_owned());
    };
    if newest != version {
        return Err(format!(
            "CHANGELOG.md's newest section is {newest} but the workspace version is {version};              bump both in the same commit"
        ));
    }
    Ok(())
}

/// Rewrite the workspace version in `Cargo.toml` and regenerate the lock file.
fn write_version(root: &Path, text: &str, new: &str) -> Result<(), String> {
    let manifest = root.join("Cargo.toml");
    let mut replaced = 0_usize;
    let mut in_workspace_deps = false;
    let out: Vec<String> = text
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            if trimmed.starts_with('[') {
                in_workspace_deps = trimmed.starts_with("[workspace.dependencies]");
            }
            if trimmed.starts_with("version = ") && !in_workspace_deps {
                replaced += 1;
                return format!("version = \"{new}\"");
            }
            // The workspace's own crates are pinned to the workspace version in
            // `[workspace.dependencies]`. Leaving those pins behind makes the next
            // `cargo update` fail, because every member then asks for a version the path
            // dependency no longer provides.
            if in_workspace_deps
                && trimmed.starts_with("stkcraft-")
                && let Some(start) = line.find("version = \"")
            {
                let rest = &line[start + "version = \"".len()..];
                if let Some(end) = rest.find('"') {
                    replaced += 1;
                    return format!("{}version = \"{new}\"{}", &line[..start], &rest[end + 1..]);
                }
            }
            line.to_owned()
        })
        .collect();
    if replaced == 0 {
        return Err("no version line to replace in [workspace.package]".to_owned());
    }
    fs::write(&manifest, format!("{}\n", out.join("\n")))
        .map_err(|e| format!("could not write Cargo.toml: {e}"))?;
    println!("version set to {new}");
    // Regenerate the lock file so the workspace versions agree.
    run_cargo(&["update", "--workspace", "--offline"])
}

fn validate_version(text: &str) -> Result<(), String> {
    let core = text.split(['-', '+']).next().unwrap_or_default();
    let parts: Vec<&str> = core.split('.').collect();
    if parts.len() != 3 || parts.iter().any(|p| p.parse::<u32>().is_err()) {
        return Err(format!(
            "`{text}` is not a semantic version like 0.2.0 or 0.2.0-rc.1"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_smallest_component_is_the_one_that_moves() {
        assert_eq!(bumped("0.1.0").as_deref(), Ok("0.1.1"));
        assert_eq!(bumped("0.1.9").as_deref(), Ok("0.1.10"));
        assert_eq!(bumped("2.0.3").as_deref(), Ok("2.0.4"));
        // A pre-release tag is dropped by a bump, which is what promoting a release means.
        assert_eq!(bumped("0.2.0-rc.1").as_deref(), Ok("0.2.1"));
        assert!(bumped("0.2").is_err());
    }

    #[test]
    fn the_newest_changelog_section_is_the_first_version_heading() {
        let text = "## [Unreleased]\n\n### Fixed\n\n## [0.1.1] — a title\n\n## [0.1.0] — older\n";
        assert_eq!(newest_changelog_version(text).as_deref(), Some("0.1.1"));
        // With no version heading at all the gate must see nothing rather than guess.
        assert_eq!(newest_changelog_version("## [Unreleased]\n"), None);
        assert_eq!(newest_changelog_version(""), None);
    }

    #[test]
    fn semver_accepts_plain_and_pre_release_forms() {
        assert!(validate_version("0.2.0").is_ok());
        assert!(validate_version("1.2.3-rc.1").is_ok());
        assert!(validate_version("1.2.3+build.5").is_ok());
    }

    #[test]
    fn semver_rejects_malformed_versions() {
        assert!(validate_version("0.2").is_err());
        assert!(validate_version("x.y.z").is_err());
        assert!(validate_version("").is_err());
    }

    #[test]
    fn the_layer_table_is_sorted_and_unique() {
        let mut previous = layers::FIRST_LAYER;
        let mut seen = std::collections::BTreeSet::new();
        for spec in LAYOUT {
            assert!(spec.layer >= previous, "{} is out of order", spec.name);
            previous = spec.layer;
            assert!(seen.insert(spec.name), "{} appears twice", spec.name);
            assert!(!spec.path.is_empty(), "{} has no path", spec.name);
        }
    }
}
