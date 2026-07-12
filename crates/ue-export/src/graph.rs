//! Building the ffmpeg command line (inputs + filter_complex).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ue_core::model::{ClipPayload, Id, Project, TrackKind};
use ue_core::TimeUs;

use crate::edl::{build_video_edl_with, edl_duration, Segment};
use crate::{ExportError, ExportResult, ExportSettings};

pub struct FfmpegPlan {
    pub args: Vec<String>,
    pub duration_us: TimeUs,
    /// Subtitle script written for this run; the runner deletes it afterwards.
    pub subs_file: Option<PathBuf>,
    /// Rasterised title images for this run; the runner deletes them too.
    pub temp_files: Vec<PathBuf>,
}

fn secs(us: TimeUs) -> String {
    format!("{:.6}", us as f64 / 1_000_000.0)
}

fn resolve_path(base: &Path, p: &str) -> PathBuf {
    let path = Path::new(p);
    if path.is_absolute() { path.to_path_buf() } else { base.join(path) }
}

/// Same path resolution the export uses, for the preview compositor.
pub fn resolve_path_pub(base: &Path, p: &str) -> PathBuf {
    resolve_path(base, p)
}

/// Audible clip: any media clip (on an audio or video track) whose asset
/// has audio, with no clip or track mute.
struct AudioItem {
    asset_id: Id,
    src_in: TimeUs,
    src_out: TimeUs,
    start: TimeUs,
    speed: f64,
    /// Static part (clip const + track volume).
    gain_db: f64,
    /// Gain curve in dB (times relative to the clip start).
    gain_curve: Option<ue_core::keyframe::KeyframeCurve>,
    /// Balance -1..1 (same law as the live mixer).
    pan: f64,
    denoise: bool,
    /// Denoised conform WAV to read audio from (DNS64 output): exact parity
    /// with the live mixer. None → original asset (afftdn fallback inline).
    input_override: Option<PathBuf>,
    fade_in_us: TimeUs,
    fade_out_us: TimeUs,
}

/// `volume` filter expression (eval=frame) for a dB curve: exact Hold/Linear
/// segments; Smooth is linearized between keys (v0). `t` starts at 0 at the
/// clip start (post atrim+asetpts+atempo). `offset_db` = track + const.
fn volume_expr(curve: &ue_core::keyframe::KeyframeCurve, offset_db: f64) -> String {
    use ue_core::keyframe::Interp;
    let keys = &curve.keys;
    if keys.is_empty() {
        return "1".into();
    }
    let lin = |db: f64| format!("pow(10,{:.4}/20)", db + offset_db);
    let ts = |us: TimeUs| format!("{:.6}", us as f64 / 1_000_000.0);
    // from the inside out: value after the last key
    let mut expr = lin(keys[keys.len() - 1].value);
    for i in (0..keys.len().saturating_sub(1)).rev() {
        let (k0, k1) = (&keys[i], &keys[i + 1]);
        let seg = match k0.interp {
            Interp::Hold => lin(k0.value),
            _ => format!(
                "pow(10,({:.4}+({:.4})*(t-{})/({:.6}))/20)",
                k0.value + offset_db,
                k1.value - k0.value,
                ts(k0.t),
                ((k1.t - k0.t).max(1)) as f64 / 1_000_000.0,
            ),
        };
        expr = format!("if(lt(t,{}),{seg},{expr})", ts(k1.t));
    }
    format!("if(lt(t,{}),{},{expr})", ts(keys[0].t), lin(keys[0].value))
}

/// atempo chain (preserves pitch). atempo accepts 0.5–2 per instance:
/// several are chained for extreme factors.
fn atempo_chain(speed: f64) -> String {
    let mut parts: Vec<String> = vec![];
    let mut rest = speed;
    while rest > 2.0 {
        parts.push("atempo=2".into());
        rest /= 2.0;
    }
    while rest < 0.5 {
        parts.push("atempo=0.5".into());
        rest /= 0.5;
    }
    if (rest - 1.0).abs() > 1e-9 {
        parts.push(format!("atempo={}", (rest * 10000.0).round() / 10000.0));
    }
    parts.join(",")
}

fn collect_audio(project: &Project, sequence_id: Id) -> Vec<AudioItem> {
    let Some(seq) = project.sequence(sequence_id) else { return vec![] };
    let mut items = vec![];
    let any_solo = seq.tracks.iter().any(|t| t.solo);
    for track in &seq.tracks {
        if track.muted || (any_solo && !track.solo) {
            continue;
        }
        for clip in &track.clips {
            if clip.audio.muted {
                continue;
            }
            if let ClipPayload::Media { asset_id, src_in, src_out } = &clip.payload {
                let Some(asset) = project.asset(*asset_id) else { continue };
                if asset.probe.audio_channels == 0 {
                    continue;
                }
                let (gain_const, gain_curve) = match &clip.audio.gain_db {
                    ue_core::keyframe::Param::Const(v) => (*v, None),
                    ue_core::keyframe::Param::Curve(c) => (0.0, Some(c.clone())),
                };
                items.push(AudioItem {
                    asset_id: *asset_id,
                    src_in: *src_in,
                    src_out: *src_out,
                    start: clip.start,
                    speed: clip.speed,
                    gain_db: gain_const + track.volume_db as f64,
                    gain_curve,
                    pan: clip.audio.pan.eval(0).clamp(-1.0, 1.0),
                    denoise: clip.audio.denoise,
                    input_override: clip
                        .audio
                        .denoise
                        .then(|| {
                            asset.audio_conform.as_ref().map(|c| {
                                ue_media::denoise::denoised_path(std::path::Path::new(c))
                            })
                        })
                        .flatten()
                        .filter(|p| p.exists()),
                    fade_in_us: clip.audio.fade_in_us,
                    fade_out_us: clip.audio.fade_out_us,
                });
            }
        }
    }
    items
}

// ---------------------------------------------------------------------------
// Line wrapping
//
// Captions are laid out on ONE line unless they don't fit, and then they wrap.
// The break decision is made from a REAL measurement of the chosen font (the
// sum of the glyph advances, `measure_text_px`), never from a character-count
// guess — an Arial Black caption and a Helvetica one break in different places
// and only the font knows where.
//
// The canvas compositor mirrors this exact algorithm (word widths + one space,
// greedy fill, never split a word), so playback wraps where the export wraps.
// ---------------------------------------------------------------------------

/// Usable fraction of the frame width for a caption.
pub const CAPTION_WIDTH_FRACTION: f64 = 0.86;

/// Width of one word in px, measured with the real font (or approximated when
/// the family resolves to no file on disk).
fn word_width(font_path: Option<&str>, word: &str, px: f64) -> f64 {
    match font_path {
        Some(p) => measure_text_px(p, word, px).unwrap_or(px * 0.5 * word.chars().count() as f64),
        // no fontfile (fontconfig `font=sans`): fall back to the old heuristic
        None => px * 0.52 * word.chars().count() as f64,
    }
}

/// Greedy word wrap into lines that each fit `max_w` px. A single word wider
/// than the line gets its own line (we never break inside a word).
pub fn wrap_words<'a>(
    font_path: Option<&str>,
    words: &[&'a str],
    px: f64,
    max_w: f64,
) -> Vec<Vec<&'a str>> {
    if words.is_empty() {
        return vec![];
    }
    let space = word_width(font_path, " ", px);
    let mut lines: Vec<Vec<&str>> = vec![];
    let mut line: Vec<&str> = vec![];
    let mut width = 0.0f64;
    for w in words {
        let ww = word_width(font_path, w, px);
        let add = if line.is_empty() { ww } else { space + ww };
        if !line.is_empty() && width + add > max_w {
            lines.push(std::mem::take(&mut line));
            width = ww;
            line.push(w);
        } else {
            width += add;
            line.push(w);
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// Wraps a whole caption string.
pub fn wrap_text(font_path: Option<&str>, text: &str, px: f64, max_w: f64) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    wrap_words(font_path, &words, px, max_w)
        .into_iter()
        .map(|l| l.join(" "))
        .collect()
}

/// Vertical offset of line `i` of `n`, so the whole block stays centred on the
/// style's `y_offset` (a 2-line caption must not drift downwards).
fn line_y_offset(i: usize, n: usize, px: f64, line_height: f32) -> f64 {
    let step = px * line_height.max(0.6) as f64;
    (i as f64 - (n as f64 - 1.0) / 2.0) * step
}

/// Escaping for drawtext's text='…' value inside a filter_complex.
fn escape_drawtext(text: &str) -> String {
    text.replace('\\', "\\\\\\\\")
        .replace('\'', "\u{2019}") // typographic apostrophe: avoids quoting hell
        .replace(':', "\\:")
        .replace('%', "\\%")
}

/// System font database (loaded once).
fn font_db() -> &'static fontdb::Database {
    static DB: std::sync::OnceLock<fontdb::Database> = std::sync::OnceLock::new();
    DB.get_or_init(|| {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        db
    })
}

