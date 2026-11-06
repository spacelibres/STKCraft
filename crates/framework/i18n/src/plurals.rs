//! Plural forms, by the CLDR cardinal rules.
//!
//! A count decides which form of a message a language uses, and languages disagree: English has
//! `one` and `other`, Russian has `one`, `few` and `many` (1 страница, 2 страницы, 5 страниц), and
//! Chinese has one form for every count. The shared catalog format stores a plural row as the
//! English `one|other` source and the locale's forms in CLDR order, so the only thing this module
//! decides is which form a count selects.
//!
//! # What is modelled
//!
//! The CLDR cardinal plural rules (Unicode CLDR, `plurals.xml`) for whole numbers. Fractions are
//! not modelled because the interface counts things — satellites, passes, files — and a fractional
//! count is a number to format, not a plural to choose. A locale whose rules this module does not
//! know gets English's: two forms, `one` for 1. That is the safe direction, because English is what
//! an untranslated interface shows anyway.
//!
//! # Adding a language
//!
//! Add its categories to [`plural_categories`] in CLDR order and its rule to [`plural_category`].
//! Neither can be checked mechanically against CLDR without carrying the CLDR data, so both carry
//! the rule in a comment and a test with the counts that would expose a mistake.

/// The CLDR cardinal plural categories a locale uses, in CLDR order.
///
/// This is the order a catalog's `@plural` translation lists its forms in, so the index of the
/// category a count selects is the index of the form to show.
#[must_use]
pub fn plural_categories(locale: &str) -> &'static [&'static str] {
    match language(locale) {
        // One form for every count.
        "zh" | "ja" | "ko" | "vi" | "th" | "id" | "ms" => &["other"],
        // `one` and `other`.
        "en" | "de" | "nl" | "es" | "it" | "pt" | "el" | "tr" | "fr" | "fi" | "et" | "hu"
        | "bg" | "ca" | "sv" | "da" | "nb" | "he" | "af" | "eu" | "sq" | "az" | "ka" => {
            &["one", "other"]
        }
        // `one`, `few`, `many`.
        "ru" | "uk" | "be" | "sr" | "hr" | "bs" | "pl" | "cs" | "sk" | "lt" | "lv" | "ro" => {
            &["one", "few", "many"]
        }
        "ar" => &["zero", "one", "two", "few", "many", "other"],
        // An unknown locale is served as English.
        _ => &["one", "other"],
    }
}

/// The language part of a locale code, lowercased (`pt-br` → `pt`).
fn language(locale: &str) -> &str {
    locale.split(['-', '_']).next().unwrap_or(locale)
}

/// The CLDR category a whole count selects in `locale`.
#[must_use]
pub fn plural_category(locale: &str, count: u64) -> &'static str {
    let count = i64::try_from(count).unwrap_or(i64::MAX);
    match language(locale) {
        // One form: every count is `other`.
        "zh" | "ja" | "ko" | "vi" | "th" | "id" | "ms" => "other",
        // The Slavic `one`/`few`/`many` split: `one` ends in 1 but not 11, `few` ends in 2–4 but
        // not 12–14, everything else is `many` (1 страница, 2 страницы, 5 страниц, 11 страниц).
        "ru" | "uk" | "be" | "sr" | "hr" | "bs" => {
            if count % 10 == 1 && count % 100 != 11 {
                "one"
            } else if (2..=4).contains(&(count % 10)) && !(12..=14).contains(&(count % 100)) {
                "few"
            } else {
                "many"
            }
        }
        // Polish agrees with Russian on `one` and differs on `few`: 22 is `few` in both, but 12 is
        // `many` in both as well — the difference is that Polish `many` covers 0 and 5–21.
        "pl" => {
            if count == 1 {
                "one"
            } else if (2..=4).contains(&(count % 10)) && !(12..=14).contains(&(count % 100)) {
                "few"
            } else {
                "many"
            }
        }
        // Czech and Slovak: `one` for 1, `few` for 2–4, `many` for everything else.
        "cs" | "sk" => match count {
            1 => "one",
            2..=4 => "few",
            _ => "many",
        },
        // Romanian: `one` for 1, `few` for 0 and 2–19 (and any count ending 01–19 above 100), else
        // `other`.
        "ro" => {
            if count == 1 {
                "one"
            } else if count == 0 || (2..=19).contains(&(count % 100)) {
                "few"
            } else {
                "other"
            }
        }
        // Arabic's six categories: 0, 1, 2, 3–10, 11–99, everything else.
        "ar" => match count {
            0 => "zero",
            1 => "one",
            2 => "two",
            3..=10 => "few",
            11..=99 => "many",
            _ => "other",
        },
        // French counts zero as singular ("0 page").
        "fr" => {
            if count <= 1 {
                "one"
            } else {
                "other"
            }
        }
        // Portuguese and Spanish count zero as plural.
        _ => {
            if count == 1 {
                "one"
            } else {
                "other"
            }
        }
    }
}

