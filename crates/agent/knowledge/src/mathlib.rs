//! Mathlib as a lexicon: the declarations the formula classifier scores against.
//!
//! `docs/design/mathematics.md` states the intent — a formula is classified against Mathlib, the largest
//! machine-readable body of mathematics under a licence this project can use — and this module is the
//! part of that which is ours: **reading Lean source into a table of names, statements and purposes**.
//!
//! Nothing of Lean is linked and no proof is checked. A row is:
//!
//! ```text
//! name          Real.sqrt            the declaration, as Lean names it
//! kind          def | theorem | ...  what sort of thing it is
//! module        Mathlib/Analysis/…   the file, which is also the taxonomy
//! domain        Analysis             the first directory under Mathlib/, for listing and filtering
//! statement     the type, as written  what it says
//! docstring     the `/-- … -/` above it, when the source has one, and elided
//! digest        SHA-256 of the declaration's text, so a re-run can prove it read the same source
//! ```
//!
//! # What the parser is, and is not
//!
//! It is a **line-oriented reader of the declaration headers**, not a Lean parser: it recognises the
//! keywords that begin a declaration at the start of a line, keeps the docstring that precedes it, records
//! the module path it was found in, and hashes the first line of the statement. Lean's syntax allows a
//! declaration to continue for a hundred lines; this records the opening, which is what a classifier can
//! match a formula's operators against, and says so rather than pretending to understand the body.

use std::path::Path;

/// One declaration, as the lexicon keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    /// The name Lean gives it, e.g. `Real.sqrt`.
    pub name: String,
    /// `def`, `theorem`, `lemma`, `structure`, `class`, `instance`, `abbrev`.
    pub kind: String,
    /// The file it lives in, relative to the repository root.
    pub module: String,
    /// The first directory under `Mathlib/`: `Analysis`, `Algebra`, `Topology`, …
    pub domain: String,
    /// The statement, as written, up to the `:=` or the end of the line.
    pub statement: String,
    /// Its docstring, with the `/--` and `-/` removed and its whitespace folded.
    pub docstring: String,
    /// SHA-256 of the declaration's header line.
    pub digest: String,
}

/// The keywords that begin a declaration at the start of a line.
///
/// `instance` and `abbrev` are here because a classifier should be able to name them; `example` and
/// `variable` are not, because they name nothing.
const KEYWORDS: [&str; 8] = [
    "def",
    "theorem",
    "lemma",
    "structure",
    "class",
    "instance",
    "abbrev",
    "noncomputable def",
];

/// Read every declaration in one Lean file.
///
/// `module` is the path used to fill [`Declaration::module`], and its first directory under `Mathlib/`
/// gives [`Declaration::domain`].
#[must_use]
pub fn parse(source: &str, module: &str) -> Vec<Declaration> {
    let domain = domain_of(module);
    let mut declarations: Vec<Declaration> = Vec::new();
    let mut docstring = String::new();
    let mut in_doc = false;
    for line in source.lines() {
        let trimmed = line.trim_start();
        // A docstring, and its continuation.
        if trimmed.starts_with("/--") {
            in_doc = true;
            docstring.clear();
            let body = trimmed.trim_start_matches("/--");
            push_doc(&mut docstring, body.trim_end_matches("-/"));
            if trimmed.contains("-/") {
                in_doc = false;
            }
            continue;
        }
        if in_doc {
            push_doc(&mut docstring, trimmed.trim_end_matches("-/"));
            if trimmed.contains("-/") {
                in_doc = false;
            }
            continue;
        }
        let Some((kind, rest)) = split_keyword(trimmed) else {
            continue;
        };
        let Some((name, statement)) = split_name(rest) else {
            continue;
        };
        declarations.push(Declaration {
            name,
            kind: kind.to_owned(),
            module: module.to_owned(),
            domain: domain.clone(),
            statement,
            docstring: std::mem::take(&mut docstring),
            digest: digest(line.trim()),
        });
    }
    declarations
}