/// Available system fonts: unique, sorted (family, path).
pub fn list_system_fonts() -> Vec<(String, String)> {
    let mut out: std::collections::BTreeMap<String, String> = Default::default();
    let db = font_db();
    for face in db.faces() {
        let Some((family, _)) = face.families.first() else { continue };
        if let fontdb::Source::File(path) = &face.source {
            out.entry(family.clone())
                .or_insert_with(|| path.to_string_lossy().into_owned());
        }
    }
    out.into_iter().collect()
}

/// Resolves a family name to a fontfile on disk, or `None`.
///
/// The generic CSS families (`sans-serif`, `serif`, `monospace`, …) are NOT
/// real family names, so fontdb can't find them; they are mapped to the first
/// real font of that class that is actually installed. This keeps the default
/// `sans-serif` renderable everywhere and makes the preview and the export
/// pick the SAME concrete file.
pub fn resolve_font_family(family: &str) -> Option<String> {
    let name = family.trim();
    match name.to_ascii_lowercase().as_str() {
        "sans-serif" | "sans" | "" => resolve_first(SANS_CANDIDATES),
        "serif" => resolve_first(SERIF_CANDIDATES),
        "monospace" | "mono" => resolve_first(MONO_CANDIDATES),
        _ => resolve_named(name),
    }
}

/// Whether a family name (generic or literal) resolves to an installed font.
/// Used to warn the agent/UI before a clip silently draws nothing.
pub fn font_is_available(family: &str) -> bool {
    resolve_font_family(family).is_some()
}

const SANS_CANDIDATES: &[&str] =
    &["Arial", "Helvetica", "Helvetica Neue", "Liberation Sans", "DejaVu Sans", "Segoe UI", "Roboto"];
const SERIF_CANDIDATES: &[&str] =
    &["Times New Roman", "Georgia", "Liberation Serif", "DejaVu Serif", "Times"];
const MONO_CANDIDATES: &[&str] =
    &["Menlo", "Courier New", "Liberation Mono", "DejaVu Sans Mono", "Consolas", "Courier"];

/// First candidate that resolves; failing that, any installed font at all.
fn resolve_first(candidates: &[&str]) -> Option<String> {
    candidates
        .iter()
        .find_map(|c| resolve_named(c))
        .or_else(|| find_font().map(str::to_string))
}

fn resolve_named(family: &str) -> Option<String> {
    let db = font_db();
    let query = fontdb::Query {
        families: &[fontdb::Family::Name(family)],
        ..Default::default()
    };
    let id = db.query(&query)?;
    let (source, _) = db.face_source(id)?;
    match source {
        fontdb::Source::File(path) => Some(path.to_string_lossy().into_owned()),
        _ => None,
    }
}

/// First available system font for drawtext (fontfile);
/// if none exist, we rely on fontconfig (font=sans).
pub fn find_font() -> Option<&'static str> {
    const CANDIDATES: &[&str] = &[
        "/System/Library/Fonts/Supplemental/Arial.ttf",
        "/System/Library/Fonts/Helvetica.ttc",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
        "C:\\Windows\\Fonts\\arial.ttf",
    ];
    CANDIDATES.iter().copied().find(|p| Path::new(p).exists())
}

/// A drawtext with the project's style, active in [from, to) of the timeline.
/// The style's fontfile (family resolved with fontdb) or the global fallback.
fn font_part_for(style: &ue_core::model::TextStyle, fallback: &str) -> String {
    resolve_font_family(&style.font)
        .map(|f| format!("fontfile={f}"))
        .unwrap_or_else(|| fallback.to_string())
}

/// drawtext x expression based on the style's alignment and X offset.
fn x_expr_for(style: &ue_core::model::TextStyle, scale: f64) -> String {
    use ue_core::model::TextAlign;
    let x_off = (style.x_offset as f64 * scale).round() as i64;
    let margin = (48.0 * scale).round() as i64;
    match style.align {
        TextAlign::Left => format!("{}", margin + x_off),
        TextAlign::Center => format!("(w-text_w)/2+{x_off}"),
        TextAlign::Right => format!("w-text_w-{}", margin - x_off),
    }
}

/// `:enable='between(t,a,b)'` for the burned-in export, or nothing for the
/// single-frame preview (which already picks the active item in Rust, so the
/// filter must draw unconditionally).
fn enable_clause(window: Option<(TimeUs, TimeUs)>) -> String {
    match window {
        Some((from, to)) => format!(":enable='between(t,{},{})'", secs(from), secs(to)),
        None => String::new(),
    }
}

/// One drawtext PER LINE: the caption is wrapped with the real font metrics and
/// each line placed itself, so the block stays centred on `y_offset` and the
/// line spacing is exactly `style.line_height`. Emitting a single drawtext with
/// embedded newlines would hand the spacing to ffmpeg and make the canvas
/// unable to match it.
fn drawtext_for(
    font_part: &str,
    content: &str,
    style: &ue_core::model::TextStyle,
    scale: f64,
    enable: Option<(TimeUs, TimeUs)>,
    out_w: u32,
) -> Vec<String> {
    let px = ((style.size as f64) * scale).round().max(8.0);
    let fontsize = px as u32;
    let color = style.color.trim_start_matches('#');
    let y_off = (style.y_offset as f64 * scale).round() as i64;
    let resolved = font_part_for(style, font_part);
    let font_path = resolved.strip_prefix("fontfile=");
    let max_w = out_w as f64 * CAPTION_WIDTH_FRACTION;
    let lines = wrap_text(font_path, content, px, max_w);
    let n = lines.len();
    lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let dy = y_off + line_y_offset(i, n, px, style.line_height).round() as i64;
            format!(
                "drawtext={resolved}:text='{}':fontsize={fontsize}:fontcolor=0x{color}:\
                 borderw={}:bordercolor=black@0.6:x={}:y=(h-text_h)/2+{dy}{}",
                escape_drawtext(line),
                (2.0 * scale).round().max(1.0) as u32,
                x_expr_for(style, scale),
                enable_clause(enable),
            )
        })
        .collect()
}

/// drawtext with an explicit x expression (karaoke: precomputed positions).
#[allow(clippy::too_many_arguments)]
fn drawtext_at(
    font_part: &str,
    content: &str,
    fontsize: u32,
    color: &str,
    x_expr: &str,
    y_off: i64,
    scale: f64,
    enable: Option<(TimeUs, TimeUs)>,
) -> String {
    format!(
        "drawtext={font_part}:text='{}':fontsize={fontsize}:fontcolor={color}:\
         borderw={}:bordercolor=black@0.6:x={x_expr}:y=(h-text_h)/2+{y_off}{}",
        escape_drawtext(content),
        (2.0 * scale).round().max(1.0) as u32,
        enable_clause(enable),
    )
}

/// Width in px of `text` with font `path` at size `px` (sum of advances).
fn measure_text_px(path: &str, text: &str, px: f64) -> Option<f64> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<String, std::sync::Arc<Vec<u8>>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let data = {
        let mut guard = cache.lock().unwrap();
        match guard.get(path) {
            Some(d) => d.clone(),
            None => {
                let d = std::sync::Arc::new(std::fs::read(path).ok()?);
                guard.insert(path.to_string(), d.clone());
                d
            }
        }
    };
    let face = ttf_parser::Face::parse(&data, 0).ok()?;
    let upem = face.units_per_em() as f64;
    let mut units = 0.0f64;
    for c in text.chars() {
        units += match face.glyph_index(c).and_then(|g| face.glyph_hor_advance(g)) {
            Some(adv) => adv as f64,
            None => upem * 0.5, // unknown glyph: half em
        };
    }
    Some(units / upem * px)
}

/// Asset time → timeline via the first media clip that contains it.
fn asset_time_to_timeline(
    seq: &ue_core::model::Sequence,
    asset_id: Id,
    t_asset: TimeUs,
) -> Option<TimeUs> {
    use ue_core::model::ClipPayload;
    for track in &seq.tracks {
        for clip in &track.clips {
            if let ClipPayload::Media { asset_id: aid, src_in, src_out } = &clip.payload {
                if *aid == asset_id && t_asset >= *src_in && t_asset < *src_out {
                    return Some(clip.start + (t_asset - src_in));
                }
            }
        }
    }
    None
}

