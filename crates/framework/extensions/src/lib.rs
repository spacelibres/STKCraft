//! The extension package contract: what a theme, a plug-in or a skill pack may be.
//!
//! `STKCraft` follows the `ArtCraft` family's extension model (`storytold/craft-extensions`), which is
//! provisional upstream and deliberately narrow. This crate is the part of it that is pure data and
//! validation: a package manifest, the rules an id and a path must satisfy, and the trust tier a
//! package is assigned from *where it came from*. It reads no file it was not handed, unpacks no
//! archive and touches no network — an unpacker is a separate, riskier thing.
//!
//! # Third-party is designed in and switched off
//!
//! [`TrustTier::ThirdParty`] exists, and [`Manifest::admit`] refuses it unless this crate was built
//! with the `third-party` feature. No release build enables that feature, and the test below fails if
//! it ever becomes a default, because opening the door is a decision the maintainers make after the
//! gates in the family's threat model pass — not a side effect of a cargo dependency.
//!
//! # What a package may say about itself
//!
//! Nothing that matters. A manifest states its id, name, version, kind and what it targets; **it never
//! states its own trust level**, because a package claiming to be trusted is exactly what an attacker
//! would write. The host assigns the tier from where the package was found.
//!
//! # What a package may contain
//!
//! - **A theme carries no code**, so a theme manifest that names an entry point is refused.
//! - **A plug-in gets no capability in version 1**: no files, no network, no clock. Its permission
//!   list must be empty, and a manifest that asks for one is refused rather than silently ignored.
//!
//! ```toml
//! id = "org.example.aurora"
//! name = "Aurora"
//! version = "1.0.0"
//! kind = "theme"          # theme | plugin | skills
//! app = "stkcraft"
//! description = "A dark palette with cold highlights."
//! ```
//!
//! # What this crate is not
//!
//! It is not the store, the installer, the unpacker or the signature check. Those are the family's
//! `craft-ext-store` and they are not built here; until they are, a package on disk is data a person
//! put there, and [`Manifest::parse`] is what decides whether it is well-formed.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
#![forbid(unsafe_code)]

use serde::Deserialize;

/// The longest id a package may have, in bytes.
pub const MAX_ID_LEN: usize = 128;

/// The longest name a package may have, in characters.
pub const MAX_NAME_CHARS: usize = 64;

/// The application id this crate's packages target.
pub const APP: &str = "stkcraft";

/// What a package is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A palette and nothing else: no code, no entry point.
    Theme,
    /// A named procedure — a `SKILL.md` a model reads.
    Skills,
    /// Code. Refused in version 1 whatever the trust tier, because a plug-in with no capability is a
    /// plug-in that cannot do anything yet, and one with a capability has no permission model.
    Plugin,
}

impl Kind {
    /// The kind, as a manifest spells it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Theme => "theme",
            Self::Skills => "skills",
            Self::Plugin => "plugin",
        }
    }
}

/// Where a package came from, which is what decides how much it may do.
///
/// Assigned by the host, never read from the package.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustTier {
    /// Shipped by the Craft organisation, or added by the person using the app from a file they
    /// chose. Allowed.
    Internal,
    /// From an outside publisher. Refused unless this build enabled the `third-party` feature.
    ThirdParty,
}

/// Why a package was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The manifest is not valid TOML, or a field is missing or of the wrong type.
    Malformed(String),
    /// The id does not follow the character rules, so it cannot be used as a directory name.
    BadId(String),
    /// The name is empty or too long to show.
    BadName(String),
    /// The version is not `major.minor.patch`.
    BadVersion(String),
    /// The manifest names another application.
    WrongApp(String),
    /// The manifest states its own trust level, which it is not allowed to do.
    SelfAssignedTrust,
    /// A theme names code, and a theme may carry none.
    ThemeWithCode,
    /// A plug-in asks for a capability, and version 1 has none.
    CapabilityRequested(String),
    /// A plug-in was offered and this build does not run third-party code.
    ThirdPartyDisabled,
    /// A plug-in was offered to a build that has no plug-in support at all.
    PluginsUnsupported,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed(message) => write!(f, "the manifest is malformed: {message}"),
            Self::BadId(id) => write!(
                f,
                "`{id}` is not a valid package id: use lowercase letters, digits, `-` and `.` in \
                 dot-separated segments, at most {MAX_ID_LEN} bytes"
            ),
            Self::BadName(name) => write!(
                f,
                "`{name}` is not a valid package name: one to {MAX_NAME_CHARS} characters"
            ),
            Self::BadVersion(version) => {
                write!(
                    f,
                    "`{version}` is not a version: expected `major.minor.patch`"
                )
            }
            Self::WrongApp(app) => write!(f, "the package targets `{app}`, not `{APP}`"),
            Self::SelfAssignedTrust => write!(
                f,
                "the manifest states its own trust level; the host assigns it from where the \
                 package came from"
            ),
            Self::ThemeWithCode => {
                write!(f, "a theme carries no code, and this one names an entry")
            }
            Self::CapabilityRequested(capability) => write!(
                f,
                "a plug-in may ask for no capability in this version, and this one asks for \
                 `{capability}`"
            ),
            Self::ThirdPartyDisabled => write!(
                f,
                "third-party packages are not enabled in this build; no release build enables them"
            ),
            Self::PluginsUnsupported => {
                write!(
                    f,
                    "this build runs no plug-ins, so a plug-in package cannot be admitted"
                )
            }
        }
    }
}