/// Read every `.lean` file under `root` (which should be a checkout of Mathlib).
///
/// A file that cannot be read is skipped rather than failing the run: the lexicon is a lexicon, and one
/// unreadable file must not cost the other four thousand.
#[must_use]
pub fn parse_tree(root: &Path) -> Vec<Declaration> {
    let mut declarations = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|extension| extension != "lean") {
                continue;
            }
            let Ok(source) = std::fs::read_to_string(&path) else {
                continue;
            };
            let module = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            // A checkout is already laid out as `Mathlib/…`; a bare subdirectory is not, and the module
            // path has to name the file the way Lean does either way.
            let module = if module.starts_with("Mathlib/") {
                module
            } else {
                format!("Mathlib/{module}")
            };
            declarations.extend(parse(&source, &module));
        }
    }
    declarations.sort_by(|a, b| a.name.cmp(&b.name));
    declarations
}

/// The taxonomy a module path implies: the first directory under `Mathlib/`.
#[must_use]
pub fn domain_of(module: &str) -> String {
    let trimmed = module.trim_start_matches("Mathlib/");
    trimmed
        .split('/')
        .next()
        .filter(|first| !first.to_ascii_lowercase().ends_with(".lean"))
        .unwrap_or("Mathlib")
        .to_owned()
}

/// The keyword a line opens with, and the rest of it.
fn split_keyword(line: &str) -> Option<(&'static str, &str)> {
    for keyword in KEYWORDS {
        if let Some(rest) = line.strip_prefix(keyword)
            && rest.starts_with(' ')
        {
            return Some((keyword, rest.trim_start()));
        }
    }
    None
}

/// The declared name and the statement that follows it.
fn split_name(rest: &str) -> Option<(String, String)> {
    // A name is one token; the statement follows up to the `:=` that starts the body, or the line's end.
    let mut end = 0_usize;
    for (index, character) in rest.char_indices() {
        let boundary = character.is_whitespace() || character == ':' || character == '(';
        if boundary {
            end = index;
            break;
        }
        end = index + character.len_utf8();
    }
    let name = rest.get(..end)?.to_owned();
    if name.is_empty() || name.starts_with('{') {
        return None;
    }
    let statement = rest
        .get(end..)
        .unwrap_or_default()
        .split(":=")
        .next()
        .unwrap_or_default()
        .trim()
        .to_owned();
    Some((name, statement))
}

/// Append one docstring line, folding whitespace.
fn push_doc(docstring: &mut String, line: &str) {
    let line = line.trim();
    if line.is_empty() {
        return;
    }
    if !docstring.is_empty() {
        docstring.push(' ');
    }
    docstring.push_str(line);
}

