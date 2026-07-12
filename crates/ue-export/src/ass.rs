//! Subtitles and titles as an ASS script, burned in with libass.
//!
//! They used to be thousands of `drawtext` filters — two per word for karaoke,
//! each carrying its own `enable='between(t,…)'`. The filtergraph grew about a
//! kilobyte per second of speech and blew past ffmpeg's parser at ~1 MB, so a
//! long karaoke export simply refused to run ("the subtitle filtergraph is too
//! large"). It also meant `drawtext`'s limits were ours:
//!
//! * **no font fallback.** `drawtext` loads exactly ONE face, so anything the
//!   chosen font lacks — every emoji, most CJK — rendered as a .notdef box.
//!   libass resolves fallbacks through fontconfig, per glyph.
//! * karaoke, line breaks and per-word colouring all had to be hand-built.
//!   ASS has `\k`, `\N` and `\pos` natively.
//!
//! The wrap is still OURS (`graph::wrap_words`, measured with the real font) and
//! is baked in as explicit `\N`, with `WrapStyle: 2` so libass adds none of its
//! own — that keeps the canvas compositor able to reproduce the same breaks.

use std::fmt::Write as _;

use ue_core::model::{Project, Sequence, TextStyle};
use ue_core::TimeUs;

use crate::graph::{
    caption_max_chars, caption_phrases_pub, resolve_font_family, wrap_words, CAPTION_WIDTH_FRACTION,
};

/// `#rrggbb` → ASS `&HAABBGGRR` (ASS is BGR, and alpha is INVERTED: 00 opaque).
fn ass_color(hex: &str, opacity: f64) -> String {
    let h = hex.trim().trim_start_matches('#');
    let c = |i: usize| u8::from_str_radix(h.get(i..i + 2).unwrap_or("00"), 16).unwrap_or(0);
    let (r, g, b) = (c(0), c(2), c(4));
    let a = ((1.0 - opacity.clamp(0.0, 1.0)) * 255.0).round() as u8;
    format!("&H{a:02X}{b:02X}{g:02X}{r:02X}")
}

/// µs → `h:mm:ss.cc` (ASS resolution is the centisecond).
fn ass_time(us: TimeUs) -> String {
    let cs = (us.max(0) + 5_000) / 10_000; // round to centiseconds
    let (h, rem) = (cs / 360_000, cs % 360_000);
    let (m, rem) = (rem / 6_000, rem % 6_000);
    let (s, cs) = (rem / 100, rem % 100);
    format!("{h}:{m:02}:{s:02}.{cs:02}")
}

/// `{`, `}` and newlines would be read as override tags / event breaks.
fn ass_escape(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('{', "\\{")
        .replace('}', "\\}")
        .replace(['\n', '\r'], " ")
}

/// Where a style wants its text, in output pixels, and which ASS alignment
/// anchors it there.
fn position(style: &TextStyle, out_w: u32, out_h: u32, scale: f64) -> (u32, i64, i64) {
    use ue_core::model::TextAlign;
    let y = (out_h as f64 / 2.0 + style.y_offset as f64 * scale).round() as i64;
    let margin = (48.0 * scale).round() as i64;
    let x_off = (style.x_offset as f64 * scale).round() as i64;
    match style.align {
        // numpad alignment: 4 = middle-left, 5 = middle-centre, 6 = middle-right
        TextAlign::Left => (4, margin + x_off, y),
        TextAlign::Center => (5, (out_w as i64) / 2 + x_off, y),
        TextAlign::Right => (6, out_w as i64 - margin + x_off, y),
    }
}

struct StyleEntry {
    name: String,
    line: String,
}

