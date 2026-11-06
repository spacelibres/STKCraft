//! The catalogs an application holds, and the lookups that use them.
//!
//! A [`Translations`] is built once from the catalogs a build embeds — one per locale — and then
//! answers lookups for the rest of the session. Nothing here allocates on a lookup, and nothing
//! fails: a missing catalog, a missing row or an unknown locale all mean "show the English", which
//! is the behaviour the shared format promises.

use std::collections::BTreeMap;

use crate::catalog::{Catalog, Row};
use crate::locale::{self, Language};
use crate::plurals;

/// The catalogs a build embeds, as `(locale, catalog text)` pairs.
///
/// The shape is the one the shared catalog project describes for an app's optional build input: a
/// list of locales and their text. A build with no catalogs compiles and runs in English.
pub type Embedded = &'static [(&'static str, &'static str)];

/// Every catalog this session can show, with English always available.
#[derive(Debug, Clone, Default)]
pub struct Translations {
    /// locale → (context, source) → translation. English is absent: it needs no table.
    by_locale: BTreeMap<String, BTreeMap<(String, String), String>>,
    /// locale → its own name, for the picker.
    names: BTreeMap<String, String>,
    /// Row counts, so a caller can report coverage without walking the tables again.
    counts: BTreeMap<String, usize>,
}

impl Translations {
    /// Build from the embedded catalogs.
    ///
    /// A catalog with problems contributes its well-formed rows: a half-translated language is the
    /// normal state, and refusing to load it would show English to someone who asked for the
    /// language they are half-way through translating. `cargo xtask i18n check` is where problems
    /// are refusals.
    #[must_use]
    pub fn from_embedded(embedded: Embedded) -> Self {
        let mut translations = Self::default();
        for (locale, text) in embedded {
            let code = locale::normalize_locale(locale).unwrap_or_else(|| (*locale).to_owned());
            if code == crate::ENGLISH {
                continue;
            }
            let (catalog, _problems) = crate::catalog::parse(&code, text);
            translations.add(&catalog);
        }
        translations
    }

    /// Add one parsed catalog, replacing any catalog of the same locale.
    pub fn add(&mut self, catalog: &Catalog) {
        let Some(code) = locale::normalize_locale(&catalog.locale) else {
            return;
        };
        let mut table = BTreeMap::new();
        for Row {
            context,
            source,
            translation,
        } in &catalog.rows
        {
            table.insert((context.clone(), source.clone()), translation.clone());
        }
        if let Some(name) = catalog.name() {
            self.names.insert(code.clone(), name.to_owned());
        }
        self.counts.insert(code.clone(), catalog.rows.len());
        self.by_locale.insert(code, table);
    }

    /// The locales a catalog exists for, in code order, English excluded.
    pub fn locales(&self) -> impl Iterator<Item = &str> {
        self.by_locale.keys().map(String::as_str)
    }

    /// Whether a locale has a catalog, directly or through its chain.
    #[must_use]
    pub fn has_locale(&self, locale: &str) -> bool {
        self.catalog_for(locale).is_some()
    }

    /// The code of the catalog that answers this locale, following the chain.
    ///
    /// This is the question a language picker asks: `de-DE` is answered by `de`'s catalog, so
    /// choosing it should select German rather than a catalog that does not exist.
    #[must_use]
    pub fn catalog_for(&self, locale: &str) -> Option<String> {
        locale::locale_chain(locale)
            .into_iter()
            .find(|code| code != crate::ENGLISH && self.by_locale.contains_key(code))
    }

    /// Which language a locale tag should select: the catalog its chain reaches, or English.
    #[must_use]
    pub fn resolve(&self, tag: &str) -> Language {
        match self.catalog_for(tag) {
            Some(code) => Language::Other(code),
            None => Language::En,
        }
    }

    /// A language's name in its own language; the code itself when the catalog does not name it.
    #[must_use]
    pub fn language_name(&self, locale: &str) -> String {
        let Some(code) = locale::normalize_locale(locale) else {
            return crate::ENGLISH.to_owned();
        };
        if code == crate::ENGLISH {
            return "English".to_owned();
        }
        self.names
            .get(&code)
            .cloned()
            .unwrap_or_else(|| code.clone())
    }

