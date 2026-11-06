//! The interface font stack, from the `ArtCraft` family's shared UI library.
//!
//! `egui` draws a character only if one of its installed faces has it. It ships a Latin face and
//! nothing else, so a Chinese, Japanese, Korean, Arabic or Thai label renders as tofu boxes — which
//! is exactly what a translated interface did here before this module existed.
//!
//! `craft_ui::fonts::UiFonts` is the family's answer and every Craft app installs it: the app's own
//! faces, then the shipped faces for the interface language and the other scripts, then **the fonts
//! installed on the machine**, then emoji and egui's defaults. Because the installed fonts are
//! searched, a CJK interface renders on any machine that has a CJK font — which every desktop with
//! the language installed does — and nothing is bundled or downloaded.
//!
//! ```text
//! app faces → language faces → other scripts → installed fonts → emoji → egui defaults
//! ```
//!
//! # What `STKCraft` supplies
//!
//! Only the two things the family's library asks an app for: the scripts its languages are written
//! in, and which language is on screen. [`Scripts`] is the first — a table rather than
//! `craft_i18n::scripts`, because this project's catalog covers fewer languages and does not want
//! ICU4X in the build to describe them — and [`install`] takes the second.
//!
//! Re-installing on a language change is cheap: the family's stack reuses the faces already in the
//! atlas, and the Han faces are reordered so a Japanese interface gets Japanese glyph shapes.

use eframe::egui;
use egui::FontFamily;

/// The scripts `STKCraft`'s interface languages are written in, by ISO 15924 code.
///
/// The first entry for a language is its main script. A language missing from this table is served
/// as Latin, which is what its interface falls back to when no catalog exists.
#[derive(Debug, Clone, Copy, Default)]
pub struct Scripts;

impl craft_ui::fonts::LanguageScripts for Scripts {
    fn scripts(&self, language: &str) -> Vec<&'static str> {
        let language = language.split(['-', '_']).next().unwrap_or(language);
        match language {
            "ar" => vec!["Arab"],
            "cs" | "de" | "en" | "es" | "fr" | "it" | "nl" | "pl" | "pt" | "ro" | "tr" | "vi" => {
                vec!["Latn"]
            }
            "el" => vec!["Grek"],
            "ja" => vec!["Hani", "Hira", "Kana"],
            "ko" => vec!["Hang", "Hani"],
            "ru" | "uk" => vec!["Cyrl"],
            "sr" => vec!["Cyrl", "Latn"],
            "zh" => vec!["Hani", "Bopo"],
            _ => vec!["Latn"],
        }
    }

    fn han(&self, language: &str) -> Option<craft_ui::fonts::Han> {
        let language = language.split(['-', '_']).next().unwrap_or(language);
        match language {
            // Han glyph shapes differ by region and a reader notices: Simplified for Chinese,
            // Traditional for Hong Kong and Taiwan, Japanese order for Japanese.
            "zh" => Some(craft_ui::fonts::Han::Simplified),
            "ja" => Some(craft_ui::fonts::Han::Japanese),
            _ => None,
        }
    }
}

/// Every language this build can show, so the stack covers all of their scripts rather than only the
/// one on screen: a Japanese interface must still draw an Arabic file name.
fn languages(interface: &str) -> Vec<String> {
    let mut languages: Vec<String> = crate::prefs::OFFERED
        .iter()
        .map(|language| (*language).to_owned())
        .collect();
    if !languages.iter().any(|language| language == "en") {
        languages.push("en".to_owned());
    }
    if !languages.iter().any(|language| language == interface) {
        languages.push(interface.to_owned());
    }
    languages
}

/// Install the family font stack for `interface`, the language on screen.
///
/// Called once at startup and again whenever the language changes — the Han faces are ordered by
/// language, so switching from English to Japanese changes which face draws a shared kanji.
pub fn install(ctx: &egui::Context, interface: &str) {
    let scripts = Scripts;
    let fonts = craft_ui::fonts::UiFonts::new(&scripts)
        .ui_language(interface)
        .languages(&languages(interface))
        // Installed fonts come after the shipped faces and before egui's defaults, which is the
        // family's order: a face the family ships wins, and a script it does not cover comes from
        // the machine. `STKCRAFT_FONTS=off` turns them off, which is the family's reproducible mode
        // and the way to see what a label looks like as tofu when a report claims it does.
        .system_fonts(system_fonts());
    let stack = fonts.install(ctx);
    report(&stack);
}