/// The index into a locale's `@plural` forms for `count`.
///
/// The forms are stored in CLDR order, so this is the category's position in
/// [`plural_categories`]. A category the list does not hold falls back to its last form, which is
/// the one every language has (`other`).
#[must_use]
pub fn plural_form_index(locale: &str, count: u64) -> usize {
    let categories = plural_categories(locale);
    let category = plural_category(locale, count);
    categories
        .iter()
        .position(|candidate| *candidate == category)
        .unwrap_or(categories.len().saturating_sub(1))
}

/// The index into an English `one|other` source for `count`.
///
/// The source column of a plural row is English, so the source side always uses English's rule: 1
/// is singular and everything else is plural.
#[must_use]
pub fn english_form_index(count: u64) -> usize {
    usize::from(count != 1)
}

#[cfg(test)]
#[allow(clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn languages_with_one_form_never_change() {
        for locale in ["zh", "ja", "ko", "vi"] {
            assert_eq!(plural_categories(locale), &["other"], "{locale}");
            for count in [0, 1, 2, 5, 21, 1000] {
                assert_eq!(plural_form_index(locale, count), 0, "{locale} {count}");
            }
        }
    }

    #[test]
    fn english_and_its_relatives_split_at_one() {
        for locale in ["en", "de", "es", "nl", "it", "pt", "el", "tr"] {
            assert_eq!(plural_form_index(locale, 1), 0, "{locale}");
            assert_eq!(plural_form_index(locale, 0), 1, "{locale}");
            assert_eq!(plural_form_index(locale, 2), 1, "{locale}");
            assert_eq!(plural_form_index(locale, 21), 1, "{locale}");
        }
    }

    #[test]
    fn russian_distinguishes_one_few_and_many() {
        // 1 страница, 2 страницы, 5 страниц, 11 страниц, 21 страница, 22 страницы.
        assert_eq!(plural_category("ru", 1), "one");
        assert_eq!(plural_category("ru", 2), "few");
        assert_eq!(plural_category("ru", 4), "few");
        assert_eq!(plural_category("ru", 5), "many");
        assert_eq!(plural_category("ru", 11), "many");
        assert_eq!(plural_category("ru", 21), "one");
        assert_eq!(plural_category("ru", 22), "few");
        assert_eq!(plural_category("ru", 111), "many");
        assert_eq!(plural_form_index("ru", 3), 1);
    }

    #[test]
    fn polish_and_czech_are_not_russians_rule_copied() {
        // Czech: 1, 2–4, everything else — 21 is `many`, where Russian says `one`.
        assert_eq!(plural_category("cs", 21), "many");
        assert_eq!(plural_category("ru", 21), "one");
        // Polish: 12 and 22 differ in form even though both end in 2.
        assert_eq!(plural_category("pl", 22), "few");
        assert_eq!(plural_category("pl", 12), "many");
        assert_eq!(plural_category("pl", 1), "one");
    }

    #[test]
    fn romanian_and_arabic_have_their_own_splits() {
        assert_eq!(plural_category("ro", 1), "one");
        assert_eq!(plural_category("ro", 0), "few");
        assert_eq!(plural_category("ro", 19), "few");
        assert_eq!(plural_category("ro", 20), "other");
        assert_eq!(plural_category("ar", 0), "zero");
        assert_eq!(plural_category("ar", 2), "two");
        assert_eq!(plural_category("ar", 3), "few");
        assert_eq!(plural_category("ar", 11), "many");
        assert_eq!(plural_category("ar", 100), "other");
        assert_eq!(plural_categories("ar").len(), 6);
    }

    #[test]
    fn french_counts_zero_as_singular() {
        assert_eq!(plural_category("fr", 0), "one");
        assert_eq!(plural_category("fr", 1), "one");
        assert_eq!(plural_category("fr", 2), "other");
    }

    #[test]
    fn an_unknown_locale_falls_back_to_englishs_rule() {
        assert_eq!(plural_categories("xx"), &["one", "other"]);
        assert_eq!(plural_form_index("xx", 1), 0);
        assert_eq!(plural_form_index("xx", 2), 1);
    }

    #[test]
    fn a_regional_code_uses_its_language() {
        assert_eq!(plural_category("pt-br", 1), "one");
        assert_eq!(plural_category("ru-ru", 2), "few");
    }

    #[test]
    fn the_english_source_index_is_one_or_other() {
        assert_eq!(english_form_index(1), 0);
        assert_eq!(english_form_index(0), 1);
        assert_eq!(english_form_index(2), 1);
    }
}