/// Declares one ASS style per TextStyle we meet (deduplicated).
fn style_entry(name: &str, style: &TextStyle, scale: f64) -> StyleEntry {
    let px = ((style.size as f64) * scale).round().max(8.0) as i64;
    // libass resolves the family through fontconfig, which is exactly what
    // gives us the per-glyph fallback drawtext never had
    let font = if style.font.trim().is_empty() || style.font == "sans-serif" {
        "Sans".to_string()
    } else {
        style.font.clone()
    };
    // karaoke: Primary is the colour a syllable turns INTO, Secondary the one
    // it waits in. For plain text only Primary is ever used.
    let primary = ass_color(style.highlight_color.as_deref().unwrap_or(&style.color), 1.0);
    let secondary = ass_color(&style.color, 0.4);
    let outline = (2.0 * scale).round().max(1.0) as i64;
    let line = format!(
        "Style: {name},{font},{px},{primary},{secondary},&H00000000,&H00000000,\
         0,0,0,0,100,100,0,0,1,{outline},0,5,10,10,10,1"
    );
    StyleEntry { name: name.to_string(), line }
}

/// A style whose Primary colour is the plain text colour (no karaoke).
fn plain_style_entry(name: &str, style: &TextStyle, scale: f64) -> StyleEntry {
    let mut s = style.clone();
    s.highlight_color = None;
    style_entry(name, &s, scale)
}

/// One Dialogue line.
fn dialogue(start: TimeUs, end: TimeUs, style_name: &str, an: u32, x: i64, y: i64, text: &str) -> String {
    format!(
        "Dialogue: 0,{},{},{style_name},,0,0,0,,{{\\an{an}\\pos({x},{y})}}{text}",
        ass_time(start),
        ass_time(end),
    )
}

