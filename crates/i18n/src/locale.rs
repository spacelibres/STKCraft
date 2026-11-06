//! Locale codes, the fallback chain, and the languages the picker offers.
//!
//! Codes are lowercase BCP 47, as the shared catalogs require: `de`, `zh`, `pt-br`, `zh-hans`. A
//! regional locale falls back to its base language before English, so `es-mx` a translator has not
//! started yet still shows Spanish rather than English — and a *script* variant does not, because
//! `sr-latn` falling back to `sr` would show Cyrillic to someone who asked for Latin.

/// A language the interface can be shown in.
///
/// English is the source: it has no catalog, and [`Language::En`] is what every lookup falls back
/// to. Every other language exists only when a catalog for it was embedded or found on disk.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Language {
    /// English, the source text itself.
    En,
    /// A locale with a catalog, named by its lowercase BCP 47 code.
    Other(String),
}

impl Language {
    /// The locale code: `"en"` for English, the catalog's directory name otherwise.
    #[must_use]
    pub fn code(&self) -> &str {
        match self {
            Self::En => crate::ENGLISH,
            Self::Other(code) => code,
        }
    }

    /// Whether this is the source language.
    #[must_use]
    pub fn is_english(&self) -> bool {
        matches!(self, Self::En)
    }

    /// A language from a locale code, a system locale (`de_DE.UTF-8`) or a UI tag (`pt-BR`).
    ///
    /// The tag is normalised, not resolved: `de-DE` parses to the locale `de-de`, which is the
    /// interface language *asked for*. Which catalog answers it is
    /// [`Translations::resolve`](crate::Translations::resolve)'s question, because that depends on
    /// which catalogs the build has.
    #[must_use]
    pub fn parse(tag: &str) -> Option<Self> {
        let code = normalize_locale(tag)?;
        if code == crate::ENGLISH {
            Some(Self::En)
        } else {
            Some(Self::Other(code))
        }
    }
}

/// Normalise a locale tag to the form a catalog directory uses.
///
/// `de_DE.UTF-8` → `de-de`, `pt-BR` → `pt-br`, `zh_Hans_CN` → `zh-hans`, `en_US` → `en-us`. Nothing
/// is dropped but the encoding and modifier: a tag is normalised, and [`locale_chain`] decides what
/// to try afterwards. Inventing a match here — turning `de-de` into `de` — would make a lookup that
/// has no catalog look as though it did.
#[must_use]
pub fn normalize_locale(tag: &str) -> Option<String> {
    // An encoding suffix and a modifier are not part of the language.
    let tag = tag.split(['.', '@']).next()?.trim();
    if tag.is_empty() {
        return None;
    }
    let mut parts = tag
        .split(['-', '_'])
        .filter(|part| !part.is_empty())
        .map(str::to_ascii_lowercase);
    let language = parts.next()?;
    if language.len() < 2 || !language.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let mut code = language;
    let mut kept_script = false;
    for part in parts {
        let is_script = part.len() == 4 && part.chars().all(|c| c.is_ascii_alphabetic());
        let is_region = (part.len() == 2 && part.chars().all(|c| c.is_ascii_alphabetic()))
            || (part.len() == 3 && part.chars().all(|c| c.is_ascii_digit()));
        // A catalog directory names a script *or* a region, never both: `zh-hans`, `sr-latn`,
        // `pt-br`. Keeping `zh-hans-cn` would name a locale no catalog in the family has.
        if is_script && !kept_script {
            code.push('-');
            code.push_str(&part);
            kept_script = true;
        } else if is_region && !kept_script {
            code.push('-');
            code.push_str(&part);
        }
    }
    Some(code)
}

