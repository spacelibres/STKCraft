//! Where the window keeps what outlives it.
//!
//! One file, `ui.json`, in the platform's configuration directory — the same shape and the same
//! place the sibling Craft apps use, so a person who knows where `GridCraft` keeps its preferences
//! knows where `STKCraft` keeps its own:
//!
//! ```text
//! Linux    ${XDG_CONFIG_HOME:-~/.config}/stkcraft/ui.json
//! macOS    ~/Library/Application Support/ai.storyteller.stkcraft/ui.json
//! Windows  %APPDATA%\stkcraft\ui.json
//! ```
//!
//! Reading is lenient and writing is uncreative: a file that cannot be read, or that a later
//! version wrote with a field this one does not know, leaves every default in place rather than
//! failing the window. Losing a preference is an annoyance; refusing to start is a bug.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The preferences this version writes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// The interface language, as a lowercase BCP 47 code. `None` means "nothing chosen yet", which
    /// is what the first run starts from: the system's language, or English.
    pub language: Option<String>,
    /// The model endpoint, when the person chose one. `None` means the shipped default.
    pub model_endpoint: Option<String>,
    /// The model name, when the person chose one.
    pub model: Option<String>,
    /// Explicit wire protocol: `deepseek` or `compatible`.
    pub model_protocol: Option<String>,
    /// Explicit `DeepSeek` thinking selection: `enabled` or `disabled`.
    pub model_thinking: Option<String>,
}

// Preferences contain a handful of short settings, not documents. Bound reads even if a file grows.
const MAX_PREFERENCES_BYTES: u64 = 64 * 1024;

/// The file the model's credential lives in, beside the preferences.
///
/// **Not in `ui.json`.** A key is a secret, and a file a person is likely to paste into a bug report
/// is the worst place for one; this file is written `0600` on the platforms that have modes, read
/// with the same leniency as the preferences, and never logged.
#[must_use]
pub fn credential_path() -> Option<PathBuf> {
    Prefs::path().and_then(|path| path.parent().map(|parent| parent.join("model.key")))
}

/// The stored credential, or `None` when there is not one.
#[must_use]
pub fn stored_credential() -> Option<String> {
    let path = credential_path()?;
    let text = std::fs::read_to_string(path).ok()?;
    let text = text.trim().to_owned();
    (!text.is_empty()).then_some(text)
}

/// Store the model's credential where only this person can read it.
///
/// # Errors
///
/// A message when the file cannot be written, because a key that silently did not save is a person
/// re-typing it every session and assuming the app is broken.
pub fn store_credential(key: &str) -> Result<(), String> {
    let path = credential_path().ok_or_else(|| {
        "this platform has no configuration directory for a credential".to_owned()
    })?;
    store_credential_at(&path, key)
}

/// The same, at an explicit path, so a test can prove the mode without a configuration directory and
/// so the CLI can report where the key went.
///
/// # Errors
///
/// A message when the file cannot be written or its permissions cannot be restricted.
pub fn store_credential_at(path: &std::path::Path, key: &str) -> Result<(), String> {
    if let Some(directory) = path.parent() {
        std::fs::create_dir_all(directory)
            .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
    }
    std::fs::write(path, key.trim())
        .map_err(|error| format!("could not write {}: {error}", path.display()))?;
    restrict(path)?;
    Ok(())
}

/// Make a file readable only by its owner, on the platforms that have modes.
// Keep the fallible Unix signature on other platforms too.
#[cfg_attr(not(unix), allow(clippy::unnecessary_wraps))]
fn restrict(path: &std::path::Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("could not restrict {}: {error}", path.display()))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

/// The model this installation uses: the person's settings first, the environment second.
///
/// The order matters. A key typed into the settings is a decision; a key in the environment is a
/// convenience for a script or a container. The decision wins, and the environment is the fallback so
/// that a headless run keeps working.
///
/// # Errors
///
/// A message saying what is missing and where to put it, because "no model" is a state the interface
/// has to be able to explain.
pub fn model_provider() -> Result<stkcraft_agent_runtime::Provider, String> {
    let prefs = Prefs::try_load()?;
    let stored = stored_credential();
    let environment = std::env::var("DEEPSEEK_API_KEY").ok();
    let base_url = std::env::var("DEEPSEEK_BASE_URL").ok();
    let model = std::env::var("DEEPSEEK_MODEL").ok();
    let protocol = std::env::var("DEEPSEEK_PROTOCOL").ok();
    let thinking = std::env::var("DEEPSEEK_THINKING").ok();
    model_provider_from(
        &prefs,
        stored.as_deref(),
        &ModelEnvironment {
            key: environment.as_deref(),
            base_url: base_url.as_deref(),
            model: model.as_deref(),
            protocol: protocol.as_deref(),
            thinking: thinking.as_deref(),
        },
    )
}

