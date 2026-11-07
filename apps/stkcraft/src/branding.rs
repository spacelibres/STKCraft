//! The albatross mark: the window icon and the texture beside the panel heading.
//!
//! The artwork is the project's own brand mark (`assets/app-icon/albatross.png`, credited in
//! `ATTRIBUTION.md`). It is embedded rather than read from disk, so a deployed binary carries its
//! identity with it, and decoded with bounds, so a damaged or replaced file cannot take the window
//! down: a mark that cannot be read draws text without it, which looks worse but is not a crash.

use std::sync::Arc;

use eframe::egui;

/// The 512 × 512 PNG embedded at build time; the largest size a desktop asks for.
const MARK_PNG: &[u8] = include_bytes!("../../../assets/app-icon/albatross.png");

/// The decoded pixels and their dimensions.
struct Mark {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

/// The icon the window manager shows for the application.
#[must_use]
pub fn window_icon() -> Option<Arc<egui::IconData>> {
    let mark = decode(MARK_PNG)?;
    Some(Arc::new(egui::IconData {
        rgba: mark.rgba,
        width: mark.width,
        height: mark.height,
    }))
}

/// The mark as an `egui` texture, for the header of the controls panel.
///
/// The caller owns the handle: dropping it releases the texture with the window.
#[must_use]
pub fn texture(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let mark = decode(MARK_PNG)?;
    let width = usize::try_from(mark.width).ok()?;
    let height = usize::try_from(mark.height).ok()?;
    // `from_rgba_unmultiplied` asserts the length; checking it here keeps a short decode from
    // becoming a panic in the draw loop, which is the one place the no-crash rule cannot bend.
    if mark.rgba.len() != width.checked_mul(height)?.checked_mul(4)? {
        return None;
    }
    let image = egui::ColorImage::from_rgba_unmultiplied([width, height], &mark.rgba);
    Some(ctx.load_texture("stkcraft-albatross", image, egui::TextureOptions::LINEAR))
}

/// Decode the embedded PNG to RGBA8, refusing a size no icon has.
///
/// `png` applies the transformations that make every legal colour type 8-bit before this reads it,
/// so the match below covers the four ways a pixel can be stored; the palette is expanded away by
/// `EXPAND`, and a palette that survived it is not this asset.
fn decode(bytes: &[u8]) -> Option<Mark> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let (width, height) = {
        let info = reader.info();
        (info.width, info.height)
    };
    // An icon is square and small. This also bounds the allocation below: 1024² × 4 is 4 MiB, so a
    // file that claims a huge canvas is refused before a byte is reserved for it.
    if width != height || !(16..=1024).contains(&width) {
        log::warn!("branding: the mark is {width}x{height}, which is not an icon size");
        return None;
    }
    let mut buffer = vec![0; reader.output_buffer_size()?];
    let frame = reader.next_frame(&mut buffer).ok()?;
    let pixels = buffer.get(..frame.buffer_size())?;
    let rgba = match frame.color_type {
        png::ColorType::Rgba => pixels.to_vec(),
        png::ColorType::Rgb => widen(pixels, 3),
        png::ColorType::Grayscale => widen(pixels, 1),
        png::ColorType::GrayscaleAlpha => widen(pixels, 2),
        png::ColorType::Indexed => return None,
    };
    Some(Mark {
        width,
        height,
        rgba,
    })
}

/// Expand packed samples to RGBA, dropping a trailing partial pixel rather than reading past it.
fn widen(pixels: &[u8], samples: usize) -> Vec<u8> {
    let capacity = pixels
        .len()
        .checked_div(samples)
        .unwrap_or(0)
        .saturating_mul(4);
    let mut rgba = Vec::with_capacity(capacity);
    for pixel in pixels.chunks_exact(samples) {
        // `chunks_exact` yields only full pixels, so one pattern always matches. The fallback is
        // unreachable, and it still does the safe thing when it is not.
        let expanded = match *pixel {
            [gray] => [gray, gray, gray, 255],
            [gray, alpha] => [gray, gray, gray, alpha],
            [red, green, blue] => [red, green, blue, 255],
            _ => continue,
        };
        rgba.extend_from_slice(&expanded);
    }
    rgba
}

#[cfg(test)]
#[expect(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_mark_decodes_to_a_square_rgba_icon() {
        let mark = decode(MARK_PNG).expect("the embedded mark decodes");
        assert_eq!(mark.width, mark.height);
        assert_eq!(
            mark.rgba.len(),
            usize::try_from(mark.width).expect("a small width")
                * usize::try_from(mark.height).expect("a small height")
                * 4
        );
        assert!(
            mark.rgba
                .as_chunks::<4>()
                .0
                .iter()
                .any(|&[_, _, _, alpha]| alpha > 0)
        );
    }

    #[test]
    fn garbage_is_refused_instead_of_panicking() {
        assert!(decode(b"not a png").is_none());
        assert!(decode(&[]).is_none());
    }

    #[test]
    fn widening_drops_a_trailing_partial_pixel() {
        assert_eq!(widen(&[1, 2, 3, 4], 3), [1, 2, 3, 255]);
        assert_eq!(widen(&[1, 2], 3), Vec::<u8>::new());
        assert_eq!(widen(&[9, 128], 2), [9, 9, 9, 128]);
    }
}