    /// How many rows the catalog that answers this locale holds.
    #[must_use]
    pub fn len(&self, locale: &str) -> usize {
        self.catalog_for(locale)
            .and_then(|code| self.counts.get(&code).copied())
            .unwrap_or(0)
    }

    /// Whether no catalog has a row at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_locale.values().all(BTreeMap::is_empty)
    }

    /// The languages the picker offers: English first, then the rest by their own name.
    ///
    /// The order is the family's: the source language leads because it is the one that is always
    /// complete, and the others follow alphabetically *in their own script* rather than by code,
    /// so a reader finds their language where they expect it. Names in a non-Latin script sort
    /// after the Latin ones, in code point order.
    #[must_use]
    pub fn picker(&self) -> Vec<Language> {
        let mut others: Vec<Language> = self
            .locales()
            .map(|code| Language::Other(code.to_owned()))
            .collect();
        others.sort_by_cached_key(|language| {
            (
                !language.code().starts_with(|c: char| c.is_ascii()),
                fold_name(&self.language_name(language.code())),
            )
        });
        std::iter::once(Language::En).chain(others).collect()
    }

    /// Translate a string in a locale, ending at the English source text.
    ///
    /// The chain is the shared one: the locale's own catalog, then its base language's, then
    /// English. An unknown locale, an absent catalog and an untranslated string all land on the
    /// English, which is why this cannot fail.
    #[must_use]
    pub fn tr<'a>(&'a self, locale: &str, source: &'a str) -> &'a str {
        self.tr_context(locale, "", source)
    }

    /// Translate a string that only makes sense in a context, such as `menu` or `@msg`.
    #[must_use]
    pub fn tr_context<'a>(&'a self, locale: &str, context: &str, source: &'a str) -> &'a str {
        for code in locale::locale_chain(locale) {
            let Some(table) = self.by_locale.get(&code) else {
                continue;
            };
            if let Some(translation) = table.get(&(context.to_owned(), source.to_owned())) {
                return translation;
            }
            // A contextual row falls back to the plain one, as the format specifies.
            if !context.is_empty()
                && let Some(translation) = table.get(&(String::new(), source.to_owned()))
            {
                return translation;
            }
        }
        source
    }

    /// Translate a count-dependent message.
    ///
    /// `one` and `other` are the English forms, which are also the plural row's source column; the
    /// catalog's row for that source carries the locale's forms in CLDR order. `{n}` in either is
    /// replaced with the count.
    #[must_use]
    pub fn trn(&self, locale: &str, one: &str, other: &str, count: u64) -> String {
        let source = format!("{one}|{other}");
        let translated = self.tr_context(locale, "@plural", &source);
        let forms: Vec<&str> = translated.split('|').collect();
        let index = if std::ptr::eq(translated, source.as_str()) {
            // No translation: use the English form for this count.
            plurals::english_form_index(count)
        } else {
            plurals::plural_form_index(locale, count)
        };
        let form = forms
            .get(index)
            .or_else(|| forms.last())
            .copied()
            .unwrap_or(other);
        form.replace("{n}", &count.to_string())
    }
}