/// The environment a provider can also be configured from, named so the resolution is testable.
///
/// A test cannot set a process environment variable (`set_var` is unsafe in edition 2024, and this
/// crate forbids unsafe), so the order of precedence is a pure function of these values instead of
/// something only a subprocess could check.
#[derive(Debug, Clone, Copy, Default)]
pub struct ModelEnvironment<'a> {
    /// `DEEPSEEK_API_KEY`.
    pub key: Option<&'a str>,
    /// `DEEPSEEK_BASE_URL`.
    pub base_url: Option<&'a str>,
    /// `DEEPSEEK_MODEL`.
    pub model: Option<&'a str>,
    /// `DEEPSEEK_PROTOCOL`.
    pub protocol: Option<&'a str>,
    /// `DEEPSEEK_THINKING`.
    pub thinking: Option<&'a str>,
}

/// Resolve credential-free protocol settings with the same precedence as the model connection.
pub fn model_options_from(
    prefs: &Prefs,
    environment: &ModelEnvironment<'_>,
) -> Result<stkcraft_agent_runtime::ProviderOptions, String> {
    stkcraft_agent_runtime::ProviderOptions::resolve(
        prefs
            .model_endpoint
            .as_deref()
            .or(environment.base_url)
            .unwrap_or(stkcraft_agent_runtime::DEEPSEEK_BASE_URL),
        prefs.model_protocol.as_deref().or(environment.protocol),
        prefs.model_thinking.as_deref().or(environment.thinking),
    )
}

/// Validate a proposed settings change without requiring or reading a credential.
pub fn validate_model_options(prefs: &Prefs) -> Result<(), String> {
    let base_url = std::env::var("DEEPSEEK_BASE_URL").ok();
    let protocol = std::env::var("DEEPSEEK_PROTOCOL").ok();
    let thinking = std::env::var("DEEPSEEK_THINKING").ok();
    model_options_from(
        prefs,
        &ModelEnvironment {
            base_url: base_url.as_deref(),
            protocol: protocol.as_deref(),
            thinking: thinking.as_deref(),
            ..ModelEnvironment::default()
        },
    )
    .map(|_| ())
}

/// Resolve the provider from the person's settings and the environment, in that order.
///
/// # Errors
///
/// A message saying what is missing and where to put it, because "no model" is a state the interface
/// has to be able to explain.
pub fn model_provider_from(
    prefs: &Prefs,
    stored: Option<&str>,
    environment: &ModelEnvironment<'_>,
) -> Result<stkcraft_agent_runtime::Provider, String> {
    let api_key = stored
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .or_else(|| environment.key.map(str::trim).filter(|key| !key.is_empty()))
        .ok_or_else(|| {
            format!(
                "no model credential: run `stkcraft model --key <KEY>`, or export DEEPSEEK_API_KEY \
                 (the settings file would be {})",
                credential_path().map_or_else(
                    || "in the configuration directory".to_owned(),
                    |path| path.display().to_string()
                )
            )
        })?;
    let endpoint = prefs
        .model_endpoint
        .clone()
        .or_else(|| environment.base_url.map(str::to_owned))
        .unwrap_or_else(|| stkcraft_agent_runtime::DEEPSEEK_BASE_URL.to_owned());
    let model = prefs
        .model
        .clone()
        .or_else(|| environment.model.map(str::to_owned))
        .unwrap_or_else(|| stkcraft_agent_runtime::DEEPSEEK_MODEL.to_owned());
    let options = model_options_from(prefs, environment)?;
    Ok(stkcraft_agent_runtime::Provider::new(endpoint, model, api_key).with_options(options))
}

impl Prefs {
    /// The file these preferences live in, if this platform has a configuration directory.
    #[must_use]
    pub fn path() -> Option<PathBuf> {
        let base = if cfg!(target_os = "windows") {
            std::env::var_os("APPDATA").map(PathBuf::from)
        } else if cfg!(target_os = "macos") {
            std::env::var_os("HOME")
                .map(|home| PathBuf::from(home).join("Library/Application Support"))
        } else {
            std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .or_else(|| {
                    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config"))
                })
        }?;
        let directory = if cfg!(target_os = "macos") {
            base.join("ai.storyteller.stkcraft")
        } else {
            base.join("stkcraft")
        };
        Some(directory.join("ui.json"))
    }