/// The digest the store uses, taken from the crate that owns persistence rather than a second hasher.
fn digest(text: &str) -> String {
    stkcraft_framework_storage::digest(text.as_bytes())
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    const SOURCE: &str = r"
/-!
# Square roots
-/

/-- `Real.sqrt` is the square root of a nonnegative real. -/
noncomputable def sqrt (x : ℝ) : ℝ :=
  if 0 ≤ x then Classical.choose (exists_sq_eq x) else 0

@[simp]
theorem sqrt_sq (h : 0 ≤ x) : sqrt (x ^ 2) = x := by rw [sqrt, if_pos (pow_nonneg h 2)]

lemma no_docstring : 1 + 1 = 2 := rfl

structure Point where
  x : ℝ
";

    #[test]
    fn declarations_are_read_with_their_statement_and_docstring() {
        let declarations = parse(SOURCE, "Mathlib/Analysis/SpecialFunctions/Sqrt.lean");
        let names: Vec<&str> = declarations.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, vec!["sqrt", "sqrt_sq", "no_docstring", "Point"]);
        let sqrt = &declarations[0];
        assert_eq!(sqrt.kind, "noncomputable def");
        assert_eq!(sqrt.domain, "Analysis");
        assert_eq!(sqrt.module, "Mathlib/Analysis/SpecialFunctions/Sqrt.lean");
        assert!(sqrt.statement.contains("x : ℝ"), "{}", sqrt.statement);
        assert!(sqrt.docstring.contains("square root"), "{}", sqrt.docstring);
        // A declaration with no docstring has an empty one, never the previous declaration's.
        assert_eq!(declarations[2].docstring, "");
        assert_eq!(declarations[3].kind, "structure");
        // Every row carries a digest, and two identical headers hash alike.
        assert_eq!(sqrt.digest.len(), 64);
        let again = parse(SOURCE, "Mathlib/Analysis/SpecialFunctions/Sqrt.lean");
        assert_eq!(again[0].digest, sqrt.digest);
    }

    #[test]
    fn the_domain_is_the_first_directory_under_mathlib() {
        assert_eq!(
            domain_of("Mathlib/Analysis/Calculus/FDeriv/Basic.lean"),
            "Analysis"
        );
        assert_eq!(domain_of("Mathlib/Algebra/Group/Basic.lean"), "Algebra");
        assert_eq!(domain_of("Mathlib/Tactic/Linarith.lean"), "Tactic");
        assert_eq!(domain_of("Other/Thing.lean"), "Other");
    }

    #[test]
    fn the_module_list_docstring_is_not_a_declaration() {
        // `/--` and `/-!` are different: the first documents the next declaration, the second the file. A
        // reader that confused them would attach the module header to whatever follows it.
        let declarations = parse(
            "module\n\n/-!\n# A file\n-/\n\ndef one : ℕ := 1\n",
            "Mathlib/Data/Nat.lean",
        );
        assert_eq!(declarations.len(), 1);
        assert_eq!(declarations[0].name, "one");
        assert_eq!(declarations[0].docstring, "");
    }

    #[test]
    fn the_parser_reads_a_real_mathlib_file() {
        // The evidence that the reader is not only right about a fixture: a real file from the Mathlib
        // repository, fetched by `cargo xtask mathlib` for the lexicon. Skipped when the checkout is
        // absent, because the corpus is not in the repository.
        let Some(root) = std::env::var_os("MATHLIB_DIR") else {
            return;
        };
        let path = Path::new(&root).join("Mathlib/Analysis/SpecialFunctions/Sqrt.lean");
        let Ok(source) = std::fs::read_to_string(&path) else {
            return;
        };
        let declarations = parse(&source, "Mathlib/Analysis/SpecialFunctions/Sqrt.lean");
        assert!(
            declarations.len() > 10,
            "{} declarations",
            declarations.len()
        );
        // The file is about `Real.sqrt`: its declarations are the derivative and smoothness lemmas, so the
        // check is that the reader found the declarations the file actually holds, not a word in a name.
        for expected in ["sqPartialHomeomorph", "hasDerivAt_sqrt", "deriv_sqrt"] {
            assert!(
                declarations.iter().any(|d| d.name == expected),
                "`{expected}` was not read; got {:?}",
                declarations
                    .iter()
                    .map(|d| d.name.as_str())
                    .collect::<Vec<_>>()
            );
        }
        assert!(declarations.iter().all(|d| d.domain == "Analysis"));
        assert!(declarations.iter().all(|d| d.digest.len() == 64));
        // Some declaration in the file documents itself; the docstring is the point of the row.
        assert!(
            declarations.iter().any(|d| !d.docstring.is_empty()),
            "no docstring was read from a file that documents its declarations"
        );
    }
}