impl std::error::Error for Refusal {}

/// The parsed manifest of one package.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// The package's id: unique, filesystem-safe, and the name of its directory in the store.
    pub id: String,
    /// A display name, in the publisher's language.
    pub name: String,
    /// `major.minor.patch`.
    pub version: String,
    /// What kind of package this is.
    pub kind: Kind,
    /// The application it targets. Present so a store can hold every app's packages.
    pub app: String,
    /// One line a person reads in a list.
    #[serde(default)]
    pub description: String,
    /// Who made it, for the attribution a bundle needs.
    #[serde(default)]
    pub author: String,
    /// What the package asks to be allowed to do. Empty in version 1.
    #[serde(default)]
    pub permissions: Vec<String>,
    /// A file the package runs or loads, for the kinds that may carry one.
    #[serde(default)]
    pub entry: Option<String>,
}

impl Manifest {
    /// Parse and validate a manifest.
    ///
    /// # Errors
    ///
    /// The [`Refusal`] that explains what is wrong, in words a publisher can act on.
    pub fn parse(text: &str) -> Result<Self, Refusal> {
        // A manifest may not state its trust level. Caught before the typed parse, because
        // `deny_unknown_fields` would report it as an unknown field and lose the reason.
        if text.lines().any(|line| {
            let line = line.trim_start();
            line.starts_with("trust") && line.contains('=')
        }) {
            return Err(Refusal::SelfAssignedTrust);
        }
        let manifest: Self =
            toml::from_str(text).map_err(|error| Refusal::Malformed(error.to_string()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Check the manifest against the rules a package has to satisfy.
    fn validate(&self) -> Result<(), Refusal> {
        if !valid_id(&self.id) {
            return Err(Refusal::BadId(self.id.clone()));
        }
        let name = self.name.trim();
        if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
            return Err(Refusal::BadName(self.name.clone()));
        }
        if !valid_version(&self.version) {
            return Err(Refusal::BadVersion(self.version.clone()));
        }
        if self.app != APP {
            return Err(Refusal::WrongApp(self.app.clone()));
        }
        if self.kind == Kind::Theme && self.entry.is_some() {
            return Err(Refusal::ThemeWithCode);
        }
        if let Some(capability) = self.permissions.first() {
            return Err(Refusal::CapabilityRequested(capability.clone()));
        }
        Ok(())
    }

    /// Whether the host may admit this package, given where it came from.
    ///
    /// # Errors
    ///
    /// [`Refusal::ThirdPartyDisabled`] for an outside package in a build without the feature, and
    /// [`Refusal::PluginsUnsupported`] for a plug-in anywhere: version 1 has no capability model, so
    /// a plug-in would either do nothing or do too much.
    pub fn admit(&self, tier: TrustTier) -> Result<(), Refusal> {
        if self.kind == Kind::Plugin {
            return Err(Refusal::PluginsUnsupported);
        }
        match tier {
            TrustTier::Internal => Ok(()),
            // The feature exists so the code path is written now and enabled later; it is not a
            // switch a release build, or a person, can flip.
            TrustTier::ThirdParty if third_party_enabled() => Err(Refusal::PluginsUnsupported),
            TrustTier::ThirdParty => Err(Refusal::ThirdPartyDisabled),
        }
    }

    /// The directory name a store would give this package: its id.
    #[must_use]
    pub fn directory(&self) -> &str {
        &self.id
    }
}

/// Whether this build runs outside code.
///
/// The function exists so the answer is one place: [`Manifest::admit`] asks it, and the test that
/// keeps the feature off asks it in a build where it must answer `false`.
#[must_use]
pub fn third_party_enabled() -> bool {
    cfg!(feature = "third-party")
}

/// Whether an id is usable as a directory name and as a key.
///
/// Lowercase ASCII letters, digits and `-`, in dot-separated segments, each starting and ending with
/// a letter or digit. No `..`, no `/`, no leading dot: the id becomes a path, and a path built from
/// hostile text is how an installer writes outside its folder.
#[must_use]
pub fn valid_id(id: &str) -> bool {
    if id.is_empty() || id.len() > MAX_ID_LEN {
        return false;
    }
    id.split('.').all(|segment| {
        !segment.is_empty()
            && segment.len() <= 63
            && segment.chars().all(|character| {
                character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
            })
            && segment.starts_with(|character: char| {
                character.is_ascii_lowercase() || character.is_ascii_digit()
            })
            && segment.ends_with(|character: char| {
                character.is_ascii_lowercase() || character.is_ascii_digit()
            })
    })
}

/// Whether a version is three dot-separated numbers.
///
/// Not a full semver parser: a package's version is shown and compared for equality, and a
/// pre-release tag that this project would accept and another would not is a compatibility problem
/// nobody has yet.
#[must_use]
pub fn valid_version(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.len() <= 9
                && part.chars().all(|character| character.is_ascii_digit())
        })
}