    /// The directory the window looks for a language catalog in.
    ///
    /// A catalog generated by `cargo xtask i18n translate` in a checkout is embedded at build time;
    /// one generated for *this* person's language does not need a rebuild, so the window also reads
    /// `locales/<locale>.tsv` from its own configuration directory. That is the file a runtime
    /// translation writes, and the manual way to add a language without touching the source.
    #[must_use]
    pub fn locales_dir() -> Option<PathBuf> {
        Self::path().and_then(|path| path.parent().map(|parent| parent.join("locales")))
    }

    /// Read the preferences, or the defaults when there are none to read.
    #[must_use]
    pub fn load() -> Self {
        Self::try_load().unwrap_or_default()
    }

    /// Read model settings without silently replacing malformed saved connection options.
    ///
    /// # Errors
    /// Unreadable, non-regular, oversized or malformed existing preference files.
    pub fn try_load() -> Result<Self, String> {
        Self::path().map_or_else(|| Ok(Self::default()), |path| Self::read_at(&path))
    }

    fn read_at(path: &std::path::Path) -> Result<Self, String> {
        use std::io::Read as _;
        let metadata = match std::fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => {
                return Err(format!(
                    "could not inspect preferences {}: {error}",
                    path.display()
                ));
            }
        };
        if !metadata.is_file() {
            return Err(format!(
                "preferences {} are not a regular file",
                path.display()
            ));
        }
        if metadata.len() > MAX_PREFERENCES_BYTES {
            return Err(format!(
                "preferences {} exceed {MAX_PREFERENCES_BYTES} bytes",
                path.display()
            ));
        }
        let file = std::fs::File::open(path)
            .map_err(|error| format!("could not open preferences {}: {error}", path.display()))?;
        if !file
            .metadata()
            .map_err(|error| format!("could not inspect preferences {}: {error}", path.display()))?
            .is_file()
        {
            return Err(format!(
                "preferences {} are not a regular file",
                path.display()
            ));
        }
        let mut bytes = Vec::new();
        file.take(MAX_PREFERENCES_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("could not read preferences {}: {error}", path.display()))?;
        if bytes.len() as u64 > MAX_PREFERENCES_BYTES {
            return Err(format!(
                "preferences {} exceed {MAX_PREFERENCES_BYTES} bytes",
                path.display()
            ));
        }
        serde_json::from_slice(&bytes).map_err(|_| format!("invalid preferences JSON in {}; repair the saved settings before configuring or using the model", path.display()))
    }

    /// Write the preferences, creating the directory if it is missing.
    ///
    /// # Errors
    ///
    /// A message when the file cannot be written. The caller decides whether that matters: a
    /// preference that could not be saved does not stop the window from working.
    pub fn save(&self) -> Result<(), String> {
        let Some(path) = Self::path() else {
            return Err("this platform has no configuration directory".to_owned());
        };
        self.write_at(&path)
    }

    fn write_at(&self, path: &std::path::Path) -> Result<(), String> {
        // A lenient UI read may have used defaults. Never overwrite a malformed saved connection.
        Self::read_at(path)?;
        if let Some(directory) = path.parent() {
            std::fs::create_dir_all(directory)
                .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
        }
        let text = serde_json::to_string_pretty(self)
            .map_err(|error| format!("could not write the preferences: {error}"))?;
        if text.len() as u64 > MAX_PREFERENCES_BYTES {
            return Err(format!(
                "preferences {} exceed {MAX_PREFERENCES_BYTES} bytes",
                path.display()
            ));
        }
        std::fs::write(path, text)
            .map_err(|error| format!("could not write {}: {error}", path.display()))
    }
}

/// The directory an installed extension package lives in, if this platform has a configuration
/// directory: one folder per package, named by its id, holding `extension.toml`.
#[must_use]
pub fn extensions_dir() -> Option<PathBuf> {
    Prefs::path().and_then(|path| path.parent().map(|parent| parent.join("extensions")))
}

/// The directory the knowledge base lives in: the person's own documents, which the platform reads
/// before a model answers.
#[must_use]
pub fn knowledge_dir() -> Option<PathBuf> {
    Prefs::path().and_then(|path| path.parent().map(|parent| parent.join("knowledge")))
}

