//! Text rasterisation: we draw the glyphs ourselves instead of asking ffmpeg to.
//!
//! Both of ffmpeg's text paths are dead ends for anything beyond plain Latin:
//!
//! * `drawtext` loads exactly ONE font face and does no fallback, so every
//!   glyph the chosen font lacks — all emoji, most CJK — comes out as an empty
//!   .notdef box.
//! * `libass` (the `ass=` filter) does resolve fallbacks through fontconfig, but
//!   it renders **vector outlines in a single colour**: it has no colour-font
//!   support at all and has not had it since the request was filed in 2020
//!   (libass#381). On macOS the only emoji font is Apple Color Emoji, which is
//!   `sbix` — colour BITMAPS — so libass draws boxes too.
//!
//! cosmic-text shapes the text with per-glyph font fallback and rasterises
//! through swash, which reads Apple `sbix`, Google `CBDT/CBLC` and Microsoft
//! `COLR/CPAL` colour glyphs. So the emoji arrive as actual colour pixels, and
//! the result is composited as an ordinary RGBA layer — the machinery for which
//! already exists (a styled text clip is already its own layer).
//!
//! The wrap is done by cosmic-text's own line breaker against the same usable
//! width the rest of the codebase uses, so a caption still breaks where it fits.

use std::io::BufWriter;
use std::path::{Path, PathBuf};

use cosmic_text::{
    Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache, SwashContent, Weight,
};
use thiserror::Error;
use ue_core::model::{TextAlign, TextStyle};

#[derive(Debug, Error)]
pub enum TextError {
    #[error("could not write the text image: {0}")]
    Io(#[from] std::io::Error),
    #[error("png: {0}")]
    Png(String),
}

pub type TextResult<T> = Result<T, TextError>;

/// System fonts are loaded once: scanning them per title would dominate the
/// cost of a short export.
fn font_system() -> &'static std::sync::Mutex<FontSystem> {
    static FS: std::sync::OnceLock<std::sync::Mutex<FontSystem>> = std::sync::OnceLock::new();
    FS.get_or_init(|| std::sync::Mutex::new(FontSystem::new()))
}

/// `#rrggbb` → (r, g, b).
fn rgb(hex: &str) -> (u8, u8, u8) {
    let h = hex.trim().trim_start_matches('#');
    let c = |i: usize| u8::from_str_radix(h.get(i..i + 2).unwrap_or("ff"), 16).unwrap_or(255);
    (c(0), c(2), c(4))
}

/// One rasterised text image: RGBA premultiplied-by-nothing, straight alpha.
pub struct TextImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl TextImage {
    pub fn write_png(&self, path: &Path) -> TextResult<()> {
        let file = std::fs::File::create(path)?;
        let mut enc = png::Encoder::new(BufWriter::new(file), self.width, self.height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().map_err(|e| TextError::Png(e.to_string()))?;
        w.write_image_data(&self.rgba).map_err(|e| TextError::Png(e.to_string()))?;
        Ok(())
    }
}

/// What a caption or title should look like, resolved to output pixels.
pub struct TextSpec<'a> {
    pub content: &'a str,
    pub style: &'a TextStyle,
    /// Output frame size (the image is the whole frame, so it can be overlaid
    /// with no positioning maths at all).
    pub out_w: u32,
    pub out_h: u32,
    /// Fraction of the frame width a line may use before it wraps.
    pub width_fraction: f64,
}

/// Usable fraction of the frame width (mirrors ue-export's caption width).
pub const DEFAULT_WIDTH_FRACTION: f64 = 0.86;