#[cfg(test)]
#[expect(clippy::expect_used)]
mod tests {
    use super::*;

    const THEME: &str = r#"
id = "org.example.aurora"
name = "Aurora"
version = "1.0.0"
kind = "theme"
app = "stkcraft"
description = "A dark palette with cold highlights."
"#;

    const SKILLS: &str = r#"
id = "org.example.mission-skills"
name = "Mission skills"
version = "0.2.1"
kind = "skills"
app = "stkcraft"
entry = "skills/"
"#;

    #[test]
    fn a_well_formed_theme_parses_and_is_admitted() {
        let manifest = Manifest::parse(THEME).expect("a valid theme");
        assert_eq!(manifest.kind, Kind::Theme);
        assert_eq!(manifest.directory(), "org.example.aurora");
        assert!(manifest.admit(TrustTier::Internal).is_ok());
    }

    #[test]
    fn a_skill_pack_may_name_what_it_loads() {
        let manifest = Manifest::parse(SKILLS).expect("a valid skill pack");
        assert_eq!(manifest.entry.as_deref(), Some("skills/"));
        assert!(manifest.admit(TrustTier::Internal).is_ok());
    }

    #[test]
    fn an_id_is_a_directory_name_and_is_checked_like_one() {
        assert!(valid_id("org.example.aurora"));
        assert!(valid_id("a"));
        assert!(valid_id("a1.b-2.c"));
        for bad in [
            "",
            "Aurora",
            "org..example",
            "org.example.",
            ".example",
            "org/example",
            "org.example/../etc",
            "org_example",
            "org example",
        ] {
            assert!(!valid_id(bad), "`{bad}` should be refused");
        }
        assert!(!valid_id(&"a".repeat(MAX_ID_LEN + 1)));
    }

    #[test]
    fn a_manifest_cannot_assign_its_own_trust() {
        // The interesting attack: a package that claims to be internal.
        let text = format!("{THEME}\ntrust = \"internal\"\n");
        assert_eq!(Manifest::parse(&text), Err(Refusal::SelfAssignedTrust));
    }

    #[test]
    fn a_theme_may_not_carry_code() {
        let text = THEME.replace(
            "app = \"stkcraft\"",
            "app = \"stkcraft\"\nentry = \"theme.wasm\"",
        );
        assert_eq!(Manifest::parse(&text), Err(Refusal::ThemeWithCode));
    }

    #[test]
    fn a_plugin_asks_for_no_capability_and_is_admitted_nowhere() {
        let text = THEME
            .replace("kind = \"theme\"", "kind = \"plugin\"")
            .replace(
                "description = \"A dark palette with cold highlights.\"",
                "entry = \"plugin.wasm\"",
            );
        let manifest = Manifest::parse(&text).expect("a well-formed plug-in manifest");
        // A plug-in is refused whatever the tier: version 1 has no capability model, so one that
        // runs would be one that can do anything.
        assert_eq!(
            manifest.admit(TrustTier::Internal),
            Err(Refusal::PluginsUnsupported)
        );
    }

