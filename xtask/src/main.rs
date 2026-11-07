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

mod api_docs;
mod downloads;
mod i18n;
mod layers;
mod translate;

use layers::{CrateSpec, LAYOUT};

/// Resolve the workspace root from the `xtask` crate's own manifest directory.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

fn main() -> ExitCode {
    // Cargo passes `--offline` and similar flags through to the xtask binary; they belong to
    // cargo, not to the task, and they arrive *before* the task name. Only those leading flags are
    // dropped: a task's own options come after its name and must survive (`fetch-data --jobs 8`).
    let raw: Vec<String> = env::args().skip(1).collect();
    let first_task = raw.iter().position(|argument| !argument.starts_with('-'));
    let args: Vec<String> = match first_task {
        Some(index) => raw.iter().skip(index).cloned().collect(),
        None => Vec::new(),
    };
    let task = args.first().map(String::as_str);

    let result = match task {
        Some("ci") => task_ci(),
        Some("fmt") => run_cargo(&["fmt", "--all", "--", "--check"]),
        Some("layers") => task_layers(),
        Some("assets") => task_assets(),
        Some("wasm") => task_wasm(),
        Some("version") => task_version(args.get(1..).unwrap_or_default()),
        Some("release") => tag_release(&workspace_root()),
        Some("fetch-data") => task_fetch_data(args.get(1..).unwrap_or_default()),
        Some("papers" | "fetch-papers") => task_fetch_papers(args.get(1..).unwrap_or_default()),
        Some("math-regions") => task_math_regions(args.get(1..).unwrap_or_default()),
        Some("formulas") => task_formulas(args.get(1..).unwrap_or_default()),
        Some("experts") => task_experts(args.get(1..).unwrap_or_default()),
        Some("capabilities") => task_capabilities(args.get(1..).unwrap_or_default()),
        Some("mathlib") => task_mathlib(args.get(1..).unwrap_or_default()),
        Some("verify-formulas") => task_verify_formulas(),
        Some("naif" | "fetch-naif") => task_fetch_naif(args.get(1..).unwrap_or_default()),
        Some("standards" | "fetch-standards") => {
            task_fetch_standards(args.get(1..).unwrap_or_default())
        }
        Some("i18n") => task_i18n(args.get(1..).unwrap_or_default()),
        Some("api-docs") => task_api_docs(args.get(1..).unwrap_or_default()),
        Some("lockfile") => task_lockfile_check(),
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

/// The workflow's lockfile check, as a task of its own: the file matches what is committed.
fn task_lockfile_check() -> Result<(), String> {
    let root = workspace_root();
    let status = Command::new("git")
        .args(["diff", "--exit-code", "--", "Cargo.lock"])
        .current_dir(&root)
        .status();
    match status {
        Ok(status) if status.success() => {
            println!("lockfile: Cargo.lock matches the repository");
            Ok(())
        }
        Ok(_) => Err(
            "Cargo.lock changed during the gate: commit it, because a build against a lockfile that \
             is not in the repository is not the build CI runs"
                .to_owned(),
        ),
        // No git, or no repository: the check is about what a commit contains, and a source tarball
        // has no commits.
        Err(_) => {
            println!("lockfile: no git checkout, skipping");
            Ok(())
        }
    }
}

fn print_help() {
    println!(
        "\
Usage: cargo xtask <task>

Tasks:
  ci        fmt check, clippy, tests, assets, experts, experts-eval, capabilities, layers, wasm,
            i18n, api-docs, version and the lockfile check — what the GitHub Actions gate runs,
            in one command
  fmt       cargo fmt --check over the workspace
  layers    enforce crate layering (a crate may depend only on lower layers)
  assets    every asset in the repo has a row in ATTRIBUTION.md
  wasm      cargo check --target wasm32-unknown-unknown for the portable crates
  i18n      interface catalogs: check them, report coverage, translate
            (`i18n`, `i18n report`, `i18n translate --language de`)
  papers    download the paper corpus the knowledge base is tested against, verified
  standards download every CCSDS publication, verified (the set we align to)
  naif      download NAIF's required reading, verified (the algorithms behind the standards)
  math-regions  print the formula regions of a PDF as TSV, for the transcription table
  formulas  check the transcription table against the corpus (needs `cargo xtask standards`)
  experts   validate expert packages and their formula records (`experts check`);
            run their evaluation cases (`experts eval`)
  capabilities  validate the object capability manifests (`capabilities check`)
  mathlib   read a Mathlib checkout into a table of declarations (MATHLIB_DIR=<checkout>)
  verify-formulas  re-extract every recorded formula, re-classify, fail on drift
  release   tag the version and print the command that publishes it (`release` checks the changelog)
  fetch-data  download the ephemeris kernels and tables, verified
  lockfile  Cargo.lock matches what is committed (the workflow's last check)
  api-docs  generate the API reference from the code; `check` verifies the committed copy
            (`api-docs translate --language de` translates it)
  version   print or set the workspace version (`version set 0.2.0`)
"
    );
}

fn run_cargo(args: &[&str]) -> Result<(), String> {
    // Keep Cargo's and the linker's temporary files with the build. A terminal that can write the
    // workspace but not the system `%TEMP%` otherwise fails with LNK1104 on the `.tmp` file
    // `link.exe` asks for, which reads as a compiler error rather than a permission one.
    let temp = workspace_root().join("target/xtask-tmp");
    std::fs::create_dir_all(&temp)
        .map_err(|error| format!("could not create {}: {error}", temp.display()))?;
    let status = Command::new("cargo")
        .args(args)
        .current_dir(workspace_root())
        .env("TMP", &temp)
        .env("TEMP", &temp)
        .env("TMPDIR", &temp)
        .status()
        .map_err(|error| format!("could not run `cargo`: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`cargo {}` failed", args.join(" ")))
    }
}

/// One step of the `ci` gate: a name to print and the task to run.
type CiStep = (&'static str, fn() -> Result<(), String>);

/// `cargo xtask experts [check]`: validate every authored expert package.
fn task_experts(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        None | Some("check") => task_experts_check(),
        Some("eval") => task_experts_eval(),
        Some(other) => Err(format!(
            "unknown `experts` subcommand `{other}`; use `cargo xtask experts check` or `eval`"
        )),
    }
}

fn task_experts_check() -> Result<(), String> {
    match stkcraft_agent_knowledge::expert::validate(&workspace_root()) {
        Ok(report) => {
            println!(
                "experts: {} package(s), {} source(s), {} formula(s), {} verified, {} implemented",
                report.experts,
                report.sources,
                report.formulas,
                report.verified,
                report.implemented
            );
            Ok(())
        }
        Err(errors) => Err(errors.join("\n")),
    }
}

fn task_experts_eval() -> Result<(), String> {
    match stkcraft_agent_knowledge::expert::evaluate(&workspace_root()) {
        Ok(report) => {
            println!(
                "experts-eval: {} package(s), {} case(s), {} formula(s), {} covered",
                report.experts, report.cases, report.formulas, report.covered
            );
            Ok(())
        }
        Err(errors) => Err(errors.join("\n")),
    }
}

/// `cargo xtask capabilities [check]`: every object manifest names a real command.
fn task_capabilities(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        None | Some("check") => task_capabilities_check(),
        Some(other) => Err(format!(
            "unknown `capabilities` subcommand `{other}`; use `cargo xtask capabilities check`"
        )),
    }
}

fn task_capabilities_check() -> Result<(), String> {
    let manifests = stkcraft_agent_knowledge::capability::load_all(&workspace_root())
        .map_err(|errors| errors.join("\n"))?;
    let registry = stkcraft_astro_engine::Registry::builtin();
    let mut errors = Vec::new();
    let mut references = 0;
    for manifest in &manifests {
        for command in &manifest.commands {
            references += 1;
            if registry.get(command).is_none() {
                errors.push(format!(
                    "capability `{}` names `{command}`, which is not in the command registry",
                    manifest.id
                ));
            }
        }
    }
    if errors.is_empty() {
        println!(
            "capabilities: {} object(s), {references} command reference(s)",
            manifests.len()
        );
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}

fn task_ci() -> Result<(), String> {
    let steps: [CiStep; 12] = [
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
        ("experts", task_experts_check),
        ("experts-eval", task_experts_eval),
        ("capabilities", task_capabilities_check),
        ("i18n", task_i18n_check),
        ("api-docs", task_api_docs_check),
        ("layers", task_layers),
        ("wasm", task_wasm),
        ("version", task_version_check),
    ];

    // The workflow's last check is `git diff --exit-code -- Cargo.lock`: a gate that changed the
    // lockfile built against a lockfile that is not the one in the repository. Locally the same fact
    // is "this run did not change it", which is what a before/after digest says — comparing against
    // `HEAD` instead would fail on a dependency that is *about* to be committed, which is the normal
    // case for the person running the gate.
    let lockfile = workspace_root().join("Cargo.lock");
    let before = fs::read(&lockfile).ok();
    for (name, step) in steps {
        println!("==> {name}");
        step()?;
    }
    let after = fs::read(&lockfile).ok();
    if before != after {
        return Err(
            "Cargo.lock changed during the gate: the build re-resolved a dependency, so the \
             lockfile in the repository is not the one this gate ran against"
                .to_owned(),
        );
    }
    println!("lockfile: unchanged by the gate");
    println!("==> ci: all gates passed");
    Ok(())
}

// ---------------------------------------------------------------------------
// interface catalogs
// ---------------------------------------------------------------------------

/// `cargo xtask i18n [check|report|translate]`.
fn task_i18n(args: &[String]) -> Result<(), String> {
    let options = i18n::parse_options(args)?;
    i18n::run(&workspace_root(), &options)
}

/// The gate's step: the committed API reference matches the code it claims to describe.
fn task_api_docs_check() -> Result<(), String> {
    api_docs::run(&workspace_root(), &api_docs::Action::Check)
}

/// `cargo xtask api-docs [generate|translate --language <code>]`.
fn task_api_docs(args: &[String]) -> Result<(), String> {
    let action = api_docs::parse_options(args)?;
    api_docs::run(&workspace_root(), &action)
}

/// The gate's step: every catalog is well-formed and names its language.
fn task_i18n_check() -> Result<(), String> {
    i18n::run(&workspace_root(), &i18n::Options::default())
}

// ---------------------------------------------------------------------------
// layering
// ---------------------------------------------------------------------------

fn task_layers() -> Result<(), String> {
    let root = workspace_root();
    let mut violations = Vec::new();
    let seen: BTreeMap<&str, &CrateSpec> = LAYOUT.iter().map(|spec| (spec.name, spec)).collect();

    for spec in LAYOUT {
        // The name repeats the directory: `crates/astro/body` is `stkcraft-astro-body`. Without this
        // the table would accept any pairing of name and path, and a crate could be called
        // `stkcraft-body` while sitting under `crates/astro/` — which is exactly the disagreement
        // docs/standards.md §7 forbids.
        if let Some(expected) = layers::implied_name(spec.path)
            && expected != spec.name
        {
            violations.push(format!(
                "{} sits in `{}`, which implies the name `{expected}`",
                spec.name, spec.path
            ));
        }

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

    let skip_directories = ignored_directories(&root);
    // Match the whole repository-relative path, not the file name: a row for `icon.png` must not
    // attribute an unrelated `app-icon.png`, and a row that names the wrong directory is a row that
    // does not describe the file it claims to.
    let referenced = attribution_paths(&attribution);
    let mut missing = Vec::new();
    let mut counted = 0_usize;
    collect_assets(&root, &skip_directories, &mut |path| {
        counted += 1;
        let relative = path
            .strip_prefix(&root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        if !referenced.iter().any(|row| row == &relative) {
            missing.push(relative);
        }
    })?;

    // An attribution row must name a file that exists, or the table rots.
    let mut orphaned = Vec::new();
    for path in &referenced {
        if !root.join(path).is_file() {
            orphaned.push(path.clone());
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

/// The asset paths an attribution table names, normalised to repository-relative form.
fn attribution_paths(text: &str) -> Vec<String> {
    let mut paths = Vec::new();
    for line in text.lines() {
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
        let referenced = rest.get(..end).unwrap_or_default().trim();
        if referenced.is_empty() || referenced.eq_ignore_ascii_case("path") {
            continue;
        }
        paths.push(referenced.replace('\\', "/"));
    }
    paths
}

/// The directory names a `.gitignore` at the workspace root tells git to ignore.
///
/// Only plain entries are read. `git` accepts `/plan`, `plan/` and `/plan/` for the same directory,
/// and this repository uses all three spellings: `.gitignore` writes `/plan` and `/data/`, while an
/// author reasonably writes `plan/`. Reading only the trailing-slash form silently kept `plan/`
/// (the local notebook `AGENTS.md` says is not part of the repository) in the asset table, so a
/// screenshot parked there failed `cargo xtask assets` — a gate that failed on exactly the files it
/// says are out of scope. A pattern with a wildcard could match anything, and one with an interior
/// slash could name a nested path this one-level check would misread; both are left to git. This is
/// not a `.gitignore` implementation, it is enough to keep local scratch directories out.
fn ignored_directories(root: &Path) -> Vec<String> {
    let Ok(text) = fs::read_to_string(root.join(".gitignore")) else {
        return Vec::new();
    };
    ignored_directory_names(&text)
}

/// The directory entries of a `.gitignore`, normalised to bare names.
fn ignored_directory_names(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter(|line| !line.contains('*') && !line.contains('!'))
        .map(|line| line.trim_matches('/'))
        .filter(|name| !name.is_empty() && !name.contains('/'))
        .map(str::to_owned)
        .collect()
}

fn collect_assets(dir: &Path, skip: &[String], visit: &mut dyn FnMut(&Path)) -> Result<(), String> {
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

        // A directory git is told to ignore is not part of the repository, so its contents are not
        // this repository's assets: `plan/` is the local scratch area `AGENTS.md` describes, and a
        // file parked there is the author's business, not the release's. Skipping them keeps the
        // gate about what ships. The check is on the directory name against `.gitignore`'s own
        // entries rather than its full pattern language, which is all this repository uses.
        if path.is_dir() && skip.iter().any(|entry| entry == name.as_ref()) {
            continue;
        }

        if path.is_dir() {
            collect_assets(&path, skip, visit)?;
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
    "stkcraft-framework-core",
    "stkcraft-framework-time",
    "stkcraft-astro-frames",
    "stkcraft-astro-sgp4",
    "stkcraft-astro-access",
    "stkcraft-astro-engine",
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

/// Tag the current version and push the tag, and tell the maintainer whether a Release is due.
///
/// Every version carries a tag, because a version without one is a version nobody can check out.
/// Only a **stable** version — 1.0.0 or later with no pre-release suffix — gets a GitHub Release;
/// the 0.x development records are tags and nothing else. The command refuses when `CHANGELOG.md`'s
/// newest section is not the workspace version (so a tag cannot point at a section that does not
/// exist), creates the annotated tag if it is missing, and pushes it. The `gh release create` line
/// is printed only for a stable version, and printed rather than run so this works without `gh`.
fn tag_release(root: &Path) -> Result<(), String> {
    let version = workspace_version(root)?;
    task_version_check()?;
    let tag = format!("v{version}");
    let exists = run_git(
        root,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/tags/{tag}"),
        ],
    )
    .is_ok();
    if exists {
        println!("{tag} already exists");
    } else {
        run_git(
            root,
            &["tag", "-a", &tag, "-m", &format!("STKCraft {version}")],
        )?;
        println!("tagged {tag}");
    }
    run_git(root, &["push", "origin", &tag])?;
    if is_stable_version(&version) {
        println!(
            "stable release: next: gh release create {tag} --title \"STKCraft {tag}\" --generate-notes"
        );
    } else {
        println!(
            "{tag} is a development version: tagged, no GitHub Release. Only a stable version \
             (1.0.0 or later, no -rc/-alpha suffix) gets a Release."
        );
    }
    Ok(())
}

/// Whether a version is stable enough for a GitHub Release.
///
/// The 0.x series is development by `SemVer`'s own definition: every version is tagged so it can be
/// checked out, but a Release implies "this is the version to use", which 0.x does not promise.
/// A pre-release suffix (`-rc.1`, `-alpha`) is a tag for the same reason. Build metadata (`+…`)
/// does not make a version unstable.
fn is_stable_version(text: &str) -> bool {
    let without_build = text.split('+').next().unwrap_or_default();
    let core = without_build.split('-').next().unwrap_or_default();
    let mut parts = core.split('.');
    let (Some(major), Some(_minor), Some(_patch), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    let Ok(major) = major.parse::<u32>() else {
        return false;
    };
    major >= 1 && !without_build.contains('-')
}

/// A `.gitignore`d directory of fetched data, and the files that belong in it with their checksums.
///
/// The checksums are what makes the fetch trustworthy: a kernel is a large binary that taints every
/// number computed from it, so a silently truncated download (which this repository has already
/// seen once) must fail loudly rather than produce a plausible ephemeris.
const DATA_FILES: [(&str, &str, &str); 12] = [
    (
        "de440s.bsp",
        "https://naif.jpl.nasa.gov/pub/naif/generic_kernels/spk/planets/de440s.bsp",
        "c1c7feeab882263fc493a9d5a5b2ddd71b54826cdf65d8d17a76126b260a49f2",
    ),
    (
        "mar099.bsp",
        "https://naif.jpl.nasa.gov/pub/naif/generic_kernels/spk/satellites/mar099.bsp",
        "9991e57b196bae1a096acc6e2afc6718102ee420d684695de0f0333064c046bc",
    ),
    (
        "jup365.bsp",
        "https://naif.jpl.nasa.gov/pub/naif/generic_kernels/spk/satellites/jup365.bsp",
        "dbf016c01ba4d022154838000cf3f06962cf958ddc503a366f7fe8f81495c5cb",
    ),
    (
        "sat441.bsp",
        "https://naif.jpl.nasa.gov/pub/naif/generic_kernels/spk/satellites/sat441.bsp",
        "d7e444a9ba7a52b8f448ff0747030789524d2f0c1212e4e2bd7f5fc3c96444d5",
    ),
    (
        "ura111xl-799.bsp",
        "https://naif.jpl.nasa.gov/pub/naif/generic_kernels/spk/satellites/ura111xl-799.bsp",
        "fa55665eb129613574a5d029bf2cc7b521d3f0d2edc7de5790add3cf950714bf",
    ),
    (
        "nep097xl-899.bsp",
        "https://naif.jpl.nasa.gov/pub/naif/generic_kernels/spk/satellites/nep097xl-899.bsp",
        "95e815cb8584e530d3c83f35e39af4bf67d0874f01b65538cbe9dbf279818df8",
    ),
    (
        "nep097xl-801.bsp",
        "https://naif.jpl.nasa.gov/pub/naif/generic_kernels/spk/satellites/nep097xl-801.bsp",
        "3b4ec5b6f23a3b10da9574bc8fb35cfd93ebf9d7951bdcba0c7e26309304bfb1",
    ),
    (
        "finals2000A.all",
        "https://datacenter.iers.org/data/9/finals2000A.all",
        "4c2308a3a782f561a84f549ab0e074e745aa950facd9dff56e2e60a7eac8f042",
    ),
    (
        "naif0012.tls",
        "https://naif.jpl.nasa.gov/pub/naif/generic_kernels/lsk/naif0012.tls",
        "678e32bdb5a744117a467cd9601cd6b373f0e9bc9bbde1371d5eee39600a039b",
    ),
    (
        "pck00011.tpc",
        "https://naif.jpl.nasa.gov/pub/naif/generic_kernels/pck/pck00011.tpc",
        "3dff7b1dbeceaa01f25467767d3fa25816051c85d162d1edf04acb310ee28bb1",
    ),
    (
        "gm_de440.tpc",
        "https://naif.jpl.nasa.gov/pub/naif/generic_kernels/pck/gm_de440.tpc",
        "924ddf4fb9ead9fe8a1aa55780bcabde40b09d00065d58226e24b68d8092f140",
    ),
    (
        "hygdata_v40.csv.gz",
        "https://raw.githubusercontent.com/astronexus/HYG-Database/main/hyg/CURRENT/hygdata_v40.csv.gz",
        "8e3ff9e67445e558a759b117910850cff1b1d4d492f45f715c2ee2db3d869bac",
    ),
];

/// Download the data files this repository does not commit, and verify them.
///
/// `cargo xtask fetch-data [dir]` writes to `data/` by default, which `.gitignore` excludes.
fn task_fetch_data(args: &[String]) -> Result<(), String> {
    let root = workspace_root();
    // `cargo xtask fetch-data [dir] [--jobs N]`: a job count is a throughput choice, not a
    // correctness one — the checksum is what decides whether a download was any good.
    let mut directory = root.join("data");
    let mut jobs = DOWNLOAD_JOBS;
    let mut force = false;
    let mut positional = 0_usize;
    let mut arguments = args.iter();
    while let Some(argument) = arguments.next() {
        if argument == "--force" || argument == "-f" {
            // Re-download even when the file is already there and correct: the way to prove the
            // download path works without deleting anything by hand.
            force = true;
            continue;
        }
        if argument == "--jobs" || argument == "-j" {
            let value = arguments
                .next()
                .ok_or_else(|| "`--jobs` needs a number".to_owned())?;
            jobs = value
                .parse::<usize>()
                .map_err(|_| format!("`{value}` is not a job count"))?
                .clamp(1, MAX_DOWNLOAD_JOBS);
            continue;
        }
        if argument.starts_with('-') {
            return Err(format!("unknown option `{argument}`"));
        }
        positional += 1;
        if positional > 1 {
            return Err("fetch-data takes one directory".to_owned());
        }
        directory = root.join(argument);
    }
    fs::create_dir_all(&directory)
        .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
    let (fetched, verified, bytes) = fetch_verified(&DATA_FILES, &directory, jobs, force)?;
    // A summary is what makes "did it actually run?" a question the output answers: a run that
    // found everything already in place says so, in numbers.
    println!(
        "data: {} file(s), {fetched} fetched, {verified} already present, {} on disk",
        DATA_FILES.len(),
        human_size(bytes)
    );
    Ok(())
}

/// Print the formula regions of a PDF as TSV: the rows the transcription table is built from.
///
/// The transcription table (`docs/design/mathematics.md`) is how "recognised well" gets measured instead
/// of asserted, and its rows have to come from a real extraction rather than from memory: this task prints
/// what the current pipeline finds, so a person can paste the correct rows into the table and then have
/// the table fail when the pipeline drifts.
///
/// A row is `page<TAB>x0<TAB>y0<TAB>x1<TAB>y1<TAB>problem<TAB>unicode`, with tabs inside a field replaced
/// by spaces and an absent problem written as `-`.
/// Check the transcription table: the regions the pipeline must find, and the ones it must not.
///
/// `share/math/transcriptions.tsv` is the table; `data/standards/` is the corpus. Every `not-formula` row
/// that a region covers is a false positive, and every `formula` row that no region covers is a miss, and
/// both are printed with the row that caused them. The exit status is the count of failures, so a caller
/// (and a person) can tell "measured, and wrong" from "measured, and right".
/// Read a Mathlib checkout into the declaration lexicon, as TSV.
///
/// The lexicon is what the formula classifier scores against (`docs/design/mathematics.md`): names,
/// statements and docstrings, with the module path giving the taxonomy. It is read from a checkout the
/// person supplies — `MATHLIB_DIR`, or the one argument — because Mathlib is a gigabyte of Lean and this
/// repository does not carry it.
///
/// A row is `name<TAB>kind<TAB>domain<TAB>module<TAB>digest<TAB>statement<TAB>docstring`, and the file goes
/// to `data/mathlib/declarations.tsv` unless a directory is named.
/// Re-extract every recorded formula, re-classify it, and fail on drift.
///
/// This is the command `docs/design/mathematics.md` names as what makes provenance re-verifiable rather
/// than a snapshot of an afternoon: the boxes in `share/math/transcriptions.tsv` are read again from the
/// corpus, the classifier runs again over the stored lexicon, and any difference is reported. It also
/// records the classification it found — `data/mathlib/classifications.tsv` — so "this formula is about
/// `Real.sqrt`, with score 1.0" has a file that a later run can contradict.
fn task_verify_formulas() -> Result<(), String> {
    let root = workspace_root();
    let table = fs::read_to_string(root.join("share/math/transcriptions.tsv"))
        .map_err(|error| format!("could not read the transcription table: {error}"))?;
    let corpus = root.join("data/standards");
    let lexicon = read_lexicon(&root);
    let mut checked = 0_usize;
    let mut failures: Vec<String> = Vec::new();
    let mut classified: Vec<Classification> = Vec::new();
    for line in table.lines() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        let [document, page, x0, y0, x1, y1, expect, note] = fields[..] else {
            failures.push(format!("malformed row: {line}"));
            continue;
        };
        let page: usize = page.parse().map_err(|_| "bad page".to_owned())?;
        let number = |text: &str| {
            text.parse::<f64>()
                .map_err(|_| format!("`{text}` is not a number"))
        };
        let (x0, y0, x1, y1) = (number(x0)?, number(y0)?, number(x1)?, number(y1)?);
        let bytes = fs::read(corpus.join(document)).map_err(|error| {
            format!("could not read {document} (run `cargo xtask standards`): {error}")
        })?;
        let extracted = stkcraft_agent_knowledge::extract(&bytes, 64 << 20)
            .map_err(|error| format!("{document}: {error}"))?;
        // The region the row names: same page, box within two points, exactly as `formulas` checks it.
        let region = extracted.regions.iter().find(|region| {
            region.page == page
                && (region.x0 - x0).abs() <= 2.0
                && (region.y0 - y0).abs() <= 2.0
                && (region.x1 - x1).abs() <= 2.0
                && (region.y1 - y1).abs() <= 2.0
        });
        checked += 1;
        match (expect, region) {
            ("formula", Some(region)) => {
                let matched = lexicon.as_ref().and_then(|declarations| {
                    stkcraft_agent_knowledge::mathlib::classify(&region.unicode, declarations)
                });
                classified.push(Classification {
                    document: document.to_owned(),
                    page: u32::try_from(page).unwrap_or(0),
                    name: matched.as_ref().map(|found| found.name.clone()),
                    score: matched.as_ref().map(|found| found.score),
                    domain: matched.as_ref().map(|found| found.domain.clone()),
                });
            }
            ("formula", None) => failures.push(format!(
                "{document}:page {page}: DRIFT, the formula is gone ({note})"
            )),
            ("not-formula", Some(_)) => failures.push(format!(
                "{document}:page {page}: DRIFT, a refused region is back ({note})"
            )),
            _ => {}
        }
    }
    let directory = root.join("data/mathlib");
    if fs::create_dir_all(&directory).is_ok() {
        {
            let mut text =
                String::from("document\tpage\tmathlib_name\tmathlib_score\tmathlib_domain\n");
            for row in &classified {
                let _ = std::fmt::Write::write_fmt(
                    &mut text,
                    format_args!(
                        "{}\t{}\t{}\t{}\t{}\n",
                        row.document,
                        row.page,
                        row.name.as_deref().unwrap_or("-"),
                        row.score
                            .map_or("-".to_owned(), |score| format!("{score:.2}")),
                        row.domain.as_deref().unwrap_or("-")
                    ),
                );
            }
            let path = directory.join("classifications.tsv");
            fs::write(&path, text)
                .map_err(|error| format!("could not write {}: {error}", path.display()))?;
        }
    }
    for failure in &failures {
        eprintln!("verify-formulas: {failure}");
    }
    let unclassified = classified.iter().filter(|row| row.name.is_none()).count();
    println!(
        "verify-formulas: {checked} row(s) re-read, {} formula(s) re-classified, {unclassified} with no match, {} drifted",
        classified.len(),
        failures.len()
    );
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!("{} recorded formula(s) drifted", failures.len()))
    }
}

/// One formula's classification, as `verify-formulas` records it.
struct Classification {
    /// The document the formula was read from.
    document: String,
    /// The page.
    page: u32,
    /// The Mathlib declaration it matched, when it matched one.
    name: Option<String>,
    /// How strongly.
    score: Option<f64>,
    /// The taxonomy the match implies.
    domain: Option<String>,
}

/// The lexicon, when `data/mathlib/declarations.tsv` is present.
fn read_lexicon(root: &Path) -> Option<Vec<stkcraft_agent_knowledge::mathlib::Declaration>> {
    let text = fs::read_to_string(root.join("data/mathlib/declarations.tsv")).ok()?;
    let mut declarations = Vec::new();
    for line in text.lines().skip(1) {
        let fields: Vec<&str> = line.split('\t').collect();
        let [name, kind, domain, module, digest, statement, docstring] = fields[..] else {
            continue;
        };
        declarations.push(stkcraft_agent_knowledge::mathlib::Declaration {
            name: name.to_owned(),
            kind: kind.to_owned(),
            module: module.to_owned(),
            domain: domain.to_owned(),
            statement: statement.to_owned(),
            docstring: docstring.to_owned(),
            digest: digest.to_owned(),
        });
    }
    (!declarations.is_empty()).then_some(declarations)
}

fn task_mathlib(args: &[String]) -> Result<(), String> {
    let root = workspace_root();
    let checkout = args
        .iter()
        .find(|argument| !argument.starts_with('-'))
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("MATHLIB_DIR").map(std::path::PathBuf::from))
        .ok_or_else(|| {
            "mathlib needs a checkout: `MATHLIB_DIR=<mathlib4> cargo xtask mathlib`".to_owned()
        })?;
    if !checkout.is_dir() {
        return Err(format!("{} is not a directory", checkout.display()));
    }
    let started = std::time::Instant::now();
    let declarations = stkcraft_agent_knowledge::mathlib::parse_tree(&checkout);
    if declarations.is_empty() {
        return Err(format!(
            "no declarations under {} — is that a Mathlib checkout?",
            checkout.display()
        ));
    }
    let directory = root.join("data/mathlib");
    fs::create_dir_all(&directory)
        .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
    let path = directory.join("declarations.tsv");
    let mut text = String::from("name\tkind\tdomain\tmodule\tdigest\tstatement\tdocstring\n");
    for declaration in &declarations {
        let clean = |field: &str| field.replace(['\t', '\n'], " ");
        let _ = std::fmt::Write::write_fmt(
            &mut text,
            format_args!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                clean(&declaration.name),
                clean(&declaration.kind),
                clean(&declaration.domain),
                clean(&declaration.module),
                declaration.digest,
                clean(&declaration.statement),
                clean(&declaration.docstring)
            ),
        );
    }
    fs::write(&path, text)
        .map_err(|error| format!("could not write {}: {error}", path.display()))?;
    let mut domains: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for declaration in &declarations {
        *domains.entry(declaration.domain.as_str()).or_default() += 1;
    }
    println!(
        "mathlib: {} declaration(s) in {} domain(s), read in {:.1} s, written to {}",
        declarations.len(),
        domains.len(),
        started.elapsed().as_secs_f64(),
        path.display()
    );
    let mut listed: Vec<(&str, usize)> = domains.into_iter().collect();
    listed.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    for (domain, count) in listed.iter().take(8) {
        println!("  {domain}: {count}");
    }
    Ok(())
}

fn task_formulas(_args: &[String]) -> Result<(), String> {
    let root = workspace_root();
    let table = fs::read_to_string(root.join("share/math/transcriptions.tsv"))
        .map_err(|error| format!("could not read the transcription table: {error}"))?;
    let corpus = root.join("data/standards");
    let mut checked = 0_usize;
    let mut failures = Vec::new();
    for line in table.lines() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        let [document, page, x0, y0, x1, y1, expect, note] = fields[..] else {
            failures.push(format!("malformed row: {line}"));
            continue;
        };
        let number = |text: &str| {
            text.parse::<f64>()
                .map_err(|_| format!("`{text}` is not a number"))
        };
        let (page, x0, y0, x1, y1) = (
            page.parse::<usize>().map_err(|_| "bad page".to_owned())?,
            number(x0)?,
            number(y0)?,
            number(x1)?,
            number(y1)?,
        );
        let bytes = fs::read(corpus.join(document)).map_err(|error| {
            format!("could not read {document} (run `cargo xtask standards`): {error}")
        })?;
        let extracted = stkcraft_agent_knowledge::extract(&bytes, 64 << 20)
            .map_err(|error| format!("{document}: {error}"))?;
        let covering = extracted.regions.iter().any(|region| {
            region.page == page
                && (region.x0 - x0).abs() <= 2.0
                && (region.y0 - y0).abs() <= 2.0
                && (region.x1 - x1).abs() <= 2.0
                && (region.y1 - y1).abs() <= 2.0
        });
        checked += 1;
        match expect {
            "formula" if !covering => {
                failures.push(format!(
                    "{document}:page {page}: MISSED, expected a region ({note})"
                ));
            }
            "not-formula" if covering => {
                failures.push(format!("{document}:page {page}: FALSE POSITIVE ({note})"));
            }
            "formula" | "not-formula" => {}
            other => failures.push(format!(
                "{document}:page {page}: unknown expectation `{other}`"
            )),
        }
    }
    for failure in &failures {
        eprintln!("formulas: {failure}");
    }
    println!(
        "formulas: {checked} row(s) checked, {} correct, {} wrong",
        checked.saturating_sub(failures.len()),
        failures.len()
    );
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!("{} transcription row(s) are wrong", failures.len()))
    }
}

fn task_math_regions(args: &[String]) -> Result<(), String> {
    let Some(path) = args.iter().find(|argument| !argument.starts_with('-')) else {
        return Err(
            "math-regions needs a PDF: `cargo xtask math-regions data/standards/502x0b1s.pdf`"
                .to_owned(),
        );
    };
    let bytes = fs::read(path).map_err(|error| format!("could not read {path}: {error}"))?;
    let extracted = stkcraft_agent_knowledge::extract(&bytes, 64 << 20)
        .map_err(|error| format!("{path}: {error}"))?;
    let clean = |text: &str| text.replace(['\t', '\n'], " ");
    for (region, problem) in extracted.regions.iter().zip(&extracted.problems) {
        println!(
            "{}\t{:.0}\t{:.0}\t{:.0}\t{:.0}\t{}\t{}",
            region.page,
            region.x0,
            region.y0,
            region.x1,
            region.y1,
            problem.as_deref().map_or_else(|| "-".to_owned(), clean),
            clean(&region.unicode)
        );
    }
    eprintln!(
        "{} region(s) in {} page(s) of {path}",
        extracted.regions.len(),
        extracted.pages.len()
    );
    Ok(())
}

fn task_fetch_naif(args: &[String]) -> Result<(), String> {
    let root = workspace_root();
    let mut directory = root.join("data").join("naif");
    let mut jobs = DOWNLOAD_JOBS;
    let mut force = false;
    let mut positional = 0_usize;
    let mut arguments = args.iter();
    while let Some(argument) = arguments.next() {
        if argument == "--force" || argument == "-f" {
            force = true;
            continue;
        }
        if argument == "--jobs" || argument == "-j" {
            let value = arguments
                .next()
                .ok_or_else(|| "`--jobs` needs a number".to_owned())?;
            jobs = value
                .parse::<usize>()
                .map_err(|_| format!("`{value}` is not a job count"))?
                .clamp(1, MAX_DOWNLOAD_JOBS);
            continue;
        }
        if argument.starts_with('-') {
            return Err(format!("unknown option `{argument}`"));
        }
        positional += 1;
        if positional > 1 {
            return Err("naif takes one directory".to_owned());
        }
        directory = root.join(argument);
    }
    fs::create_dir_all(&directory)
        .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
    let urls: Vec<String> = NAIF_FILES
        .iter()
        .map(|(name, _, _)| format!("{NAIF_REQ}{name}"))
        .collect();
    let entries: Vec<(&str, &str, &str)> = NAIF_FILES
        .iter()
        .zip(urls.iter())
        .map(|((name, hash, _), url)| (*name, url.as_str(), *hash))
        .collect();
    let (fetched, verified, bytes) = fetch_verified(&entries, &directory, jobs, force)?;
    println!(
        "naif: {} document(s), {fetched} fetched, {verified} already present, {} on disk",
        NAIF_FILES.len(),
        human_size(bytes)
    );
    println!(
        "next: STKCRAFT_KNOWLEDGE={} cargo run --release -p stkcraft -- knowledge \"<query>\"",
        directory.display()
    );
    Ok(())
}

fn task_fetch_standards(args: &[String]) -> Result<(), String> {
    let root = workspace_root();
    let mut directory = root.join("data").join("standards");
    let mut jobs = DOWNLOAD_JOBS;
    let mut force = false;
    let mut positional = 0_usize;
    let mut arguments = args.iter();
    while let Some(argument) = arguments.next() {
        if argument == "--force" || argument == "-f" {
            force = true;
            continue;
        }
        if argument == "--jobs" || argument == "-j" {
            let value = arguments
                .next()
                .ok_or_else(|| "`--jobs` needs a number".to_owned())?;
            jobs = value
                .parse::<usize>()
                .map_err(|_| format!("`{value}` is not a job count"))?
                .clamp(1, MAX_DOWNLOAD_JOBS);
            continue;
        }
        if argument.starts_with('-') {
            return Err(format!("unknown option `{argument}`"));
        }
        positional += 1;
        if positional > 1 {
            return Err("standards takes one directory".to_owned());
        }
        directory = root.join(argument);
    }
    fs::create_dir_all(&directory)
        .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
    let urls: Vec<String> = STANDARD_FILES
        .iter()
        .map(|(name, _, _)| format!("{CCSDS_PUBS}{name}"))
        .collect();
    let entries: Vec<(&str, &str, &str)> = STANDARD_FILES
        .iter()
        .zip(urls.iter())
        .map(|((name, hash, _), url)| (*name, url.as_str(), *hash))
        .collect();
    let (fetched, verified, bytes) = downloads::fetch_verified(&entries, &directory, jobs, force)?;
    println!(
        "standards: {} file(s), {fetched} fetched, {verified} already present, {} on disk",
        STANDARD_FILES.len(),
        human_size(bytes)
    );
    println!(
        "next: STKCRAFT_KNOWLEDGE={} cargo run --release -p stkcraft -- knowledge \"<query>\"",
        directory.display()
    );
    Ok(())
}

fn task_fetch_papers(args: &[String]) -> Result<(), String> {
    let root = workspace_root();
    // `cargo xtask papers [dir] [--jobs N] [--force]`, exactly as `fetch-data`.
    let mut directory = root.join("data").join("papers");
    let mut jobs = DOWNLOAD_JOBS;
    let mut force = false;
    let mut positional = 0_usize;
    let mut arguments = args.iter();
    while let Some(argument) = arguments.next() {
        if argument == "--force" || argument == "-f" {
            force = true;
            continue;
        }
        if argument == "--jobs" || argument == "-j" {
            let value = arguments
                .next()
                .ok_or_else(|| "`--jobs` needs a number".to_owned())?;
            jobs = value
                .parse::<usize>()
                .map_err(|_| format!("`{value}` is not a job count"))?
                .clamp(1, MAX_DOWNLOAD_JOBS);
            continue;
        }
        if argument.starts_with('-') {
            return Err(format!("unknown option `{argument}`"));
        }
        positional += 1;
        if positional > 1 {
            return Err("fetch-papers takes one directory".to_owned());
        }
        directory = root.join(argument);
    }
    fs::create_dir_all(&directory)
        .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
    let (fetched, verified, bytes) = fetch_verified(&PAPER_FILES, &directory, jobs, force)?;
    println!(
        "papers: {} file(s), {fetched} fetched, {verified} already present, {} on disk",
        PAPER_FILES.len(),
        human_size(bytes)
    );
    // The point of the corpus is that a person can use it, so the command that uses it is printed
    // rather than left to the documentation.
    println!(
        "next: STKCRAFT_KNOWLEDGE={} cargo run --release -p stkcraft -- knowledge \"<query>\"",
        directory.display()
    );
    Ok(())
}

/// Download, verify or confirm one table of files, and report what each one cost.
///
/// Shared by the two fetches — the ephemeris data and the paper corpus — because the rule is the
/// same for both: a file from outside this repository is only usable once its checksum matches the
/// one recorded in the documentation, and a mismatch is an error rather than a warning.
fn fetch_verified(
    entries: &[(&str, &str, &str)],
    directory: &Path,
    jobs: usize,
    force: bool,
) -> Result<(usize, usize, u64), String> {
    let mut fetched = 0_usize;
    let mut verified = 0_usize;
    let mut bytes = 0_u64;
    for (name, url, expected) in entries {
        let path = directory.join(name);
        if path.exists() && force {
            fs::remove_file(&path).map_err(|error| format!("{name}: {error}"))?;
        }
        if path.exists() {
            let actual = sha256_of(&path)?;
            if actual == *expected {
                println!("{name}: present and verified");
                verified += 1;
                bytes += fs::metadata(&path).map_or(0, |meta| meta.len());
                continue;
            }
            return Err(format!(
                "{name}: on disk it hashes {actual}, but the recorded checksum is {expected}; \
                 delete it and fetch again"
            ));
        }
        fetch_file(name, url, &path, jobs)?;
        let actual = sha256_of(&path)?;
        fetched += 1;
        bytes += fs::metadata(&path).map_or(0, |meta| meta.len());
        if actual != *expected {
            return Err(format!(
                "{name}: downloaded file hashes {actual}, expected {expected}"
            ));
        }
        println!("{name}: fetched and verified");
    }
    Ok((fetched, verified, bytes))
}

/// NAIF's required-reading documentation, with the SHA-256 of each page.
///
/// This is the *algorithmic* reference the project is checked against: `spk.html` states what an SPK
/// segment type 2 record contains (a Chebyshev polynomial and its mid-point and radius), `pck.html` the
/// orientation models, `time.html` the time systems, `frames.html` the frame chain, `daf.html`/`das.html`
/// the file and array structures a kernel is made of. NASA's decades of practice are what those pages
/// record, and a claim in this repository about SGP4, an epoch conversion or a frame rotation can point
/// at the page that defines it.
///
/// The URL of each is `https://naif.jpl.nasa.gov/pub/naif/toolkit_docs/C/req/<name>`, so only the name,
/// hash and size are stored.
static NAIF_FILES: [(&str, &str, usize); 27] = [
    (
        "abcorr.html",
        "c801451d5480c4875e54bc76e9658b74b3decce75eca0576b4a665fdd6869dab",
        28_058,
    ),
    (
        "cells.html",
        "2a81da6a56a142332ae35801519f477867d21053f42b57bcf966711432248830",
        18_982,
    ),
    (
        "ck.html",
        "a447ec34b113f0400a8315dabd4e338b44192264bb8c20fe3d5e2e282015469f",
        195_638,
    ),
    (
        "cspice.html",
        "9c1be8b3afe45a852a82f1274e5daad51440e074429faabceb39ca43055e9dd7",
        39_631,
    ),
    (
        "daf.html",
        "f2481e3d121df056037090e6433fd12c9fbfbf9b92bf9eb99da34216ba96338c",
        90_490,
    ),
    (
        "das.html",
        "039e49caede4ac1ff36ea4011de46c3e2e6891a15c01ab2872c26272e7ba3bca",
        79_847,
    ),
    (
        "dla.html",
        "7a964569c724b4a6c4a4a85cb820d945b693c200fec0002b26ec83a0367fcf9a",
        22_217,
    ),
    (
        "dsk.html",
        "f4fac7a104c942a12ae1cd8b6aa14a7b22c7b77b5f3c7200de741c3110ebc763",
        123_147,
    ),
    (
        "ek.html",
        "e7784f586127a667be04b9e1a7ad9fbdc6114d54ccd1ced64230c057a48ab08c",
        117_881,
    ),
    (
        "ellipses.html",
        "33037d9cf96f6aad7aed1a0f8bf60b18813e126f59c7a8945849dcda4c416a47",
        28_800,
    ),
    (
        "error.html",
        "8f1ecc9d0816e2158591836a5b86544f7675d90c58d11c7ee5315a82257c6591",
        78_112,
    ),
    (
        "frames.html",
        "c9a03998b4d1c652d7dba8154ae0c90a4c5bf9f4654ac8624c59a8687937df36",
        203_675,
    ),
    (
        "gf.html",
        "c96147b444c0c86c508d7017e741573f1553a500f95be30e7d012e62ee6871d3",
        161_428,
    ),
    (
        "kernel.html",
        "98ee9e4f6b32cf7b6f87f0127be915299bc6a93f2a29f9954b22ccbf0747c0a5",
        84_870,
    ),
    (
        "naif_ids.html",
        "4b647f8cc5f3101dc16011a51fe22c6beb8e5c2db49c7619cd11787a4eaf36c3",
        54_644,
    ),
    (
        "pck.html",
        "771177b1c21a448bd81b5b5f872840797d30e0c1400af363aa0fde730bf1585f",
        89_169,
    ),
    (
        "planes.html",
        "5f331c64983b88251b9b19089fec85b9ec72fd44fb844dcbc3455f85cb309eb5",
        24_721,
    ),
    (
        "problems.html",
        "e18412a0ab82ef7d3066af2c122186487c17739a0a31ed1453c5cc1a06526ae8",
        64_507,
    ),
    (
        "rotation.html",
        "729046b42faa732c0f3b0dedb9ca99fbff2a302ee930a288754c7785eacfdcf2",
        118_680,
    ),
    (
        "scanning.html",
        "f511c09346e65106d19c93e6a877f2b6a293b242d5e30ac2b8ec0745702462f2",
        10_292,
    ),
    (
        "sclk.html",
        "ffe5ad1440bd92822508e1565cae432fc5cc33ba7ab8939b96d5f541766d3ff5",
        73_244,
    ),
    (
        "sets.html",
        "f76bd5c7e8b5d12fb1673db156d069e49fa5b28fb3fd4f3b887542673a8ca79c",
        21_455,
    ),
    (
        "spc.html",
        "9ebf57fdc6b2af9cf921a65be908d4593ebcf47528bbef5e91a9511b1239190c",
        32_687,
    ),
    (
        "spk.html",
        "8aa1f05a4fe60e8ad705f24a840acb4b696d2160ded176bbd585c28440905c88",
        216_199,
    ),
    (
        "symbols.html",
        "7fb9ddb395a118a030883779242d860f820106905179e14793560168a06e52e5",
        26_002,
    ),
    (
        "time.html",
        "025bd684a9997feca2f51a54550a52898e1d7686d500fee0e7b0ff3c9ebc54a1",
        122_505,
    ),
    (
        "windows.html",
        "14e032225ef9b30350872b989f03748a05cd7fb84fd6f5b7ac1315b37af73660",
        25_220,
    ),
];

/// Where NAIF publishes its required reading.
const NAIF_REQ: &str = "https://naif.jpl.nasa.gov/pub/naif/toolkit_docs/C/req/";

/// Every CCSDS publication in the public catalogue, with the SHA-256 of the file as downloaded.
///
/// The URL of each is `https://public.ccsds.org/Pubs/<name>`, so only the name and the hash are stored:
/// a table of seven hundred near-identical URLs is a table where one typo is invisible.
///
/// This is the standard set the engine and the knowledge base are aligned to — the blue books this
/// project parses and emits, and the green, orange and silver books around them. `cargo xtask standards`
/// downloads them into `data/standards/`, which `.gitignore` excludes, verified against these rows.
static STANDARD_FILES: [(&str, &str, usize); 495] = [
    (
        "100x0g1s.pdf",
        "24860cabdd3f491859591edb56952060a86b4b807ab417f2a7616675c4218a7a",
        313_501,
    ),
    (
        "101x0b1s.pdf",
        "3479e39f1102ecfb5ee5bf02806011424dc2e61f30f3b8665bc466f1fc445150",
        4_595_923,
    ),
    (
        "101x0b3s.pdf",
        "cc5f49d00a805064e2208ab8bfa0935254c931d15105986702ee655bcf3203c4",
        317_838,
    ),
    (
        "101x0b4s.pdf",
        "b4583c5830299f4f5b86cde36fd57f7fad46f0d85fe331f2bfdeeb81959a037d",
        420_951,
    ),
    (
        "101x0b5s.pdf",
        "100979f97dca93f8d1c6ebe2b996d8b4aa308b884ef51ee216dde900c5cad831",
        558_278,
    ),
    (
        "101x0b6s.pdf",
        "7354caaad258dfbc7586bb035a55c8166a6ab21b8d457b7304272b42f130d2a4",
        621_475,
    ),
    (
        "102x0b1s-1.pdf",
        "25b0ad0808b54752640a94a049dade521f39afad6845faeafa4c430d61a3cfdc",
        7_361_319,
    ),
    (
        "102x0b4s.pdf",
        "0a977c7417b40a6234472e51e3ef95bdf623f5c5423aea89d95f29360e628fa3",
        548_399,
    ),
    (
        "102x0b5s.pdf",
        "27a460902259e90aba1d71f9a2643db686e136afd36e570c574c6f83ec2ab8c7",
        446_184,
    ),
    (
        "103x0b1s.pdf",
        "3bef948d1ab7329f0e0dd8b9e3dd5e4ca1cfe06203324e05df5b37b2f2699987",
        448_596,
    ),
    (
        "103x0b2s.pdf",
        "16b94255dae81bceade72e4792d6e732bd7406652a2ac69c1483011591a37ba2",
        598_013,
    ),
    (
        "120x0g1s.pdf",
        "56f38be7a2471cc816bdbefa71155cdb303ebd49784dc55d069e4358f0bb3753",
        776_683,
    ),
    (
        "120x0g2s.pdf",
        "6f9fc14877a68382222942cc334bdb1e244985811c8997d78d140db2facfc0c3",
        1_092_934,
    ),
    (
        "120x0g3s.pdf",
        "7cd8319736fb5e9932b811696b2e2d387c44c16b0b6df2e726b82ed3356227a1",
        1_458_397,
    ),
    (
        "120x0g4.pdf",
        "d1cf6615b50ac07e0ba4eeec26f6d936a0f3308e3554603cf01ce38a2a924c4c",
        1_154_500,
    ),
    (
        "120x1g1e2s.pdf",
        "2b815febe208c718f4f74bdaf90672c02743abc9787812ec966d22e5ef32cb79",
        5_511_618,
    ),
    (
        "120x1g2s.pdf",
        "0d0ad3ea58b36e14b959191f3472f5c06e18957cedee73aabbe5c80f7cae095a",
        6_018_005,
    ),
    (
        "120x1g3.pdf",
        "9d5d1b0434de8b92b0f6113d59ac4ba9d944d6790093ceec0279e9f1c7d92ee0",
        6_213_823,
    ),
    (
        "120x2g1s.pdf",
        "fe67f117ed438bcd6f7bda125e5b21efa5a4be0203c44bfeb166b8567a1c4266",
        3_083_027,
    ),
    (
        "120x2g2.pdf",
        "8634d30327004c87809e63564efc89c39e89bc221f76f9c59ba0c6559d85e2b5",
        8_199_623,
    ),
    (
        "120x3g1.pdf",
        "3bf7e4eb8211dea52df852c37c290483e9cab688e75e8bc5abb26b1f8a4972f3",
        2_593_176,
    ),
    (
        "121x0b1sc2.pdf",
        "ba1d0db59ddccab814f310f48cda3bef734f6470cd56933753d2393bdf0c68ce",
        571_590,
    ),
    (
        "121x0b1sc2_TCA633.pdf",
        "74359682b801fef3e813daa8126b25590309ecb2073d2e4fa53d84dd0355eb7d",
        84_329,
    ),
    (
        "121x0b1sc2_tca724.pdf",
        "1f18cb49a3524004070fb5fe16f4eaf0913e8540706d3fa06f98ae6f4b370441",
        19_978,
    ),
    (
        "121x0b2ec1s.pdf",
        "f5093931f207b5f49a82295688a1a70f5f4e30568fb75c9b2ff4c0ffdd035ed9",
        1_047_142,
    ),
    (
        "121x0b3.pdf",
        "d09d41b718dcb13e9ef915b2b505edcf490de87f47603b9a85959e7f0d03dfa4",
        679_604,
    ),
    (
        "122x0b1c3s.pdf",
        "e5ef22e986a3c7bba5383f9a090e255ec921f28d877d4921f826c4b941b0119b",
        1_495_321,
    ),
    (
        "122x0b1c3s_1_TCA621.pdf",
        "eff1158912224818f94d50f38ba7449fc574021da7ff606d6574753d43f29b18",
        704_348,
    ),
    (
        "122x0b1c3s_2_tca808.pdf",
        "9b81c16d0b66324f23fdb8013da9d5a1718a854c14822d3af588f8e0b080106d",
        654_626,
    ),
    (
        "122x0b1c3s_3_tc1129.pdf",
        "ce7b022a1a6d93bc6ebf8ac817c35049499dd6e34b4002ca4c378f7084101bbf",
        650_745,
    ),
    (
        "122x0b2e1.pdf",
        "a780af5c0981b592084b4e631be25f7f31b8ef1a86f58178a312c1ea0d5963db",
        1_366_481,
    ),
    (
        "122x1b1e1.pdf",
        "90d1d6811d35272a08ae4de80cb0504f4a0cc83791b749c55759f98840426ee9",
        1_137_557,
    ),
    (
        "123x0b1ec1s.pdf",
        "a565060dce93b403be6f90a0561ac64e64e479d6045bede69ab9f09a4654fa12",
        1_099_488,
    ),
    (
        "123x0b2c1_tc1918.pdf",
        "61b2603a47303d70af413e190b16c51447254dfacfca78a5e6bef3d9c9928a25",
        91_325,
    ),
    (
        "123x0b2c2_tc2023.pdf",
        "ec19a14e03e279921a3c2879c4ec40403e40c7f40a26456c9841173db5aeee2d",
        17_524,
    ),
    (
        "123x0b2e1c3_tc2101.pdf",
        "864662bec53cd4f73eec2c2db7a07a2795ce6b57c13b18fe1352062c6077e291",
        76_911,
    ),
    (
        "123x0b2e2c3.pdf",
        "a169a5e237a6180b826421704a405f59ac2f1dd318586a19f9330458c4041187",
        1_318_968,
    ),
    (
        "124x0b1.pdf",
        "302e3772e3670bb11cfe01121de1a611300b88c5f50b4cf93b8fa705e654ce31",
        634_484,
    ),
    (
        "130x0g1s.pdf",
        "33cfa268ef2d7aded63c2afde2955ac3794e4b50ebd833efcd7f8eb564c85653",
        698_929,
    ),
    (
        "130x0g2s.pdf",
        "aa97e4d5cefe9ccf948d42c218d3a91edda70716d1caf2d58f3339ee6f777656",
        682_935,
    ),
    (
        "130x0g3s.pdf",
        "33a7eebaf5805b36021e081f94660dd61232e3a56782fa651fca42f63f9b23c6",
        928_593,
    ),
    (
        "130x0g4e1.pdf",
        "5565e0f0fefaa7a48cd24aa084b4a8d9a50dd0f4d54c15962772506714ea9b92",
        568_145,
    ),
    (
        "130x11g1s.pdf",
        "3e112f54495ccb083e9aa2d27f41809e2a024f3ef3793ec0231b70dfaddeda72",
        2_900_434,
    ),
    (
        "130x11g2.pdf",
        "c59db96a314d318fea2600f1dddb787536a82bd13f0e45c6b4a965a6f179a39d",
        2_346_119,
    ),
    (
        "130x12g1s.pdf",
        "7ab477549a08210619c45d95cd6017b3d79d40de54ab3ba5137133d715e01937",
        1_395_119,
    ),
    (
        "130x12g2.pdf",
        "0e4240f411a196fc0edd964b844e439708d2f94902ce16f5249349b73cace78e",
        1_166_808,
    ),
    (
        "130x1g1e1s.pdf",
        "8b02729c0c3755e6b50dd40587fdc122f27825dda1a26a34a3382c5650472f47",
        1_230_334,
    ),
    (
        "130x1g2s.pdf",
        "835bd910e7d214cec97742102d4804ef072ddb03d6ff9bde784d5f3c7b233bf7",
        3_426_730,
    ),
    (
        "130x1g3e1.pdf",
        "a5f7d1d5bb133ff200ef18920ee79ec46c09d9407f86fc9a6649022eaa9844b9",
        2_181_752,
    ),
    (
        "130x2g1s.pdf",
        "22dbfd57e2a45cd9b4e41cdc877130f42f7c6c6df3b022e8ea3413ec99ff2ef7",
        653_085,
    ),
    (
        "130x2g2s.pdf",
        "c951f6dfc903c37332cddb9cec210502c0e89038edb5093e16ed48b686ff1747",
        868_152,
    ),
    (
        "130x2g3.pdf",
        "c59f051f2498c54ca57e328bd0f3d528c09db08467b167431b837f236e120f8f",
        275_496,
    ),
    (
        "130x3g1.pdf",
        "17a114a6a44023893c98615a73252c78d1e4098402648127a0c42e9fca860859",
        502_538,
    ),
    (
        "131x0b1s.pdf",
        "0fecd8fb0b630166c71ae3fd480529d6aa97dab5ac2deafb0a407caecf7e1154",
        910_645,
    ),
    (
        "131x0b2ec1s.pdf",
        "d59ee962c7ce0ae63e5b495fc1d7a0a7d6bab0e26ef49febd80e63103d520956",
        1_231_991,
    ),
    (
        "131x0b3s.pdf",
        "58bc91e6412732b051cd6618db9cb943146a5c36e1d1e26933d5c62ddad24904",
        1_316_424,
    ),
    (
        "131x0b4s.pdf",
        "cc31da09ac59d846a4eecd637d61205c7e426741ad1202bc6f85235379309529",
        1_721_667,
    ),
    (
        "131x1o1s.pdf",
        "160f374883253d37da02be89eb26ba37aeda8a5894356f155d5aece7d3af78bf",
        1_157_593,
    ),
    (
        "131x1o2e2s.pdf",
        "d9db1ebec390d90c2a249e720554907a8c7cb76aed8ccd9dea2381dd415cd880",
        1_535_627,
    ),
    (
        "131x21o1c1.pdf",
        "763aeda7df5e31d99c196d1e36e68cef84d77a284931527f97b5fc75ddafdcd8",
        1_080_783,
    ),
    (
        "131x21o1c1_tc2210.pdf",
        "d8ba843619dba97e33ac393b269b85e7b4819244d61bff2b07b0f69e6e36b71b",
        20_265,
    ),
    (
        "131x2b1s.pdf",
        "d9287772af06a61cedf79cfb8b6c7520d344ccf3bf4edead05c45ab3663c1bbf",
        1_274_802,
    ),
    (
        "131x2o1c1s.pdf",
        "346d1c338fa042fe3481d55177a43d3824009a802f1690f0d3549fbcdffb6012",
        1_016_940,
    ),
    (
        "131x2o1c1s_TC_11-09.pdf",
        "a3398f81749d2bb2c9850cbbcf25879ebfa727196a0021b6fe2e67f4bce79f6c",
        30_518,
    ),
    (
        "131x31o1c1.pdf",
        "6116f28b32bef149f2459d8693ad24ada62282b608cd0e77a4ff1a006efc6cec",
        1_192_473,
    ),
    (
        "131x31o1c1_tc2211.pdf",
        "f78c96b5d9e6f5c0b389cf90a79978e7c836cf483727e5511ae6bf47baad66c5",
        24_858,
    ),
    (
        "131x3b1s.pdf",
        "60e9c053a2cc5279de63a38dd10c3a978a627651082a6e2c21a6330446056647",
        868_685,
    ),
    (
        "131x3b2e1.pdf",
        "a576eae6f8ff663bcf8ac2da8d40186171f9f91348ea3f25fa570820036405ef",
        598_715,
    ),
    (
        "131x4m1s.pdf",
        "e9bc3ca12fe113fba50ff65c78427acc0d17f519a8026553301cca963b264a86",
        798_306,
    ),
    (
        "131x5o1c1.pdf",
        "6b7b92d784ef4ee20e911c4c4cc810703fc2f95b1bad40f4cf1956f3b4a11eec",
        1_462_517,
    ),
    (
        "131x5o1c1_tc2423.pdf",
        "4d43579ba6b772bbb628b5d7de2125355074ea4fb9e89b24daf586c92f772c4f",
        15_361,
    ),
    (
        "132x0b1c1e1s.pdf",
        "d692e1989b7364478bd2decf1645ad442e77693091de86a988817c5629804310",
        922_397,
    ),
    (
        "132x0b1c1s_tc1226.pdf",
        "3ffc9520c2550009a6ffbc87d5009249a57e5c7499cce9386dcbb05c81a0fe2b",
        17_010,
    ),
    (
        "132x0b2s.pdf",
        "f5d85905b5a002bbf7a0dafea5e2c8ce056843430f627854eb99a068c3ff53ac",
        1_056_772,
    ),
    (
        "132x0b3.pdf",
        "be715bb3b1f92a4e5655719969277aee3b250dd706748b53dccd9971896603ad",
        893_096,
    ),
    (
        "133x0b1s.pdf",
        "e1dd0c6397b55e719c8ac1d7d3d9e99a5fa8ad727cc5794665f99f50d351050c",
        879_652,
    ),
    (
        "133x0b1s_tc1020.pdf",
        "59b50e69f162bce6cfc349a42dcc553d6c1a558740606ed4c8a6148e30e73297",
        22_134,
    ),
    (
        "133x0b1s_tc1227.pdf",
        "3c6fe1bf215fd472ea169418e58c0d1029a995a2bd3bf1e3eea2d3a990006969",
        16_959,
    ),
    (
        "133x0b2e2.pdf",
        "a7d3acaa7d8af5f306917dff454b69de187855c45a51bb76ac4636fb5b23e34e",
        663_940,
    ),
    (
        "133x1b1c1s.pdf",
        "794b0a3c02d4cb2f27353810e872e2477f5d35ba428640068d0d38115169d11e",
        366_856,
    ),
    (
        "133x1b1c1s_TCA628.pdf",
        "e3320994095bb055a780154bebce23d8c652b4b881b3a5ff879e165cd42adbeb",
        20_653,
    ),
    (
        "133x1b2sc2.pdf",
        "cc7fbb862a9a86b7ebac00c0572ebf0f85250b66cab3ce6016e98c13bd51af73",
        918_654,
    ),
    (
        "133x1b3e1.pdf",
        "1c19bf6d355a6d3d2b4fd917355f3b81d6b086f9e5d5422b10800ec29d2657b3",
        456_320,
    ),
    (
        "135x0b1s.pdf",
        "e78dc2fa369b3c5eaee815d30bec968edfce87c6ee1730f65c1238c5ceb586ea",
        375_627,
    ),
    (
        "135x0b2s.pdf",
        "61b395d1d98e3418d3932c60d85f1e8978534bef7bed5c692ba8c91e67213c81",
        249_373,
    ),
    (
        "135x0b3c1s.pdf",
        "a71c93987dfc769f99c452a51ff054fe9539cc327f624c474ae8f141c482c99f",
        335_460,
    ),
    (
        "135x0b3c1s_TCA804.pdf",
        "15ce714830330c713724a7158b9647d2cc6f0d810427af1b8d3fd290564dbd89",
        23_773,
    ),
    (
        "135x0b4s.pdf",
        "f38910170cc9ac67aa6e45772e935e84d21a49b4c2fb405dbe3b462002a5c515",
        402_299,
    ),
    (
        "140x1g1s.pdf",
        "df7b197fab1b7cc716fb9ccc752e61ac55db1439b8b51a94a5e9e99c42f81acb",
        5_543_072,
    ),
    (
        "140x1g2.pdf",
        "2a4c1db6cafac30c8854904b876c7f3843cfcf43ffe4897dc38a444f5bb1ae32",
        5_127_714,
    ),
    (
        "141x10.pdf",
        "a84ca7a95f19c23e770037dd96659b8be713b95279777a19dbbb060ddeea6b32",
        822_908,
    ),
    (
        "141x11o1e2.pdf",
        "dc9fb7fc36d121265624baba6905c97d0a73ad1ca378adb8ad73984fb089c57d",
        673_102,
    ),
    (
        "141x1m1e1.pdf",
        "8f124111d721abc6573fa3862e76cc7e8dfa5466fe03b69723bc6f45edc3bee2",
        4_064_824,
    ),
    (
        "142x10o1e1.pdf",
        "23cf1a97caf14143eb23d0d56a9eae8f385f8c31912d75e6a953c6eccc1abed7",
        683_938,
    ),
    (
        "200x0g6.pdf",
        "d65830e9093dc66011da6ca80e1e586236fe7a29b8a95e6874a6de8c664fc444",
        214_630,
    ),
    (
        "201x0b2s-1.pdf",
        "b070688c31fe788771cf77e59df89178541b6978d9ef90932e17d5770183e650",
        529_968,
    ),
    (
        "201x0b3s.pdf",
        "1f3538dd4dd79e8be5d0e665d6e06b39a10df6d69c162a618de1ba9fda9ca0b7",
        563_009,
    ),
    (
        "202x0b2s.pdf",
        "e756edd90665c8d5ab3191edbb1f870a3a54b0ab6cea68fd8402d2024023bd8d",
        544_702,
    ),
    (
        "202x0b3s.pdf",
        "762baef01b08edd5ad51e23c99a148b754e8463f23206e8d6f4be84481c814fc",
        778_527,
    ),
    (
        "202x1b1s.pdf",
        "9953019a89d5d5eac8d8a149347a5e687541786f86f50ccb1fff79acbcc555cc",
        561_990,
    ),
    (
        "202x1b2s.pdf",
        "6828b05fd548706797e6cf9adc42b382869906750d69df3122cc8e444df37a96",
        771_193,
    ),
    (
        "203x0b1s.pdf",
        "cd1465afff128011731f4a9cb2791b3653590964fe8ebbee9a8e869ea5c170b1",
        521_140,
    ),
    (
        "203x0b2s.pdf",
        "81a0885522786083cf0f12b6b02e7fb10b983fff6708b568b4a68ab99b9f16f5",
        758_684,
    ),
    (
        "210x0g1s.pdf",
        "6a29b169b505ab78766ee66ca0450c66aede6136463033898faeddf7b8699fc1",
        1_163_439,
    ),
    (
        "210x0g2e1.pdf",
        "703019ea5cb9a877f2e63017aba3b4b25352297929afb2fcb245061c5b569922",
        1_838_207,
    ),
    (
        "211x0b1s.pdf",
        "c8750c8c5be1e60a42b0a7e4b37c0dc53f5a0a28692643c83fcc59c52626b879",
        1_456_045,
    ),
    (
        "211x0b2s.pdf",
        "3d9183e3bd877b9a4e7974e514853790a8705718b7f96be040499b49e3e3d5e9",
        1_368_664,
    ),
    (
        "211x0b3s.pdf",
        "55abaceac1ec2da93305d0a2f1d970fbc58acc79237679814696cb5eb93021aa",
        1_357_400,
    ),
    (
        "211x0b4s.pdf",
        "efe9a62f58eeed1a793b8b79998cafa30a12b91ff95e1c4a1430c0ecd2cffd53",
        1_075_463,
    ),
    (
        "211x0b5s.pdf",
        "8c52410433abc79dad12c5bda81672f5327ce815afb3e3b322f9a4fd87516bfe",
        1_533_264,
    ),
    (
        "211x0b6e1.pdf",
        "59b16ea2ccb85de890c860d17a6116b365448c21fd2a2ceb2a628ea24a5a3720",
        1_698_878,
    ),
    (
        "211x1b1s.pdf",
        "71bff1536fe868323482326f8e21c307a24254f173214338cc977c4483e00ed3",
        706_013,
    ),
    (
        "211x1b2s.pdf",
        "4785a52334c173bdc67210f2a9467ab011edd10f926449da7dd50686e65d52e5",
        717_316,
    ),
    (
        "211x1b3sec1s.pdf",
        "2afb8553cb8d468b841e2c20d15ed2864806e9060a1fac8e9e22a4a6ef1fe4a9",
        705_110,
    ),
    (
        "211x1b4e1.pdf",
        "d68682b2371d74a48faffe5f95ddbca85000fb3dd71c72ef31dd5e572f7d7ac0",
        289_808,
    ),
    (
        "211x2b1s.pdf",
        "333acb3740e35948e07ec1812996e79fa7a3c1b31d097b191c388ca4252c8f00",
        989_940,
    ),
    (
        "211x2b2s.pdf",
        "e4ec791583c00eca6c6398be02fcdc6ed16805c61e3a190dba99eb91cd8fef7f",
        892_369,
    ),
    (
        "211x2b3.pdf",
        "e07ab19a7484197b49bc743fb816bd29e8d4584f05e8fb50e2ad261657e3095d",
        547_545,
    ),
    (
        "230x0g1s.pdf",
        "c38ecdb720cf1bd36cdcd3c5d08ee7d1ab45ae96d738d585cb77aa68c6a1ba84",
        904_083,
    ),
    (
        "230x1g2e1s.pdf",
        "f205abe07ce0537b728b9383f86f8930bdb73f90922154b37bc8ac4de908812a",
        1_393_931,
    ),
    (
        "230x1g3e1.pdf",
        "2ce64320ef8cad6fb8d5a2a80c48ffdf1f2584e5773f0c48eb255bb8b2f04ae7",
        1_655_725,
    ),
    (
        "230x2g1.pdf",
        "bbb868f440d965814ec3e4715c8db6b0e51dadd68a4067ab10afbc6db3769ff6",
        416_590,
    ),
    (
        "231x0b1c1s.pdf",
        "d4a78dc9271fba7b323ad515dd43fdc8b6c8c4970861002aac5d5012e8492b05",
        730_751,
    ),
    (
        "231x0b1c1s_TCA613.pdf",
        "8349f09b864f22899b290ecc300da8db224b32637175101046b514313fcb5e70",
        13_527,
    ),
    (
        "231x0b2c1s.pdf",
        "6f50f68fa1001db29bd5059bb4be244dd43db408cf7da7d64777d16c03a9a4c2",
        1_004_896,
    ),
    (
        "231x0b2c1s_tc1308.pdf",
        "cd80cdb6624c5f685c50ce084dab960a3d90959f9c10cffc50d070e3f20c479d",
        654_075,
    ),
    (
        "231x0b3s.pdf",
        "938eb4123dc4be7b30cc2c8fe3119055e7e0901bbe2357daefe8f3fb7ba0f145",
        1_045_581,
    ),
    (
        "231x1o1s.pdf",
        "4e02522bb23773f5a7c49d7eb631df915be7f98b266a27f3df98c78593540b4c",
        1_474_602,
    ),
    (
        "232x0b1s.pdf",
        "6a2c5b7066414626ef5ce8c354d44e1d5a1e2502b508f15d8fd2aa1fc9fff964",
        966_814,
    ),
    (
        "232x0b2c1s.pdf",
        "1993b2f402524b33137e4aafb269be40bbe3dd7107444145277b26aac314e445",
        1_335_200,
    ),
    (
        "232x0b2c1s_tc1229.pdf",
        "4a19748fbc57466bc17df88bf5a1da17bef4a753404154971202ce57f412633f",
        16_935,
    ),
    (
        "232x0b3s.pdf",
        "4ba3876bf6d039b5f859676ce703d70c8b74850e292d8ffc4623d896118b4df4",
        1_112_241,
    ),
    (
        "232x0b4e1c1.pdf",
        "0892557c3f28373498c2e1a9f089cd8a63e46eddff2325c90ff1c39ea6c4d792",
        1_044_397,
    ),
    (
        "232x0b4e1c1_tc2325.pdf",
        "2822668ddae64851c9b84c3a7d3e5d820e4c71ff247e345485762233b25f00d9",
        19_842,
    ),
    (
        "232x1b1s.pdf",
        "3208add3f128faeca8f7eccf60452d182e7342e1848ba2500ca55e123d17f5cf",
        969_719,
    ),
    (
        "232x1b2e2c1.pdf",
        "526d310281fa2a462a05b2db11accce56149e235a438b17ef5528a802698f0ae",
        751_904,
    ),
    (
        "232x1b2e2c1_tc1911.pdf",
        "e72bcda4ff27e8bfbce74cb9ae9a11802f397a77994be8db160063c8a9f67d07",
        73_516,
    ),
    (
        "301x0b2s.pdf",
        "2ebea5d4c80ba3238b56e1dfb0f21ca7add12933d6d13a69180d7c44aac09052",
        328_740,
    ),
    (
        "301x0b3s.pdf",
        "00dcf5ed91864869883cc71887aed1091e0b6cc610c2ee66ca4b93319557a9dd",
        623_289,
    ),
    (
        "301x0b4e1.pdf",
        "d431183dbe57548dd4384d0e0cc4e8cbd9b14317972355d3012d14e72ab897f5",
        218_354,
    ),
    (
        "311x0m1s.pdf",
        "e80f887ab6b32f275f133a527872021df158aa5b23f5ea0d0c990a50ec40b190",
        1_310_910,
    ),
    (
        "311x0m2.pdf",
        "97abca0869f052917c2649efefe197ff9173ee7a57af7fab3e14b0422bd75772",
        4_009_490,
    ),
    (
        "312x0g1.pdf",
        "ed1458c56c29f8dd01f03d16ff2b9c6d84f547dbcffe63f32bca3cc56e21a273",
        1_009_529,
    ),
    (
        "313x0y1s.pdf",
        "7b951be2bdad624b6d0feae7d2efc873043e44a553ab701da98cfe89a730c8bd",
        837_970,
    ),
    (
        "313x0y2s.pdf",
        "693c82f6debe6df1c69c5631a61876286cb482c5bb05fc44a604e124779f18fa",
        856_165,
    ),
    (
        "313x0y3.pdf",
        "947bc520da6c16971728fbf29af8fae4d08cc57af0d5fb2322d6d9da17213702",
        380_609,
    ),
    (
        "313x1y1s.pdf",
        "95614574f43591ce0a941eb9ad646d1bc82065e80a56fe177481a77c4301647b",
        1_099_661,
    ),
    (
        "313x1y2.pdf",
        "73cd7fb9ad71fcc73d5771667b8500bcf65c86497b19ab1c25cc50917c1c2da2",
        1_164_600,
    ),
    (
        "313x2y1s.pdf",
        "d068325b78fea90073e9bee2d7b9c31e5c9db5e1b4a8677db3d0bd5b5930bb07",
        827_578,
    ),
    (
        "313x2y2.pdf",
        "c64db096cb4349c920371022a9defdb3bfbadcfd61c7d4136a7fbf0ed82e022d",
        459_369,
    ),
    (
        "315x1y1.pdf",
        "70b4cafd462e73b93ac4800961ee2893ad94009c7a99bf5601b97f8a5aa70bcd",
        320_903,
    ),
    (
        "320x0b1s-Corr.pdf",
        "283ef2381bbf5e3a00c6ea0bd283a76e3e99eaec8e1697406fc3ad2202437587",
        36_784,
    ),
    (
        "320x0b1s.pdf",
        "d7b6e93cd5a432872e4ea9134631b090f335f7a45764f5b72abaeb4bc3fbe56f",
        292_087,
    ),
    (
        "320x0b2s.pdf",
        "e0dd131b2d1b0f15edf7bebafa059cb73ab932816376378b00d628df0de11029",
        409_207,
    ),
    (
        "320x0b3s.pdf",
        "b8334b734420bdf9505f8c82b80c5f5009fab73f6eccac33581448ebbba66739",
        636_694,
    ),
    (
        "320x0b4s.pdf",
        "183c3f2f12b6a328b3cbcf203414f2055d81cb56448b941f2bc6e56ea2bac371",
        537_911,
    ),
    (
        "320x0b5sec1.pdf",
        "fe36804102d64e218f206162ce0ab7dcd21710a7c9649c07a96a2e74b1bf7a3f",
        540_314,
    ),
    (
        "320x0b6c1s.pdf",
        "7f9d86dba88780a5281dbcc77f0dd9a477fc7ee230d3849a4fb36a0432a18161",
        806_349,
    ),
    (
        "320x0b6c1s_tc1328.pdf",
        "cc3281787f231fe355c48bae463b299de5a783fd563f9d41e42ab6a53ac4b52c",
        17_555,
    ),
    (
        "320x0m7c1.pdf",
        "a2cc9c36f6e9ac2c0f5219cd9d351a9c8f565f32cf9f265030bf763740932a1e",
        445_065,
    ),
    (
        "320x0m7c1_tc1919.pdf",
        "bef8cc5d6bb5a96a73f4f0a444ad5093b2c4b8dcfef8b32bd137904ab2241630",
        43_975,
    ),
    (
        "350x0g1s.pdf",
        "5767dec9dd1304e71a0fb8a03b93da55de86b38d6e63abd0f51beac2d2633267",
        583_146,
    ),
    (
        "350x0g2s.pdf",
        "9181aab44b1d1e85ee9614952c40d4cf4e537474ee22446ace552e8da6220912",
        961_374,
    ),
    (
        "350x0g3.pdf",
        "66898dbc179a18dd670fc9fcbb94a7ff89c410153002ad637aad118b7b9e588c",
        495_536,
    ),
    (
        "350x11g1e1.pdf",
        "906a1dbe0691ee8c2bcb4d2dc208d8b2de44920b53cd68e20dc165dab56159d7",
        755_474,
    ),
    (
        "350x1g1s.pdf",
        "63c8d1f50465c61c206c34362e4b341762e0e86f152b605b3841244267f74f27",
        826_665,
    ),
    (
        "350x1g2s.pdf",
        "87f8b676e74e1fc7dcdb1acf5a15904d8d4fd3f92a2a17fc0632b82084da3b1a",
        1_123_686,
    ),
    (
        "350x1g3.pdf",
        "a2da2aafe37c698c53ab8dbc2fabdac8d2405b1cec7d8186915e0b1faff4df19",
        1_491_984,
    ),
    (
        "350x2g1s.pdf",
        "cb4d83beba050bdeab135e603c480c01dc6bd3fddfefece35f06db55a4023fa6",
        817_796,
    ),
    (
        "350x3g1s.pdf",
        "d62b88b9492c9233a18a9dbf0f3334a00384594b1a33a19585ad78f25592e18f",
        787_749,
    ),
    (
        "350x4g1s.pdf",
        "769bc545f4a3e2c9bf00866a8f55b7a4570e5b9cf8589cc6f1b01275d35eb036",
        1_006_766,
    ),
    (
        "350x4g2.pdf",
        "9899042d95287fd64d94c218307424247463b02b016bce33b26ca3305d6671df",
        544_974,
    ),
    (
        "350x5g1s.pdf",
        "16ad69d3d1f379e3e1fbfe768f2bab06b2885eaf4eb29b60b5e70f55754f507e",
        2_266_194,
    ),
    (
        "350x5g2.pdf",
        "616e43f7bbd1e144e0d5409b0874ae0815c46970f781bb4f3164d5de20d6171b",
        2_272_083,
    ),
    (
        "350x6g1.pdf",
        "16a45bad3f059db19c7964b0c4de9348e42ae40eebc4c571fd4b8ed5bb90755d",
        408_323,
    ),
    (
        "350x7g1s.pdf",
        "0d147c7f4dabdf347fcc69cb452a2b3c5bcdd1c5753c182a6a8d0f04d365538f",
        924_692,
    ),
    (
        "350x7g2.pdf",
        "da0e8d69b7a33f4b1bec4e4c61274cc4c43f25bbfe41669471db8eb8ec9a3f5d",
        508_595,
    ),
    (
        "350x8g1s.pdf",
        "796af4eacd19e16d66c2630ede6a03b3d1ead666e2cfd00e0e62183f21d0e294",
        795_707,
    ),
    (
        "350x8m2s.pdf",
        "d7cab79805ae5c0a9c25e8a06baf1b1f845947f0e971d6e000612caa3ed9df1b",
        962_698,
    ),
    (
        "350x8m3.pdf",
        "a9bb56a2811fbd9c3aed90ef6ee8d2d51e0ca4429daed672d259f039cb52ceeb",
        445_229,
    ),
    (
        "350x9g1e1s.pdf",
        "5215610d51e17de1ffa4a581cd3b8f21732cd030403e968ac8746ad50197c02f",
        845_289,
    ),
    (
        "350x9g2.pdf",
        "e79b4180e5e61ef6428cb1f5ae6a8ea009b0165223ad179bd238651189860e41",
        504_126,
    ),
    (
        "351x0m1.pdf",
        "4bdcf457ce941678283e1464feced248cd7e2f966674e16b92c5066b742c5a4a",
        416_713,
    ),
    (
        "352x0b1s.pdf",
        "1fe77fc03f46fe3281e97df628561b0c9c02db3ecf5ade2e1e500ddbde8a694f",
        824_040,
    ),
    (
        "352x0b2.pdf",
        "3dba694b9e20b2e4d2d4d001127d4140435dd5d0fce07faa801ff085a22a9ab0",
        370_836,
    ),
    (
        "354x0m1.pdf",
        "b58429ecc006254fe52969090796db4b0b7a22589d3efd21c00053bf4cf5e71b",
        531_482,
    ),
    (
        "355x0b1s.pdf",
        "10c478159e2fd225fe780eaaa64e0050e26dd084b5087c2d59d3aa3b8cd12662",
        1_165_336,
    ),
    (
        "355x0b2.pdf",
        "86717aedd33886013333054355b92d2d32ffaf84a4a3eb63b7b6ffc6f17d5818",
        764_682,
    ),
    (
        "355x1b1.pdf",
        "5864d866e9c66fc2e644225d6e61724bc6f2c140359e444377e74ceb1a722483",
        935_515,
    ),
    (
        "356xb1.pdf",
        "f39b8bafae0405d2852963b3907679e61914eecf1e181f401510e797639dddce",
        397_925,
    ),
    (
        "357x0b1.pdf",
        "26f01667905c52987326a1baa209718ce8746826829808b3553d11178b3d4bd3",
        249_245,
    ),
    (
        "357x1o1.pdf",
        "144bc213b4a8159e195c0217050ade42fcb124ab7e5399fa04778e5680586c64",
        887_274,
    ),
    (
        "371x0g1.pdf",
        "371051ca0e52dc141d4c6baedb3ef03ac0d42ee960d13ea6e3adad2637188c8a",
        2_586_262,
    ),
    (
        "401x0b04s.pdf",
        "39206750d37c60ea659ea7d0937c09cc166a464c0a79cf3e3bd95f028a366347",
        893_682,
    ),
    (
        "401x0b07s.pdf",
        "9468aee5a1ebc8e96edbabbd31362fbce04ede3bb401923e0d3b978feea841d1",
        1_384_904,
    ),
    (
        "401x0b08s.pdf",
        "c61052233fd028ee38edd27dd82a470aaa79ae488ca631b3c081231eeb9fc195",
        1_393_228,
    ),
    (
        "401x0b09s.pdf",
        "645e66d02493a4f18972ab67d883a9bd81dc4cf52b1d22dba7ba108fa83ce7c1",
        1_405_659,
    ),
    (
        "401x0b10s.pdf",
        "8bd6e38642ac76c9a12cf723154cbe7370eb0271d1868cb41877b1b13877e502",
        1_451_049,
    ),
    (
        "401x0b14s.pdf",
        "2859140aac4882ffc6b14602d59070d5a6f2cacede38c1d1fcf66894fc20dfe5",
        1_649_592,
    ),
    (
        "401x0b15s.pdf",
        "2ca52fad86cabea54cc27bd2ca221f1a681502a06f18f8054621e8155f390f8f",
        1_267_745,
    ),
    (
        "401x0b16s.pdf",
        "db354bf30afed936c22a8a5b3d7445f12d5fab39f16c8676c6032dacfbe740bd",
        1_493_349,
    ),
    (
        "401x0b17s.pdf",
        "efc93036cb222fb4f99b4fc1bebfaa28ca0ad22bede613c3637465f8a3b7f012",
        1_936_476,
    ),
    (
        "401x0b18s.pdf",
        "b120549afbe22057041fc4130d44a6db961d091a3f8b4971301ad9ed4deddc6b",
        1_578_244,
    ),
    (
        "401x0b19s.pdf",
        "1ddc398b91c9fb464fb0dc8535fadb913ed4b0295ee558e958246d90f72549cb",
        1_844_763,
    ),
    (
        "401x0b20ec3s.pdf",
        "264f7cc35dfac41990dbc85893cb660ced85b0e7ff4ae15ceac86e4a04e22c4c",
        1_549_893,
    ),
    (
        "401x0b21tc1_tc1213s.pdf",
        "92622d6a8b952a29e12921d0e99dc037fe73cbdf3ab648b96b0dffa4a97153a8",
        65_107,
    ),
    (
        "401x0b21tc1s.pdf",
        "12125c7a17cb7f206fd7c4a39fb999dd6b3b95a050b5e2b96f1720b32537dd3a",
        1_430_349,
    ),
    (
        "401x0b22s.pdf",
        "608dc36ae351eca6da93a5cf31e4f5256e0c3b25741bf6d6f5cfc549b7bb8919",
        1_646_564,
    ),
    (
        "401x0b23s.pdf",
        "45188941a2041322c6f7771879d530957619282f201672ca29b83fff696cbb0a",
        1_400_706,
    ),
    (
        "401x0b24s.pdf",
        "855b7fd6eb15f6c20ddc2f7f6c24fad971a77e1dbe0cbe73f5a789c216f518bd",
        2_193_703,
    ),
    (
        "401x0b25s.pdf",
        "5c8b9c287c0d47c883929ee731449caccf7c3c2234bd58148b01c772bd87b02e",
        2_365_560,
    ),
    (
        "401x0b26s.pdf",
        "0d1c3002e339112f7189d5c736f829aec653f4389e32bd8fe3e53c8d4278f616",
        2_531_816,
    ),
    (
        "401x0b27s.pdf",
        "06ed29150d97915b64ba87c0f481523989ec8388fc39875d5627356478ef100d",
        2_670_631,
    ),
    (
        "401x0b28s.pdf",
        "3d594d0904206e1d753a74a8084a811dee34487a49b664bfc7de7a0b137668ae",
        2_493_229,
    ),
    (
        "401x0b29s.pdf",
        "7eb27fc650947214a70020071760f4678679fe26854f53730ade7d71bcffa61e",
        2_652_879,
    ),
    (
        "401x0b30e1s.pdf",
        "6b6b59d2137337102c4f6896d55a100f84aaa463dff6be858082825e90d1366a",
        2_895_105,
    ),
    (
        "401x0b31s.pdf",
        "0f3c035683f96c6a31011e19838fbb141eb70ab174a3a5117d7f9999c44c22f4",
        3_163_971,
    ),
    (
        "401x0b32.pdf",
        "ba719fdaa4b5ca277c315f1136812576fcf2d521a3318563afddb894dbc2dfb7",
        2_574_606,
    ),
    (
        "411x0g3s.pdf",
        "90ecf0744432b350b194afe32cd8fb9a77e7530e7c9218d32250268adcde1a8b",
        2_138_504,
    ),
    (
        "412x0g1s.pdf",
        "4b69068da842c716cd8980657f524f05ce05b83e65266ef70180ff2838278c92",
        520_184,
    ),
    (
        "413x0g1s.pdf",
        "8e187aef9c416b85277158ce63c9808b7399380c67285b6e8e6df42dbbd40dff",
        2_451_267,
    ),
    (
        "413x0g2s.pdf",
        "fbc1bba50084acd5d013e4daf5027cfa5fb17812bbca8247fa5fafbc78aacccc",
        2_545_207,
    ),
    (
        "413x0g3e1.pdf",
        "a99a4bbd32e2e811120ec05d6eec5081e106e14f71c687d0eda312b7fed5b3ea",
        3_165_446,
    ),
    (
        "413x1g2.pdf",
        "a89044be79e9c91d46fc993a059a4a5a2dc1cc032eaba62707e97c7066df40cc",
        10_956_060,
    ),
    (
        "414x0g1s.pdf",
        "14fd7cd14087179e4cb419cffbdf6d9b7c3e5014b1a236d5b3396ee6086ce781",
        2_305_195,
    ),
    (
        "414x0g2.pdf",
        "f4ba14d326d63a9995ad3ec95ec147139436247db07a3a3a23f05875b08f2b11",
        2_038_661,
    ),
    (
        "414x1b1s.pdf",
        "1c009d4900ee16920f0409064027a966fd5d271c1747214ab64d7aaa36ad3b30",
        671_799,
    ),
    (
        "414x1b2s.pdf",
        "3582fc94bbcd3592e6dfe0be35696cc35e8de70a31078dc8fc9461a1f0cae169",
        916_287,
    ),
    (
        "414x1b3e1.pdf",
        "6e8f92da4c2a11d05a60824bbe5d65cf957edb280fb786248959412c1c12a446",
        550_071,
    ),
    (
        "415x0g1.pdf",
        "023ba0f6e8f232be1f5a5040a621c4254991f9932dcb299e6214480af9a02b96",
        442_245,
    ),
    (
        "415x1b1.pdf",
        "5c817ae69f42303e7b810524e951dac653ae1597f459c0eddccc38ad74ae6caf",
        368_929,
    ),
    (
        "431x0b1e1c1.pdf",
        "22425aabc8c57af9d382d78e852756d1a9c0bfbf94c8ee27064f42236c974a9a",
        565_785,
    ),
    (
        "431x0b1e1c1_tc2209.pdf",
        "8152928a53a4813f5449744e13e04e7c45e974eb7152c1a5982ce80e7c637887",
        74_824,
    ),
    (
        "500x0g1s.pdf",
        "93491f7fcaa154426e2a90d4c00c273f22c850970deb490c6116449d002d00e2",
        614_889,
    ),
    (
        "500x0g2s.pdf",
        "d9a8a9a6254e11509ed541b6b384468b7c558cbb0f76ba2a6abb5f6869ddbcc9",
        1_343_376,
    ),
    (
        "500x0g3s.pdf",
        "c3b308fa0fa0112a17c0eb4e0b375de3fb763ea9c39eef201327394c0b206b96",
        4_269_473,
    ),
    (
        "500x0g4.pdf",
        "e9a012dc485511aef66b7d6c01cfd17f4359c1232b92b9266ce515f9ef446859",
        1_256_366,
    ),
    (
        "500x1g1s.pdf",
        "6957fe336dc5a978476ded66791eb89d7accb2eacab6abb1e12f0c7300eb1501",
        1_242_003,
    ),
    (
        "500x1g2.pdf",
        "fe286aa8b851a1c87ca65136d2cce4e861bf2096af1db11489195afe5e3c1f7c",
        1_895_410,
    ),
    (
        "500x2g1s.pdf",
        "c8abdda442526b280285967073eb68bb93edc865ad90587b1a18436c466fe199",
        862_614,
    ),
    (
        "500x2g2s.pdf",
        "1571bbfb90773e820dc2f0c87ed4a8a8b1a8f68c685a1188cdb99148b67439dd",
        1_021_723,
    ),
    (
        "500x2g3.pdf",
        "f9ac61100cb7dd79f0af604345659abc0eaff9f73f5a5017606bbfcee32e85d9",
        452_358,
    ),
    (
        "501x0b1s.pdf",
        "801d071bfc34e9a94d40651a752b48a87bba47faf0495bf839b985c5c9933593",
        475_740,
    ),
    (
        "502x0b1s.pdf",
        "fab306a994e5bce9cad8c64d77af172a5819af38faf459faab1b7e2f6163223c",
        651_066,
    ),
    (
        "502x0b2s.pdf",
        "3dd6102c119cd4d33a97f03437787b5bdfb6b6acaa036a0e0b716c6c9f5747db",
        1_381_441,
    ),
    (
        "502x0b2s_tc1214.pdf",
        "065cad33f42c986e5b9c39503db49eb3d4e1a749e2163df9188c49e6f62cc75d",
        22_264,
    ),
    (
        "502x1y1.pdf",
        "59f6fe604f3b34330f6ee578edd43a98d583d8868746ba72696f5669b29a986d",
        431_702,
    ),
    (
        "503x0b1s.pdf",
        "251a5bcdf1f254f06ac1b7214c9e3e7411829c03681ceaea6e287fcf757d1e0f",
        1_092_633,
    ),
    (
        "503x0b1s_tc1022.pdf",
        "375b3d9ac00997a375e5a88f85213ffdf36f33ec249a3adbde81c04b472d600b",
        20_798,
    ),
    (
        "503x0b2c1.pdf",
        "6bbe6169eccb3c6b744ba002f02785e1861f8531d3cd4dcf6fa9056efe515677",
        938_326,
    ),
    (
        "503x0b2c1_tc2136.pdf",
        "8ecd7fb3c4da8454a4ebb6300687e17f6e56a4feb267d6249c30d0709880698c",
        22_396,
    ),
    (
        "504x0b1c1s.pdf",
        "70fa4f29d79a6c0c26f171703ccdb0ac5d5c23dbdd13308e2990956c4720c746",
        1_054_914,
    ),
    (
        "504x0b1c1s_tc1518.pdf",
        "0a973876b8880bc80631b947c77f022e49c53f45c6cc7d629f81ccbf8da228d8",
        15_371,
    ),
    (
        "504x0b2.pdf",
        "0869187cb326dbf009e6f17a1fd6e920b7b4a4f8f6446706b314e2d9a6121214",
        1_179_220,
    ),
    (
        "505x0b1s.pdf",
        "1065819e3379965f7befc7b5a6a0e9a3ffde0759bd7d98d5eefc41c2f0b9d655",
        1_325_302,
    ),
    (
        "505x0b2s.pdf",
        "411f21545126d1e7818ea87ff95ed8b98e2890d4c853b4b971b10c6593a2963c",
        800_500,
    ),
    (
        "505x0b3e2.pdf",
        "21516570863fd85a3f9d4a322ae008054113e7d6840cd43918c44bc2bb238c09",
        724_294,
    ),
    (
        "506x0m1s.pdf",
        "38e68cf503dc837a5e68fab963d009437ddd408aef53b47bacfce8e0b9db7439",
        1_042_622,
    ),
    (
        "506x0m2e1.pdf",
        "bb399ad0bc4edd9cf32194c1ee91230f1ae2be42f3ed2d3770a1a24a72666932",
        767_534,
    ),
    (
        "506x1b1.pdf",
        "4b53cc45975b931c5ed08e96ea0592175102558fc45568e5c1a3a5afc7937c28",
        387_684,
    ),
    (
        "506x3m1.pdf",
        "80fe8a15bff1f178f48b2fffb24fcb0c796f7b2e25b59ea48f95fd6fc5a29687",
        196_824,
    ),
    (
        "508x0b1e2c2.pdf",
        "6afd6dad4b5f6c2ce17c3b310fdbda62bbfa6f248c32decea7de6f9116276978",
        843_546,
    ),
    (
        "508x0b1e2c2_tc1814.pdf",
        "7ecd36d0e09a46d0770d5420098126abd3811cd8cd75160d8508d065fd35b01d",
        18_182,
    ),
    (
        "508x0b1e2c2_tc2137.pdf",
        "200bc425418cfc0a2a1fdf31991b0507b387049fbb5866485f05676cd7d3acf6",
        23_646,
    ),
    (
        "508x0b1e2s.pdf",
        "1835c96e568c1d04b871acc91ac4135686ae092f06a8865c7a07f6565b7e2d20",
        959_561,
    ),
    (
        "508x1b1c1.pdf",
        "8e761c3eb0fb08292f58b624292d5276cabbe0d53b1719ce09a79f9fd63df27c",
        855_759,
    ),
    (
        "508x1b1c1_tc2138.pdf",
        "6b97d0110ea1c250e29f0caebae8fc4818a5ef978101b96dca3f038c89a1844c",
        22_442,
    ),
    (
        "509x0b1c2.pdf",
        "82dcb61f6ba211eda7d886e11bf911e745d3ce34542b2c38314cc47c994f4a13",
        1_369_219,
    ),
    (
        "509x0b1c2_tc2117.pdf",
        "77608d44eac6a5a9e8ec5a9c5f18cb74cce67aab6ad8a016926453e08cebdc57",
        19_428,
    ),
    (
        "509x0b1c2_tc2326.pdf",
        "50a78998defeba564d6cf9b0a96cef2001561f5ed27136480ccf9ea39d7f57f1",
        24_673,
    ),
    (
        "520x0g1s.pdf",
        "221dccdb0ce4b165eccb07c6cd765f33d80b3aeefa0447785f23a51b3a9811ff",
        1_423_506,
    ),
    (
        "520x0g2s.pdf",
        "44b7caaec912849e219db6488c6f0a33a5bb9ad069d6b9dd864b321fe7184a82",
        748_652,
    ),
    (
        "520x0g3.pdf",
        "6e945a8683098973d0548b38a1312a0e7c4d5b046eae15a2c65245202c267cea",
        676_593,
    ),
    (
        "520x1m1.pdf",
        "8ac0b5bfcce4f1a38ea642723cbc4968635b43578595904a5b1837fd9e1640b0",
        540_736,
    ),
    (
        "521x0b1s.pdf",
        "5d0b4f0855e0926aa6423c237fc6272cc811188642b5c0942afa9983926463e3",
        1_338_146,
    ),
    (
        "521x0b2s.pdf",
        "3ca5e991e26bb08aafcdf0c3aa1c593714624e9c6918692f3b52ef13575292ef",
        1_301_644,
    ),
    (
        "521x0b3.pdf",
        "98870d808f475a4b8a1cfcd5f57d39373c70e1405783e637c0868a0ab069ff63",
        1_379_083,
    ),
    (
        "521x1b1.pdf",
        "6e5fcaba0f1bbd22bde660825e80d4806bc83bf2186778515234c8346096bb4f",
        328_575,
    ),
    (
        "522x0b1.pdf",
        "31ea5b9bc619d1f3e3a201a6b2796c5c8602ac7e31c07c2a3b10d34b6583e926",
        792_547,
    ),
    (
        "522x1b1.pdf",
        "e8ad24e5b9321c313e408e0a7b811108287f0cc767435f0b0dad60f42f9ccf7c",
        876_960,
    ),
    (
        "523x1m1.pdf",
        "affc15974738cdc0194a99bd59cb11c46f52bd8a172a77994630e3fb20872390",
        880_065,
    ),
    (
        "523x2m1.pdf",
        "707b00426ad22abf4698079d8c5fe598b83ee8ea716d3eae6f37c188c3de551f",
        779_185,
    ),
    (
        "524x1b1.pdf",
        "6daa871c4f1298fbd0789dd6073841f5c71e7860e44a0b613c17b156939332a4",
        403_507,
    ),
    (
        "524x2b1.pdf",
        "bd1b397241f8566ce8ccc5d21de98cb0142a6fcbf7d4cba123da97378fbbe721",
        512_483,
    ),
    (
        "524x3b1.pdf",
        "648141ad8c460f81dc5075df091886d553d820069a1223ebcb7b8c1adc854cc3",
        317_793,
    ),
    (
        "524x4b1e1.pdf",
        "856ae9dee223d95bf3c60c090972335912e43a59eed56329fe15c5b6a7496c80",
        758_698,
    ),
    (
        "529x0g1.pdf",
        "fde9e1364d82241077a8c071aee2549a63122153344e45dc95f4a144b9682471",
        834_295,
    ),
    (
        "540x0g1.pdf",
        "5f92d4133e2f0fed10c82d4650ef4d469a8429941c0b96d061ecdb18575fa474",
        736_783,
    ),
    (
        "551x1o1e1s.pdf",
        "d30f2fed889ad8cef003e78ecb8026b5e38365ee0520bd537ab32fac3d0c567d",
        1_221_384,
    ),
    (
        "551x1o2e1R.pdf",
        "dcfb39e33fddaa50466681cc8e3336ab744ac207d4b3c9a5d683953c69884735",
        3_606_294,
    ),
    (
        "551x1o2e2.pdf",
        "3869012d8f9d7afac6c97bda5ac1424ce103580b0ea775510ce67c700f252c30",
        1_414_939,
    ),
    (
        "610x0g5.pdf",
        "89830e8163456ae0d80864fb7c6ca26c80b3e6797f0de32b0fb6774116a89c8b",
        141_018,
    ),
    (
        "611x0y1s.pdf",
        "f8da0adf44662a0273154806d71d9031df33c03eb5e9b664427591dcc819d880",
        602_417,
    ),
    (
        "620x0b2.pdf",
        "e04570a761d5f1a5a76c8a438c35c716e271e781d8844af0a8d50291554e6c8a",
        513_983,
    ),
    (
        "621x0g1.pdf",
        "e4ecb9f532cad7bdc0fa2f12261e1368c43e8042d0fb22c14659e0b5cad664e1",
        1_292_867,
    ),
    (
        "622x0b1.pdf",
        "fd39dba1de926b4eb70d9ae35cc5e38acdd5cde221c88579e28340e3b9fea25c",
        192_020,
    ),
    (
        "630x0b1.pdf",
        "bdde5c62f56b7d060e5ad086e16b1f1816617fb7af2504ba47147fc63d86249c",
        97_835,
    ),
    (
        "631x0g2.pdf",
        "47aa73dd0be88c768fd8a302a3317ac94a983cbcf469df3b264a90b31faf3c95",
        213_874,
    ),
    (
        "632x0b1.pdf",
        "6ee3da7d4c709847b0b30729a1b4166c792b1704410b995dedde4ba5c4328d98",
        121_473,
    ),
    (
        "641x0b1s.pdf",
        "0db893854d41ea1a52ff61fe97fb6903ab0a26b6466c54b187de185d58b1ed27",
        391_168,
    ),
    (
        "641x0b2.pdf",
        "0c83a501d0ed4e380a1ca7a40f66f84f6a3baffe85ce68a0a2876c2b3dda23a2",
        347_952,
    ),
    (
        "641x0g2.pdf",
        "2097101ab58380acb321c01748d4e882e55678dc5ee887b98a3f009c5a4cedfe",
        437_079,
    ),
    (
        "642x1g1s.pdf",
        "d4b1b0e9fdadf7f5bfcfe622bdc1b378491061b9c62dd50058e32cb6674592ac",
        90_449,
    ),
    (
        "643x0b1.pdf",
        "840c55d1fa8a6f1d590371d244c794b8f62a87a2f22ed7d5cee186a2aecd34fd",
        68_468,
    ),
    (
        "644x0b1s.pdf",
        "76991b88d2d29a623da155f4ca6e9fdfb5c2d375e2d682d93e674fb0cb0884b3",
        640_041,
    ),
    (
        "644x0b2s.pdf",
        "0f7afc8df3400fbf53ae46563d83a9b502653de820b27f3bd9820cc2324120df",
        817_884,
    ),
    (
        "644x0b3.pdf",
        "e4d2424ff07c1f72007171ea7f7cd7ddd0a96019123c37d1122a07a61a77e711",
        537_661,
    ),
    (
        "645x0g1.pdf",
        "84652a9f8bb2c1d2d97a6ac6208a17d70cc0cd190c7d000f6ea97a7cda547a67",
        430_977,
    ),
    (
        "646x0g1.pdf",
        "addb45f21a4ebc2f0c8f7f30dab65a8113acbed518eb23a10120908a73c23a24",
        89_086,
    ),
    (
        "647x1b1.pdf",
        "0b116f0d2d1a0856ac4ab7f91fe6c94a3ac1b26f3c62637253233f0d9f602995",
        400_908,
    ),
    (
        "647x2b1.pdf",
        "daf9feba93a1ffe0c677cb6e3565ec6e0f738a75d28681127cefa0e9b912cb5f",
        319_585,
    ),
    (
        "647x3b1.pdf",
        "025aec6ceaf4f9f3498c2a6070f118b4934c156b5ce26876b4aa20dcde607418",
        370_952,
    ),
    (
        "647x4o1.pdf",
        "c29a79b96d13bd96ab293e336ab9a4de7d16d9df3c776c351508d0db1cf665e9",
        485_519,
    ),
    (
        "650x0b1s.pdf",
        "c5f67fa617dfa6b0d6fe0ce05f6c6d82b0e04dcd177dc53bea815d0521e75f96",
        749_260,
    ),
    (
        "650x0b1sF.pdf",
        "f512ca9d81cff564ce40d643ac4516c6eba3414d8c52ee8076f19dd5703a8b9c",
        1_051_932,
    ),
    (
        "650x0m2F.pdf",
        "ad347ac5cba06be09dac0f3e978adcd96553f56e06a5227ef5c18a4d6750396b",
        3_178_032,
    ),
    (
        "650x0m2s.pdf",
        "745edaacb308ef3f57d37d759a955545a2775303df87b2dd3c18f92ceb66aaca",
        2_554_107,
    ),
    (
        "650x0m3.pdf",
        "ff8ed9a06cc4755035a3b284d0dc03d136a0a59eb22b8b788e48dec8e808715e",
        5_162_898,
    ),
    (
        "651x0m1.pdf",
        "89d5ca4a2cf75a2f1a7c098438b890fbcda882577d358df86d934131d95ef584",
        521_980,
    ),
    (
        "651x1b1.pdf",
        "a286e4c3499d7b654fa2439ba4ebbaf729c1b90a0398986fcea274281c5605eb",
        603_351,
    ),
    (
        "651x2g1.pdf",
        "f78761c58a122db1b0aecaa8cd580bbccffa1ae3fd69ec4e580dce65ab7239a3",
        2_332_029,
    ),
    (
        "652x0m1s.pdf",
        "1b6feaa671a218abd708935f58179e71ba71cd6f7ecc0442c0136d4f3ffea06e",
        1_050_662,
    ),
    (
        "652x0m2.pdf",
        "b3f633018b8e510acbf83352ba1e7d05163aab186310f519ac52a3f8936f2c12",
        715_318,
    ),
    (
        "652x1m1s.pdf",
        "4a2a3b932a160e10ac5e84519e02704487b6204f7d0062858920b531a8a47f62",
        579_187,
    ),
    (
        "652x1m2s.pdf",
        "b840cf03d327f47c865f3c6f9b14ebacae33fc21f61e5a84398afb4ab830efeb",
        820_133,
    ),
    (
        "652x1m3.pdf",
        "35471877a01cea2d6771489c36565728896b83af6445582bb07316f86eb45953",
        491_762,
    ),
    (
        "653x0m1.pdf",
        "4bfc95943b0f018f40615aae0b21c213399a5afde73783465cfd1721892bd457",
        1_098_202,
    ),
    (
        "660x0b1s.pdf",
        "2a7c9b6234a26049d5eaaedc9d5083a8d82763f9e8371d6732e14e690e20e95f",
        1_263_531,
    ),
    (
        "660x0b2.pdf",
        "160c7ac6f40938c5c95b23cd65fbf684fff57230a66d5145eb503b9c39995b64",
        460_648,
    ),
    (
        "660x0g1s.pdf",
        "77c4f8099bd8b16ef83b6beeef5a9959a72af55dc929e3921e2539f8399eea74",
        1_863_869,
    ),
    (
        "660x1g1s.pdf",
        "ed409d45908645ce466ee3369b546ac1208c1627529c338286eb747bbc67b016",
        2_567_896,
    ),
    (
        "660x1g2.pdf",
        "280f6614f1d87546047ef014adae7587fa5b7158f8a76a84fa22ceda7ac60375",
        3_699_236,
    ),
    (
        "660x2g2.pdf",
        "e7f793a0ce4d6529959dc0075da6e7184831eb219615ca3abb7870af5adb352b",
        564_953,
    ),
    (
        "661x0b1.pdf",
        "e340df4e6e046927d812c7fdee272b6f1c229544d3eb481db0ed629eb97f64d4",
        3_434_796,
    ),
    (
        "700x0g3.pdf",
        "0f98f06896e57a3be09715434b3c1b06afaceb76ce1c033d83c431d2885651d7",
        179_002,
    ),
    (
        "700x1g1.pdf",
        "7ec879d24e35d10e11eee6872ef3632fbcab139c3ab35539ad5c577fa51d6bdf",
        786_766,
    ),
    (
        "701x0b2s.pdf",
        "34428b7d63bebec4708f1d7b5eae83c06a27fb5729d68020e3ed7c4823618f07",
        775_588,
    ),
    (
        "701x0b3s.pdf",
        "1fdb926dcd9560184d7d75d2d060688d126ff3c1547ecc0ef4770cde7515ac94",
        1_005_523,
    ),
    (
        "702x1b1c2.pdf",
        "a60a7be932788027a2009603a5adfb4c51773bbe024d08dd9f75475628192e87",
        484_279,
    ),
    (
        "702x1b1c2_tc1413.pdf",
        "22d002eae9b587bcad6bdbacc2a1e9d9eb490fa96b82895f3af19f4718a94ce7",
        18_237,
    ),
    (
        "702x1b1c2_tc2327.pdf",
        "3d0c33f75cc5a8cc09b891ebf282e7764934c7f284486095a5233ce7586f4a67",
        21_651,
    ),
    (
        "704x0b1s.pdf",
        "2c34583b241abfa3e539ec086789bbab680dd4bd648017ffa54ce33f248307a6",
        542_343,
    ),
    (
        "705x1b1s.pdf",
        "b772a96aa1d9ef4589125446e55e2b4aae90b1fb180496d0e80759da06f47695",
        541_784,
    ),
    (
        "705x2b1s.pdf",
        "2502bc8706d5d3adbb409d7b0f2368244da26aafb35227839e001491e13eac76",
        520_831,
    ),
    (
        "705x3b1s.pdf",
        "e98aff7f2041d6822059a25e998f806c8acc78d21aa51c05f3179b92f3659571",
        453_303,
    ),
    (
        "705x4b1s.pdf",
        "f0634f76053dd11b03bde595764ea497d63a2fb1ff4adcfb2e79884e469af710",
        444_210,
    ),
    (
        "706x1g1s.pdf",
        "85176e1e11b23311d89502e4cc4bbf3c5fbbdb3e9a9e66ca65f1ee9081e59e56",
        2_411_944,
    ),
    (
        "706x1g2s.pdf",
        "5f62f611b7eb5c636b84be79fa3e974e4609dd7c318342391cd934aa86c97d96",
        2_474_519,
    ),
    (
        "706x2g1s.pdf",
        "7af71c9a45e734734edec7ef38c943840e2265b1557c8906b304a1d34180c6bd",
        2_140_239,
    ),
    (
        "706x2g2.pdf",
        "cdb9a3ed70ed0fcb885073963a2fac5054cd7b75e7e9572f32216d589b2013a5",
        1_907_964,
    ),
    (
        "713x0b1s.pdf",
        "2813679744c63d68f5360cb49353c6fd121084b4a34f26f4ecfea6f53660a6a5",
        748_519,
    ),
    (
        "713x5b1sc1.pdf",
        "2087b364c927538bac61d16a7264e8100f2ab5a28b404c050bb91b59100b9abb",
        675_493,
    ),
    (
        "713x5b1sc1_TC1018.pdf",
        "6197bf5b3c531e861f1678438ed07b0385e2c68de68fd608fb70f295a63c8240",
        21_219,
    ),
    (
        "714x0b1c1s.pdf",
        "19f0bb45f2bb642f4955c071fca8937935ef31218b845cc88d065b8273134e26",
        792_454,
    ),
    (
        "714x0b1c1s_TCA532.pdf",
        "0b25237320f32ebd9fd5c04eba66c1eb664661d67f28204c0c90fb003813376c",
        80_220,
    ),
    (
        "714x0b2.pdf",
        "53b2e87744e0862f94161125ee979e20667318e22f1afaeefa6b952757e9bf78",
        1_198_722,
    ),
    (
        "717x0b1s.pdf",
        "815ec017eb724306454f291d19faf9765a6abc093a654e450de2019ad5600a0f",
        691_167,
    ),
    (
        "720x1g2s.pdf",
        "a9af89d33588f8920c8fcf33cbb822f8dbd54fa65e71e2678cfa999a78891924",
        1_081_799,
    ),
    (
        "720x1g3s.pdf",
        "81baf7b1e2f53168bea68caa281e44b690440fc88efd778980807309fc59848d",
        579_961,
    ),
    (
        "720x1g4.pdf",
        "7ce36999711b7b20b783f79a7ef00f334b94e09226959da83f29b5a30cea05c2",
        876_363,
    ),
    (
        "720x2g2s.pdf",
        "b2d945dd0ca87d21edb02c1fdb75c400a899d50e1ffd28120e778bcd2f4e32a6",
        2_159_599,
    ),
    (
        "720x2g3ec1s.pdf",
        "452ad23196512588c4fe8b7dc88bb63c6f6420cc69a9bff9b8bf26f8c045e96f",
        2_464_362,
    ),
    (
        "720x2g4.pdf",
        "6b4738ba340c70667c1106c82ecf199257e93c41a561e6991b771a262c4e8310",
        2_135_899,
    ),
    (
        "720x3g1.pdf",
        "754d6adaff6edb016170fc8b966090d586069415c115b9896acc4bac77ff7e45",
        304_427,
    ),
    (
        "720x4y1.pdf",
        "4895aa114a48778d725153e803483da31a931e337187baeacf153fdb7d1912d5",
        276_895,
    ),
    (
        "720x5y1.pdf",
        "2df28688a69466505e3204ec4dcc3b7ccd8d5e9ba9b8c1eefa129a15041b013e",
        247_663,
    ),
    (
        "720x6y1.pdf",
        "17e4de489492ce9edfc666541b4f0679cb70746f5ae1dab7c54988c62cc98ad4",
        308_335,
    ),
    (
        "727x0b1s.pdf",
        "7f657bcdd6cabe8ee5cd53e83a1cc5a0a212a57d090d2f35f042e6fd15c916bf",
        957_849,
    ),
    (
        "727x0b2s.pdf",
        "b669ad4d35e45be21f831ad7932373fbfdf963825b8999c8df5a30dfc2011b34",
        1_255_711,
    ),
    (
        "727x0b3s.pdf",
        "9c710b29addfa23bbfff2dd9a1fec3cf8f30538c56a1b8b1b3ad39c1d5be8787",
        895_348,
    ),
    (
        "727x0b4s.pdf",
        "adaeef7588ca784fa0a45ee7aa1d6c2bae59ffc8a4e8c7cd06888f1f68c9b45c",
        1_153_880,
    ),
    (
        "727x0b5e1.pdf",
        "8b84c694be2312cd7364b39a8fd053a75c1bd48cb477965841750ec46c13108c",
        1_159_291,
    ),
    (
        "730x0g1s.pdf",
        "743f36b84cf1d73739034a50f1f90131b574f651f1a84854190e6f525273346d",
        1_171_720,
    ),
    (
        "730x1g1.pdf",
        "552fb828ed364553d61c7921b895a1077e032e78fd409706465dadcc93259d4a",
        3_868_623,
    ),
    (
        "730x2g1.pdf",
        "31b37def8415ed2d3b034346c4afe5a1b74f2823376ac777e669bf6391f32938",
        1_955_913,
    ),
    (
        "732x0b1s.pdf",
        "2b991cfdd6df9a46ea3bb67c78177fc58695d084655e53a11928c03297c98ac2",
        911_650,
    ),
    (
        "732x0b2c1s.pdf",
        "1019c88627a5d38b1d09489f16030d0d0624ccbf3eb0c27a8ebb642d55af12f2",
        1_102_084,
    ),
    (
        "732x0b2c1s_tc1230.pdf",
        "f99ac9a30a0889f2cf1d62fba30006ce850a3a809b0ff9d230796e1da7bbd5a5",
        16_941,
    ),
    (
        "732x0b3e1s.pdf",
        "de7c05a7f22811ba528380035b0278168f113dde3f4b328472ccd59e35a46655",
        1_092_037,
    ),
    (
        "732x1b1s.pdf",
        "4aa23202351aae7f53fb3f115d5c9af2e48aa4cd547feb185d26347dd0f7fe43",
        1_316_682,
    ),
    (
        "732x1b3e1.pdf",
        "3d931ae1b9ffb6a9282fedcc79e619356e61f67729d1e3f9bf433f8e719420ff",
        1_364_346,
    ),
    (
        "732x5o1s.pdf",
        "e7014c4e991a3c4a8cecc72079422a438004d3b501c2b0c63acfed75ffa40518",
        650_226,
    ),
    (
        "733x0o1s.pdf",
        "0ecaa027402df7ca706d44052b3e21fb9c736c6af2c144238273486fbd28dd9f",
        676_358,
    ),
    (
        "733x5o1s.pdf",
        "da78fdca187abe258ef529679fa8bb6f63db9fd83e98b85c65166e49992ecf7d",
        652_630,
    ),
    (
        "734x0g1e1.pdf",
        "3a7c766f36effc3182c15883df5ac494384b657e72bd574b04f1a9d7ab402e0d",
        1_276_532,
    ),
    (
        "734x2b1.pdf",
        "c37f025602d28800c31c2ba1770ba27ed4e977305d0b3a69ecad8ba67f5748dc",
        591_061,
    ),
    (
        "734x3b1.pdf",
        "b9ae81db038188291fbf20ce229359aaa96ba56c587bb7d147fd7fd2198dcfb2",
        477_204,
    ),
    (
        "735x0g1.pdf",
        "d382d4e302a7bbf9857a30e1dff6565152569cd36cc32f5e8617839b10bb587f",
        380_050,
    ),
    (
        "735x1b1.pdf",
        "13d39d022fa241be0bc815a1eb8a15c35549dc6b04c93021fd64d58364c9858b",
        640_701,
    ),
    (
        "740x0g1e1s.pdf",
        "6c5a606102eaeac9901ef7a4412b3ee9531a3bcb96112f05d7c330076a12fe41",
        1_199_775,
    ),
    (
        "766x1b1s.pdf",
        "e358b7245307834202240fa51cd11d3c2eee12bb9592cefd597c4895d97ad5a4",
        941_703,
    ),
    (
        "766x1b2s.pdf",
        "be7620a1ad1a9d6a2e6316ebf5928a318d3f5a3bbb9730c406ccc7e9050fc55d",
        942_694,
    ),
    (
        "766x1b3.pdf",
        "08b6c113a1c3a6a19d3a2f4b1605cb0ba9f6c8eea56f8e923232f3f0cf0666ca",
        544_173,
    ),
    (
        "766x2b1.pdf",
        "ef9dcdcca04efde2e476146b6011e172616c6ead2512f8725be48423865eaa4f",
        643_986,
    ),
    (
        "850x0g1s.pdf",
        "83d5f9e7aae2c2f334b1cdf0d51c1b99a33273d5e484eba9751b4b4e0c26b591",
        626_495,
    ),
    (
        "850x0g2.pdf",
        "4e4d65ce23bcb8c67521df270a491a8e0d6dbaa6902dde7e75be656f3d489ac7",
        1_060_872,
    ),
    (
        "851x0m1.pdf",
        "d8cb6aaef6f540991cc3b6dca9ff60af5c5f69b0d458b78cffa5f1436d1ed97a",
        328_280,
    ),
    (
        "852x0m1.pdf",
        "e4ac0205aa13a0a7f96dce8128377f7b3b0722d9a16c8a8280abf57f904e1bf3",
        305_819,
    ),
    (
        "853x0m1.pdf",
        "096e02ac8f4e40e712265573a8a605d4ac4bcc294a745490eccdfae91265cf00",
        273_130,
    ),
    (
        "854x0m1.pdf",
        "7faa67d0b7529fc2292f9f1ad5c768406848275f5d848a272c39993d1e987e54",
        277_233,
    ),
    (
        "855x0m1.pdf",
        "75108a20cec890346508feee9908ddaeccdf0e6ceaceac278f1c2887c2ca2ce1",
        268_343,
    ),
    (
        "870x0g1.pdf",
        "c83be7daf2a0ad6ea43d184a3da8059c2c9988267fc2a7fb722a3e267c5464ec",
        1_079_326,
    ),
    (
        "870x10y1.pdf",
        "d654919595dbf4b0bcb81164acbc23489311b8484d77c65c2223c92af2dd2a87",
        1_615_649,
    ),
    (
        "871x0m1s.pdf",
        "47e06d28b26944e7e802886d0b521e121f3dad40ab6a69ad1c9f68c9ed717d2e",
        1_408_123,
    ),
    (
        "871x1m1s.pdf",
        "ee554aea58cb3eee9c074a3a2f7b37937d265eb895b39fe370fe4f0c610ff8b0",
        869_040,
    ),
    (
        "871x2m1s.pdf",
        "6786651e6eb25f2f0c89cd1089ed8d160f2fa90169d8142c24d528657178fe74",
        849_517,
    ),
    (
        "871x3m1s.pdf",
        "44e3c26a980bc8569e1779fbcb4cfabbd5c5966196b561e91d7e2c6983c16758",
        872_600,
    ),
    (
        "872x0m1s.pdf",
        "25e762e5520c4f6bdee720547cabe577ff381683dc4d47a0d4947191e081a748",
        962_624,
    ),
    (
        "873x0m1s.pdf",
        "36c711c756c8526fe99cf8805a9c2d4e676f9d199135d21eba03836fe34bd67a",
        1_193_102,
    ),
    (
        "875x0m1s.pdf",
        "6eb9ebd50a2efe4b80e969f4db7b319e573951026704c2d63695fae5dc2ffe08",
        850_604,
    ),
    (
        "876x0b1.pdf",
        "c6be84699b667ac1130995adab42f8c10bca7d717133529a9d724886e9b63959",
        819_726,
    ),
    (
        "876x1m1.pdf",
        "022888ff166583fec682160523e2d028e2d163ab2d3be1b5ef61d0be9f72a098",
        704_697,
    ),
    (
        "880x0g1s.pdf",
        "18f95ed5d7a38be055cdd464a86164f4726094a22323d0cf5067213fede09705",
        5_991_513,
    ),
    (
        "880x0g2s.pdf",
        "51ec2be18a27d022662eb086fe0812e4e9d3bdbc083bc2ed3dd803a499c46434",
        6_855_731,
    ),
    (
        "880x0g3.pdf",
        "d6e544496b3b25c7e0db4f0737de3c6e7bec451f5ed0f15b01c347dacf7f4f59",
        5_974_172,
    ),
    (
        "881x0m1.pdf",
        "4d9a936ad4cc91466444024c7093a4f85533e6d4784a63a124ac4d39331d0a84",
        295_263,
    ),
    (
        "881x1b1.pdf",
        "aae6183fba3fa5789ba93dbc28c14815b6551d0f1165ffac9ef8cb6761a52d97",
        323_465,
    ),
    (
        "882x0m1.pdf",
        "b2890fa1548c17bb0548452aaa57c4a075bf4ddc1675790c21b35b3cc97e93fb",
        337_791,
    ),
    (
        "883x0b1s.pdf",
        "faaf944fa999bcf8c2c7e321a2b68fb1a70ef5dfaff36709498f245df307182a",
        1_986_697,
    ),
    (
        "883x0b2.pdf",
        "fb038b2fb1b8767cfa822e39bd47ef63ff19b22016b515b8144b890a22ca4546",
        1_571_435,
    ),
    (
        "901x0g1.pdf",
        "be608b41d89e53ffa4e32f6bfe52e81888ffc866f598cce63d89f6529371c586",
        4_663_271,
    ),
    (
        "901x1m1.pdf",
        "7ec60e1b64d56de766cf735d262bd12cac2d88f699712cfb7d9385afa9973906",
        1_492_720,
    ),
    (
        "901x31y1.pdf",
        "495539ba8b30b2d250605ed1824f5039f620a24926a085e8f411a68da216622f",
        1_213_328,
    ),
    (
        "901x3m1e1.pdf",
        "df043bc77b705d5191adb027723979b4aa97cf5ce10a92188642ed25f61ec74a",
        1_357_027,
    ),
    (
        "902x0g1.pdf",
        "12da4ed070d2c8bf52ef2d743ad0aac16f9e7eadf8a66f561d1907106a24bcb2",
        1_176_202,
    ),
    (
        "902x13m1e1c1.pdf",
        "2493efe35b1f1e4ca7d6b6f9020762f76f8366a2afaf0ae7c69cfd5480ef987d",
        658_906,
    ),
    (
        "902x13m1e1c1_tc2405.pdf",
        "d986b3a30b1d987ed09b4880a4343fbbd8e8273d42daf57bdff2ea8d6b037a14",
        267_659,
    ),
    (
        "902x1b1c1.pdf",
        "e3b57919afb592ec31d902a3dd104c439a230bd2109069295eb41c11c4a26f14",
        1_098_529,
    ),
    (
        "902x1b1c1_tc2145.pdf",
        "9468a81e528c627ce59cd0b7d8d87fac7e416752e9db3b55d05c008d5183ecf9",
        32_965,
    ),
    (
        "902x2b1.pdf",
        "ee5bbfbd027876447a2d3eaddfa74061a99037bcd65963132c477d85d21f1077",
        1_100_260,
    ),
    (
        "910x0g2s.pdf",
        "82b66633f0bd6f0ae879b7affad385dfb9dd012e3c054de2b72bcb7bf88354c7",
        766_289,
    ),
    (
        "910x0y1s.pdf",
        "e1c19abf3325e8ce67191353a146612ffc3c3b0055b8d6a9329b610532e9a62d",
        704_146,
    ),
    (
        "910x11b1s.pdf",
        "8be03a23d3021711f5c984e242c8270658a9f2e49b9ccbe10493fd501595620f",
        3_727_493,
    ),
    (
        "910x14g1s.pdf",
        "b72dc504acb44dd1d93efe1652b65ad8de546298e1b0bf746635b4d914c28671",
        2_519_649,
    ),
    (
        "910x2g1.pdf",
        "33003daa0ba4fc4efda7da6020c27c820ba47fb89d8db37ed659abe8917bb929",
        172_051,
    ),
    (
        "910x3g2s.pdf",
        "b92b4e03622045e18cecab38f1db063f94c2e473fd261bbbcd7eb017f65b9c34",
        1_273_761,
    ),
    (
        "910x3g3.pdf",
        "2e82705bc7ab322065597284c9d09324954637383335c8b7f47bbf2cf015a660",
        1_000_849,
    ),
    (
        "910x4b1s.pdf",
        "da16aa129dd8028d92e42ae1fcbf1c4ac3aacdee926bc30b560d6c0a06c139c3",
        662_487,
    ),
    (
        "910x4b2e1s.pdf",
        "38a211b048cb5a7e21968c9b8a5e8edfff721740068b703e2abfa060c25aab5d",
        976_648,
    ),
    (
        "911x1b1s.pdf",
        "79ffe91cce9a4223f11cb82cb571cda34d06ce7fdcaea3ba9264a0dcdd20b47e",
        767_815,
    ),
    (
        "911x1b2s.pdf",
        "af3ce02d3ac1be17f125dd69ac5d9b512fca3eb11e0838cfa4ad5e67aef5448a",
        756_689,
    ),
    (
        "911x1b3ec1s.pdf",
        "6916a0ad3aec6080a21504016ee0ac4d73d95b97f68cbb77c76348011d159ba3",
        1_128_256,
    ),
    (
        "911x1b4s.pdf",
        "c6c143e397c0f52713530bc5398fedb92871e1f24c94db36efceb4ebbf203d29",
        1_135_733,
    ),
    (
        "911x1b5e1.pdf",
        "593a15b7057d861285585d11516eaa7384972fa9616ea3d982d468a216074c41",
        954_686,
    ),
    (
        "911x1r1x7.pdf",
        "86a9cab91c2f61384b0e9764b9adb1b594cb416495f7f0c977dda2f430edbf2b",
        746_636,
    ),
    (
        "911x2b1s.pdf",
        "aaad842b612874b2bbb92ab9a32014b205f5b25c6b10110bd036e1e90a009b2b",
        777_105,
    ),
    (
        "911x2b2ec1s.pdf",
        "3d0048eafad04ecb5d85b1bd58923f916208d24ec1a795ad1ef2299e67412728",
        1_158_685,
    ),
    (
        "911x2b3s.pdf",
        "e64442512884205aa891c8bcac6183cb40f3c6acaa8d76227e62d31c49bdb13c",
        1_165_245,
    ),
    (
        "911x2b4e1.pdf",
        "646ee96f3b1e525ed5564b7544dc24d5682634ab8c751ec44d11b94d6a119fc7",
        990_074,
    ),
    (
        "911x2r1x7.pdf",
        "c85f6d60d8c7550b752cdd9cfe5121db2c91bf87edc73f13088bbb4b1bcec07d",
        743_233,
    ),
    (
        "911x5b1s.pdf",
        "63e28519b6014ef977fa3cd352f7be49e78d91e4b4b5e04e4dcbac635d4873fa",
        804_076,
    ),
    (
        "911x5b2ec1s.pdf",
        "1f61a5855927418dec3373640f031ebc174a0a9368ac85c6f34b1a86df5a96ee",
        1_174_044,
    ),
    (
        "911x5b3s.pdf",
        "4cfe2dbf7814b5924bee8015c9b96cb427e5bd75c9e4a0000a59c50590eed2ff",
        1_178_091,
    ),
    (
        "911x5b4e1.pdf",
        "1bc3da87af5539a50213d76c4da95ca357c771448fd24b8cf895d43dceaa2e37",
        1_051_707,
    ),
    (
        "912X1R1.99h.pdf",
        "ff3a330bdd0c1bc5a041308ecaabefbc21775c9dffb0490b36258daaf012d43a",
        762_955,
    ),
    (
        "912x11o1.pdf",
        "01849867501a19ab60886997f3dd2dfdd04ac4690f1baf7267a39bf85cb3cb59",
        729_578,
    ),
    (
        "912x1b1s.pdf",
        "4e7a65c4b851829a5aa8b916daa5e756ab3da01d113c3f397a017859bbfa2597",
        785_905,
    ),
    (
        "912x1b2c1s.pdf",
        "9f11f0c6cf3657c16107e417d5da4d25c252602ece9ee75dac84704d3ec318c1",
        639_337,
    ),
    (
        "912x1b2c1s_tca710.pdf",
        "90c75697d9551b5c360618996e690afcaf546637b3f04b8ffa8029f0ccfd1b31",
        17_505,
    ),
    (
        "912x1b3ec1s.pdf",
        "9e045575f7305b5d3629c371a97a91d8703d868a9d12d502a69665278068747d",
        1_160_012,
    ),
    (
        "912x1b4s.pdf",
        "ca3e8c6a3a1c12e3d2719ef78d5891ed1fdbfd52cf806c335b8f9507a791f8b6",
        1_145_170,
    ),
    (
        "912x1b5e1.pdf",
        "6d98ce88a53dc3e3744335cf43036c73f010b10f3c5b211d67a2e63d59ef34d0",
        990_238,
    ),
    (
        "912x3b1s.pdf",
        "a718eda214a29a3c9fcedb3fcc3aa643b63f2595d1cae6a16b32df81264e2ebe",
        948_702,
    ),
    (
        "912x3b2ec1s.pdf",
        "fd493902160506f56705f42678167b505c4b8def9b31c3353c113b43822757d2",
        1_330_437,
    ),
    (
        "912x3b3s.pdf",
        "dd1feedff5f74eb0b4985c40d9990010b43b70ed91f11adc2db735112ccf1d1c",
        1_350_880,
    ),
    (
        "913x1b1s.pdf",
        "9a4372ba2cbabe18568063badf88df0a926ca8ff64287dd2f7be8d5c8a17544f",
        812_463,
    ),
    (
        "913x1b2.pdf",
        "068807b8eab03124262e8ccde284771b760fb45dc58534da07da0b77fae10ee6",
        334_916,
    ),
    (
        "914x0m1ec1s.pdf",
        "3ab35377fcf1348b3a19114d072c9a18b94e0ed922136e4b3574a0c9777ac7c5",
        1_936_581,
    ),
    (
        "914x0m2c1.pdf",
        "0c3abe66439f96ca84cdbc1d7a85ca52b0436de98e237846e1b065c50ca39907",
        1_215_515,
    ),
    (
        "914x0m2c1_tc1615.pdf",
        "904f1939eaccfdd8ab778b7d394fdf90db7671d011ea2f70ff1d22aa07379171",
        22_152,
    ),
    (
        "914x1g1.pdf",
        "51bf88b8af23a561675fb525fdd91af03f9be7325b604f3620dc23db8351af89",
        432_719,
    ),
    (
        "914x2g1s.pdf",
        "bdc39beb6192d60bcc53f57406350ce6ee0e01bcf6e6e286643b3e3f7432e441",
        525_897,
    ),
    (
        "914x2g2.pdf",
        "2189b9d40d77fae7892524294e665ae9be218f6af8a3ecc00dd31238bca444f8",
        637_014,
    ),
    (
        "915x1m1s.pdf",
        "52c81f753bd01ff57f20a723453a7581ca072b43d6646d65cc2c558c3caec753",
        994_107,
    ),
    (
        "915x1m2.pdf",
        "ddfe28e981f1887fe618984ed915193b49c45510fca4bd0bbdc80a5788efcd20",
        289_512,
    ),
    (
        "915x2m1s.pdf",
        "eb5552d3381a2142c1763ea365d52aebc2df3f30d925f1ef56a31f8cf30f9fa0",
        745_686,
    ),
    (
        "915x2m2.pdf",
        "f5e5808ba3f7e47209d674e80303701710e15fee2449b8d32d61f71b3078f829",
        293_733,
    ),
    (
        "915x5m1s.pdf",
        "f264c025cb229a6c7e73594b029b967b6e925dee86939ec7a4ef7501d628e14c",
        775_108,
    ),
    (
        "915x5m2.pdf",
        "ae2e314d4f88a915b9d91bfcc9959ba0a8d11f0807bd2dc70ac8e4a8214cb4fa",
        312_218,
    ),
    (
        "916x1m1s.pdf",
        "e27938e6c4d8cd5207c594d9307e033db14a61aa4809ca01d34a60c53f887334",
        859_308,
    ),
    (
        "916x1m2.pdf",
        "5faf0503dda088579307d01b1953e58c29d6b78ead048984ddc59afe62fab5fc",
        418_443,
    ),
    (
        "916x3m1s.pdf",
        "a056abef43f412af4cac081f6f0d9c5420707d0d53bec5e1266570c17dece903",
        1_030_966,
    ),
    (
        "916x3m2.pdf",
        "e565b47bcf8e4a7fbf1b648230c5fe7e3d89a9ea8a8887be7132ecd82dd651c5",
        473_163,
    ),
    (
        "920x0g1.pdf",
        "385e6586283fc6686f8c2ada24bea47037b57a14f64f416e20d1e0accccc74c4",
        896_877,
    ),
    (
        "921x1b1s.pdf",
        "2634b6371acf212c9438fab45f979cdcad06b6f41f6547d59146deeaae4a14ae",
        1_939_415,
    ),
    (
        "921x1b2e1.pdf",
        "516d015cfa349a2304238a496f4d2305fcfe5e8d299621457d6d6af9d5cf2f7c",
        2_619_050,
    ),
    (
        "921x2m1e1c1.pdf",
        "e9a9de1be5dc749d8bb25fc22c9c91903d8d9b6c91ca2d31b6e78b14b09f5ccd",
        1_128_912,
    ),
    (
        "921x2m1e1c1_tc2022.pdf",
        "624e0acad95e9d3112cea293685f84506a847d82837c4ee9c92a2931b1941776",
        22_054,
    ),
    (
        "922x1b1s.pdf",
        "ad60a697752a638c9a58a28c9b707110dbcd887340ed2a0f1955daa5e95979e9",
        1_123_703,
    ),
    (
        "922x1b2.pdf",
        "8489371d015cc66298e20d6f6b1c497dcc78ab10bb9ed0d086f90ca87c750500",
        990_575,
    ),
    (
        "922x27o1.pdf",
        "bcedcc4dbc3ffc9ef0974a434ecad01653314a8cce2066225a0b6d17d3dded96",
        816_847,
    ),
    (
        "922x2b1s.pdf",
        "67f5fa5922170d287c149860a550f8b82ce0d2aa1a91275062751596a409bbbf",
        1_534_879,
    ),
    (
        "922x2b2.pdf",
        "b4d0e443ddaefe8690738662078d1499be7be4b91380e631dbc791195bca1d1e",
        964_444,
    ),
    (
        "922x3b1.pdf",
        "70de9d7ea0d7d3e1f0e974e4a2a51ea9d40dfa1a7d5e93f26d30a013f54f259d",
        1_686_364,
    ),
    (
        "927x1b1e1.pdf",
        "d4656953594ddbb3c076a355d2a940460d8e0b10cf2710c35433593d303e0ac5",
        1_112_722,
    ),
];

/// Where a CCSDS publication lives on the public server.
const CCSDS_PUBS: &str = "https://public.ccsds.org/Pubs/";

/// The paper corpus: the documents a test of the knowledge base reads, and a real library to search.
///
/// This is not data the engine computes from — a wrong paper cannot corrupt an orbit — it is the
/// corpus the PDF reader and the search are measured against, and the one a person can point
/// `STKCRAFT_KNOWLEDGE` at to see the feature work. The checksums still matter: a citation into a
/// document that is not the document the release documented is a citation nobody can check.
const PAPER_FILES: [(&str, &str, &str); 6] = [
    (
        "ccsds-502.0-b-3-odm.pdf",
        "https://public.ccsds.org/Pubs/502x0b3e1.pdf",
        "97d1687b4a461ffdf965b9b65923820cc6aa9d1d1c4d4df77a598a53d4a49d81",
    ),
    (
        "ccsds-503.0-b-2-tdm.pdf",
        "https://public.ccsds.org/Pubs/503x0b2c1.pdf",
        "6bbe6169eccb3c6b744ba002f02785e1861f8531d3cd4dcf6fa9056efe515677",
    ),
    (
        "ccsds-504.0-b-2-adm.pdf",
        "https://public.ccsds.org/Pubs/504x0b2.pdf",
        "0869187cb326dbf009e6f17a1fd6e920b7b4a4f8f6446706b314e2d9a6121214",
    ),
    (
        "ccsds-508.0-b-1-cdm.pdf",
        "https://public.ccsds.org/Pubs/508x0b1e2c2.pdf",
        "6afd6dad4b5f6c2ce17c3b310fdbda62bbfa6f248c32decea7de6f9116276978",
    ),
    (
        "arxiv-2402.04830-differentiable-sgp4.pdf",
        "https://arxiv.org/pdf/2402.04830v5",
        "391fbf4e8cc97e510c86a113e2d171c83992a67e4da422068bf752d3eb1b9dcb",
    ),
    (
        "arxiv-2605.19850-starlink-tle-trust.pdf",
        "https://arxiv.org/pdf/2605.19850v1",
        "610d3caab852ce224eb41c1b1b3a25574271006414990ddacf51453f8de26420",
    ),
];

/// How many ranges a large file is split into by default.
///
/// Eight is where a satellite link stops being the limit and the server starts being one; the
/// number is a parameter because a deployment may know better than this constant does.
const DOWNLOAD_JOBS: usize = 8;

/// The largest job count accepted: past this a download is hammering one server for no gain.
const MAX_DOWNLOAD_JOBS: usize = 32;

/// Files below this size are fetched in one request: the bookkeeping would cost more than it saves.
const PARALLEL_THRESHOLD_BYTES: u64 = 16 * 1024 * 1024;

/// A byte count a person can read: megabytes below a gigabyte, gigabytes above.
fn human_size(bytes: u64) -> String {
    let gb = 1e9_f64;
    let mb = 1e6_f64;
    let size = bytes as f64;
    if size >= gb {
        format!("{:.1} GB", size / gb)
    } else {
        format!("{:.1} MB", size / mb)
    }
}

/// Fetch one file, in parallel ranges when it is large enough to be worth it.
fn fetch_file(name: &str, url: &str, path: &Path, jobs: usize) -> Result<(), String> {
    let size = remote_size(url)?;
    match size {
        Some(size) if size >= PARALLEL_THRESHOLD_BYTES && jobs > 1 => {
            println!(
                "{name}: fetching {url} in {jobs} parallel ranges ({})",
                human_size(size)
            );
            download_ranges(name, url, path, size, jobs)
        }
        _ => {
            println!("{name}: fetching {url}");
            run_curl(&["-sS", "--fail", "-L"], Some(path), url, None)
        }
    }
}

/// The remote file's size from its `Content-Length`, if the server says.
fn remote_size(url: &str) -> Result<Option<u64>, String> {
    let output = std::process::Command::new("curl")
        .args(["-sSIL", url])
        .output()
        .map_err(|error| format!("could not run curl: {error}"))?;
    if !output.status.success() {
        return Ok(None);
    }
    let text = String::from_utf8_lossy(&output.stdout);
    // The last `content-length` in a redirect chain belongs to the final response.
    let size = text
        .lines()
        .filter_map(|line| {
            let lower = line.to_ascii_lowercase();
            lower
                .strip_prefix("content-length:")
                .and_then(|rest| rest.trim().parse::<u64>().ok())
        })
        .next_back();
    Ok(size)
}

/// Download a file as `jobs` byte ranges at once, then join the pieces.
///
/// Each range is its own `curl` process on its own thread: `curl` already knows how to resume, to
/// follow redirects and to fail loudly, and the alternative — an HTTP client in the build tool —
/// would be a dependency for one command. The pieces are checked for length before they are joined,
/// so a truncated range is an error rather than a short file.
fn download_ranges(
    name: &str,
    url: &str,
    path: &Path,
    size: u64,
    jobs: usize,
) -> Result<(), String> {
    let chunk = size.div_ceil(u64::try_from(jobs).unwrap_or(1));
    let mut handles = Vec::new();
    for index in 0..jobs {
        let start = chunk * u64::try_from(index).unwrap_or(0);
        if start >= size {
            break;
        }
        let end = (start + chunk).min(size) - 1;
        let part = part_path(path, index);
        let url = url.to_owned();
        handles.push(std::thread::spawn(move || -> Result<(), String> {
            run_curl(
                &["-sS", "--fail", "-L", "-r"],
                Some(&part),
                &url,
                Some((start, end)),
            )?;
            let expected = end - start + 1;
            let actual = fs::metadata(&part)
                .map_err(|error| format!("range {index}: {error}"))?
                .len();
            if actual != expected {
                return Err(format!(
                    "range {index}: got {actual} bytes, expected {expected}"
                ));
            }
            Ok(())
        }));
    }
    let mut failure = None;
    for handle in handles {
        match handle.join() {
            Ok(Ok(())) => {}
            Ok(Err(error)) => failure = Some(failure.unwrap_or(error)),
            Err(_) => failure = Some("a download thread panicked".to_owned()),
        }
    }
    if let Some(error) = failure {
        // Leave no half-written target behind: a stale file would be caught by the checksum, but
        // only after the next run has tried to use it.
        let _ = fs::remove_file(path);
        return Err(format!("{name}: {error}"));
    }
    let target = fs::File::create(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut writer = std::io::BufWriter::new(target);
    for index in 0..jobs {
        let part = part_path(path, index);
        if !part.exists() {
            break;
        }
        let mut reader =
            fs::File::open(&part).map_err(|error| format!("{}: {error}", part.display()))?;
        std::io::copy(&mut reader, &mut writer)
            .map_err(|error| format!("{name}: could not join the pieces: {error}"))?;
        let _ = fs::remove_file(&part);
    }
    std::io::Write::flush(&mut writer).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(())
}

/// Where one range lands while it is being fetched.
fn part_path(path: &Path, index: usize) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".part{index}"));
    path.with_file_name(name)
}