/// Renders `spec` into a frame-sized transparent RGBA image.
///
/// Sizes and offsets are 1080p-relative, the same convention every other text
/// path in the project uses, so a style keeps its look at any resolution.
pub fn render(spec: &TextSpec) -> TextImage {
    let (w, h) = (spec.out_w.max(2), spec.out_h.max(2));
    let mut rgba = vec![0u8; (w as usize) * (h as usize) * 4];
    if spec.content.trim().is_empty() {
        return TextImage { width: w, height: h, rgba };
    }

    let scale = h as f32 / 1080.0;
    let px = (spec.style.size * scale).max(8.0);
    let line_h = px * spec.style.line_height.max(0.6);
    let max_w = (w as f64 * spec.width_fraction) as f32;

    let fs = font_system();
    let mut fs = fs.lock().expect("font system");
    let mut cache = SwashCache::new();

    let mut buffer = Buffer::new(&mut fs, Metrics::new(px, line_h));
    let mut buf = buffer.borrow_with(&mut fs);
    buf.set_size(Some(max_w), None);
    buf.set_wrap(cosmic_text::Wrap::Word);

    // an empty / generic family lets fontconfig-style matching pick the default;
    // cosmic-text falls back per glyph regardless, which is the point
    let family = spec.style.font.trim();
    let attrs = if family.is_empty() || family.eq_ignore_ascii_case("sans-serif") {
        Attrs::new().family(Family::SansSerif)
    } else {
        Attrs::new().family(Family::Name(family))
    }
    .weight(Weight::NORMAL);

    buf.set_text(spec.content, &attrs, Shaping::Advanced, None);
    buf.shape_until_scroll(true);

    // measure the block so it can be centred on the style's y_offset
    let lines: Vec<_> = buf.layout_runs().collect();
    if lines.is_empty() {
        drop(buf);
        return TextImage { width: w, height: h, rgba };
    }
    let n = lines.len() as f32;
    let block_h = n * line_h;
    let centre_y = h as f32 / 2.0 + spec.style.y_offset * scale;
    let top = centre_y - block_h / 2.0;
    let x_off = spec.style.x_offset * scale;
    let margin = 48.0 * scale;

    // per-line horizontal placement, honouring the style's alignment
    let offsets: Vec<f32> = lines
        .iter()
        .map(|run| match spec.style.align {
            TextAlign::Left => margin + x_off,
            TextAlign::Right => w as f32 - margin + x_off - run.line_w,
            TextAlign::Center => (w as f32 - run.line_w) / 2.0 + x_off,
        })
        .collect();
    drop(lines);

    let (tr, tg, tb) = rgb(&spec.style.color);
    let fill = Color::rgba(tr, tg, tb, 255);
    // the same 2px-at-1080p outline the ffmpeg paths drew, so styles look the
    // same as before for text that was already rendering
    let outline_px = (2.0 * scale).round().max(1.0) as i32;

    let mut plot = |x: i32, y: i32, r: u8, g: u8, b: u8, a: u8| {
        if a == 0 || x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
            return;
        }
        let i = ((y as usize) * (w as usize) + x as usize) * 4;
        // straight-alpha "over"
        let sa = a as f32 / 255.0;
        let da = rgba[i + 3] as f32 / 255.0;
        let oa = sa + da * (1.0 - sa);
        if oa <= 0.0 {
            return;
        }
        for (k, sc) in [r, g, b].into_iter().enumerate() {
            let dc = rgba[i + k] as f32 / 255.0;
            let oc = (sc as f32 / 255.0 * sa + dc * da * (1.0 - sa)) / oa;
            rgba[i + k] = (oc * 255.0).round().clamp(0.0, 255.0) as u8;
        }
        rgba[i + 3] = (oa * 255.0).round().clamp(0.0, 255.0) as u8;
    };

    // Collect every glyph's rasterisation key and where it lands, THEN draw.
    // (Shaping borrows the font system; so does rasterising — they cannot both
    // hold it, and mixing them is what the borrow checker was complaining about.)
    struct Placed {
        key: cosmic_text::CacheKey,
        x: i32,
        y: i32,
    }
    let mut placed: Vec<Placed> = vec![];
    {
        let mut buf = buffer.borrow_with(&mut fs);
        for (li, run) in buf.layout_runs().enumerate() {
            let ox = offsets.get(li).copied().unwrap_or(0.0);
            // `run.line_y` is the BASELINE measured from the top of the buffer and
            // already carries the line index; adding `li * line_h` on top of it
            // counted every line twice and marched the block down the frame.
            for glyph in run.glyphs.iter() {
                let phys = glyph.physical((ox, top), 1.0);
                placed.push(Placed {
                    key: phys.cache_key,
                    x: phys.x,
                    y: phys.y + run.line_y as i32,
                });
            }
        }
    }

    // Two passes: the dark outline first, then the glyphs on top. Colour glyphs
    // (the emoji) carry their own RGB and are drawn as-is; monochrome ones are
    // a coverage mask tinted with the style colour.
    for pass in 0..2 {
        for g in &placed {
            let Some(img) = cache.get_image_uncached(&mut fs, g.key) else { continue };
            let gx = g.x + img.placement.left;
            let gy = g.y - img.placement.top;
            let (iw, ih) = (img.placement.width as i32, img.placement.height as i32);
            match img.content {
                SwashContent::Mask => {
                    for py in 0..ih {
                        for pxx in 0..iw {
                            let a = img.data[(py * iw + pxx) as usize];
                            if a == 0 {
                                continue;
                            }
                            if pass == 0 {
                                // outline: stamp the coverage around the glyph
                                for dy in -outline_px..=outline_px {
                                    for dx in -outline_px..=outline_px {
                                        if dx * dx + dy * dy > outline_px * outline_px {
                                            continue;
                                        }
                                        plot(
                                            gx + pxx + dx,
                                            gy + py + dy,
                                            0,
                                            0,
                                            0,
                                            (a as f32 * 0.6) as u8,
                                        );
                                    }
                                }
                            } else {
                                plot(gx + pxx, gy + py, fill.r(), fill.g(), fill.b(), a);
                            }
                        }
                    }
                }
                SwashContent::Color => {
                    // a COLOUR glyph: an emoji, already RGBA. No outline pass —
                    // an emoji does not want one.
                    if pass == 0 {
                        continue;
                    }
                    for py in 0..ih {
                        for pxx in 0..iw {
                            let o = ((py * iw + pxx) * 4) as usize;
                            plot(
                                gx + pxx,
                                gy + py,
                                img.data[o],
                                img.data[o + 1],
                                img.data[o + 2],
                                img.data[o + 3],
                            );
                        }
                    }
                }
                SwashContent::SubpixelMask => {}
            }
        }
    }

    TextImage { width: w, height: h, rgba }
}

/// Renders and writes a PNG into `dir`, returning its path.
pub fn render_to_png(spec: &TextSpec, dir: &Path) -> TextResult<PathBuf> {
    let img = render(spec);
    let path = dir.join(format!("ue_text_{}.png", ue_core::model::Id::new()));
    img.write_png(&path)?;
    Ok(path)
}