    #[test]
    fn a_capability_is_refused_by_name() {
        let text = format!("{SKILLS}\npermissions = [\"filesystem\"]\n");
        assert_eq!(
            Manifest::parse(&text),
            Err(Refusal::CapabilityRequested("filesystem".to_owned()))
        );
    }

    #[test]
    fn an_outside_package_is_refused_in_this_build() {
        // This build has no third-party feature, which is the property that matters: the answer
        // comes from `third_party_enabled`, not from the manifest.
        let manifest = Manifest::parse(THEME).expect("a valid theme");
        assert!(
            !third_party_enabled(),
            "the third-party feature is not a default"
        );
        assert_eq!(
            manifest.admit(TrustTier::ThirdParty),
            Err(Refusal::ThirdPartyDisabled)
        );
    }

    #[test]
    fn the_third_party_feature_is_not_enabled_by_any_release_build() {
        // Opening third-party packages needs the family's threat-model gates, an outside review and
        // an explicit decision. This test is what turns "we probably will not" into "the build fails
        // if we do by accident".
        // A compile-time fact, so the assertion is on a constant by design: this is the one place
        // the value must be `false`, and the lint that objects to constant assertions is wrong here.
        #[allow(clippy::assertions_on_constants)]
        {
            assert!(
                !cfg!(feature = "third-party"),
                "the `third-party` feature must not be enabled: opening it is a decision, not a build flag"
            );
        }
    }

    #[test]
    fn the_other_apps_packages_are_refused() {
        let text = THEME.replace("app = \"stkcraft\"", "app = \"gridcraft\"");
        assert_eq!(
            Manifest::parse(&text),
            Err(Refusal::WrongApp("gridcraft".to_owned()))
        );
    }

    #[test]
    fn a_malformed_manifest_says_where_it_is_wrong() {
        let error = Manifest::parse("id = ").expect_err("that is not TOML");
        assert!(matches!(error, Refusal::Malformed(_)), "{error}");
        let error = Manifest::parse(&THEME.replace("version = \"1.0.0\"", "version = \"1.0\""))
            .expect_err("not three components");
        assert!(matches!(error, Refusal::BadVersion(_)), "{error}");
        let error = Manifest::parse(&THEME.replace("name = \"Aurora\"", "name = \"  \""))
            .expect_err("an empty name");
        assert!(matches!(error, Refusal::BadName(_)), "{error}");
    }

    #[test]
    fn every_refusal_says_something_a_publisher_can_act_on() {
        let refusals = [
            Refusal::SelfAssignedTrust,
            Refusal::ThemeWithCode,
            Refusal::ThirdPartyDisabled,
            Refusal::PluginsUnsupported,
            Refusal::BadId("../escape".to_owned()),
        ];
        for refusal in refusals {
            let message = refusal.to_string();
            assert!(message.len() > 20, "{message}");
            assert!(!message.contains("unknown"), "{message}");
        }
    }
}

/// A package that has been found on disk, validated, and admitted.
///
/// This is the difference between "a manifest parses" and "an extension loads": a package is discovered in
/// a directory, its manifest is checked, its trust tier decides whether it may run at all, and what it
/// *contributes* is read from the directory beside it — skills, catalogs, a knowledge root and command
/// aliases. Nothing here executes code: an extension contributes **data**, which is what makes it safe to
/// load from a directory a person can edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    /// Its manifest.
    pub manifest: Manifest,
    /// Where it lives, so a caller can open its files.
    pub directory: std::path::PathBuf,
    /// A `SKILL.md` directory it brings, if any.
    pub skills: Option<std::path::PathBuf>,
    /// A directory of `<locale>.tsv` catalogs it brings, if any.
    pub locales: Option<std::path::PathBuf>,
    /// A directory of documents it adds to the knowledge base, if any.
    pub knowledge: Option<std::path::PathBuf>,
    /// Commands it adds: an id of its own, the engine command it runs, and the parameters it presets.
    pub commands: Vec<Alias>,
}

