//! Turning a document into searchable tokens.
//!
//! Two things have to be true of a tokenizer here. It must be **deterministic** — the same text
//! gives the same tokens on every machine, or a ranking test means nothing — and it must handle
//! **Chinese and Japanese**, where words are not separated by spaces and a whitespace tokenizer
//! would index a whole sentence as one token and match nothing.
//!
//! So: ASCII words are lowercased runs of letters, digits and the punctuation inside a number
//! (`0.5`, `1e-3`, `24 h`), and a run of Han, kana or Hangul characters is indexed as **character
//! bigrams** as well as unigrams. Bigrams are the standard cheap answer for languages without
//! spaces: 万向接头 is indexed as 万, 万向, 向, 向接, 接, 接头, 头, so a query for 万向 finds it and a
//! query for an unrelated pair does not.

/// One token, and whether it is punctuation-only — used to skip it when ranking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// The token text, lowercased and normalized.
    pub text: String,
}

/// Every token in `text`, in order, including repeats (a term's frequency is what ranking uses).
#[must_use]
pub fn tokens(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut word = String::new();
    let mut cjk_run: Vec<char> = Vec::new();
    for character in text.chars() {
        if is_cjk(character) {
            flush_word(&mut word, &mut tokens);
            cjk_run.push(character);
            continue;
        }
        if !cjk_run.is_empty() {
            flush_cjk(&mut cjk_run, &mut tokens);
        }
        if character.is_alphanumeric() || is_word_joiner(character) {
            // A hyphen or a decimal point inside a word keeps it one token; a trailing one is not
            // part of it, which `flush_word` trims.
            word.push(character.to_ascii_lowercase());
        } else {
            flush_word(&mut word, &mut tokens);
        }
    }
    flush_word(&mut word, &mut tokens);
    flush_cjk(&mut cjk_run, &mut tokens);
    tokens
}

/// Function words that carry no evidence on their own.
///
/// Dropped from a **query**, never from an index: a document's "the" still counts towards a term's
/// document frequency, where it correctly pushes the term's weight towards zero. Dropping them from
/// the query is what stops "the" from matching every passage in a small library, where the
/// frequency table is too short for the weight to fall on its own.
const STOPWORDS: [&str; 46] = [
    "a", "an", "and", "are", "as", "at", "be", "by", "for", "from", "has", "have", "how", "in",
    "into", "is", "it", "its", "of", "on", "or", "that", "the", "their", "then", "there", "these",
    "they", "this", "to", "was", "were", "what", "when", "which", "who", "why", "will", "with",
    "would", "you", "your", "about", "de", "la", "el",
];

/// The tokens a query is scored with: the text's tokens, minus function words.
///
/// An empty result means the query says nothing a library could answer — "the and of" is not a
/// question — and the caller reports that as a miss rather than handing the model noise.
#[must_use]
pub fn query_tokens(text: &str) -> Vec<String> {
    tokenize(text)
        .into_iter()
        .filter(|token| !STOPWORDS.contains(&token.as_str()))
        .collect()
}

/// The token texts alone, which is what an index stores.
#[must_use]
pub fn tokenize(text: &str) -> Vec<String> {
    tokens(text).into_iter().map(|token| token.text).collect()
}

/// Whether `character` is written without spaces between words.
fn is_cjk(character: char) -> bool {
    matches!(u32::from(character),
        0x3040..=0x30FF        // Hiragana, Katakana
        | 0x3400..=0x4DBF      // CJK extension A
        | 0x4E00..=0x9FFF      // CJK unified ideographs
        | 0xAC00..=0xD7AF      // Hangul syllables
        | 0xF900..=0xFAFF      // CJK compatibility ideographs
        | 0x20000..=0x2FA1F    // CJK extensions B–F
    )
}

/// Characters that may sit inside a word: a decimal point, an exponent sign and a hyphen between
/// letters or digits (`SGP4`, `1e-3`, `WGS-84`).
fn is_word_joiner(character: char) -> bool {
    matches!(character, '.' | '-' | '+' | '_' | '/' | '\'')
}

/// Finish the current ASCII word.
fn flush_word(word: &mut String, tokens: &mut Vec<Token>) {
    let trimmed = word.trim_matches(|character: char| !character.is_alphanumeric());
    if !trimmed.is_empty() {
        tokens.push(Token {
            text: trimmed.to_owned(),
        });
    }
    word.clear();
}

/// Emit a run of CJK characters as unigrams and bigrams.
fn flush_cjk(run: &mut Vec<char>, tokens: &mut Vec<Token>) {
    if run.is_empty() {
        return;
    }
    for (index, character) in run.iter().enumerate() {
        tokens.push(Token {
            text: character.to_string(),
        });
        if let Some(next) = run.get(index + 1) {
            let mut bigram = String::with_capacity(8);
            bigram.push(*character);
            bigram.push(*next);
            tokens.push(Token { text: bigram });
        }
    }
    run.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latin_words_are_lowercased_and_punctuation_is_a_separator() {
        assert_eq!(
            tokenize("The maximum OFF-NADIR angle is 45°."),
            vec!["the", "maximum", "off-nadir", "angle", "is", "45"]
        );
    }

    #[test]
    fn a_number_keeps_its_decimal_point_and_exponent() {
        // A specification's numbers are the reason this library exists: `1e-3` and `1E-3` are one
        // token, and `0.5` is not two.
        assert_eq!(
            tokenize("sigma 1e-3, rate 0.5 Hz"),
            vec!["sigma", "1e-3", "rate", "0.5", "hz"]
        );
    }

    #[test]
    fn chinese_is_indexed_as_unigrams_and_bigrams() {
        // Without this, a Chinese sentence is one token and nothing matches.
        let tokens = tokenize("万向接头");
        assert_eq!(tokens, vec!["万", "万向", "向", "向接", "接", "接头", "头"]);
    }

    #[test]
    fn mixed_text_separates_scripts() {
        assert_eq!(
            tokenize("Titan 半径 2575 km"),
            vec!["titan", "半", "半径", "径", "2575", "km"]
        );
    }

    #[test]
    fn whitespace_and_empty_input_yield_nothing() {
        assert_eq!(tokenize("   \n\t "), Vec::<String>::new());
        assert_eq!(tokenize(""), Vec::<String>::new());
        assert_eq!(tokenize("--- ... ///"), Vec::<String>::new());
    }

    #[test]
    fn a_trailing_hyphen_is_not_part_of_the_word() {
        assert_eq!(tokenize("well- known"), vec!["well", "known"]);
        assert_eq!(tokenize("state-of-the-art"), vec!["state-of-the-art"]);
    }
}