/// The catalogs this person has, as `(locale, text)` pairs, from the configuration directory.
///
/// A file that cannot be read is skipped rather than reported: it is one language, and the others
/// still work.
#[must_use]
pub fn user_catalogs() -> Vec<(String, String)> {
    let mut catalogs = Vec::new();
    let Some(directory) = Prefs::locales_dir() else {
        return catalogs;
    };
    let Ok(entries) = std::fs::read_dir(&directory) else {
        return catalogs;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "tsv") {
            continue;
        }
        let Some(locale) = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
        else {
            continue;
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        catalogs.push((locale, text));
    }
    catalogs.sort_by(|a, b| a.0.cmp(&b.0));
    catalogs
}

/// A language's name for a translation run, before the model answers with its own.
///
/// The languages the window offers are named here so a person can pick one that has no catalog: the
/// name goes into the prompt ("translate into Japanese (`ja`), called 日本語") and into the catalog's
/// header, and the model's answer replaces it with its own wording.
#[must_use]
pub fn language_name_for(locale: &str) -> String {
    match locale.split('-').next().unwrap_or(locale) {
        "ar" => "العربية",
        "cs" => "Čeština",
        "de" => "Deutsch",
        "el" => "Ελληνικά",
        "en" => "English",
        "es" => "Español",
        "fr" => "Français",
        "it" => "Italiano",
        "ja" => "日本語",
        "ko" => "한국어",
        "nl" => "Nederlands",
        "pl" => "Polski",
        "pt" => "Português",
        "ro" => "Română",
        "ru" => "Русский",
        "sr" => "Srpski",
        "tr" => "Türkçe",
        "uk" => "Українська",
        "vi" => "Tiếng Việt",
        "zh" => "中文",
        other => other,
    }
    .to_owned()
}

/// The languages the window offers when it has no catalog for them.
///
/// It is the family's list, by base language, so a person sees the language they think in rather than
/// a region code. A language that already has a catalog appears through [`Translations::picker`]
/// instead; this list is what makes "translate it with my key" reachable for the rest.
pub const OFFERED: [&str; 19] = [
    "ar", "cs", "de", "el", "es", "fr", "it", "ja", "ko", "nl", "pl", "pt", "ro", "ru", "sr", "tr",
    "uk", "vi", "zh",
];

/// The language the operating system asks for, best effort.
///
/// `LC_ALL`, then `LC_MESSAGES`, then `LANG`, in the order the POSIX locale rules give them. An
/// unset or unusable value is `None`, which the caller reads as English.
#[must_use]
pub fn system_locale() -> Option<String> {
    for variable in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(value) = std::env::var(variable)
            && let Some(code) = stkcraft_framework_i18n::parse_system_locale(&value)
        {
            return Some(code);
        }
    }
    None
}

