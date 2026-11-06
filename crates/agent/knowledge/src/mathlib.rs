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