/// A language name with the Latin accents in the names of languages folded away, so a byte-wise
/// sort reads alphabetically ("Čeština" under C). Other scripts are left as they are.
fn fold_name(name: &str) -> String {
    name.chars()
        .flat_map(char::to_lowercase)
        .map(|character| match character {
            'à'..='å' | 'ā' | 'ă' | 'ą' => 'a',
            'ç' | 'ć' | 'č' => 'c',
            'è'..='ë' | 'ē' | 'ě' | 'ę' => 'e',
            'ì'..='ï' | 'ī' => 'i',
            'ñ' | 'ń' | 'ň' => 'n',
            'ò'..='ö' | 'ø' | 'ő' => 'o',
            'ś' | 'š' => 's',
            'ù'..='ü' | 'ū' | 'ů' | 'ű' => 'u',
            'ý' | 'ÿ' => 'y',
            'ź' | 'ż' | 'ž' => 'z',
            other => other,
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::indexing_slicing)]
mod tests {
    use super::*;

    const GERMAN: &str = "@id\tlanguage.name\tDeutsch\n\
                          \tSave\tSpeichern\n\
                          \tOpen\tÖffnen\n\
                          menu\tOpen Table…\tTabelle öffnen…\n\
                          @plural\t{n} page|{n} pages\t{n} Seite|{n} Seiten\n";

    const PORTUGUESE: &str = "@id\tlanguage.name\tPortuguês (Brasil)\n\
                              \tSave\tSalvar\n";

    const PT: &str = "@id\tlanguage.name\tPortuguês\n\
                      \tOpen\tAbrir\n";

    fn translations() -> Translations {
        Translations::from_embedded(&[("de", GERMAN), ("pt-br", PORTUGUESE), ("pt", PT)])
    }

    #[test]
    fn a_translated_string_is_translated_and_a_missing_one_is_english() {
        let translations = translations();
        assert_eq!(translations.tr("de", "Save"), "Speichern");
        assert_eq!(translations.tr("de", "Close"), "Close");
        assert_eq!(translations.tr("fr", "Save"), "Save");
        assert_eq!(translations.tr("en", "Save"), "Save");
    }

    #[test]
    fn a_contextual_row_wins_and_falls_back_to_the_plain_one() {
        let translations = translations();
        assert_eq!(
            translations.tr_context("de", "menu", "Open Table…"),
            "Tabelle öffnen…"
        );
        assert_eq!(translations.tr_context("de", "", "Open"), "Öffnen");
        // A context the catalog does not have falls back to the plain row before English.
        assert_eq!(translations.tr_context("de", "preview", "Open"), "Öffnen");
        assert_eq!(translations.tr_context("de", "preview", "Close"), "Close");
    }

    #[test]
    fn a_regional_locale_falls_back_to_its_base_language() {
        let translations = translations();
        assert_eq!(translations.tr("pt-br", "Save"), "Salvar");
        // Portuguese has no `Close`, but Brazil's catalog is checked before the base language's…
        assert_eq!(translations.tr("pt-br", "Open"), "Abrir");
        assert_eq!(translations.tr("pt", "Save"), "Save");
    }

    #[test]
    fn plurals_follow_the_language_that_owns_them() {
        let translations = translations();
        assert_eq!(
            translations.trn("de", "{n} page", "{n} pages", 1),
            "1 Seite"
        );
        assert_eq!(
            translations.trn("de", "{n} page", "{n} pages", 5),
            "5 Seiten"
        );
        // No catalog: the English form for the count.
        assert_eq!(translations.trn("fr", "{n} page", "{n} pages", 1), "1 page");
        assert_eq!(
            translations.trn("fr", "{n} page", "{n} pages", 3),
            "3 pages"
        );
    }

    #[test]
    fn the_picker_leads_with_english_and_sorts_by_the_name_a_reader_sees() {
        let translations = translations();
        let codes: Vec<String> = translations
            .picker()
            .into_iter()
            .map(|language| language.code().to_owned())
            .collect();
        assert_eq!(codes, ["en", "de", "pt", "pt-br"]);
        assert_eq!(translations.language_name("de"), "Deutsch");
        assert_eq!(translations.language_name("en"), "English");
        // A locale with no `@id` row is named by its code rather than disappearing.
        assert_eq!(translations.language_name("xx"), "xx");
    }

    #[test]
    fn coverage_is_countable_without_walking_the_tables() {
        let translations = translations();
        assert_eq!(translations.len("de"), 5);
        // A regional tag reaches the catalog its chain names, so coverage is reportable for it.
        assert_eq!(translations.len("de-DE"), 5);
        assert!(translations.has_locale("de-DE"));
        assert_eq!(translations.catalog_for("de-DE").as_deref(), Some("de"));
        assert_eq!(
            translations.resolve("pt-BR"),
            Language::Other("pt-br".to_owned())
        );
        assert_eq!(translations.resolve("fr"), Language::En);
        assert!(!translations.has_locale("fr"));
        assert!(!translations.is_empty());
        assert!(Translations::default().is_empty());
    }
}
