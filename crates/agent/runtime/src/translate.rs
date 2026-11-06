//! Translating a batch of interface strings with a model.
//!
//! One request, one JSON answer, one validation pass — the same work whether it is `cargo xtask
//! i18n translate` filling a catalog in a checkout or a window translating itself for the person
//! using it. Two implementations would be two prompt wordings and two acceptance rules, and the one
//! that drifts is always the one a user meets.
//!
//! # What it will not do
//!
//! It refuses a translation it cannot check: a dropped or invented `{name}`, a plural row with the
//! wrong number of forms for the locale, an ellipsis that went missing, an empty string. A row that
//! fails is reported and left untranslated — English is a correct answer, and a translation nobody
//! can trust is not.
//!
//! # The transport
//!
//! It goes through [`ChatModel`], so a scripted model exercises everything below the network and a
//! test needs no key. The real one is [`DeepSeek`](crate::DeepSeek), which speaks `curl`.

use stkcraft_framework_i18n::Source;
use stkcraft_framework_i18n::catalog::{self, Row};

use crate::{ChatModel, Message};

/// How many strings go into one request.
///
/// A row is a sentence and the answer is JSON, so a batch of this size is a document a model handles
/// comfortably; larger batches risk a truncated answer, which loses the whole batch rather than one
/// row.
pub const BATCH_SIZE: usize = 60;

/// What one request produced: the rows that passed, the strings that did not with the reason, and
/// the language's name as the model wrote it.
type Batch = (Vec<(Source, String)>, Vec<(String, String)>, String);

/// What one translation run produced.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Catalogue {
    /// The language's name in that language, as the model wrote it.
    pub language_name: String,
    /// The rows it translated and that passed validation.
    pub rows: Vec<Row>,
    /// The strings it could not translate, with the reason, so a caller can report them.
    pub refused: Vec<(String, String)>,
}