/// Run `curl` with an output file and, for a ranged fetch, the byte range.
fn run_curl(
    flags: &[&str],
    output: Option<&Path>,
    url: &str,
    range: Option<(u64, u64)>,
) -> Result<(), String> {
    // Built as an explicit argument list rather than a chain of `arg` calls: the order of
    // `-r <range>` and `-o <file>` decides whether curl writes to the file or to stdout, and that
    // is exactly the mistake this function made once.
    let mut arguments: Vec<String> = flags.iter().map(|flag| (*flag).to_owned()).collect();
    if let Some((start, end)) = range {
        arguments.push(format!("{start}-{end}"));
    }
    if let Some(output) = output {
        arguments.push("-o".to_owned());
        arguments.push(output.display().to_string());
    }
    arguments.push(url.to_owned());
    let status = std::process::Command::new("curl")
        .args(&arguments)
        .status()
        .map_err(|error| format!("could not run curl: {error}"))?;
    if !status.success() {
        return Err(format!("curl exited with {status}"));
    }
    Ok(())
}

/// The lower-case hex SHA-256 of an ordinary file, streamed with bounded memory.
fn sha256_of(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    use std::fmt::Write as _;
    use std::io::Read;

    let metadata = fs::metadata(path)
        .map_err(|error| format!("could not inspect {}: {error}", path.display()))?;
    if !metadata.is_file() {
        return Err(format!("not a regular file: {}", path.display()));
    }
    let mut file = fs::File::open(path)
        .map_err(|error| format!("could not open {}: {error}", path.display()))?;
    if !file
        .metadata()
        .map_err(|error| format!("could not inspect {}: {error}", path.display()))?
        .is_file()
    {
        return Err(format!("not a regular file: {}", path.display()));
    }
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let count = match file.read(&mut buffer) {
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(format!("could not read {}: {error}", path.display())),
        };
        if count == 0 {
            break;
        }
        let bytes = buffer
            .get(..count)
            .ok_or_else(|| format!("invalid read length for {}", path.display()))?;
        hasher.update(bytes);
    }
    let mut hex = String::with_capacity(64);
    for byte in hasher.finalize() {
        write!(&mut hex, "{byte:02x}")
            .map_err(|error| format!("could not format SHA-256: {error}"))?;
    }
    Ok(hex)
}

