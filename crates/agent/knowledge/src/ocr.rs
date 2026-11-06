//! OCR for the pages that have no text layer, through a command the person configures.
//!
//! A scanned page has text in a picture and none in its file. Recognising it needs an engine, and the
//! engines are not ours: `Tesseract`, `PaddleOCR` and the model-based readers are all large, platform-specific
//! and versioned, and linking one would decide the build matrix for everybody. So the engine is
//! **configuration**, in the same way the model transport is `curl` rather than a linked HTTP client:
//!
//! ```text
//! STKCRAFT_OCR='pdftoppm -f $STKCRAFT_OCR_PAGE -l $STKCRAFT_OCR_PAGE -r 300 -png "$STKCRAFT_OCR_FILE" - | tesseract - - -l eng'
//! ```
//!
//! The command receives two variables — `STKCRAFT_OCR_FILE`, the PDF, and `STKCRAFT_OCR_PAGE`, the
//! one-based page — writes the recognised text to standard output, and is otherwise free: rasterising the
//! page is part of the command, because the rasteriser is as much a packaging decision as the engine.
//!
//! # What this module will not do
//!
//! Invent text. A command that fails, that prints nothing, or that is not configured leaves the page
//! without text, and the caller records the page as read by nobody. Output is bounded, because a command
//! is a program and a program can print for ever.

use std::path::Path;
use std::process::Command;

/// The environment variable that names the command.
pub const VARIABLE: &str = "STKCRAFT_OCR";

/// The most characters a page's OCR output may contribute.
///
/// A page of text is a few thousand characters; a command that prints a megabyte is not recognising a
/// page, and the bound is what stops it from becoming the library's memory problem.
pub const MAX_OUTPUT_CHARS: usize = 200_000;

/// Whether an OCR command is configured.
#[must_use]
pub fn available() -> bool {
    std::env::var(VARIABLE).is_ok_and(|value| !value.trim().is_empty())
}

/// Recognise one page with the configured command.
///
/// # Errors
///
/// A message when the variable is not set, the command cannot start, it exits non-zero, or it prints more
/// than [`MAX_OUTPUT_CHARS`]. A page it cannot read is not an error the caller has to handle: it is a page
/// with no text, which is what [`page_text`]'s empty string means.
pub fn recognise(file: &Path, page: usize) -> Result<String, String> {
    let command = std::env::var(VARIABLE)
        .map_err(|_| format!("{VARIABLE} is not set; export it to recognise scanned pages"))?;
    recognise_with(&command, file, page)
}

/// Recognise one page with an explicit command, which is what a test does.
///
/// # Errors
///
/// As [`recognise`].
pub fn recognise_with(command: &str, file: &Path, page: usize) -> Result<String, String> {
    let output = Command::new("/bin/sh")
        .arg("-c")
        .arg(command)
        .env("STKCRAFT_OCR_FILE", file)
        .env("STKCRAFT_OCR_PAGE", page.to_string())
        .output()
        .map_err(|error| format!("could not run the OCR command: {error}"))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(format!(
            "the OCR command failed (status {}): {detail}",
            output.status
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    if text.chars().count() > MAX_OUTPUT_CHARS {
        return Err(format!(
            "the OCR command printed more than {MAX_OUTPUT_CHARS} characters for one page"
        ));
    }
    Ok(text)
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn a_command_reads_a_page_and_its_output_is_the_text() {
        // The plumbing, with the engine replaced by one that prints a fixed page: the variables are handed
        // over, standard output is the text, and nothing is added by us.
        let page = Path::new("/tmp/a-paper.pdf");
        let text = recognise_with(
            "echo \"file=$STKCRAFT_OCR_FILE page=$STKCRAFT_OCR_PAGE\"; echo 'the recognised line'",
            page,
            7,
        )
        .expect("recognised");
        assert!(text.contains("file=/tmp/a-paper.pdf"), "{text}");
        assert!(text.contains("page=7"), "{text}");
        assert!(text.contains("the recognised line"), "{text}");
    }

    #[test]
    fn a_failing_command_names_its_own_failure_rather_than_returning_empty_text() {
        let error = recognise_with(
            "echo 'engine: no tessdata' >&2; exit 3",
            Path::new("p.pdf"),
            1,
        )
        .expect_err("a failing command");
        assert!(error.contains("status"), "{error}");
        assert!(error.contains("no tessdata"), "{error}");
    }

    #[test]
    fn output_is_bounded() {
        // A command that prints more than a page can hold is refused rather than trusted: the bound is
        // what keeps recognising a document from becoming the library's memory problem.
        let error = recognise_with(
            "i=0; while [ $i -lt 250000 ]; do printf x; i=$((i+1)); done",
            Path::new("p.pdf"),
            1,
        )
        .expect_err("an output flood");
        assert!(error.contains("characters"), "{error}");
    }
}
