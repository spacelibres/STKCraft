//! Embeds the interface catalogs from `locales/` into the binary.
//!
//! The catalogs are the build input the shared format describes: `locales/<locale>/stkcraft.tsv`.
//! They are *optional* — a checkout with no `locales/` builds and runs in English — so the text is
//! included through a generated module rather than `include_str!` of a file that may not exist.
//!
//! Re-running is driven by `cargo:rerun-if-changed=../../locales`, so editing a catalog rebuilds
//! the crate that embeds it and a translated string cannot sit stale in a binary.
//!
//! It also extracts **every interface string the window can show** and embeds the list. A window
//! that can translate itself has to know what to translate, and the only honest answer comes from
//! the same extractor `cargo xtask i18n` uses, run at build time over this crate's own sources.
//! That is what lets a person pick a language the project does not ship and have their own key fill
//! it in, without a checkout and without cargo.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_owned());
    let locales = Path::new(&manifest).join("../../locales");
    println!("cargo:rerun-if-changed={}", locales.display());

    let mut entries: Vec<(String, String)> = Vec::new();
    if let Ok(directories) = std::fs::read_dir(&locales) {
        for directory in directories.flatten() {
            if !directory.path().is_dir() {
                continue;
            }
            let locale = directory.file_name().to_string_lossy().into_owned();
            if locale.starts_with('.') {
                continue;
            }
            let catalog = directory.path().join("stkcraft.tsv");
            if catalog.is_file() {
                entries.push((locale, catalog.display().to_string()));
            }
        }
    }
    entries.sort();

    let mut source = String::from("/// The catalogs this build embeds, `(locale, text)`.\n");
    source.push_str("pub const EMBEDDED: stkcraft_i18n::Embedded = &[\n");
    for (locale, path) in &entries {
        let _ = writeln!(source, "    ({locale:?}, include_str!({path:?})),");
    }
    source.push_str("];\n");

    let sources = interface_sources(Path::new(&manifest).join("src").as_path());
    println!(
        "cargo:rerun-if-changed={}",
        Path::new(&manifest).join("src").display()
    );
    source.push_str(
        "\n/// Every interface string this window can show, as `(context, english)`.\n\
         pub const SOURCES: &[(&str, &str)] = &[\n",
    );
    for (context, text) in &sources {
        let _ = writeln!(source, "    ({context:?}, {text:?}),");
    }
    source.push_str("];\n");

    let out =
        Path::new(&std::env::var("OUT_DIR").unwrap_or_else(|_| ".".to_owned())).join("catalogs.rs");
    if let Err(error) = std::fs::write(&out, source) {
        // A build script that cannot write is a broken build, and saying so is better than a
        // binary that silently shipped English.
        println!("cargo:warning=could not write the catalog module: {error}");
    }
}

/// Every interface string in this crate's sources, deduplicated and sorted.
///
/// The extractor is the one the i18n crate owns, so the build and `cargo xtask i18n` cannot disagree
/// about which strings exist — which is the only way a window's self-translation and a catalog a
/// translator wrote describe the same interface.
fn interface_sources(directory: &Path) -> Vec<(String, String)> {
    let mut files = Vec::new();
    collect(directory, &mut files);
    let mut sources: Vec<(String, String)> = Vec::new();
    for file in files {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        for source in stkcraft_i18n::extract::sources(&text) {
            let entry = (source.context, source.text);
            if !sources.contains(&entry) {
                sources.push(entry);
            }
        }
    }
    sources.sort();
    sources
}

/// Collect `.rs` files under a directory.
fn collect(directory: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}