#[cfg(test)]
#[expect(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn preference_reads_are_bounded_and_default_ui_writes_preserve_invalid_connections() {
        let base = std::env::temp_dir()
            .canonicalize()
            .expect("temporary directory");
        let root = base.join(format!("stkcraft-preference-bounds-{}", std::process::id()));
        assert!(root.starts_with(&base));
        std::fs::create_dir(&root).expect("unique preference fixture");
        let path = root.join("ui.json");
        assert_eq!(
            Prefs::read_at(&path).expect("missing file"),
            Prefs::default()
        );
        assert!(
            Prefs::read_at(&root).is_err(),
            "directories are not preference files"
        );
        for bytes in [b"{broken-json".to_vec(), vec![b' '; 65_537]] {
            std::fs::write(&path, &bytes).expect("invalid fixture");
            assert!(Prefs::read_at(&path).is_err());
            assert!(
                Prefs::default().write_at(&path).is_err(),
                "UI defaults must not overwrite saved invalid settings"
            );
            assert_eq!(std::fs::read(&path).expect("preserved file"), bytes);
        }
        std::fs::write(&path, r#"{"language":"zh","model_endpoint":"https://private.invalid/chat/completions","future":true}"#).expect("valid old preferences");
        let prefs = Prefs::read_at(&path).expect("unknown fields remain compatible");
        assert_eq!(prefs.language.as_deref(), Some("zh"));
        assert_eq!(
            prefs.model_endpoint.as_deref(),
            Some("https://private.invalid/chat/completions")
        );
        let before = std::fs::read(&path).expect("valid fixture");
        let oversized = Prefs {
            model: Some("x".repeat(65_537)),
            ..Prefs::default()
        };
        assert!(oversized.write_at(&path).is_err());
        assert_eq!(
            std::fs::read(&path).expect("oversized writes preserve file"),
            before
        );
        let resolved = root.canonicalize().expect("fixture root");
        assert!(resolved.starts_with(&base));
        std::fs::remove_dir_all(resolved).expect("remove isolated fixtures");
    }

    #[cfg(unix)]
    #[test]
    fn preference_reads_refuse_device_files() {
        assert!(Prefs::read_at(std::path::Path::new("/dev/zero")).is_err());
    }

    #[test]
    fn model_protocol_invalid_explicit_settings_are_errors() {
        for raw in [
            r#"{"model_protocol":"unknown"}"#,
            r#"{"model_thinking":"turbo"}"#,
        ] {
            let prefs: Prefs = serde_json::from_str(raw).expect("settings object");
            let result =
                model_provider_from(&prefs, Some("test-only-key"), &ModelEnvironment::default());
            assert!(
                result.is_err(),
                "invalid explicit settings were accepted: {raw}"
            );
        }
    }

    #[test]
    fn model_protocol_settings_win_and_proxy_opt_in_is_explicit() {
        use stkcraft_agent_runtime::{ProviderProtocol, ThinkingMode};
        let environment = ModelEnvironment {
            key: Some("test-only-key"),
            protocol: Some("compatible"),
            thinking: Some("enabled"),
            ..ModelEnvironment::default()
        };
        let prefs = Prefs {
            model: Some("deepseek-chat".to_owned()),
            model_protocol: Some("deepseek".to_owned()),
            model_thinking: Some("disabled".to_owned()),
            ..Prefs::default()
        };
        let provider = model_provider_from(&prefs, None, &environment).expect("saved settings win");
        assert_eq!(provider.options.protocol, ProviderProtocol::DeepSeek);
        assert_eq!(provider.options.thinking, ThinkingMode::Disabled);
        assert_eq!(
            provider.model, "deepseek-chat",
            "explicit model selections survive"
        );

        let proxy = ModelEnvironment {
            key: Some("test-only-key"),
            base_url: Some("https://proxy.invalid/chat/completions"),
            ..ModelEnvironment::default()
        };
        let provider = model_provider_from(&Prefs::default(), None, &proxy).expect("generic proxy");
        assert_eq!(
            provider.options.protocol,
            ProviderProtocol::OpenAiCompatible
        );
        let enabled_proxy = ModelEnvironment {
            protocol: Some("deepseek"),
            thinking: Some("enabled"),
            ..proxy
        };
        let provider = model_provider_from(&Prefs::default(), None, &enabled_proxy)
            .expect("explicit proxy opt-in");
        assert_eq!(provider.options.protocol, ProviderProtocol::DeepSeek);
        assert_eq!(provider.options.thinking, ThinkingMode::Enabled);
    }

    #[test]
    fn model_protocol_shipped_defaults_and_legacy_preferences_are_compatible() {
        use stkcraft_agent_runtime::{ProviderProtocol, ThinkingMode};
        let prefs: Prefs = serde_json::from_str(r#"{"language":"zh"}"#).expect("old preferences");
        assert!(prefs.model_protocol.is_none());
        assert!(prefs.model_thinking.is_none());
        let provider =
            model_provider_from(&prefs, Some("test-only-key"), &ModelEnvironment::default())
                .expect("shipped defaults");
        assert_eq!(provider.options.protocol, ProviderProtocol::DeepSeek);
        assert_eq!(provider.options.thinking, ThinkingMode::Disabled);
        assert_eq!(provider.model, "deepseek-flash");
        assert!(
            model_options_from(
                &Prefs::default(),
                &ModelEnvironment {
                    protocol: Some("typo"),
                    ..ModelEnvironment::default()
                }
            )
            .is_err()
        );
        assert!(
            model_options_from(
                &Prefs::default(),
                &ModelEnvironment {
                    thinking: Some("turbo"),
                    ..ModelEnvironment::default()
                }
            )
            .is_err()
        );
        assert!(
            model_options_from(
                &Prefs::default(),
                &ModelEnvironment {
                    protocol: Some("compatible"),
                    thinking: Some("enabled"),
                    ..ModelEnvironment::default()
                }
            )
            .is_err()
        );
    }

    #[test]
    fn preferences_round_trip_through_json() {
        let prefs = Prefs {
            language: Some("de".to_owned()),
            model_endpoint: Some("https://mine.example/v1/chat/completions".to_owned()),
            model: Some("mine".to_owned()),
            model_protocol: Some("compatible".to_owned()),
            model_thinking: Some("disabled".to_owned()),
        };
        let text = serde_json::to_string(&prefs).expect("serialised");
        let read: Prefs = serde_json::from_str(&text).expect("deserialised");
        assert_eq!(read, prefs);
    }

    #[test]
    fn a_file_from_a_later_version_keeps_the_fields_it_knows() {
        let read: Prefs = serde_json::from_str(r#"{"language":"zh","theme":"dark","zoom":1.5}"#)
            .expect("unknown fields are ignored");
        assert_eq!(read.language.as_deref(), Some("zh"));
    }

    #[test]
    fn an_empty_file_is_the_defaults() {
        assert_eq!(Prefs::default().language, None);
        let read: Prefs = serde_json::from_str("{}").expect("an empty object");
        assert_eq!(read.language, None);
    }

    #[test]
    fn a_system_locale_normalises_to_a_catalog_code() {
        assert_eq!(
            stkcraft_framework_i18n::parse_system_locale("zh_CN.UTF-8").as_deref(),
            Some("zh-cn")
        );
        assert_eq!(stkcraft_framework_i18n::parse_system_locale("C"), None);
    }

    #[test]
    fn the_stored_credential_wins_over_the_environment() {
        // A key typed into the settings is a decision; a key in the environment is a convenience for
        // a script. The decision wins, and the environment is the fallback so a container still works.
        let prefs = Prefs::default();
        let environment = ModelEnvironment {
            key: Some("from-environment"),
            base_url: Some("https://env.example/v1/chat/completions"),
            model: Some("env-model"),
            ..ModelEnvironment::default()
        };
        let provider =
            model_provider_from(&prefs, Some("from-settings"), &environment).expect("a provider");
        assert_eq!(provider.api_key, "from-settings");
        // The endpoint and model still come from the environment, because nothing overrode them.
        assert_eq!(provider.base_url, "https://env.example/v1/chat/completions");
        assert_eq!(provider.model, "env-model");
        assert!(provider.has_credential());
    }

    #[test]
    fn the_environment_is_the_fallback_and_the_defaults_are_shipped() {
        let prefs = Prefs::default();
        let provider = model_provider_from(
            &prefs,
            None,
            &ModelEnvironment {
                key: Some("  spaced-out  "),
                ..ModelEnvironment::default()
            },
        )
        .expect("a provider");
        // A pasted key often carries whitespace; a key with a trailing newline is not a different key.
        assert_eq!(provider.api_key, "spaced-out");
        assert_eq!(provider.base_url, stkcraft_agent_runtime::DEEPSEEK_BASE_URL);
        assert_eq!(provider.model, stkcraft_agent_runtime::DEEPSEEK_MODEL);
        assert_eq!(provider.timeout_seconds, 120);

        // Settings override both, and an empty stored value is not a credential.
        let chosen = Prefs {
            language: None,
            model_endpoint: Some("https://mine.example/v1/chat/completions".to_owned()),
            model: Some("mine".to_owned()),
            ..Prefs::default()
        };
        let provider = model_provider_from(
            &chosen,
            Some("   "),
            &ModelEnvironment {
                key: Some("env"),
                ..ModelEnvironment::default()
            },
        )
        .expect("a provider");
        assert_eq!(provider.api_key, "env");
        assert_eq!(
            provider.base_url,
            "https://mine.example/v1/chat/completions"
        );
        assert_eq!(provider.model, "mine");
    }

    #[test]
    fn no_credential_says_how_to_set_one() {
        let error = model_provider_from(&Prefs::default(), None, &ModelEnvironment::default())
            .expect_err("no credential anywhere");
        assert!(error.contains("stkcraft model --key"), "{error}");
        assert!(error.contains("DEEPSEEK_API_KEY"), "{error}");
    }

    #[test]
    fn the_credential_file_is_private_and_trimmed() {
        let directory = std::env::temp_dir().join("stkcraft-model-key");
        let _ = std::fs::remove_dir_all(&directory);
        let path = directory.join("model.key");
        store_credential_at(&path, "  sk-abc  \n").expect("stored");
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "sk-abc");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path)
                .expect("metadata")
                .permissions()
                .mode();
            // 0600: the owner reads and writes, nobody else does anything.
            assert_eq!(mode & 0o777, 0o600, "mode {mode:o}");
        }
        let _ = std::fs::remove_dir_all(&directory);
    }
}