/// Our own wrap, baked in as explicit `\N` breaks (libass adds none: WrapStyle 2).
fn wrapped_text(font_path: Option<&str>, words: &[&str], px: f64, max_w: f64) -> String {
    wrap_words(font_path, words, px, max_w)
        .into_iter()
        .map(|line| line.iter().map(|w| ass_escape(w)).collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join("\\N")
}

/// Builds the whole ASS script for a sequence's titles and subtitles, or `None`
/// when there is no text at all.
///
/// `only` renders a single clip (a styled text clip that is composited as its
/// own layer); `None` renders every plain one.
pub fn build_script(
    project: &Project,
    seq: &Sequence,
    out_w: u32,
    out_h: u32,
    only: Option<ue_core::model::Id>,
) -> Option<String> {
    use ue_core::model::{ClipPayload, SubtitleMode, TrackKind};
    let scale = out_h as f64 / 1080.0;
    let max_w = out_w as f64 * CAPTION_WIDTH_FRACTION;

    let mut styles: Vec<StyleEntry> = vec![];
    let mut events: Vec<String> = vec![];

    for track in seq.tracks.iter().filter(|t| t.kind == TrackKind::Video && !t.muted) {
        for clip in &track.clips {
            match only {
                Some(id) if clip.id != id => continue,
                None if crate::graph::text_is_styled_pub(clip) => continue, // its own layer
                _ => {}
            }
            match &clip.payload {
                // TITLES are not here any more: they are rasterised by ue-text
                // (which reads colour fonts) and composited as image layers, so
                // an emoji in a title is an emoji and not a .notdef box. libass
                // still owns SUBTITLES, where its native karaoke pays for itself
                // and Whisper never emits an emoji anyway.
                ClipPayload::Subtitles { transcript_id, style, mode, max_words } => {
                    let Some(doc) = project.transcripts.iter().find(|t| t.id == *transcript_id)
                    else {
                        continue;
                    };
                    let px = (style.size as f64) * scale;
                    let font = resolve_font_family(&style.font);
                    let karaoke = *mode == SubtitleMode::Karaoke;
                    let name = format!("s{}", styles.len());
                    styles.push(if karaoke {
                        style_entry(&name, style, scale)
                    } else {
                        plain_style_entry(&name, style, scale)
                    });

                    // WORD mode keeps its own bigger style
                    let mut wstyle = style.clone();
                    if *mode == SubtitleMode::Word {
                        wstyle.size *= 1.6;
                    }
                    let wpx = (wstyle.size as f64) * scale;
                    let wname = if *mode == SubtitleMode::Word {
                        let n = format!("w{}", styles.len());
                        styles.push(plain_style_entry(&n, &wstyle, scale));
                        n
                    } else {
                        name.clone()
                    };
                    let (an, x, y) = position(&wstyle, out_w, out_h, scale);

                    match mode {
                        SubtitleMode::Word => {
                            for w in doc.words.iter().filter(|w| !w.rejected) {
                                let Some(tl) =
                                    crate::graph::asset_time_to_timeline_pub(seq, doc.asset_id, w.start_us)
                                else {
                                    continue;
                                };
                                let from = tl.max(clip.start);
                                let to = (tl + (w.end_us - w.start_us)).min(clip.end());
                                if to <= from {
                                    continue;
                                }
                                events.push(dialogue(
                                    from, to, &wname, an, x, y,
                                    &ass_escape(w.label()),
                                ));
                            }
                        }
                        SubtitleMode::Phrase | SubtitleMode::Karaoke => {
                            let phrases =
                                caption_phrases_pub(doc, caption_max_chars(out_w, px), *max_words);
                            for (text, ps, pe) in &phrases {
                                let Some(tl) =
                                    crate::graph::asset_time_to_timeline_pub(seq, doc.asset_id, *ps)
                                else {
                                    continue;
                                };
                                let from = tl.max(clip.start);
                                let to = (tl + (pe - ps)).min(clip.end());
                                if to <= from {
                                    continue;
                                }
                                let body = if karaoke {
                                    // \k<centiseconds> per word: libass fades each
                                    // one from Secondary to Primary as it is spoken
                                    let words: Vec<&ue_core::model::Word> = doc
                                        .words
                                        .iter()
                                        .filter(|w| !w.rejected && w.start_us >= *ps && w.start_us < *pe)
                                        .collect();
                                    if words.is_empty() {
                                        continue;
                                    }
                                    let labels: Vec<&str> = words.iter().map(|w| w.label()).collect();
                                    let lines = wrap_words(font.as_deref(), &labels, px, max_w);
                                    let mut out = String::new();
                                    let mut i = 0usize;
                                    for (li, line) in lines.iter().enumerate() {
                                        if li > 0 {
                                            out.push_str("\\N");
                                        }
                                        for w in line {
                                            let word = words[i];
                                            // how long this word holds the highlight
                                            let dur_cs = ((word.end_us - word.start_us).max(1)
                                                / 10_000)
                                                .max(1);
                                            let _ = write!(
                                                out,
                                                "{{\\k{dur_cs}}}{} ",
                                                ass_escape(w)
                                            );
                                            i += 1;
                                        }
                                    }
                                    out.trim_end().to_string()
                                } else {
                                    let words: Vec<&str> = text.split_whitespace().collect();
                                    wrapped_text(font.as_deref(), &words, px, max_w)
                                };
                                events.push(dialogue(from, to, &name, an, x, y, &body));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    if events.is_empty() {
        return None;
    }

    let mut s = String::new();
    let _ = write!(
        s,
        "[Script Info]\n\
         ScriptType: v4.00+\n\
         PlayResX: {out_w}\n\
         PlayResY: {out_h}\n\
         WrapStyle: 2\n\
         ScaledBorderAndShadow: yes\n\
         YCbCr Matrix: None\n\n\
         [V4+ Styles]\n\
         Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, \
         BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, \
         BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n"
    );
    for st in &styles {
        let _ = writeln!(s, "{}", st.line);
    }
    let _ = write!(
        s,
        "\n[Events]\n\
         Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n"
    );
    for e in &events {
        let _ = writeln!(s, "{e}");
    }
    Some(s)
}

/// Writes the script next to `dir` with a unique name and returns its path.
/// The caller deletes it once ffmpeg has run.
pub fn write_script(script: &str, dir: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
    let name = format!("ue_subs_{}.ass", ue_core::model::Id::new());
    let path = dir.join(name);
    std::fs::write(&path, script)?;
    Ok(path)
}

/// The `ass=` filter, with the path escaped for a filter_complex.
pub fn ass_filter(path: &std::path::Path) -> String {
    let p = path.to_string_lossy();
    // inside a filter argument: \ : ' and , all need escaping
    let esc = p
        .replace('\\', "\\\\")
        .replace(':', "\\:")
        .replace('\'', "\\'")
        .replace(',', "\\,");
    format!("ass='{esc}'")
}