/// AUDIO-ONLY export plan (.m4a): mixes the audio chains without video.
fn build_audio_only_args(
    project: &Project,
    sequence_id: Id,
    base_dir: &Path,
    output: &Path,
    settings: &ExportSettings,
) -> ExportResult<FfmpegPlan> {
    let audio_items = collect_audio(project, sequence_id);
    if audio_items.is_empty() {
        return Err(ExportError::EmptyTimeline);
    }
    let total_us = audio_items
        .iter()
        .map(|i| i.start + (((i.src_out - i.src_in) as f64) / i.speed).round() as TimeUs)
        .max()
        .unwrap_or(0);

    let mut input_index: BTreeMap<PathBuf, usize> = BTreeMap::new();
    let mut inputs: Vec<PathBuf> = vec![];
    let mut input_of_path = |path: PathBuf| -> usize {
        *input_index.entry(path.clone()).or_insert_with(|| {
            inputs.push(path);
            inputs.len() - 1
        })
    };
    let mut fc: Vec<String> = vec![];
    let mut alabels: Vec<String> = vec![];
    for (k, item) in audio_items.iter().enumerate() {
        let path = item.input_override.clone().unwrap_or_else(|| {
            let asset = project.asset(item.asset_id).expect("validated when collecting");
            resolve_path(base_dir, &asset.path)
        });
        let idx = input_of_path(path);
        let label = format!("a{k}");
        let dur_us = (((item.src_out - item.src_in) as f64) / item.speed).round() as TimeUs;
        let mut chain = format!(
            "[{idx}:a]atrim=start={}:end={},asetpts=PTS-STARTPTS,\
             aresample=48000,aformat=channel_layouts=stereo",
            secs(item.src_in),
            secs(item.src_out),
        );
        if item.denoise && item.input_override.is_none() {
            // fallback: the denoised conform isn't rendered yet
            chain.push(',');
            chain.push_str(ue_media::denoise::DENOISE_FILTER);
        }
        if (item.speed - 1.0).abs() > 1e-9 {
            chain.push(',');
            chain.push_str(&atempo_chain(item.speed));
        }
        match &item.gain_curve {
            Some(curve) => chain.push_str(&format!(
                ",volume=volume='{}':eval=frame",
                volume_expr(curve, item.gain_db)
            )),
            None if item.gain_db.abs() > 1e-9 => {
                chain.push_str(&format!(",volume={:.2}dB", item.gain_db));
            }
            None => {}
        }
        if item.pan.abs() > 1e-3 {
            let (pl, pr) = ((1.0 - item.pan).min(1.0), (1.0 + item.pan).min(1.0));
            chain.push_str(&format!(",pan=stereo|c0={pl:.4}*c0|c1={pr:.4}*c1"));
        }
        if item.fade_in_us > 0 {
            chain.push_str(&format!(",afade=t=in:st=0:d={}", secs(item.fade_in_us)));
        }
        if item.fade_out_us > 0 {
            chain.push_str(&format!(
                ",afade=t=out:st={}:d={}",
                secs(dur_us - item.fade_out_us),
                secs(item.fade_out_us),
            ));
        }
        if item.start > 0 {
            chain.push_str(&format!(",adelay={}:all=1", item.start / 1000));
        }
        chain.push_str(&format!("[{label}]"));
        fc.push(chain);
        alabels.push(format!("[{label}]"));
    }
    let master = if settings.loudnorm {
        ",loudnorm=I=-14:TP=-1.5:LRA=11,aresample=48000".to_string()
    } else {
        String::new()
    };
    fc.push(format!(
        "{}amix=inputs={}:duration=longest:normalize=0,atrim=0:{}{}[aout]",
        alabels.join(""),
        alabels.len(),
        secs(total_us),
        master,
    ));
    let mut alabel = "[aout]".to_string();
    let mut duration_us = total_us;
    if let Some((r_in, r_out)) = settings.range {
        let a = r_in.clamp(0, total_us);
        let b = r_out.clamp(a, total_us);
        if b > a {
            fc.push(format!(
                "[aout]atrim=start={}:end={},asetpts=PTS-STARTPTS[aoutr]",
                secs(a),
                secs(b)
            ));
            alabel = "[aoutr]".into();
            duration_us = b - a;
        }
    }
    let mut args: Vec<String> = vec!["-y".into(), "-v".into(), "error".into()];
    for input in &inputs {
        args.push("-i".into());
        args.push(input.to_string_lossy().into_owned());
    }
    args.push("-filter_complex".into());
    args.push(fc.join(";"));
    args.extend(["-map".into(), alabel, "-vn".into()]);
    args.extend([
        "-c:a".into(),
        "aac".into(),
        "-b:a".into(),
        format!("{}k", settings.audio_bitrate_k),
        "-movflags".into(),
        "+faststart".into(),
    ]);
    args.push(output.to_string_lossy().into_owned());
    Ok(FfmpegPlan { args, duration_us, subs_file: None, temp_files: vec![] })
}

/// Caption-sized max line length for a canvas: Whisper segments can span
/// MINUTES of continuous speech; captions must be chunked to fit the frame.
pub fn caption_max_chars(out_w: u32, font_px: f64) -> usize {
    ((out_w as f64 * 0.86) / (font_px * 0.52)).round().clamp(12.0, 64.0) as usize
}

/// Pause longer than this starts a new caption.
const CAPTION_GAP_US: i64 = 900_000;
/// No caption stays on screen longer than this.
const CAPTION_MAX_DUR_US: i64 = 6_000_000;
/// A caption lingers this long after its last word (until the next one).
const CAPTION_LINGER_US: i64 = 600_000;

/// Phrase-level chunking of a transcript from its WORD timestamps: the same
/// grouping the captions use (`max_chars` ~= a caption line). Public so agents
/// can ask for "sentences with timestamps" instead of the raw 100k-word dump.
/// Returns (text, start_us, end_us); the end includes the caption linger.
pub fn transcript_phrases(
    doc: &ue_core::model::TranscriptDoc,
    max_chars: usize,
) -> Vec<(String, i64, i64)> {
    caption_phrases(doc, max_chars.clamp(8, 200), None)
}

/// The exact chunker the export, the preview and the ASS builder all call.
pub fn caption_phrases_pub(
    doc: &ue_core::model::TranscriptDoc,
    max_chars: usize,
    max_words: Option<u32>,
) -> Vec<(String, i64, i64)> {
    caption_phrases(doc, max_chars, max_words)
}

/// Asset time → timeline, shared with the ASS builder.
pub fn asset_time_to_timeline_pub(
    seq: &ue_core::model::Sequence,
    asset_id: Id,
    t_asset: TimeUs,
) -> Option<TimeUs> {
    asset_time_to_timeline(seq, asset_id, t_asset)
}

/// Test hook: the exact chunker the export and the preview both call.
pub fn caption_phrases_for_test(
    doc: &ue_core::model::TranscriptDoc,
    max_chars: usize,
    max_words: Option<u32>,
) -> Vec<(String, i64, i64)> {
    caption_phrases(doc, max_chars, max_words)
}

/// Build caption phrases straight from the WORDS (Whisper's own segment
/// grouping can span minutes of continuous speech). New caption on: line
/// full, pause > CAPTION_GAP_US, or duration > CAPTION_MAX_DUR_US. Windows
/// are contiguous up to a small linger. Also used to slice karaoke lines.
fn caption_phrases(
    doc: &ue_core::model::TranscriptDoc,
    max_chars: usize,
    max_words: Option<u32>,
) -> Vec<(String, i64, i64)> {
    let words: Vec<&ue_core::model::Word> =
        doc.words.iter().filter(|w| !w.rejected).collect();
    if words.is_empty() {
        // old/wordless transcripts: fall back to the segments as-is
        return doc.segments.iter().map(|s| (s.text.clone(), s.start_us, s.end_us)).collect();
    }
    let cap = max_words.map(|w| w.clamp(1, 20) as usize);
    let mut cuts: Vec<usize> = vec![0];
    let mut chars = 0usize;
    let mut count = 0usize;
    let mut chunk_start = words[0].start_us;
    for (i, w) in words.iter().enumerate() {
        if i == 0 {
            chars = w.label().len();
            count = 1;
            continue;
        }
        let gap = w.start_us - words[i - 1].end_us;
        // an explicit word cap REPLACES the width heuristic: the user asked for
        // N words a line, so give exactly N (the line may then be long)
        let full = match cap {
            Some(n) => count >= n,
            None => chars + 1 + w.label().len() > max_chars,
        };
        let too_slow = w.end_us - chunk_start > CAPTION_MAX_DUR_US;
        if full || gap > CAPTION_GAP_US || too_slow {
            cuts.push(i);
            chars = w.label().len();
            count = 1;
            chunk_start = w.start_us;
        } else {
            chars += 1 + w.label().len();
            count += 1;
        }
    }
    cuts.push(words.len());
    let mut out = Vec::with_capacity(cuts.len() - 1);
    for c in cuts.windows(2) {
        let group = &words[c[0]..c[1]];
        let text = group.iter().map(|w| w.label()).collect::<Vec<_>>().join(" ");
        let start = group[0].start_us;
        let natural_end = group.last().unwrap().end_us + CAPTION_LINGER_US;
        let end = match words.get(c[1]) {
            Some(next) => natural_end.min(next.start_us),
            None => natural_end,
        };
        out.push((text, start, end.max(start + 1)));
    }
    out
}

