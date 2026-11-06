//! Interface translations, in the format the Craft apps share.
//!
//! An interface string is written in English in the code and translated by a catalog: a plain-text
//! table of `context <TAB> English <TAB> translation` rows, one per line. **English is the key.**
//! There is no English catalog and no symbol table to keep in step, so the source text a
//! translator sees is exactly the text the interface shows, and a string that nothing translates
//! simply appears in English.
//!
//! ```text
//! # Columns, separated by a TAB:  context <TAB> English source text <TAB> translation
//! <TAB>Save<TAB>Speichern
//! menu<TAB>Open<TAB>Öffnen
//! @plural<TAB>{n} page|{n} pages<TAB>{n} Seite|{n} Seiten
//! @id<TAB>scenario.rename<TAB>Szenario umbenennen
//! ```
//!
//! # Why this shape
//!
//! It is the `ArtCraft` family's format, not this project's invention: `storytold/craft-i18n` holds
//! the shared catalogs, arbitrates the rules CI enforces, and the other Craft apps read the same
//! tables. A translation written here can be contributed there, and a rule that disagrees with the
//! shared checker is a bug in this crate rather than a local preference. See `docs/standards.md`
//! and `docs/guides/localization.md`.
//!
//! The rules, in short:
//!
//! - **Contexts.** Usually empty. A plain word (`menu`, `preview`) tells apart two meanings of the
//!   same English, and only `@plural`, `@id` and `@msg` mean something to the code.
//! - **Placeholders.** `{name}` in the source must appear in the translation, in any order, and a
//!   trailing `…` in one needs one in the other. A translation that drops a placeholder produces a
//!   sentence that cannot be filled in, which is worse than showing English.
//! - **Plurals.** An `@plural` row's source is `one|other`; its translation lists the locale's
//!   forms in CLDR order. [`translations::Translations::trn`] chooses the form.
//! - **Partial catalogs are fine.** A missing row is not an error anywhere in this crate.
//!
//! # What this crate is not
//!
//! No network, no filesystem, no `unsafe`, and no panics: catalogs are parsed from text a caller
//! read, and every problem is reported with the line that caused it. The `DeepSeek` translation that
//! fills catalogs lives in `xtask`, above the app, because a translation request is a network call
//! and the kernel must not have one.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
#![forbid(unsafe_code)]

pub mod catalog;
pub mod extract;
pub mod locale;
pub mod plurals;
pub mod translations;

pub use catalog::{Catalog, Problem, Row, parse};
pub use extract::Source;
pub use locale::{
    LANGUAGE_NAME_CONTEXT, LANGUAGE_NAME_SOURCE, Language, language_name_row, locale_chain,
    parse_system_locale,
};
pub use plurals::{plural_categories, plural_form_index};
pub use translations::{Embedded, Translations};

/// The locale of the source language, which has no catalog of its own.
pub const ENGLISH: &str = "en";

/// Translate an interface string: `tr!(translations, locale, "Save")`.
///
/// The macro exists so that every user-visible string is *findable*: `cargo xtask i18n` extracts the
/// English strings the code passes to `tr!`, `trc!` and `trn!`, and a string built at run time is a
/// string no catalog can be checked against. The first two arguments are the [`Translations`] and the
/// locale; only the literals are the source text.
///
/// [`Translations`]: translations::Translations
#[macro_export]
macro_rules! tr {
    ($translations:expr, $locale:expr, $source:literal) => {
        $crate::Translations::tr($translations, $locale, $source)
    };
}

/// Translate a string that only makes sense in a context: `trc!(translations, locale, "menu", "Open")`.
///
/// The context tells apart two meanings of one English word, which is the format's answer to a
/// catalog keyed by the text a reader sees.
#[macro_export]
macro_rules! trc {
    ($translations:expr, $locale:expr, $context:literal, $source:literal) => {
        $crate::Translations::tr_context($translations, $locale, $context, $source)
    };
}

/// Translate a count-dependent message: `trn!(translations, locale, "{n} page", "{n} pages", count)`.
#[macro_export]
macro_rules! trn {
    ($translations:expr, $locale:expr, $one:literal, $other:literal, $count:expr) => {
        $crate::Translations::trn($translations, $locale, $one, $other, $count)
    };
}
