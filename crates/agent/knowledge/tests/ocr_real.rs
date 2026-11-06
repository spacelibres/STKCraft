//! OCR against a real engine, in CI.
//!
//! `docs/design/mathematics.md` and `crates/agent/knowledge/src/ocr.rs` make the engine *configuration*:
//! `STKCRAFT_OCR` names a command that receives the file and the page, rasterises it however it likes and
//! writes text. That keeps a C library out of everybody's build matrix, but it also means the module's unit
//! tests only prove the plumbing. This test proves the path with a real engine.
//!
//! It is `#[ignore]`d because the engine is not on a developer's machine by default — the workflow job
//! `ocr` installs `tesseract-ocr`, `poppler-utils`, `imagemagick` and `img2pdf` and runs it with
//! `--ignored`. A machine without those tools skips it; a machine with them and a broken path fails.
//!
//! What it does: draw a sentence with `ImageMagick`, wrap the picture in a one-page PDF with `img2pdf`, and ask
//! the library to read the document. The page has no text layer at all, so the only way text can appear is
//! through the configured command.

#![allow(clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;

/// The sentence the picture holds, and the words the engine must find.
const SENTENCE: &str = "SGP4 propagates two line elements";

fn run(command: &str, args: &[&str]) -> bool {
    Command::new(command)
        .args(args)
        .status()
        .is_ok_and(|status| status.success())
}

/// A one-page PDF whose content is a picture of [`SENTENCE`].
///
/// Drawn with `Pillow` rather than `ImageMagick`: `convert` on a bare runner fails with "unable to read font
/// (null)" unless a face is named, and naming one through its CLI is a moving target across versions. Pillow
/// takes a font path and is in the same apt list.
fn scanned_pdf(directory: &Path) -> PathBuf {
    let pdf = directory.join("scan.pdf");
    let script = format!(
        "from PIL import Image, ImageDraw, ImageFont\n\
         img = Image.new('RGB', (2480, 400), 'white')\n\
         font = ImageFont.truetype('/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf', 72)\n\
         ImageDraw.Draw(img).text((80, 160), {sentence:?}, fill='black', font=font)\n\
         img.save({path:?})\n",
        sentence = SENTENCE,
        path = pdf.to_string_lossy(),
    );
    assert!(
        run("python3", &["-c", &script]),
        "pillow could not draw the page"
    );
    pdf
}

#[test]
#[ignore = "needs tesseract, poppler and imagemagick; the `ocr` workflow job runs it"]
fn a_scanned_page_is_read_by_the_configured_engine() {
    // The command a person would configure: rasterise the page at 300 dpi and pipe it to the engine.
    let command = "pdftoppm -r 300 -png -f \"$STKCRAFT_OCR_PAGE\" -l \"$STKCRAFT_OCR_PAGE\" \
                   \"$STKCRAFT_OCR_FILE\" - | tesseract - - -l eng 2>/dev/null";
    // The library's own entry point takes the command from the environment; this test hands it over
    // directly so it cannot depend on the order another test sets variables in.
    let directory = std::env::temp_dir().join(format!("stkcraft-ocr-{}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("a scratch directory");
    let pdf = scanned_pdf(&directory);
    let text = stkcraft_agent_knowledge::ocr::recognise_with(command, &pdf, 1)
        .expect("the engine read the page");
    let folded = text.to_lowercase();
    for word in ["sgp4", "propagates", "elements"] {
        assert!(
            folded.contains(word),
            "the engine did not read `{word}`: {text:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&directory);
}