impl Catalogue {
    /// Whether nothing was translated.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// Translate every source string that has no row yet.
///
/// `existing` is what the catalog already holds, so a second run sends only what is missing rather
/// than paying for the whole interface again.
///
/// # Errors
///
/// A message when the model cannot be reached or answers something unusable. Rows that fail
/// *validation* are not an error: they are reported in [`Catalogue::refused`], because one bad row
/// must not throw away the good ones beside it.
pub fn translate_catalogue<M: ChatModel>(
    model: &M,
    locale: &str,
    language_name: &str,
    sources: &[Source],
    existing: &[Row],
) -> Result<Catalogue, String> {
    let wanted: Vec<&Source> = sources
        .iter()
        .filter(|source| {
            !existing
                .iter()
                .any(|row| row.context == source.context && row.source == source.text)
        })
        .collect();
    let mut catalogue = Catalogue {
        language_name: language_name.to_owned(),
        ..Catalogue::default()
    };
    if wanted.is_empty() {
        return Ok(catalogue);
    }
    for chunk in wanted.chunks(BATCH_SIZE) {
        let (batch, refused, name) =
            translate_batch(model, locale, &catalogue.language_name, chunk)?;
        if !name.is_empty() {
            catalogue.language_name = name;
        }
        for (source, translation) in batch {
            catalogue.rows.push(Row {
                context: source.context,
                source: source.text,
                translation,
            });
        }
        catalogue.refused.extend(refused);
    }
    Ok(catalogue)
}

/// One request: the prompt, the answer, the rows that survived validation, and why the rest did not.
fn translate_batch<M: ChatModel>(
    model: &M,
    locale: &str,
    language_name: &str,
    sources: &[&Source],
) -> Result<Batch, String> {
    let prompt = prompt(locale, language_name, sources);
    let turn = model.complete(
        &[Message::System(SYSTEM.to_owned()), Message::User(prompt)],
        &[],
    )?;
    let json = json_object(&turn.text)
        .ok_or_else(|| format!("the model's answer is not a JSON object: {}", turn.text))?;
    let mut rows = Vec::new();
    let mut refused = Vec::new();
    for source in sources {
        let Some(translation) = json.get(&source.text).and_then(serde_json::Value::as_str) else {
            refused.push((
                source.text.clone(),
                "the model did not answer for this string".to_owned(),
            ));
            continue;
        };
        match check_row(source, translation, locale) {
            Ok(()) => rows.push(((*source).clone(), translation.to_owned())),
            Err(reason) => refused.push((source.text.clone(), reason)),
        }
    }
    let name = json
        .get(stkcraft_framework_i18n::LANGUAGE_NAME_SOURCE)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned();
    Ok((rows, refused, name))
}

/// The instruction the model works from.
const SYSTEM: &str = "You translate software interfaces. You answer with JSON only, exactly as \
                      asked.";

/// The request text.
fn prompt(locale: &str, language_name: &str, sources: &[&Source]) -> String {
    let items: Vec<serde_json::Value> = sources
        .iter()
        .map(|source| {
            serde_json::json!({
                "context": source.context,
                "english": source.text,
                "placeholders": catalog::placeholders(&source.text),
                "plural": source.context == "@plural",
            })
        })
        .collect();
    format!(
        "Translate STKCraft's interface strings into the language with the BCP 47 code `{locale}` \
         (called `{language_name}` in that language).\n\
         \n\
         The input is a JSON array. Each item has `context` (usually empty), `english` (the source \
         text), `placeholders` (the `{{name}}` tokens it contains) and `plural` (whether the source \
         is English `one|other` and the translation must list this language's CLDR plural forms in \
         order, `|`-separated: 1 form for Chinese, Japanese, Korean or Vietnamese, 2 for \
         English-like languages, 3 for Russian, Czech, Polish or Romanian, 6 for Arabic).\n\
         \n\
         Rules: keep every placeholder exactly as it is; keep a trailing `…` if the English has one; \
         do not translate command ids or acronyms; write short interface labels, not explanations; \
         use the language's own script.\n\
         \n\
         Answer with JSON only, no prose: an object whose keys are the exact `english` strings and \
         whose values are the translations. Also include the key `{name}` with this language's name \
         written in that language.\n\
         \n\
         Input:\n{input}",
        name = stkcraft_framework_i18n::LANGUAGE_NAME_SOURCE,
        input = serde_json::to_string_pretty(&items).unwrap_or_else(|_| "[]".to_owned())
    )
}

/// The first JSON object in a model's answer, tolerating a fenced code block.
///
/// A model that wraps its JSON in prose is a model whose answer is still usable, and refusing it
/// would throw away a whole batch over a formality.
#[must_use]
pub fn json_object(text: &str) -> Option<serde_json::Value> {
    let trimmed = text.trim();
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    if end < start {
        return None;
    }
    serde_json::from_str(&trimmed[start..=end]).ok()
}

/// Whether one proposed row passes the catalog's own checks.
///
/// The same parser the gate uses, given one row: a proposal is not a row until the format accepts it,
/// and the format is what a reader of the catalog will see.
fn check_row(source: &Source, translation: &str, locale: &str) -> Result<(), String> {
    if translation.trim().is_empty() {
        return Err("the translation is empty".to_owned());
    }
    let row = Row {
        context: source.context.clone(),
        source: source.text.clone(),
        translation: translation.to_owned(),
    };
    let text = catalog::row_text(&row);
    let (_, problems) = catalog::parse(locale, &text);
    match problems.first() {
        None => Ok(()),
        Some(problem) => Err(problem.message.clone()),
    }
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// A model that answers from a script: `Ok(text)` or `Err(message)`.
    struct Script {
        answers: RefCell<Vec<Result<String, String>>>,
        seen: RefCell<Vec<Vec<Message>>>,
    }

    impl Script {
        fn new(answers: Vec<Result<String, String>>) -> Self {
            Self {
                answers: RefCell::new(answers),
                seen: RefCell::new(Vec::new()),
            }
        }
    }

    impl ChatModel for Script {
        fn complete(
            &self,
            messages: &[Message],
            _tools: &[serde_json::Value],
        ) -> Result<crate::Turn, String> {
            self.seen.borrow_mut().push(messages.to_vec());
            let mut answers = self.answers.borrow_mut();
            if answers.is_empty() {
                return Ok(crate::Turn::default());
            }
            match answers.remove(0) {
                Ok(text) => Ok(crate::Turn {
                    text,
                    calls: Vec::new(),
                }),
                Err(message) => Err(message),
            }
        }
    }

    fn source(context: &str, text: &str) -> Source {
        Source {
            context: context.to_owned(),
            text: text.to_owned(),
        }
    }

    #[test]
    fn a_batch_is_translated_and_the_name_comes_back() {
        let model = Script::new(vec![Ok(
            r#"{"Save":"Speichern","language.name":"Deutsch"}"#.to_owned(),
        )]);
        let catalogue = translate_catalogue(&model, "de", "de", &[source("", "Save")], &[])
            .expect("a translation");
        assert_eq!(catalogue.language_name, "Deutsch");
        assert_eq!(catalogue.rows.len(), 1);
        assert_eq!(catalogue.rows[0].translation, "Speichern");
        assert_eq!(catalogue.refused, Vec::new());
        // The prompt carries the locale and the plural rule, and nothing that was already translated.
        let prompt = match &model.seen.borrow()[0][1] {
            Message::User(text) => text.clone(),
            other => panic!("expected a user message, got {other:?}"),
        };
        assert!(prompt.contains("`de`"), "{prompt}");
        assert!(prompt.contains("CLDR"), "{prompt}");
    }

    #[test]
    fn an_existing_row_is_not_sent_again() {
        let model = Script::new(vec![Ok(r#"{"Open":"Öffnen"}"#.to_owned())]);
        let existing = vec![Row {
            context: String::new(),
            source: "Save".to_owned(),
            translation: "Speichern".to_owned(),
        }];
        let catalogue = translate_catalogue(
            &model,
            "de",
            "Deutsch",
            &[source("", "Save"), source("", "Open")],
            &existing,
        )
        .expect("a translation");
        assert_eq!(catalogue.rows.len(), 1);
        assert_eq!(catalogue.rows[0].source, "Open");
        let prompt = match &model.seen.borrow()[0][1] {
            Message::User(text) => text.clone(),
            other => panic!("expected a user message, got {other:?}"),
        };
        assert!(
            !prompt.contains("\"Save\""),
            "an existing row was sent again: {prompt}"
        );
    }

    #[test]
    fn a_row_that_breaks_the_format_is_refused_and_the_others_are_kept() {
        // The interesting failure: a model drops a placeholder. Keeping the row would ship an
        // interface sentence with a hole, so it is dropped and named.
        let model = Script::new(vec![Ok(
            r#"{"Added {name}":"Hinzugefügt","Save":"Speichern"}"#.to_owned(),
        )]);
        let catalogue = translate_catalogue(
            &model,
            "de",
            "Deutsch",
            &[source("", "Added {name}"), source("", "Save")],
            &[],
        )
        .expect("a translation");
        assert_eq!(catalogue.rows.len(), 1, "{:?}", catalogue.rows);
        assert_eq!(catalogue.rows[0].source, "Save");
        assert_eq!(catalogue.refused.len(), 1);
        assert_eq!(catalogue.refused[0].0, "Added {name}");
    }

    #[test]
    fn a_plural_row_has_to_list_the_locales_forms() {
        let plural = source("@plural", "{n} page|{n} pages");
        // Chinese takes one form.
        let model = Script::new(vec![Ok(r#"{"{n} page|{n} pages":"{n} 页"}"#.to_owned())]);
        let catalogue =
            translate_catalogue(&model, "zh", "中文", std::slice::from_ref(&plural), &[])
                .expect("a translation");
        assert_eq!(catalogue.rows.len(), 1, "{catalogue:?}");
        // English takes two, so one form is refused.
        let model = Script::new(vec![Ok(r#"{"{n} page|{n} pages":"{n} Seite"}"#.to_owned())]);
        let catalogue =
            translate_catalogue(&model, "de", "Deutsch", &[plural], &[]).expect("a translation");
        assert_eq!(catalogue.rows, Vec::new());
        assert_eq!(catalogue.refused.len(), 1);
    }

    #[test]
    fn a_model_that_fails_is_an_error_and_not_an_empty_success() {
        let model = Script::new(vec![Err("the model refused the request: 400".to_owned())]);
        let error = translate_catalogue(&model, "de", "Deutsch", &[source("", "Save")], &[])
            .expect_err("a failure is a failure");
        assert!(error.contains("400"), "{error}");
    }

    #[test]
    fn an_answer_wrapped_in_prose_is_still_json() {
        let value =
            json_object("Here you go:\n```json\n{\"Save\":\"Speichern\"}\n```").expect("json");
        assert_eq!(value["Save"], "Speichern");
        assert!(json_object("no json here").is_none());
    }

    #[test]
    fn nothing_missing_means_no_request_at_all() {
        let model = Script::new(vec![]);
        let existing = vec![Row {
            context: String::new(),
            source: "Save".to_owned(),
            translation: "Speichern".to_owned(),
        }];
        let catalogue =
            translate_catalogue(&model, "de", "Deutsch", &[source("", "Save")], &existing)
                .expect("nothing to do");
        assert!(catalogue.is_empty());
        assert!(
            model.seen.borrow().is_empty(),
            "a request was made with nothing to ask"
        );
    }
}