/// Which installed-font policy the stack uses.
///
/// Installed fonts are what make a CJK interface draw at all, so they are on by default. `off` exists
/// because a screenshot comparison needs a machine-independent answer, and because "is this tofu or a
/// real bug?" is a question that has to be answerable without guessing.
fn system_fonts() -> craft_ui::fonts::SystemFonts {
    match std::env::var("STKCRAFT_FONTS").as_deref() {
        Ok("off") => craft_ui::fonts::SystemFonts::Off,
        _ => craft_ui::fonts::SystemFonts::AfterCraft,
    }
}

/// How many faces the stack ended up with, on stderr.
///
/// A one-line answer to "why is my text tofu": if the installed-font faces are missing, the machine
/// had none that matched, and the log says so rather than leaving someone to guess.
fn report(stack: &craft_ui::fonts::Stack) {
    let proportional = stack
        .definitions
        .families
        .get(&FontFamily::Proportional)
        .map_or(0, Vec::len);
    let monospace = stack
        .definitions
        .families
        .get(&FontFamily::Monospace)
        .map_or(0, Vec::len);
    eprintln!(
        "fonts: {proportional} face(s) for proportional text, {monospace} for monospace, \
         {} face(s) loaded in all",
        stack.definitions.font_data.len()
    );
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use craft_ui::fonts::LanguageScripts;

    /// The characters in a catalog's translations that the installed stack cannot draw.
    fn undrawable(ctx: &egui::Context, locale: &str, catalog: &str) -> Vec<String> {
        install(ctx, locale);
        let stack = craft_ui::fonts::UiFonts::new(&Scripts)
            .ui_language(locale)
            .languages(&languages(locale))
            .system_fonts(system_fonts())
            .stack();
        let samples: Vec<craft_ui::fonts::Sample<'_>> = catalog
            .lines()
            .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
            .filter_map(|line| {
                let mut columns = line.split('\t');
                columns.next()?;
                columns.next()?;
                let translation = columns.next()?;
                Some(craft_ui::fonts::Sample {
                    language: locale,
                    key: "catalog",
                    text: translation,
                })
            })
            .collect();
        craft_ui::fonts::missing_glyphs(&stack, &FontFamily::Proportional, samples, true)
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    #[test]
    fn the_font_stack_can_draw_the_shipped_catalogs() {
        // The check the family runs over its translations, over ours: every character a shipped
        // catalog contains must be drawable. A failure names the character and the string, which is
        // the difference between "the Chinese looks wrong" and a bug report somebody can act on.
        let mut checked = 0;
        for (locale, text) in crate::catalogs::EMBEDDED {
            let ctx = egui::Context::default();
            let missing = undrawable(&ctx, locale, text);
            assert!(
                missing.is_empty(),
                "`{locale}` has characters no interface font can draw: {missing:?}"
            );
            checked += 1;
        }
        assert!(checked > 0, "no catalog was checked");
    }

    #[test]
    fn every_offered_language_names_its_scripts() {
        for language in crate::prefs::OFFERED {
            let scripts = Scripts.scripts(language);
            assert!(!scripts.is_empty(), "{language} has no script");
            assert!(
                scripts.iter().all(|script| script.len() == 4),
                "{language} names a script that is not ISO 15924: {scripts:?}"
            );
        }
        assert_eq!(Scripts.scripts("zh-CN"), vec!["Hani", "Bopo"]);
        assert_eq!(Scripts.scripts("ja"), vec!["Hani", "Hira", "Kana"]);
        assert_eq!(Scripts.scripts("ar"), vec!["Arab"]);
    }

    #[test]
    fn han_shapes_follow_the_language() {
        // A shared kanji draws differently in a Japanese and a Chinese interface, and the family's
        // stack picks the face from this answer.
        assert_eq!(Scripts.han("ja"), Some(craft_ui::fonts::Han::Japanese));
        assert_eq!(Scripts.han("zh"), Some(craft_ui::fonts::Han::Simplified));
        assert_eq!(Scripts.han("ru"), None);
    }

    #[test]
    fn the_interface_language_is_always_among_the_covered_ones() {
        let languages = languages("zh");
        assert!(languages.iter().any(|language| language == "zh"));
        assert!(languages.iter().any(|language| language == "en"));
        assert!(languages.iter().any(|language| language == "ja"));
    }
}