/// The operators a formula and a declaration can share, as canonical tokens.
///
/// This is the whole of the normalisation: a symbol in either rendering of a formula becomes the word a
/// declaration would use, so `√x` and `Real.sqrt` meet at `sqrt`. The table is short on purpose — a
/// classifier that scored on words would match prose, and the point of classifying *formulas* is that they
/// are operators.
pub const OPERATORS: [(&str, &str); 23] = [
    // The arithmetic a polynomial is made of. It is here because the first recorded equations were
    // polynomials: without it, `F(x) = x⁸+x⁷+x²+x+1` had no operator the classifier could name.
    ("^", "pow"),
    ("\\^", "pow"),
    ("+", "add"),
    ("·", "mul"),
    ("\\cdot", "mul"),
    ("√", "sqrt"),
    ("\\sqrt", "sqrt"),
    ("∑", "sum"),
    ("\\sum", "sum"),
    ("∏", "product"),
    ("\\prod", "product"),
    ("∫", "integral"),
    ("\\int", "integral"),
    ("∂", "deriv"),
    ("\\partial", "deriv"),
    ("\\frac", "div"),
    ("\\div", "div"),
    ("∞", "infinity"),
    ("≤", "le"),
    ("≥", "ge"),
    ("≠", "ne"),
    ("≈", "approx"),
    ("∇", "gradient"),
];

/// Operators that are arithmetic but not *evidence*: every equation in the world has them.
///
/// Measured, not assumed: with `add` and `pow` in the scoring set, `F(x) = x⁸+x⁷+x²+x+1` matched
/// `abv_add` — an absolute-value addition lemma — with a score of 1.00. Generic arithmetic says nothing
/// about which concept a formula is about, so it is counted for reporting and ignored for matching.
pub const GENERIC: [&str; 4] = ["add", "pow", "mul", "div"];

/// A classification: the declaration a formula was matched to, and how strongly.
#[derive(Debug, Clone, PartialEq)]
pub struct Match {
    /// The declaration's name, e.g. `Real.sqrt`.
    pub name: String,
    /// Its module, which is also the taxonomy.
    pub module: String,
    /// The first directory under `Mathlib/`.
    pub domain: String,
    /// The operators the formula and the declaration share, for the citation's "why".
    pub shared: Vec<&'static str>,
    /// Shared operators divided by the formula's operators: 1.0 means every operator the formula uses is
    /// one the declaration names.
    pub score: f64,
}

/// The operators a piece of text uses, normalised to canonical tokens.
#[must_use]
pub fn operators_in(text: &str) -> Vec<&'static str> {
    let mut found: Vec<&'static str> = Vec::new();
    for (symbol, token) in OPERATORS {
        if text.contains(symbol) && !found.contains(&token) {
            found.push(token);
        }
    }
    found
}

/// Classify a formula against a lexicon.
///
/// The rule is deliberately blunt and deliberately refuses: a formula **must use an operator** that a
/// declaration names, and the score is the share of the formula's operators the declaration covers. A
/// formula with no operator, or a lexicon with no declaration for it, returns `None` — a classification
/// this project cannot justify is a classification it does not make.
#[must_use]
pub fn classify(formula: &str, declarations: &[Declaration]) -> Option<Match> {
    // Only the operators that distinguish one concept from another take part in a match.
    let formula_operators: Vec<&'static str> = operators_in(formula)
        .into_iter()
        .filter(|token| !GENERIC.contains(token))
        .collect();
    if formula_operators.is_empty() {
        return None;
    }
    let mut best: Option<Match> = None;
    for declaration in declarations {
        let haystack = format!(
            "{} {} {}",
            declaration.name, declaration.statement, declaration.docstring
        );
        let haystack = haystack.to_ascii_lowercase();
        // Words, not substrings: "add" inside "addition" and "pow" inside "power" made a derivative lemma
        // match a polynomial with a score of 1.00, which is exactly the plausible-but-wrong answer this
        // project refuses.
        let words: Vec<&str> = haystack
            .split(|character: char| !character.is_alphanumeric())
            .filter(|word| !word.is_empty())
            .collect();
        let mut shared: Vec<&'static str> = Vec::new();
        for token in &formula_operators {
            // A declaration names an operator either by its word (`sqrt`, `sum`) or by the symbol.
            // A symbol counts only when it cannot be prose: `+` appears in the statement of half of
            // Analysis, so matching on it made a derivative lemma "cover" a polynomial. `√`, `∑`, `∂` and
            // their kin do not appear by accident.
            let named = words.contains(token)
                || OPERATORS.iter().any(|(symbol, name)| {
                    name == token && !symbol.is_ascii() && haystack.contains(symbol)
                });
            if named {
                shared.push(token);
            }
        }
        if shared.is_empty() {
            continue;
        }
        let score = shared.len() as f64 / formula_operators.len() as f64;
        // A share is a ratio of small integers, so comparing the counts is exact and avoids a float
        // comparison entirely; ties then break on the name, so the same input always gives the same answer.
        let matched = shared.len();
        let total = formula_operators.len();
        let better = best.as_ref().is_none_or(|current| {
            let current_matched = (current.score * total as f64).round() as usize;
            matched > current_matched
                || (matched == current_matched && declaration.name < current.name)
        });
        if better {
            best = Some(Match {
                name: declaration.name.clone(),
                module: declaration.module.clone(),
                domain: declaration.domain.clone(),
                shared,
                score,
            });
        }
    }
    best.filter(|matched| matched.score >= 0.5)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::indexing_slicing)]
