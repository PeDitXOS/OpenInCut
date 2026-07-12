//! The whole reason this crate exists: emoji that are actually emoji.

use ue_core::model::TextStyle;
use ue_text::{render, TextSpec, DEFAULT_WIDTH_FRACTION};

fn spec<'a>(content: &'a str, style: &'a TextStyle) -> TextSpec<'a> {
    TextSpec { content, style, out_w: 1280, out_h: 720, width_fraction: DEFAULT_WIDTH_FRACTION }
}

/// ffmpeg's drawtext drew .notdef boxes (one font, no fallback) and libass drew
/// them too (vector outlines only, no colour-font support since the request was
/// filed in 2020). The check is for COLOUR on purpose: a hollow box is ink, so
/// "there are more pixels than without the emoji" is a test the BUG passes.
#[test]
fn emoji_render_as_colour_glyphs() {
    let style = TextStyle { size: 120.0, color: "#ffffff".into(), ..Default::default() };
    let img = render(&spec("HOLA 🎬🔥", &style));
    img.write_png(std::path::Path::new(
        &std::env::temp_dir().join("ue_text_emoji.png"),
    ))
    .unwrap();

    let mut coloured = 0usize;
    let mut ink = 0usize;
    for p in img.rgba.chunks_exact(4) {
        if p[3] < 40 {
            continue;
        }
        ink += 1;
        let (mx, mn) = (p[0].max(p[1]).max(p[2]) as i32, p[0].min(p[1]).min(p[2]) as i32);
        if mx - mn > 60 {
            coloured += 1;
        }
    }
    eprintln!("ink {ink} px, of which coloured {coloured} px");
    assert!(ink > 2000, "the text rendered at all ({ink} px)");
    assert!(
        coloured > 500,
        "the emoji are COLOUR glyphs, not white boxes ({coloured} coloured px)"
    );
}

/// The Latin text still has to look like text: white glyphs, dark outline.
#[test]
fn plain_text_keeps_its_colour_and_outline() {
    let style = TextStyle { size: 120.0, color: "#ffffff".into(), ..Default::default() };
    let img = render(&spec("HOLA", &style));
    let white = img
        .rgba
        .chunks_exact(4)
        .filter(|p| p[3] > 200 && p[0] > 200 && p[1] > 200 && p[2] > 200)
        .count();
    let dark = img
        .rgba
        .chunks_exact(4)
        .filter(|p| p[3] > 60 && p[0] < 60 && p[1] < 60 && p[2] < 60)
        .count();
    eprintln!("white glyph px {white}, dark outline px {dark}");
    assert!(white > 1000, "the glyphs are the style colour ({white} px)");
    assert!(dark > 500, "and they carry the outline ({dark} px)");
}

/// A long line wraps, and the block stays centred on the frame.
#[test]
fn a_long_line_wraps_and_stays_centred() {
    let style = TextStyle { size: 80.0, ..Default::default() };
    let img = render(&spec(
        "esta frase es demasiado larga para caber en una sola linea del video",
        &style,
    ));
    let (w, h) = (img.width as usize, img.height as usize);
    let rows: Vec<usize> = (0..h)
        .filter(|y| (0..w).any(|x| img.rgba[(y * w + x) * 4 + 3] > 60))
        .collect();
    let mut lines = 0;
    for (i, y) in rows.iter().enumerate() {
        if i == 0 || *y > rows[i - 1] + 3 {
            lines += 1;
        }
    }
    let widest = (0..h)
        .map(|y| (0..w).filter(|x| img.rgba[(y * w + x) * 4 + 3] > 60).count())
        .max()
        .unwrap_or(0);
    let mid = (rows.first().unwrap() + rows.last().unwrap()) / 2;
    eprintln!("{lines} lines, widest row {widest} px, block centre {mid} (frame centre {})", h / 2);
    assert!(lines >= 2, "it wrapped ({lines} lines)");
    assert!(widest < w, "no line spills out of the frame");
    assert!((mid as i64 - (h / 2) as i64).abs() < 40, "the block stays centred ({mid})");
}
