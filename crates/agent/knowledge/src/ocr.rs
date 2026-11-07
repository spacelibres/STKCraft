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
//! Unix commands run through `/bin/sh -c`; Windows commands run through `cmd.exe /D /S /C`
//! and use `%STKCRAFT_OCR_FILE%` and `%STKCRAFT_OCR_PAGE%` for these variables. Quote the file
//! variable when passing it to a program, and configure the program to write UTF-8 text.
//!
//! # What this module will not do
//!
//! Invent text. A command that fails, that prints nothing, or that is not configured leaves the page
//! without text, and the caller records the page as read by nobody. The accepted text length is bounded
//! after capture; process output capture itself is not bounded and commands must terminate.

use std::path::Path;
use std::process::Command;

/// The environment variable that names the command.
pub const VARIABLE: &str = "STKCRAFT_OCR";

/// The most characters a page's OCR output may contribute.
///
/// A page of text is a few thousand characters; a command that prints a megabyte is not recognising a
/// page. This bounds accepted text, not the memory used while capturing the process output.
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
    #[cfg(windows)]
    let mut shell = {
        let mut shell = Command::new("cmd.exe");
        shell.args(["/D", "/S", "/C"]);
        shell
    };
    #[cfg(not(windows))]
    let mut shell = {
        let mut shell = Command::new("/bin/sh");
        shell.arg("-c");
        shell
    };
    let output = shell
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
        //
        // The folder name is ASCII on purpose. On Windows, `cmd` echoes the variable through the console code
        // page, and a non-ASCII path came back as replacement characters, so the test failed on the codec
        // rather than on the plumbing it is meant to check.
        let page = std::env::temp_dir().join("OCR papers").join("a paper.pdf");
        #[cfg(windows)]
        let command = "chcp 65001 >nul & echo file=%STKCRAFT_OCR_FILE% page=%STKCRAFT_OCR_PAGE% & echo the recognised line";
        #[cfg(not(windows))]
        let command =
            "echo \"file=$STKCRAFT_OCR_FILE page=$STKCRAFT_OCR_PAGE\"; echo 'the recognised line'";
        let text = recognise_with(command, &page, 7).expect("recognised");
        assert!(text.contains(&format!("file={}", page.display())), "{text}");
        assert!(text.contains("page=7"), "{text}");
        assert!(text.contains("the recognised line"), "{text}");
    }

    #[test]
    fn a_failing_command_names_its_own_failure_rather_than_returning_empty_text() {
        #[cfg(windows)]
        let command = "echo engine: no tessdata >&2 & exit /b 3";
        #[cfg(not(windows))]
        let command = "echo 'engine: no tessdata' >&2; exit 3";
        let error = recognise_with(command, Path::new("p.pdf"), 1).expect_err("a failing command");
        assert!(error.contains("status") && error.contains('3'), "{error}");
        assert!(error.contains("no tessdata"), "{error}");
    }

    #[test]
    fn output_is_bounded() {
        // A command that prints more than a page can hold is refused rather than trusted: the bound is
        // a limit on accepted text, applied after the process exits and its output is captured.
        #[cfg(windows)]
        let command = "for /L %i in (1,1,25000) do @echo xxxxxxxxxx";
        #[cfg(not(windows))]
        let command = "i=0; while [ $i -lt 250000 ]; do printf x; i=$((i+1)); done";
        let error = recognise_with(command, Path::new("p.pdf"), 1).expect_err("an output flood");
        assert!(error.contains("characters"), "{error}");
    }
}