/// A command an extension contributes: a name of its own over an engine command, with presets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alias {
    /// The id the extension exposes, e.g. `acme.passes-over-madrid`.
    pub id: String,
    /// The engine command it runs, e.g. `access.compute`.
    pub command: String,
    /// Parameters to merge under the caller's, so a preset can be overridden.
    pub preset: Vec<(String, String)>,
}

/// What loading a directory produced.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Loaded {
    /// The packages that were admitted, in id order.
    pub packages: Vec<Package>,
    /// The ones that were not, with the reason: `("acme.bad", "…")`.
    pub refused: Vec<(String, String)>,
}

impl Loaded {
    /// Every command alias the loaded packages contribute.
    #[must_use]
    pub fn aliases(&self) -> Vec<&Alias> {
        self.packages
            .iter()
            .flat_map(|package| package.commands.iter())
            .collect()
    }

    /// The directories the knowledge base should add, for the packages that bring documents.
    #[must_use]
    pub fn knowledge_roots(&self) -> Vec<&std::path::Path> {
        self.packages
            .iter()
            .filter_map(|package| package.knowledge.as_deref())
            .collect()
    }

    /// The skill directories to add, so a contributed skill is listed like a shipped one.
    #[must_use]
    pub fn skill_directories(&self) -> Vec<&std::path::Path> {
        self.packages
            .iter()
            .filter_map(|package| package.skills.as_deref())
            .collect()
    }
}

/// Load every extension package in `directory`, admitted at `tier`.
///
/// A directory that does not exist is an empty result rather than an error: a person who has installed
/// nothing is the normal case. A package that cannot be read, parsed or admitted is **refused with its
/// reason**, never skipped silently — a person who installed something and cannot use it needs to know why.
#[must_use]
pub fn load(directory: &std::path::Path, tier: TrustTier) -> Loaded {
    let mut loaded = Loaded::default();
    let Ok(entries) = std::fs::read_dir(directory) else {
        return loaded;
    };
    let mut paths: Vec<std::path::PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    paths.sort();
    for path in paths {
        if !path.is_dir() {
            continue;
        }
        // The app's directory uses `extension.toml`; `manifest.toml` is accepted as well so a package
        // written before that name settled still loads.
        let manifest_path = if path.join("extension.toml").exists() {
            path.join("extension.toml")
        } else {
            path.join("manifest.toml")
        };
        let name = path
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        let Ok(text) = std::fs::read_to_string(&manifest_path) else {
            loaded.refused.push((name, "no extension.toml".to_owned()));
            continue;
        };
        let manifest = match Manifest::parse(&text) {
            Ok(manifest) => manifest,
            Err(refusal) => {
                loaded.refused.push((name, refusal.to_string()));
                continue;
            }
        };
        if let Err(refusal) = manifest.admit(tier) {
            loaded
                .refused
                .push((manifest.id.clone(), refusal.to_string()));
            continue;
        }
        let directory_of = |child: &str| {
            let candidate = path.join(child);
            candidate.is_dir().then_some(candidate)
        };
        let commands = read_aliases(&path, &manifest.id, &mut loaded.refused);
        loaded.packages.push(Package {
            skills: directory_of("skills"),
            locales: directory_of("locales"),
            knowledge: directory_of("knowledge"),
            commands,
            manifest,
            directory: path,
        });
    }
    loaded
        .packages
        .sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
    loaded
}

/// A package's `commands.toml`.
#[derive(serde::Deserialize)]
struct CommandsFile {
    #[serde(default)]
    command: Vec<CommandEntry>,
}

/// One alias in that file.
#[derive(serde::Deserialize)]
struct CommandEntry {
    id: String,
    command: String,
    #[serde(default)]
    preset: std::collections::BTreeMap<String, String>,
}