/// Karaoke: per segment, each word in a dim color for the whole phrase and
/// in a highlighted color from when it is spoken until the end of the segment.
/// None if there is no measurable fontfile (the caller falls back to word mode).
/// `at = Some(t)`: emit ONLY the phrase on screen at `t`, with no `enable`
/// clauses and the highlight already resolved (words whose time has come are
/// drawn in the highlight colour). That makes the paused preview show the very
/// same karaoke frame the export burns in — it used to fall back to a plain
/// phrase line, so pause and playback simply looked different.
#[allow(clippy::too_many_arguments)]
fn karaoke_overlays(
    seq: &ue_core::model::Sequence,
    doc: &ue_core::model::TranscriptDoc,
    clip: &ue_core::model::Clip,
    style: &ue_core::model::TextStyle,
    fallback_font: &str,
    scale: f64,
    out_w: u32,
    export_windows: &[(TimeUs, TimeUs)],
    at: Option<TimeUs>,
    max_words: Option<u32>,
) -> Option<Vec<String>> {
    let font_part = font_part_for(style, fallback_font);
    let font_path = font_part.strip_prefix("fontfile=")?.to_string();
    let px = ((style.size as f64) * scale).round().max(8.0);
    let fontsize = px as u32;
    let y_off = (style.y_offset as f64 * scale).round() as i64;
    let base_color = format!("0x{}@0.4", style.color.trim_start_matches('#'));
    let hi_color = format!(
        "0x{}",
        style
            .highlight_color
            .as_deref()
            .unwrap_or("#FFB224")
            .trim_start_matches('#')
    );
    let space_w = measure_text_px(&font_path, " ", px)?;

    // karaoke lines = the same word-driven caption phrases as phrase mode
    let phrases = caption_phrases(doc, caption_max_chars(out_w, px), max_words);

    let mut out = vec![];
    for (_text, ph_start, ph_end) in &phrases {
        let words: Vec<&ue_core::model::Word> = doc
            .words
            .iter()
            .filter(|w| !w.rejected && w.start_us >= *ph_start && w.start_us < *ph_end)
            .collect();
        if words.is_empty() {
            continue;
        }
        // phrase window on the timeline, clamped to the clip
        let Some(seg_tl) = asset_time_to_timeline(seq, doc.asset_id, *ph_start) else {
            continue;
        };
        let seg_from = seg_tl.max(clip.start);
        let seg_to = (seg_tl + (ph_end - ph_start)).min(clip.end());
        if seg_to <= seg_from {
            continue;
        }
        match at {
            // preview: only the phrase actually on screen at t
            Some(t) if t < seg_from || t >= seg_to => continue,
            // export: only phrases inside the rendered ranges — keeps the
            // per-word drawtext (two per word) bounded so ffmpeg doesn't choke
            None if !in_export_windows(seg_from, seg_to, export_windows) => continue,
            _ => {}
        }
        // MULTI-LINE layout: wrap the phrase's words with the real font metrics,
        // then place every word by (line, x). Each line is centred on its own
        // width, and the block of lines is centred on the style's y_offset.
        let labels: Vec<&str> = words.iter().map(|w| w.label()).collect();
        let max_w = out_w as f64 * CAPTION_WIDTH_FRACTION;
        let wrapped = wrap_words(Some(&font_path), &labels, px, max_w);
        let n_lines = wrapped.len();
        // (line index, x offset within the line) for every word, in order
        let mut placed: Vec<(usize, f64, f64)> = Vec::with_capacity(words.len()); // (line, prefix, line_total)
        for (li, line) in wrapped.iter().enumerate() {
            let widths: Vec<f64> = line
                .iter()
                .map(|w| measure_text_px(&font_path, w, px).unwrap_or(px * 0.5))
                .collect();
            let total: f64 =
                widths.iter().sum::<f64>() + space_w * (line.len().saturating_sub(1)) as f64;
            let mut prefix = 0.0f64;
            for wpx in &widths {
                placed.push((li, prefix, total));
                prefix += wpx + space_w;
            }
        }
        for (i, w) in words.iter().enumerate() {
            let (li, prefix, total) = placed.get(i).copied().unwrap_or((0, 0.0, 0.0));
            let x_expr = format!("(w-{total:.0})/2+{prefix:.0}");
            let y_off = y_off + line_y_offset(li, n_lines, px, style.line_height).round() as i64;
            let word_tl = asset_time_to_timeline(seq, doc.asset_id, w.start_us)
                .unwrap_or(seg_tl)
                .max(clip.start);
            let hi_from = word_tl.min(seg_to);
            match at {
                Some(t) => {
                    // draw unconditionally: dim, then the highlight if the
                    // word has already been spoken at t (what export shows)
                    out.push(drawtext_at(
                        &font_part, w.label(), fontsize, &base_color, &x_expr, y_off, scale, None,
                    ));
                    if t >= hi_from {
                        out.push(drawtext_at(
                            &font_part, w.label(), fontsize, &hi_color, &x_expr, y_off, scale, None,
                        ));
                    }
                }
                None => {
                    // dim layer (whole phrase visible during the segment)
                    out.push(drawtext_at(
                        &font_part, w.label(), fontsize, &base_color, &x_expr, y_off, scale,
                        Some((seg_from, seg_to)),
                    ));
                    // highlighted layer (from when the word plays)
                    out.push(drawtext_at(
                        &font_part, w.label(), fontsize, &hi_color, &x_expr, y_off, scale,
                        Some((hi_from, seg_to)),
                    ));
                }
            }
        }
    }
    Some(out)
}

/// A text/subtitles clip that carries effects or a moved/scaled/rotated
/// transform can no longer be a plain `drawtext` burned onto the finished
/// video: it has to become its own RGBA layer so the SAME effect+transform
/// chain a media clip gets applies to it. Plain ones keep the cheap path.
pub fn text_is_styled_pub(clip: &ue_core::model::Clip) -> bool {
    clip.effects.iter().any(|e| e.enabled)
        || clip.transform != ue_core::model::Transform2D::default()
}

/// Does `[from, to)` overlap any export window? (Empty windows = whole timeline,
/// so everything overlaps.)
fn in_export_windows(from: TimeUs, to: TimeUs, windows: &[(TimeUs, TimeUs)]) -> bool {
    windows.is_empty() || windows.iter().any(|&(a, b)| from < b && to > a)
}

/// drawtext chain for the titles/subtitles ACTIVE at timeline time `t_us`,
/// WITHOUT any `enable` window (the single-frame preview picks the active item
/// here and draws it unconditionally). Built from the exact same chunking,
/// fonts, sizes and positions as the export path (`text_overlays_inner`), so the paused
/// preview matches the export. Karaoke degrades to its phrase line (the
/// per-word highlight is export-only).
///
/// `out_h`/`out_w` must be the SEQUENCE canvas, so the caller has to composite
/// the frame at canvas size before applying this and only downscale afterwards.
pub fn text_overlays_at(
    project: &Project,
    seq: &ue_core::model::Sequence,
    out_h: u32,
    out_w: u32,
    t_us: TimeUs,
) -> Option<String> {
    text_overlays_inner(project, seq, out_h, out_w, Some(t_us), &[], None, true)
}

/// The drawtext chain for ONE text/subtitles clip, unconditional (no `enable`)
/// when `at` is given. Used to render a styled text clip as its own layer.
pub fn text_clip_chain(
    project: &Project,
    seq: &ue_core::model::Sequence,
    clip_id: Id,
    out_h: u32,
    out_w: u32,
    at: Option<TimeUs>,
    export_windows: &[(TimeUs, TimeUs)],
) -> Option<String> {
    text_overlays_inner(project, seq, out_h, out_w, at, export_windows, Some(clip_id), false)
}

