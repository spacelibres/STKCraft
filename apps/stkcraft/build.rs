//! Embeds the interface catalogs from `locales/` into the binary.
//!
//! The catalogs are the build input the shared format describes: `locales/<locale>/stkcraft.tsv`.
//! They are *optional* — a checkout with no `locales/` builds and runs in English — so the text is
//! included through a generated module rather than `include_str!` of a file that may not exist.
//!
//! Re-running is driven by `cargo:rerun-if-changed=../../locales`, so editing a catalog rebuilds
//! the crate that embeds it and a translated string cannot sit stale in a binary.

use std::fmt::Write as _;
use std::path::Path;

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

    let out =
        Path::new(&std::env::var("OUT_DIR").unwrap_or_else(|_| ".".to_owned())).join("catalogs.rs");
    if let Err(error) = std::fs::write(&out, source) {
        // A build script that cannot write is a broken build, and saying so is better than a
        // binary that silently shipped English.
        println!("cargo:warning=could not write the catalog module: {error}");
    }
}