/// Run `git` in the workspace root and fail with its output if it does.
fn run_git(root: &Path, args: &[&str]) -> Result<(), String> {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|error| format!("could not run git: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    Err(format!(
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr).trim()
    ))
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
    fn only_a_stable_version_gets_a_github_release() {
        // The 0.x series is a development record: tag it, do not publish a Release.
        assert!(!is_stable_version("0.1.88"));
        assert!(!is_stable_version("1.0.0-rc.1"));
        assert!(!is_stable_version("1.0.0-alpha"));
        // A stable version may carry build metadata; that does not make it a pre-release.
        assert!(is_stable_version("1.0.0"));
        assert!(is_stable_version("1.2.3+build.7"));
    }

    #[test]
    fn attribution_rows_are_read_as_paths_not_names() {
        let table = "| Path | Title |\n|---|---|\n\
                     | `assets/app-icon/albatross.png` | mark |\n\
                     | `docs/images/viewport.png` | shot |\n";
        assert_eq!(
            attribution_paths(table),
            ["assets/app-icon/albatross.png", "docs/images/viewport.png"]
        );
    }

    #[test]
    fn gitignore_directory_entries_are_read_in_every_spelling() {
        // Git treats `/plan`, `plan/` and `/plan/` alike; the asset gate must too, or the local
        // notebook `AGENTS.md` describes fails the release check.
        let names = ignored_directory_names(
            "# a comment\n/target\nplan/\n/data/\n/craft-fonts\n*.bsp\n/dist\n/sub/dir\n",
        );
        assert_eq!(names, ["target", "plan", "data", "craft-fonts", "dist"]);
    }

    #[test]
    fn sha256_handles_spaces_and_unicode_paths() -> Result<(), String> {
        let directory =
            std::env::temp_dir().join(format!("stkcraft sha256 星 {}", std::process::id()));
        fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        let path = directory.join("known bytes 星.txt");
        fs::write(&path, b"abc").map_err(|error| error.to_string())?;
        let actual = sha256_of(&path);
        fs::remove_dir_all(&directory).map_err(|error| error.to_string())?;
        // NIST FIPS 180-4 SHA-256 example message "abc".
        assert_eq!(
            actual.as_deref(),
            Ok("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        );
        Ok(())
    }

    #[test]
    fn sha256_matches_empty_and_multi_buffer_vectors() -> Result<(), String> {
        let directory =
            std::env::temp_dir().join(format!("stkcraft-hash-vectors-{}", std::process::id()));
        fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        let empty = directory.join("empty");
        let million_a = directory.join("million-a");
        fs::write(&empty, b"").map_err(|error| error.to_string())?;
        fs::write(&million_a, vec![b'a'; 1_000_000]).map_err(|error| error.to_string())?;
        let empty_hash = sha256_of(&empty);
        let million_a_hash = sha256_of(&million_a);
        fs::remove_dir_all(&directory).map_err(|error| error.to_string())?;
        // SHA-256 standard vectors: empty message and NIST's one million ASCII 'a' bytes.
        assert_eq!(
            empty_hash.as_deref(),
            Ok("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
        );
        assert_eq!(
            million_a_hash.as_deref(),
            Ok("cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0")
        );
        Ok(())
    }

    #[test]
    fn sha256_rejects_missing_files_and_directories() -> Result<(), String> {
        let directory =
            std::env::temp_dir().join(format!("stkcraft-hash-invalid-{}", std::process::id()));
        fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        let missing = sha256_of(&directory.join("missing"));
        let not_file = sha256_of(&directory);
        fs::remove_dir_all(&directory).map_err(|error| error.to_string())?;
        assert!(matches!(missing, Err(error) if error.contains("could not inspect")));
        assert!(matches!(not_file, Err(error) if error.contains("not a regular file")));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn sha256_rejects_devices_before_reading() {
        assert!(matches!(
            sha256_of(Path::new("/dev/zero")),
            Err(error) if error.contains("not a regular file")
        ));
    }

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

    #[test]
    fn every_crate_name_repeats_its_directory() {
        // The table is the check, so the table itself has to satisfy it: a spec whose name does not
        // repeat its path is a name a reader cannot trace back to a file.
        for spec in LAYOUT {
            let Some(expected) = layers::implied_name(spec.path) else {
                continue;
            };
            assert_eq!(
                expected, spec.name,
                "{} sits in `{}`, which implies `{expected}`",
                spec.name, spec.path
            );
        }
    }

    #[test]
    fn a_group_crate_is_named_for_its_group_including_the_platform() {
        // The two halves of the rule: a grouped crate spells the group out, and the platform — the
        // part a sibling app shares — does not.
        assert_eq!(
            layers::implied_name("crates/astro/body").as_deref(),
            Some("stkcraft-astro-body")
        );
        assert_eq!(
            layers::implied_name("crates/agent/runtime").as_deref(),
            Some("stkcraft-agent-runtime")
        );
        assert_eq!(
            layers::implied_name("crates/ui/viewport").as_deref(),
            Some("stkcraft-ui-viewport")
        );
        assert_eq!(
            layers::implied_name("crates/framework/time").as_deref(),
            Some("stkcraft-framework-time")
        );
        // Paths outside `crates/` are named by their own rule, and a path that is not a crate at all
        // yields nothing rather than a guess.
        assert_eq!(layers::implied_name("apps/stkcraft"), None);
        assert_eq!(layers::implied_name("xtask"), None);
        assert_eq!(layers::implied_name("crates/astro"), None);
        assert_eq!(layers::implied_name("crates/astro/body/deep"), None);
        assert_eq!(layers::implied_name(""), None);
        // The control case: the disagreement this check exists to catch. A crate under `crates/astro/`
        // named for its subject alone is what `stkcraft-body` was before 0.1.37.
        assert_ne!(
            layers::implied_name("crates/astro/body").as_deref(),
            Some("stkcraft-body")
        );
    }
}