/// The languages to try, most specific first, ending with English.
///
/// A locale whose last subtag is a region falls back to its base language; a script variant does
/// not, because `sr-latn` to `sr` would swap the alphabet under the reader.
#[must_use]
pub fn locale_chain(locale: &str) -> Vec<String> {
    let Some(code) = normalize_locale(locale) else {
        return vec![crate::ENGLISH.to_owned()];
    };
    let mut chain = vec![code.clone()];
    if let Some((base, last)) = code.rsplit_once('-') {
        let is_region = (last.len() == 2 && last.chars().all(|c| c.is_ascii_alphabetic()))
            || (last.len() == 3 && last.chars().all(|c| c.is_ascii_digit()));
        if is_region && base != crate::ENGLISH {
            chain.push(base.to_owned());
        }
    }
    if chain.iter().all(|code| code != crate::ENGLISH) {
        chain.push(crate::ENGLISH.to_owned());
    }
    chain
}

/// The `@id language.name` row's source, which every catalog uses to name itself.
pub const LANGUAGE_NAME_SOURCE: &str = "language.name";

/// The context of the row that names the language.
pub const LANGUAGE_NAME_CONTEXT: &str = "@id";

/// A language's name in its own language, from a catalog's `@id language.name` row.
///
/// `None` when the catalog does not name itself, which a picker shows as the code.
#[must_use]
pub fn language_name_row(name: &str) -> (String, String, String) {
    (
        LANGUAGE_NAME_CONTEXT.to_owned(),
        LANGUAGE_NAME_SOURCE.to_owned(),
        name.to_owned(),
    )
}

/// Parse a system locale string into a code, for the first-run default.
///
/// This is environment parsing rather than locale detection: the app hands what the operating
/// system reported (`LANG`, the browser's `navigator.language`, the Windows locale name) and this
/// turns it into a catalog code. Anything unrecognised is `None`, which means English.
#[must_use]
pub fn parse_system_locale(reported: &str) -> Option<String> {
    let code = normalize_locale(reported)?;
    Some(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_locales_normalise_to_catalog_codes() {
        assert_eq!(normalize_locale("de_DE.UTF-8").as_deref(), Some("de-de"));
        assert_eq!(normalize_locale("en_US.UTF-8").as_deref(), Some("en-us"));
        assert_eq!(normalize_locale("zh_CN.UTF-8").as_deref(), Some("zh-cn"));
        assert_eq!(normalize_locale("pt-BR").as_deref(), Some("pt-br"));
        assert_eq!(normalize_locale("zh_Hans_CN").as_deref(), Some("zh-hans"));
        assert_eq!(normalize_locale("sr_Latn_RS").as_deref(), Some("sr-latn"));
        assert_eq!(normalize_locale("C").as_deref(), None);
        assert_eq!(normalize_locale("").as_deref(), None);
        assert_eq!(normalize_locale("ru_RU@euro").as_deref(), Some("ru-ru"));
        assert_eq!(normalize_locale("de").as_deref(), Some("de"));
    }

    #[test]
    fn the_fallback_chain_ends_with_english() {
        assert_eq!(locale_chain("de"), vec!["de", "en"]);
        assert_eq!(locale_chain("pt-BR"), vec!["pt-br", "pt", "en"]);
        // A script variant never falls back to the base language: `sr-latn` to `sr` would swap the
        // alphabet under the reader.
        assert_eq!(locale_chain("sr-Latn"), vec!["sr-latn", "en"]);
        assert_eq!(locale_chain("de-DE"), vec!["de-de", "de", "en"]);
        assert_eq!(locale_chain("zh-hans"), vec!["zh-hans", "en"]);
        assert_eq!(locale_chain("en"), vec!["en"]);
        assert_eq!(locale_chain("not a locale"), vec!["en"]);
    }

    #[test]
    fn a_language_parses_from_every_form_the_system_reports() {
        assert_eq!(
            Language::parse("en").map(|l| l.code().to_owned()),
            Some("en".to_owned())
        );
        assert_eq!(
            Language::parse("de_DE.UTF-8").map(|l| l.code().to_owned()),
            Some("de-de".to_owned())
        );
        assert!(Language::parse("en").is_some_and(|language| language.is_english()));
        assert_eq!(Language::parse("C"), None);
    }
}