/// Read `commands.toml` beside a manifest: `[[command]] id = …, command = …, preset = { … }`.
///
/// A malformed file refuses the *commands*, not the package: a package's skills and catalogs are still
/// useful when its aliases are wrong, and losing them all over a typo would be the wrong trade.
fn read_aliases(
    directory: &std::path::Path,
    id: &str,
    refused: &mut Vec<(String, String)>,
) -> Vec<Alias> {
    let path = directory.join("commands.toml");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    match toml::from_str::<CommandsFile>(&text) {
        Ok(file) => file
            .command
            .into_iter()
            .filter(|entry| {
                // An alias is namespaced by its package, so two extensions cannot shadow each other or an
                // engine command.
                let namespaced = entry.id.starts_with(&format!("{id}."));
                if !namespaced {
                    refused.push((
                        entry.id.clone(),
                        format!("an alias must start with `{id}.`"),
                    ));
                }
                namespaced && valid_id(&entry.id)
            })
            .map(|entry| Alias {
                id: entry.id,
                command: entry.command,
                preset: entry.preset.into_iter().collect(),
            })
            .collect(),
        Err(error) => {
            refused.push((format!("{id}.commands"), error.to_string()));
            Vec::new()
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::indexing_slicing)]
mod load_tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let directory = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("a directory");
        directory
    }

    fn package(root: &std::path::Path, id: &str) -> std::path::PathBuf {
        let directory = root.join(id);
        std::fs::create_dir_all(&directory).expect("a directory");
        std::fs::write(
            directory.join("manifest.toml"),
            format!(
                "id = \"{id}\"\nname = \"Probe\"\nversion = \"0.1.0\"\nkind = \"skills\"\napp = \"stkcraft\"\n"
            ),
        )
        .expect("a manifest");
        directory
    }

    #[test]
    fn a_package_loads_with_what_it_contributes() {
        // The difference between a manifest that parses and an extension that loads: the skills, catalogs and
        // knowledge root beside it are found, and its commands are namespaced by the package.
        let root = scratch("stkcraft-extensions-load");
        let directory = package(&root, "acme.sgp4");
        std::fs::create_dir_all(directory.join("skills/sgp4")).expect("a skill");
        std::fs::create_dir_all(directory.join("knowledge")).expect("knowledge");
        std::fs::write(
            directory.join("commands.toml"),
            "[[command]]\nid = \"acme.sgp4.passes\"\ncommand = \"access.compute\"\n\
             [command.preset]\nlat = \"40.4\"\nlon = \"-3.7\"\n",
        )
        .expect("commands");

        let loaded = load(&root, TrustTier::Internal);
        assert_eq!(loaded.refused, Vec::new(), "{:?}", loaded.refused);
        assert_eq!(loaded.packages.len(), 1);
        let package = &loaded.packages[0];
        assert_eq!(package.manifest.id, "acme.sgp4");
        assert!(
            package.skills.is_some(),
            "the skills directory was not found"
        );
        assert!(package.knowledge.is_some());
        assert_eq!(package.commands.len(), 1);
        assert_eq!(package.commands[0].command, "access.compute");
        assert_eq!(
            package.commands[0].preset,
            vec![
                ("lat".to_owned(), "40.4".to_owned()),
                ("lon".to_owned(), "-3.7".to_owned())
            ]
        );
        assert_eq!(loaded.aliases().len(), 1);
        assert_eq!(loaded.knowledge_roots().len(), 1);
        assert_eq!(loaded.skill_directories().len(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_bad_package_is_refused_with_its_reason() {
        // A person who installed something and cannot use it needs to know why, so nothing is skipped
        // silently: a missing manifest, a malformed one, and an alias that is not namespaced all refuse.
        let root = scratch("stkcraft-extensions-refused");
        let no_manifest = root.join("acme.empty");
        std::fs::create_dir_all(&no_manifest).expect("a directory");
        let malformed = root.join("acme.bad");
        std::fs::create_dir_all(&malformed).expect("a directory");
        std::fs::write(malformed.join("manifest.toml"), "id = \"acme.bad\"\n").expect("a manifest");
        let directory = package(&root, "acme.other");
        std::fs::write(
            directory.join("commands.toml"),
            "[[command]]\nid = \"passes\"\ncommand = \"access.compute\"\n",
        )
        .expect("commands");

        let loaded = load(&root, TrustTier::Internal);
        assert_eq!(loaded.packages.len(), 1, "only acme.other is usable");
        let reasons: Vec<&str> = loaded.refused.iter().map(|(_, why)| why.as_str()).collect();
        assert!(
            reasons.iter().any(|why| why.contains("extension.toml")),
            "{reasons:?}"
        );
        assert!(
            reasons.iter().any(|why| why.contains("acme.other.")),
            "the un-namespaced alias was accepted: {reasons:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_directory_is_empty_and_not_an_error() {
        // The normal case of a fresh installation: nothing installed is not a failure.
        let loaded = load(
            std::path::Path::new("/nonexistent/stkcraft/extensions"),
            TrustTier::Internal,
        );
        assert_eq!(loaded, Loaded::default());
    }
}