/// Shared body: `at = None` burns the whole timeline in (export); `at = Some(t)`
/// emits only what is on screen at `t`, without enable clauses (preview).
/// `export_windows` bounds the export path to the ranges being rendered.
#[allow(clippy::too_many_arguments)]
fn text_overlays_inner(
    project: &Project,
    seq: &ue_core::model::Sequence,
    out_h: u32,
    out_w: u32,
    at: Option<TimeUs>,
    export_windows: &[(TimeUs, TimeUs)],
    // `Some(id)` = emit ONLY this clip (it is being rendered as its own layer).
    only: Option<Id>,
    // Skip clips that carry effects/transform: those are layers, not burn-ins.
    skip_styled: bool,
) -> Option<String> {
    use ue_core::model::{ClipPayload, SubtitleMode, TrackKind};
    let scale = out_h as f64 / 1080.0;
    let font_part = match find_font() {
        Some(f) => format!("fontfile={f}"),
        None => "font=sans".to_string(),
    };
    // for an item spanning [from, to): the enable window (export) or, in
    // preview mode, `Some(None)` when it is on screen at `t` and `None` when
    // it is not (so the caller skips it). In export it also skips anything
    // outside the rendered ranges.
    let window = |from: TimeUs, to: TimeUs| -> Option<Option<(TimeUs, TimeUs)>> {
        if at.is_none() && !in_export_windows(from, to, export_windows) {
            return None;
        }
        match at {
            None => Some(Some((from, to))),
            Some(t) if from <= t && t < to => Some(None),
            Some(_) => None,
        }
    };
    let mut parts: Vec<String> = vec![];
    for track in seq.tracks.iter().filter(|t| t.kind == TrackKind::Video && !t.muted) {
        for clip in &track.clips {
            if let Some(id) = only {
                if clip.id != id {
                    continue;
                }
            } else if skip_styled && text_is_styled_pub(clip) {
                continue; // rendered as a layer, with its effects and transform
            }
            match &clip.payload {
                ClipPayload::Text { content, style } => {
                    if content.trim().is_empty() {
                        continue;
                    }
                    if let Some(enable) = window(clip.start, clip.end()) {
                        parts.extend(drawtext_for(&font_part, content, style, scale, enable, out_w));
                    }
                }
                ClipPayload::Subtitles { transcript_id, style, mode, max_words } => {
                    let Some(doc) =
                        project.transcripts.iter().find(|t| t.id == *transcript_id)
                    else {
                        continue;
                    };
                    // karaoke: full phrase with the current word highlighted
                    // (progressive fill); needs font metrics. The PREVIEW takes
                    // the same path with `at`, so a paused karaoke frame is the
                    // export's frame, highlight and all.
                    if *mode == SubtitleMode::Karaoke {
                        if let Some(chains) = karaoke_overlays(
                            seq, doc, clip, style, &font_part, scale, out_w, export_windows, at,
                            *max_words,
                        ) {
                            parts.extend(chains);
                            continue;
                        }
                        // no metrics → falls back to phrase mode below
                    }
                    // phrase / karaoke-preview: caption-sized chunks (a segment
                    // can span minutes of continuous speech);
                    // word: one big word at a time (shorts style)
                    let owned_items: Vec<(String, i64, i64)> = match mode {
                        SubtitleMode::Word => doc
                            .words
                            .iter()
                            .filter(|w| !w.rejected)
                            .map(|w| (w.label().to_string(), w.start_us, w.end_us))
                            .collect(),
                        SubtitleMode::Phrase | SubtitleMode::Karaoke => {
                            let px = (style.size as f64) * scale;
                            caption_phrases(doc, caption_max_chars(out_w, px), *max_words)
                        }
                    };
                    let word_scale = if *mode == SubtitleMode::Word { 1.6 } else { 1.0 };
                    let mut wstyle = style.clone();
                    wstyle.size *= word_scale as f32;
                    for (text, s_us, e_us) in &owned_items {
                        if text.trim().is_empty() {
                            continue;
                        }
                        let Some(tl_start) = asset_time_to_timeline(seq, doc.asset_id, *s_us)
                        else {
                            continue; // that slice of the asset is not on the timeline
                        };
                        let tl_end = tl_start + (e_us - s_us);
                        let from = tl_start.max(clip.start);
                        let to = tl_end.min(clip.end());
                        if to <= from {
                            continue;
                        }
                        if let Some(enable) = window(from, to) {
                            parts.extend(drawtext_for(&font_part, text, &wstyle, scale, enable, out_w));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(","))
    }
}

/// Supported transition kinds (our id → xfade transition).
pub const TRANSITION_KINDS: &[(&str, &str, &str)] = &[
    ("core.crossfade", "fade", "Cross fade"),
    ("core.wipeleft", "wipeleft", "Wipe ←"),
    ("core.wiperight", "wiperight", "Wipe →"),
    ("core.slideleft", "slideleft", "Slide ←"),
    ("core.slideright", "slideright", "Slide →"),
    ("core.slideup", "slideup", "Slide ↑"),
    ("core.circleopen", "circleopen", "Circle open"),
    ("core.circleclose", "circleclose", "Circle close"),
    ("core.dissolve", "dissolve", "Dissolve"),
    ("core.pixelize", "pixelize", "Pixelize"),
    ("core.radial", "radial", "Radial"),
];

/// Sharp circular alpha mask for the circle transitions. ffmpeg's xfade
/// circles are so feathered they read as a plain fade; this geq mask matches
/// the crisp circle the play compositor draws, anchored to the frame centre.
/// `p_expr` = progress 0..1 (a literal or a `T`-based expression).
pub fn circle_mask(kind: &str, p_expr: &str, leaving: bool) -> Option<String> {
    let (inside, r) = match (kind, leaving) {
        ("circleopen", false) => (true, format!("({p_expr})")),
        ("circleopen", true) => (true, format!("(1-({p_expr}))")),
        ("circleclose", false) => (false, format!("(1-({p_expr}))")),
        ("circleclose", true) => (false, format!("({p_expr})")),
        _ => return None,
    };
    let md = "hypot(W/2,H/2)";
    let dist = "hypot(X-W/2,Y-H/2)";
    let m = if inside {
        format!("clip(({r}*{md}-{dist})/(0.02*{md}),0,1)")
    } else {
        format!("clip(({dist}-{r}*{md})/(0.02*{md}),0,1)")
    };
    Some(format!("format=rgba,geq=r='r(X,Y)':g='g(X,Y)':b='b(X,Y)':a='alpha(X,Y)*{m}'"))
}

pub fn xfade_kind(effect_id: &str) -> &'static str {
    TRANSITION_KINDS
        .iter()
        .find(|(id, _, _)| *id == effect_id)
        .map(|(_, kind, _)| *kind)
        .unwrap_or("fade")
}

pub fn build_ffmpeg_args(
    project: &Project,
    sequence_id: Id,
    base_dir: &Path,
    output: &Path,
    settings: &ExportSettings,
) -> ExportResult<FfmpegPlan> {
    // RANGES ARE APPLIED FIRST, NOT LAST.
    //
    // They used to be a `trim` bolted onto the END of the filtergraph, so
    // ffmpeg rendered the whole timeline from t=0 and discarded everything
    // before the range: a 46 s clip starting at minute 22 spent ~12 minutes
    // writing nothing, and `-progress` (which reports OUTPUT time) sat at 0 the
    // entire time and then jumped to 1. Cutting the sequence down to the wanted
    // material up front means the graph carries nothing to throw away — the
    // cost becomes proportional to the range's LENGTH, not to where it starts.
    let pieces: Vec<(TimeUs, TimeUs)> = if !settings.ranges.is_empty() {
        settings.ranges.clone()
    } else {
        settings.range.into_iter().collect()
    };
    let restricted;
    let mut settings_owned;
    let (project, settings) = if pieces.is_empty() {
        (project, settings)
    } else {
        restricted = crate::range::restrict_to_ranges(project, sequence_id, &pieces);
        settings_owned = settings.clone();
        settings_owned.range = None;
        settings_owned.ranges = vec![];
        (&restricted, &settings_owned)
    };

    let seq = project
        .sequence(sequence_id)
        .ok_or(ExportError::NoSequence(sequence_id))?;
    let audio_only = settings.format == crate::ExportFormat::M4a;
    if audio_only {
        return build_audio_only_args(project, sequence_id, base_dir, output, settings);
    }

    // MULTI-LAYER: the lowest video track WITH CONTENT (base) drives the EDL;
    // media clips on tracks above it are composited on top with overlay
    // (+opacity). Empty tracks below don't count — otherwise material living
    // only on V2+ exported as "empty timeline".
    let video_tracks: Vec<&ue_core::model::Track> = seq
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Video && !t.muted)
        .collect();
    let has_content = |t: &&ue_core::model::Track| {
        t.clips
            .iter()
            .any(|c| matches!(c.payload, ClipPayload::Media { .. } | ClipPayload::Generator { .. }))
    };
    let base_idx = video_tracks.iter().position(has_content);
    let registry =
        ue_render::merge_registries(ue_render::core_registry(), settings.extra_packs.clone());
    let generators = ue_render::core_generators();
    enum LayerSrc {
        Media { asset_id: Id, src_in: TimeUs, src_out: TimeUs, speed: f64 },
        Gen { source: String },
    }
    struct LayerClip {
        src: LayerSrc,
        start: TimeUs,
        out_dur: TimeUs,
        vf: Option<String>,
        /// Transition-in: on a layer it always runs as an ENTRANCE from
        /// transparent (the tracks below stay visible through it).
        trans: Option<(TimeUs, String)>,
        /// Transition-out: the layer's tail EXITS to transparent.
        trans_out: Option<(TimeUs, String)>,
    }
    let mut layer_clips: Vec<LayerClip> = vec![];
    for track in video_tracks.iter().skip(base_idx.map_or(usize::MAX, |i| i + 1)) {
        for clip in &track.clips {
            // layers run in timeline time: t relative to the clip
            let tvar = format!("(t-{})", secs(clip.start));
            let vf = || {
                ue_render::clip_vf_layer(
                    &registry,
                    &clip.effects,
                    &clip.transform,
                    Some(seq.resolution),
                    &tvar,
                )
            };
            match &clip.payload {
                ClipPayload::Media { asset_id, src_in, src_out } => {
                    if project.asset(*asset_id).is_none() {
                        return Err(ExportError::MissingAsset(*asset_id));
                    }
                    layer_clips.push(LayerClip {
                        src: LayerSrc::Media {
                            asset_id: *asset_id,
                            src_in: *src_in,
                            src_out: *src_out,
                            speed: clip.speed,
                        },
                        start: clip.start,
                        out_dur: (((*src_out - *src_in) as f64) / clip.speed).round() as TimeUs,
                        vf: vf(),
                        trans: clip
                            .transition_in
                            .as_ref()
                            .map(|t| (t.duration, t.effect_id.clone())),
                        trans_out: clip
                            .transition_out
                            .as_ref()
                            .map(|t| (t.duration, t.effect_id.clone())),
                    });
                }
                ClipPayload::Generator { generator_id, params, color_params } => {
                    let Some(def) = ue_render::find_generator(&generators, generator_id) else {
                        continue;
                    };
                    layer_clips.push(LayerClip {
                        src: LayerSrc::Gen {
                            source: ue_render::render_generator(
                                def,
                                params,
                                color_params,
                                seq.fps,
                                clip.duration,
                            ),
                        },
                        start: clip.start,
                        out_dur: clip.duration,
                        vf: vf(),
                        trans: clip
                            .transition_in
                            .as_ref()
                            .map(|t| (t.duration, t.effect_id.clone())),
                        trans_out: clip
                            .transition_out
                            .as_ref()
                            .map(|t| (t.duration, t.effect_id.clone())),
                    });
                }
                _ => {}
            }
        }
    }
    let multilayer = !layer_clips.is_empty();
    let edl_res = if multilayer {
        // EDL with only the base track: (visually) mute the others
        let mut base_project = project.clone();
        if let Some(s) = base_project.sequence_mut(sequence_id) {
            let mut vid_i = 0usize;
            for t in &mut s.tracks {
                if t.kind == TrackKind::Video && !t.muted {
                    if Some(vid_i) != base_idx {
                        t.muted = true;
                    }
                    vid_i += 1;
                }
            }
        }
        build_video_edl_with(&base_project, sequence_id, &settings.extra_packs)
    } else {
        build_video_edl_with(project, sequence_id, &settings.extra_packs)
    };
    let edl = match edl_res {
        Ok(edl) => edl,
        Err(ExportError::EmptyTimeline) => {
            // no media anywhere, but titles/subtitles still export over black
            // (title cards) — the preview shows exactly that
            let text_end = seq
                .tracks
                .iter()
                .filter(|t| t.kind == TrackKind::Video && !t.muted)
                .flat_map(|t| &t.clips)
                .filter(|c| {
                    matches!(c.payload, ClipPayload::Text { .. } | ClipPayload::Subtitles { .. })
                })
                .map(|c| c.end())
                .max()
                .unwrap_or(0);
            if text_end <= 0 {
                return Err(ExportError::EmptyTimeline);
            }
            vec![Segment::Black { duration: text_end }]
        }
        Err(e) => return Err(e),
    };
    let base_dur = edl_duration(&edl);
    // the master lasts until the end of the longest layer
    let layers_end = layer_clips.iter().map(|l| l.start + l.out_dur).max().unwrap_or(0);
    let total_us = base_dur.max(layers_end);
    let audio_items = collect_audio(project, sequence_id);

    let (mut out_w, mut out_h) = seq.resolution;
    if let Some(mh) = settings.max_height {
        if out_h > mh {
            out_w = (out_w as u64 * mh as u64 / out_h as u64) as u32 & !1;
            out_h = mh & !1;
        }
    }
    let fps = format!("{}/{}", seq.fps.0, seq.fps.1);

    // ---- INPUTS: one per (file, seek point), with an INPUT-SIDE `-ss` --------
    //
    // The old scheme opened one input per FILE and reached the wanted material
    // with a `trim=start=X` filter — an OUTPUT-side seek, which makes ffmpeg
    // decode the file from byte zero and throw the result away. Exporting a
    // clip that starts at minute 22 of a 24-minute recording therefore decoded
    // 22 minutes of video before writing a single frame.
    //
    // `-ss` placed BEFORE `-i` is an input-side seek: ffmpeg jumps to the
    // nearest keyframe and decodes forward only to the exact requested time
    // (accurate_seek is on by default), so the cost is proportional to the GOP,
    // not to the offset. Each segment gets its own input at its own seek point,
    // which also removes the implicit `split` ffmpeg used to insert when one
    // input fed several segments — and with it the memory blow-up when a
    // reordered edit made one branch buffer frames while another was consumed.
    struct Inputs {
        /// (path, seek µs, loop the still image?)
        specs: Vec<(PathBuf, TimeUs, bool)>,
        index: BTreeMap<(PathBuf, TimeUs), usize>,
    }
    impl Inputs {
        fn at(&mut self, path: PathBuf, seek_us: TimeUs) -> usize {
            let is_image = ue_media::is_image_path(&path);
            // a still image has nothing to seek into (and `-loop 1` + `-ss`
            // would just spin), so images always share one input at 0
            let seek = if is_image { 0 } else { seek_us.max(0) };
            let key = (path.clone(), seek);
            if let Some(i) = self.index.get(&key) {
                return *i;
            }
            self.specs.push((path, seek, is_image));
            let i = self.specs.len() - 1;
            self.index.insert(key, i);
            i
        }
    }
    let mut inputs = Inputs { specs: vec![], index: BTreeMap::new() };

    // ---- video chains ----
    let mut fc: Vec<String> = vec![];
    let norm = format!(
        "fps={fps},scale={out_w}:{out_h}:force_original_aspect_ratio=decrease,\
         pad={out_w}:{out_h}:(ow-iw)/2:(oh-ih)/2,setsar=1,format=yuv420p"
    );
    for (k, seg) in edl.iter().enumerate() {
        let label = format!("v{k}");
        match seg {
            Segment::Source { asset_id, src_in, src_out, speed, vf, .. } => {
                let asset = project.asset(*asset_id).expect("validated in the EDL");
                let idx = inputs.at(resolve_path(base_dir, &asset.path), *src_in);
                let effects = match vf {
                    Some(chain) => format!("{chain},"),
                    None => String::new(),
                };
                let setpts = if (*speed - 1.0).abs() > 1e-9 {
                    format!("setpts=(PTS-STARTPTS)/{speed}")
                } else {
                    "setpts=PTS-STARTPTS".to_string()
                };
                // `-ss` already parked the demuxer at src_in, so the trim only
                // has to say how MUCH material this segment wants
                fc.push(format!(
                    "[{idx}:v]trim=start=0:end={},{setpts},{effects}{norm}[{label}]",
                    secs(*src_out - *src_in),
                ));
            }
            Segment::Gen { source, vf, .. } => {
                let effects = match vf {
                    Some(chain) => format!("{chain},"),
                    None => String::new(),
                };
                fc.push(format!("{source},{effects}{norm}[{label}]"));
            }
            Segment::Black { duration } => {
                fc.push(format!(
                    "color=black:size={out_w}x{out_h}:rate={fps}:duration={}[{label}]",
                    secs(*duration),
                ));
            }
        }
    }

    // Entrances: a transition that could not be an A/B xfade (single clip, a
    // gap before, no spare material) still RUNS — as an xfade from black over
    // the segment's own head. Never a silent no-op.
    let mut seg_label: Vec<String> = (0..edl.len()).map(|k| format!("v{k}")).collect();
    for (k, seg) in edl.iter().enumerate() {
        let Segment::Source { entrance: Some((d, effect_id)), .. } = seg else { continue };
        let d = (*d).min(seg.duration()).max(40_000);
        let kind = xfade_kind(effect_id);
        // black lives as long as the segment: xfade emits the blend during
        // [0, d) and the segment alone afterwards
        fc.push(format!(
            "color=black:size={out_w}x{out_h}:rate={fps}:duration={},format=yuv420p,settb=AVTB[en{k}b];\
             [v{k}]settb=AVTB[en{k}s];\
             [en{k}b][en{k}s]xfade=transition={kind}:duration={}:offset=0,format=yuv420p[v{k}f]",
            secs(seg.duration()),
            secs(d),
        ));
        seg_label[k] = format!("v{k}f");
    }
    // Exits: the mirrored wrap — the segment's tail xfades TO black.
    for (k, seg) in edl.iter().enumerate() {
        let Segment::Source { exit: Some((d, effect_id)), .. } = seg else { continue };
        let d = (*d).min(seg.duration()).max(40_000);
        let kind = xfade_kind(effect_id);
        let src = seg_label[k].clone();
        fc.push(format!(
            "color=black:size={out_w}x{out_h}:rate={fps}:duration={},format=yuv420p,settb=AVTB[ex{k}b];\
             [{src}]settb=AVTB[ex{k}s];\
             [ex{k}s][ex{k}b]xfade=transition={kind}:duration={}:offset={},format=yuv420p[v{k}g]",
            secs(d),
            secs(d),
            secs(seg.duration() - d),
        ));
        seg_label[k] = format!("v{k}g");
    }

    // Sequential combination: concat on hard cuts, xfade on transitions.
    let mut current = seg_label[0].clone();
    let mut acc_dur = edl[0].duration();
    for (k, seg) in edl.iter().enumerate().skip(1) {
        let out_label = format!("m{k}");
        let transition = match seg {
            Segment::Source { transition_in, .. } => transition_in.clone(),
            _ => None,
        };
        match transition {
            Some((d, effect_id)) => {
                let offset = acc_dur - d;
                let kind = xfade_kind(&effect_id);
                // xfade requires BOTH inputs on the same timebase; after a
                // concat the accumulated stream may sit on 1/AV_TIME_BASE
                // while a fresh normed segment sits on 1/fps — without the
                // settb pair the whole export dies with "timebase … do not
                // match" as soon as a transition follows any earlier cut.
                fc.push(format!(
                    "[{current}]settb=AVTB[xa{k}];[{}]settb=AVTB[xb{k}];\
                     [xa{k}][xb{k}]xfade=transition={kind}:duration={}:offset={}[{out_label}]",
                    seg_label[k],
                    secs(d),
                    secs(offset),
                ));
                acc_dur += seg.duration() - d;
            }
            None => {
                fc.push(format!(
                    "[{current}][{}]concat=n=2:v=1:a=0[{out_label}]",
                    seg_label[k],
                ));
                acc_dur += seg.duration();
            }
        }
        current = out_label;
    }
    // if the layers last longer than the base, extend the base with black
    if multilayer && layers_end > base_dur {
        fc.push(format!(
            "color=black:size={out_w}x{out_h}:rate={fps}:duration={}[basetail]",
            secs(layers_end - base_dur),
        ));
        fc.push(format!("[{current}][basetail]concat=n=2:v=1:a=0[basefull]"));
        current = "basefull".to_string();
    }

    // ---- upper layers: overlay in track order (bottom to top) ----
    for (k, layer) in layer_clips.iter().enumerate() {
        let effects = match &layer.vf {
            Some(chain) => format!("{chain},"),
            None => String::new(),
        };
        // the sequence canvas may be larger than the file: clamp to the canvas
        let fit = format!(
            "scale='min({out_w},iw)':'min({out_h},ih)':force_original_aspect_ratio=decrease"
        );
        // opacity (static or animated) is already applied in the clip's vf
        let alpha = "format=rgba,".to_string();
        let start = layer.start;
        // Entrance/exit: xfade against a fully transparent copy, applied to
        // the RAW source (0-based, before effects/transform), so the pattern
        // anchors to the VIDEO's own frame — a circle opens from the middle
        // of the image, not from the middle of the canvas the transform later
        // positions it on. Effects/transform (with their `t`-based
        // expressions) run afterwards on timeline PTS as always.
        let clamp = |t: &(TimeUs, String)| ((t.0).min(layer.out_dur).max(40_000), xfade_kind(&t.1));
        let entrance = layer.trans.as_ref().map(clamp);
        let exit = layer.trans_out.as_ref().map(clamp);
        let has_trans = entrance.is_some() || exit.is_some();
        let (pre_label, tail_label) =
            if has_trans { (format!("ly{k}p"), format!("ly{k}q")) } else { (format!("ly{k}p"), format!("ly{k}p")) };
        match &layer.src {
            LayerSrc::Media { asset_id, src_in, src_out, speed } => {
                let asset = project.asset(*asset_id).expect("validated when collecting");
                let idx = inputs.at(resolve_path(base_dir, &asset.path), *src_in);
                fc.push(format!(
                    "[{idx}:v]trim=start=0:end={},setpts=(PTS-STARTPTS)/{speed},format=rgba,fps={fps}[{pre_label}]",
                    secs(*src_out - *src_in),
                ));
            }
            LayerSrc::Gen { source } => {
                fc.push(format!(
                    "{source},setpts=PTS-STARTPTS,format=rgba,fps={fps}[{pre_label}]",
                ));
            }
        }
        if has_trans {
            let mut cur = pre_label.clone();
            if let Some((d, kind)) = &entrance {
                if let Some(mask) = circle_mask(kind, &format!("clip(T/{},0,1)", secs(*d)), false)
                {
                    fc.push(format!("[{cur}]{mask}[ly{k}e]"));
                } else {
                    fc.push(format!(
                        "[{cur}]split=2[ly{k}ea][ly{k}eb];\
                         [ly{k}ea]colorchannelmixer=aa=0,settb=AVTB[ly{k}et];\
                         [ly{k}eb]settb=AVTB[ly{k}es];\
                         [ly{k}et][ly{k}es]xfade=transition={kind}:duration={}:offset=0,format=rgba[ly{k}e]",
                        secs(*d),
                    ));
                }
                cur = format!("ly{k}e");
            }
            if let Some((d, kind)) = &exit {
                let t0 = (layer.out_dur - d).max(0);
                if let Some(mask) = circle_mask(
                    kind,
                    &format!("clip((T-{})/{},0,1)", secs(t0), secs(*d)),
                    true,
                ) {
                    fc.push(format!("[{cur}]{mask}[ly{k}x]"));
                } else {
                    fc.push(format!(
                        "[{cur}]split=2[ly{k}xa][ly{k}xb];\
                         [ly{k}xb]colorchannelmixer=aa=0,settb=AVTB[ly{k}xt];\
                         [ly{k}xa]settb=AVTB[ly{k}xs];\
                         [ly{k}xs][ly{k}xt]xfade=transition={kind}:duration={}:offset={},format=rgba[ly{k}x]",
                        secs(*d),
                        secs(t0),
                    ));
                }
                cur = format!("ly{k}x");
            }
            fc.push(format!("[{cur}]copy[{tail_label}]"));
        }
        fc.push(format!(
            "[{tail_label}]setpts=PTS+{}/TB,{effects}{fit},{alpha}copy[ly{k}]",
            secs(start),
        ));
        let out_label = format!("lc{k}");
        fc.push(format!(
            "[{current}][ly{k}]overlay=x=(W-w)/2:y=(H-h)/2:eof_action=pass:enable='between(t,{},{})'[{out_label}]",
            secs(start),
            secs(start + layer.out_dur),
        ));
        current = out_label;
    }
    if multilayer {
        // flatten accumulated alpha before text/avatars
        fc.push(format!("[{current}]format=yuv420p[flat]"));
        current = "flat".to_string();
    }

    // ---- STYLED TEXT LAYERS -------------------------------------------------
    // A title or subtitles clip that carries effects (blur, drop shadow, colour
    // correct…) or a moved/scaled/rotated transform is rendered as its own RGBA
    // layer and pushed through the very SAME chain a media layer gets, then
    // overlaid. Plain text keeps the cheap burn-in below. Without this, every
    // effect and every transform silently did nothing on text.
    let export_windows_pre: Vec<(TimeUs, TimeUs)> = if !settings.ranges.is_empty() {
        settings.ranges.clone()
    } else {
        settings.range.into_iter().collect()
    };
    let mut temp_files: Vec<PathBuf> = vec![];
    let mut tk = 0usize;

    // ---- TITLES: rasterised by us, composited as image layers ---------------
    //
    // ffmpeg cannot draw a colour emoji by any route: `drawtext` loads one font
    // face and does no fallback, and libass renders vector outlines in a single
    // colour (no colour-font support, open since 2020). Apple Color Emoji is an
    // `sbix` BITMAP font, so both drew empty boxes. ue-text shapes with per-glyph
    // fallback and rasterises through swash, which reads sbix/CBDT/COLR — so the
    // title arrives here as a finished RGBA frame and just gets overlaid, going
    // through the very same effect + transform chain as any other layer.
    let tmp_dir = output.parent().unwrap_or(Path::new(".")).to_path_buf();
    for track in seq.tracks.iter().filter(|t| t.kind == TrackKind::Video && !t.muted) {
        for clip in &track.clips {
            let ClipPayload::Text { content, style } = &clip.payload else { continue };
            if content.trim().is_empty() {
                continue;
            }
            let img = ue_text::render_to_png(
                &ue_text::TextSpec {
                    content,
                    style,
                    out_w,
                    out_h,
                    width_fraction: ue_text::DEFAULT_WIDTH_FRACTION,
                },
                &tmp_dir,
            )
            .map_err(|e| ExportError::Ffmpeg(format!("could not render the title: {e}")))?;
            let idx = inputs.at(img.clone(), 0);
            temp_files.push(img);
            let tvar = format!("(t-{})", secs(clip.start));
            let vf = ue_render::clip_vf_layer(
                &registry,
                &clip.effects,
                &clip.transform,
                Some((out_w, out_h)),
                &tvar,
            )
            .map(|c| format!("{c},"))
            .unwrap_or_default();
            let label = format!("tl{tk}");
            fc.push(format!(
                "[{idx}:v]setpts=PTS-STARTPTS+{}/TB,{vf}format=rgba,fps={fps}[{label}]",
                secs(clip.start),
            ));
            let out_label = format!("tc{tk}");
            fc.push(format!(
                "[{current}][{label}]overlay=x=(W-w)/2:y=(H-h)/2:eof_action=pass:\
                 enable='between(t,{},{})'[{out_label}]",
                secs(clip.start),
                secs(clip.end()),
            ));
            current = out_label;
            tk += 1;
        }
    }

    // ---- styled SUBTITLES clips still ride the drawtext layer path ----------
    for track in seq.tracks.iter().filter(|t| t.kind == TrackKind::Video && !t.muted) {
        for clip in &track.clips {
            if !matches!(clip.payload, ClipPayload::Subtitles { .. }) {
                continue;
            }
            if !text_is_styled_pub(clip) {
                continue; // burned in by libass with the others
            }
            let Some(chain) =
                text_clip_chain(project, seq, clip.id, out_h, out_w, None, &export_windows_pre)
            else {
                continue;
            };
            // transparent canvas the size of the frame, drawn in TIMELINE time so
            // the drawtext `enable` windows and any keyframes line up
            let tvar = format!("(t-{})", secs(clip.start));
            let vf = ue_render::clip_vf_layer(
                &registry,
                &clip.effects,
                &clip.transform,
                Some((out_w, out_h)),
                &tvar,
            )
            .map(|c| format!("{c},"))
            .unwrap_or_default();
            let label = format!("tl{tk}");
            fc.push(format!(
                // setpts FIRST: drawtext `enable` windows are in TIMELINE time, so the
                // layer's clock must be the timeline's before they run
                "color=c=black@0:s={out_w}x{out_h}:r={fps}:d={},setpts=PTS-STARTPTS+{}/TB,\
                 format=rgba,{chain},{vf}format=rgba[{label}]",
                secs(clip.duration),
                secs(clip.start),
            ));
            let out_label = format!("tc{tk}");
            fc.push(format!(
                "[{current}][{label}]overlay=x=(W-w)/2:y=(H-h)/2:eof_action=pass:                 enable='between(t,{},{})'[{out_label}]",
                secs(clip.start),
                secs(clip.end()),
            ));
            current = out_label;
            tk += 1;
        }
    }
    if tk > 0 {
        fc.push(format!("[{current}]format=yuv420p[tflat]"));
        current = "tflat".to_string();
    }

    // burn titles and subtitles onto the combined video, bounded to the ranges
    // actually being rendered (a full-transcript karaoke would otherwise emit
    // thousands of drawtext and crash ffmpeg)
    let export_windows: Vec<(TimeUs, TimeUs)> = if !settings.ranges.is_empty() {
        settings.ranges.clone()
    } else {
        settings.range.into_iter().collect()
    };
    let _ = &export_windows; // ranges are pre-applied now; nothing left to bound
    // TITLES AND SUBTITLES GO THROUGH libass, NOT drawtext.
    //
    // The old path emitted one drawtext per line and TWO per karaoke word, each
    // with its own `enable='between(t,…)'`; the filtergraph grew ~1 KB per second
    // of speech and ffmpeg's parser gave up around 1 MB. An ASS script keeps all
    // of that in a FILE, so the graph carries a single `ass=` filter no matter
    // how long the transcript is — and libass, unlike drawtext, does per-glyph
    // font fallback, which is what finally makes emoji render at all.
    let mut subs_file: Option<PathBuf> = None;
    match crate::ass::build_script(project, seq, out_w, out_h, None) {
        Some(script) => {
            let dir = output.parent().unwrap_or(Path::new("."));
            let path = crate::ass::write_script(&script, dir)
                .map_err(|e| ExportError::Ffmpeg(format!("could not write the subtitles: {e}")))?;
            fc.push(format!("[{current}]{}[vout]", crate::ass::ass_filter(&path)));
            subs_file = Some(path);
        }
        None => fc.push(format!("[{current}]null[vout]")),
    }

    // ---- audio chains (the GIF has none) ----
    let is_gif = settings.format == crate::ExportFormat::Gif;
    let mut alabels: Vec<String> = vec![];
    for (k, item) in audio_items.iter().enumerate().filter(|_| !is_gif) {
        let path = item.input_override.clone().unwrap_or_else(|| {
            let asset = project.asset(item.asset_id).expect("validated when collecting");
            resolve_path(base_dir, &asset.path)
        });
        let idx = inputs.at(path, item.src_in);
        let label = format!("a{k}");
        let dur_us = (((item.src_out - item.src_in) as f64) / item.speed).round() as TimeUs;
        let mut chain = format!(
            "[{idx}:a]atrim=start=0:end={},asetpts=PTS-STARTPTS,\
             aresample=48000,aformat=channel_layouts=stereo",
            secs(item.src_out - item.src_in),
        );
        if item.denoise && item.input_override.is_none() {
            // fallback: the denoised conform isn't rendered yet
            chain.push(',');
            chain.push_str(ue_media::denoise::DENOISE_FILTER);
        }
        if (item.speed - 1.0).abs() > 1e-9 {
            chain.push(',');
            chain.push_str(&atempo_chain(item.speed));
        }
        match &item.gain_curve {
            Some(curve) => chain.push_str(&format!(
                ",volume=volume='{}':eval=frame",
                volume_expr(curve, item.gain_db)
            )),
            None if item.gain_db.abs() > 1e-9 => {
                chain.push_str(&format!(",volume={:.2}dB", item.gain_db));
            }
            None => {}
        }
        if item.pan.abs() > 1e-3 {
            let (pl, pr) = (
                (1.0 - item.pan).min(1.0),
                (1.0 + item.pan).min(1.0),
            );
            chain.push_str(&format!(",pan=stereo|c0={pl:.4}*c0|c1={pr:.4}*c1"));
        }
        if item.fade_in_us > 0 {
            chain.push_str(&format!(",afade=t=in:st=0:d={}", secs(item.fade_in_us)));
        }
        if item.fade_out_us > 0 {
            chain.push_str(&format!(
                ",afade=t=out:st={}:d={}",
                secs(dur_us - item.fade_out_us),
                secs(item.fade_out_us),
            ));
        }
        if item.start > 0 {
            chain.push_str(&format!(",adelay={}:all=1", item.start / 1000)); // ms
        }
        chain.push_str(&format!("[{label}]"));
        fc.push(chain);
        alabels.push(format!("[{label}]"));
    }
    let has_audio = !alabels.is_empty();
    if has_audio {
        let master = if settings.loudnorm {
            // R128 single pass (streaming): -14 LUFS YouTube style
            ",loudnorm=I=-14:TP=-1.5:LRA=11,aresample=48000".to_string()
        } else {
            String::new()
        };
        fc.push(format!(
            "{}amix=inputs={}:duration=longest:normalize=0,atrim=0:{}{}[aout]",
            alabels.join(""),
            alabels.len(),
            secs(total_us),
            master,
        ));
    }

    // The I-O range no longer lives here: the sequence was already cut down to
    // it before the graph was built (see the top of this function), so there is
    // nothing left to trim off the end.
    let mut vlabel = "[vout]".to_string();
    let alabel = "[aout]".to_string();
    let duration_us = total_us;

    // ---- GIF: optimized palette over the already-trimmed master ----
    if is_gif {
        fc.push(format!(
            "{vlabel}fps=12,scale='min(480,iw)':-2:flags=lanczos,split[gifa][gifb];\
             [gifa]palettegen=stats_mode=diff[pal];\
             [gifb][pal]paletteuse=dither=bayer:bayer_scale=4[gifout]"
        ));
        vlabel = "[gifout]".into();
    }

    // ---- command line ----
    let mut args: Vec<String> = vec!["-y".into(), "-v".into(), "error".into()];
    for (path, seek, is_image) in &inputs.specs {
        // a still image is one frame: loop it so it produces frames for the
        // whole clip. Without this a trim/overlay only saw a single frame and
        // the image flashed for ~one frame then vanished (base OR layer).
        if *is_image {
            args.extend(["-loop".into(), "1".into()]);
        }
        // INPUT-side seek: jumps by keyframe instead of decoding the file from
        // byte zero. This is what makes exporting a clip from deep inside a
        // long recording cost the clip's length, not its offset.
        if *seek > 0 {
            args.extend(["-ss".into(), secs(*seek)]);
        }
        args.push("-i".into());
        args.push(path.to_string_lossy().into_owned());
    }
    args.push("-filter_complex".into());
    args.push(fc.join(";"));
    args.extend(["-map".into(), vlabel]);
    if is_gif {
        args.push("-an".into());
        args.extend(["-loop".into(), "0".into()]);
    } else {
        if has_audio {
            args.extend(["-map".into(), alabel]);
            args.extend([
                "-c:a".into(),
                "aac".into(),
                "-b:a".into(),
                format!("{}k", settings.audio_bitrate_k),
            ]);
        } else {
            args.push("-an".into());
        }
        args.extend([
            "-c:v".into(),
            "libx264".into(),
            "-preset".into(),
            settings.preset.clone(),
            "-crf".into(),
            settings.crf.to_string(),
            "-movflags".into(),
            "+faststart".into(),
        ]);
    }
    args.push(output.to_string_lossy().into_owned());

    Ok(FfmpegPlan { args, duration_us, subs_file, temp_files })
}