mod classifier_tests {
    use super::*;

    /// A lexicon shaped like the part of Mathlib a paper about square roots would need.
    fn lexicon() -> Vec<Declaration> {
        let source = r"
/-- The square root of a nonnegative real. -/
noncomputable def sqrt (x : ℝ) : ℝ := 0

/-- `HasDerivAt f f' x` means `f` has derivative `f'` at `x`. -/
def HasDerivAt (f : ℝ → ℝ) (f' x : ℝ) : Prop := True

/-- The sum of a finite set. -/
def Finset.sum (s : Finset ι) (f : ι → α) : α := 0

/-- An unrelated definition: a group homomorphism. -/
structure MonoidHom (M N : Type) where
  toFun : M → N
";
        parse(source, "Mathlib/Analysis/SpecialFunctions/Sqrt.lean")
    }

    #[test]
    fn a_formula_is_matched_to_the_declaration_that_names_its_operators() {
        let declarations = lexicon();
        let matched = classify("√ (x ^ 2) = x", &declarations).expect("a square root matches");
        assert_eq!(matched.name, "sqrt");
        assert_eq!(matched.domain, "Analysis");
        assert_eq!(matched.shared, vec!["sqrt"]);
        // `^` is arithmetic and not evidence, so the coverage is the one operator that distinguishes it.
        assert!(
            (matched.score - 1.0).abs() < f64::EPSILON,
            "{}",
            matched.score
        );
        // The citation a hit prints: the concept, the taxonomy and the strength.
        assert_eq!(
            matched.module,
            "Mathlib/Analysis/SpecialFunctions/Sqrt.lean"
        );
    }

    #[test]
    fn a_formula_without_an_operator_is_never_classified() {
        // The rule that keeps prose out of the classification: a keyword line has no operator, and a
        // classifier that guessed would be labelling sentences with mathematical concepts.
        let declarations = lexicon();
        assert_eq!(classify("Keyword = Value Notation", &declarations), None);
        assert_eq!(classify("", &declarations), None);
        assert_eq!(classify("8-BIT WORD = OCTET", &declarations), None);
    }

    #[test]
    fn an_operator_no_declaration_names_is_a_no_match_rather_than_a_weak_match() {
        // `∇` is in the operator table and in no declaration of this lexicon: the answer is `None`, not the
        // first declaration that shared nothing.
        let declarations = lexicon();
        assert_eq!(classify("∇f = 0", &declarations), None);
        // And a partially covered formula scores by share, then must clear the floor.
        let partial = classify("∫ √x dx = 1", &lexicon()).expect("sqrt is covered");
        assert_eq!(partial.name, "sqrt");
        assert!(
            (partial.score - 0.5).abs() < f64::EPSILON,
            "{}",
            partial.score
        );
    }

    #[test]
    fn the_operators_of_a_formula_are_read_in_both_renderings() {
        assert_eq!(operators_in("√x"), vec!["sqrt"]);
        assert_eq!(operators_in("\\sqrt{x}"), vec!["sqrt"]);
        assert_eq!(operators_in("∑ x"), vec!["sum"]);
        assert_eq!(operators_in("plain words").len(), 0);
    }
}
