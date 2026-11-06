//! `stkcraft extensions`: what is installed, and what the contract makes of it.
//!
//! A package is a directory under the configuration directory's `extensions/`, named by its id,
//! holding `extension.toml`. This command reads each one with [`stkcraft_framework_extensions::Manifest`] and
//! reports one of three things per package: admitted, refused with the reason, or malformed with the
//! reason.
//!
//! It is a listing and nothing more. Installing, unpacking and running a package is not built, and
//! this command exists so the contract has a real consumer instead of living only in tests: a
//! validator with no user is a validator nobody has run.

use stkcraft_framework_extensions::{Manifest, Refusal, TrustTier, third_party_enabled};

/// The file a package's manifest lives in.
pub const MANIFEST_FILE: &str = "extension.toml";

/// One package found on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// The directory name, which is what the store will use as the package's identity if the
    /// manifest cannot be read.
    pub directory: String,
    /// The manifest, when it parsed and validated.
    pub admitted: Option<Manifest>,
    /// Why it was not admitted, when it was not.
    pub refused: Option<String>,
}

/// Read every package in a directory.
///
/// A missing directory is no packages rather than an error: nothing installed is the normal state of
/// a fresh checkout.
#[must_use]
pub fn scan(directory: &std::path::Path) -> Vec<Found> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(directory) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let directory = entry.file_name().to_string_lossy().into_owned();
        if directory.starts_with('.') {
            continue;
        }
        let manifest_path = path.join(MANIFEST_FILE);
        let Ok(text) = std::fs::read_to_string(&manifest_path) else {
            found.push(Found {
                directory,
                admitted: None,
                refused: Some(format!("no {MANIFEST_FILE} in the package directory")),
            });
            continue;
        };
        match Manifest::parse(&text) {
            Ok(manifest) => {
                // The tier comes from where the package is, not from what it says: everything in
                // this directory was put there by the person using the app.
                let refused = manifest
                    .admit(TrustTier::Internal)
                    .err()
                    .map(|refusal: Refusal| refusal.to_string());
                found.push(Found {
                    directory,
                    admitted: refused.is_none().then_some(manifest),
                    refused,
                });
            }
            Err(refusal) => found.push(Found {
                directory,
                admitted: None,
                refused: Some(refusal.to_string()),
            }),
        }
    }
    found.sort_by(|a, b| a.directory.cmp(&b.directory));
    found
}

/// Run the command.
///
/// # Errors
///
/// A message when the listing cannot be produced. Refused packages are *reported*, not fatal: a
/// broken package on disk must not stop the app from telling you about the good ones.
pub fn list() -> Result<(), String> {
    let Some(directory) = crate::prefs::extensions_dir() else {
        return Err("this platform has no configuration directory to look in".to_owned());
    };
    let found = scan(&directory);
    println!(
        "STKCraft {} — extensions in {}",
        env!("CARGO_PKG_VERSION"),
        directory.display()
    );
    if found.is_empty() {
        println!("  none installed");
    }
    for package in &found {
        match (&package.admitted, &package.refused) {
            (Some(manifest), _) => println!(
                "  {} {} ({}) — {}",
                manifest.id,
                manifest.version,
                manifest.kind.as_str(),
                manifest.name
            ),
            (None, Some(reason)) => println!("  {} — refused: {reason}", package.directory),
            (None, None) => println!("  {} — refused", package.directory),
        }
    }
    if !third_party_enabled() {
        println!(
            "  third-party packages: off in this build. A package outside the Craft organisation is\n\
             \x20 refused; no release build enables them."
        );
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, clippy::expect_used)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let directory = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("a scratch directory");
        directory
    }

    #[test]
    fn a_package_directory_is_read_and_admitted() {
        let directory = scratch("stkcraft-extensions-scan");
        let package = directory.join("org.example.aurora");
        std::fs::create_dir_all(&package).expect("the package directory");
        std::fs::write(
            package.join(MANIFEST_FILE),
            "id = \"org.example.aurora\"\nname = \"Aurora\"\nversion = \"1.0.0\"\n\
             kind = \"theme\"\napp = \"stkcraft\"\n",
        )
        .expect("the manifest");
        let found = scan(&directory);
        assert_eq!(found.len(), 1);
        let manifest = found[0].admitted.as_ref().expect("an admitted package");
        assert_eq!(manifest.name, "Aurora");
        assert_eq!(found[0].refused, None);
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_bad_package_is_reported_and_does_not_hide_the_good_one() {
        let directory = scratch("stkcraft-extensions-scan-mixed");
        std::fs::create_dir_all(directory.join("org.example.good")).expect("a directory");
        std::fs::write(
            directory.join("org.example.good").join(MANIFEST_FILE),
            "id = \"org.example.good\"\nname = \"Good\"\nversion = \"2.0.0\"\nkind = \"skills\"\n\
             app = \"stkcraft\"\n",
        )
        .expect("the manifest");
        std::fs::create_dir_all(directory.join("org.example.bad")).expect("a directory");
        std::fs::write(
            directory.join("org.example.bad").join(MANIFEST_FILE),
            "id = \"org.example.bad\"\nname = \"Bad\"\nversion = \"1.0\"\nkind = \"theme\"\n\
             app = \"stkcraft\"\n",
        )
        .expect("the manifest");
        std::fs::create_dir_all(directory.join("empty")).expect("a directory");

        let found = scan(&directory);
        assert_eq!(found.len(), 3);
        let good = found
            .iter()
            .find(|package| package.directory == "org.example.good")
            .expect("the good package");
        assert!(good.admitted.is_some());
        let bad = found
            .iter()
            .find(|package| package.directory == "org.example.bad")
            .expect("the bad package");
        assert!(
            bad.refused
                .as_deref()
                .is_some_and(|reason| reason.contains("version"))
        );
        let empty = found
            .iter()
            .find(|package| package.directory == "empty")
            .expect("the empty package");
        assert!(
            empty
                .refused
                .as_deref()
                .is_some_and(|reason| reason.contains(MANIFEST_FILE))
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_missing_directory_is_no_packages() {
        assert_eq!(
            scan(std::path::Path::new("/nonexistent/stkcraft/extensions")),
            Vec::new()
        );
    }
}
