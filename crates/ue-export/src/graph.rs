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
}

fn secs(us: TimeUs) -> String {
    format!("{:.6}", us as f64 / 1_000_000.0)
}

fn resolve_path(base: &Path, p: &str) -> PathBuf {
    let path = Path::new(p);
    if path.is_absolute() { path.to_path_buf() } else { base.join(path) }
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
                    fade_in_us: clip.audio.fade_in_us,
                    fade_out_us: clip.audio.fade_out_us,
                });
            }
        }
    }
    items
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

/// Resolves a family to its fontfile; None if not found.
pub fn resolve_font_family(family: &str) -> Option<String> {
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

fn drawtext_for(
    font_part: &str,
    content: &str,
    style: &ue_core::model::TextStyle,
    scale: f64,
    from: TimeUs,
    to: TimeUs,
) -> String {
    let fontsize = ((style.size as f64) * scale).round().max(8.0) as u32;
    let color = style.color.trim_start_matches('#');
    let y_off = (style.y_offset as f64 * scale).round() as i64;
    let font_part = font_part_for(style, font_part);
    format!(
        "drawtext={font_part}:text='{}':fontsize={fontsize}:fontcolor=0x{color}:\
         borderw={}:bordercolor=black@0.6:x={}:y=(h-text_h)/2+{y_off}:\
         enable='between(t,{},{})'",
        escape_drawtext(content),
        (2.0 * scale).round().max(1.0) as u32,
        x_expr_for(style, scale),
        secs(from),
        secs(to),
    )
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
    from: TimeUs,
    to: TimeUs,
) -> String {
    format!(
        "drawtext={font_part}:text='{}':fontsize={fontsize}:fontcolor={color}:\
         borderw={}:bordercolor=black@0.6:x={x_expr}:y=(h-text_h)/2+{y_off}:\
         enable='between(t,{},{})'",
        escape_drawtext(content),
        (2.0 * scale).round().max(1.0) as u32,
        secs(from),
        secs(to),
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

    let mut input_index: BTreeMap<Id, usize> = BTreeMap::new();
    let mut inputs: Vec<PathBuf> = vec![];
    let mut input_of = |asset_id: Id, project: &Project| -> usize {
        *input_index.entry(asset_id).or_insert_with(|| {
            let asset = project.asset(asset_id).expect("validated when collecting");
            inputs.push(resolve_path(base_dir, &asset.path));
            inputs.len() - 1
        })
    };
    let mut fc: Vec<String> = vec![];
    let mut alabels: Vec<String> = vec![];
    for (k, item) in audio_items.iter().enumerate() {
        let idx = input_of(item.asset_id, project);
        let label = format!("a{k}");
        let dur_us = (((item.src_out - item.src_in) as f64) / item.speed).round() as TimeUs;
        let mut chain = format!(
            "[{idx}:a]atrim=start={}:end={},asetpts=PTS-STARTPTS,\
             aresample=48000,aformat=channel_layouts=stereo",
            secs(item.src_in),
            secs(item.src_out),
        );
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
    Ok(FfmpegPlan { args, duration_us })
}

/// Karaoke: per segment, each word in a dim color for the whole phrase and
/// in a highlighted color from when it is spoken until the end of the segment.
/// None if there is no measurable fontfile (the caller falls back to word mode).
fn karaoke_overlays(
    seq: &ue_core::model::Sequence,
    doc: &ue_core::model::TranscriptDoc,
    clip: &ue_core::model::Clip,
    style: &ue_core::model::TextStyle,
    fallback_font: &str,
    scale: f64,
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

    let mut out = vec![];
    for seg in &doc.segments {
        let words: Vec<&ue_core::model::Word> = doc
            .words
            .iter()
            .filter(|w| !w.rejected && w.start_us >= seg.start_us && w.start_us < seg.end_us)
            .collect();
        if words.is_empty() {
            continue;
        }
        // segment window on the timeline, clamped to the clip
        let Some(seg_tl) = asset_time_to_timeline(seq, doc.asset_id, seg.start_us) else {
            continue;
        };
        let seg_from = seg_tl.max(clip.start);
        let seg_to = (seg_tl + (seg.end_us - seg.start_us)).min(clip.end());
        if seg_to <= seg_from {
            continue;
        }
        // line layout: per-word widths + spaces
        let widths: Vec<f64> = words
            .iter()
            .map(|w| measure_text_px(&font_path, &w.text, px).unwrap_or(px * 0.5))
            .collect();
        let total: f64 =
            widths.iter().sum::<f64>() + space_w * (words.len().saturating_sub(1)) as f64;
        let mut prefix = 0.0f64;
        for (i, w) in words.iter().enumerate() {
            let x_expr = format!("(w-{total:.0})/2+{prefix:.0}");
            let word_tl = asset_time_to_timeline(seq, doc.asset_id, w.start_us)
                .unwrap_or(seg_tl)
                .max(clip.start);
            // dim layer (whole phrase visible during the segment)
            out.push(drawtext_at(
                &font_part, &w.text, fontsize, &base_color, &x_expr, y_off, scale, seg_from,
                seg_to,
            ));
            // highlighted layer (from when the word plays)
            out.push(drawtext_at(
                &font_part, &w.text, fontsize, &hi_color, &x_expr, y_off, scale,
                word_tl.min(seg_to), seg_to,
            ));
            prefix += widths[i] + space_w;
        }
    }
    Some(out)
}

/// drawtext chain for the sequence's titles and automatic subtitles.
/// The style's size/offset is referenced to 1080p and scaled to `out_h`.
fn build_text_overlays(
    project: &Project,
    seq: &ue_core::model::Sequence,
    out_h: u32,
) -> Option<String> {
    use ue_core::model::{ClipPayload, TrackKind};
    let scale = out_h as f64 / 1080.0;
    let font_part = match find_font() {
        Some(f) => format!("fontfile={f}"),
        None => "font=sans".to_string(),
    };
    let mut parts: Vec<String> = vec![];
    for track in seq.tracks.iter().filter(|t| t.kind == TrackKind::Video && !t.muted) {
        for clip in &track.clips {
            match &clip.payload {
                ClipPayload::Text { content, style } => {
                    if content.trim().is_empty() {
                        continue;
                    }
                    parts.push(drawtext_for(
                        &font_part,
                        content,
                        style,
                        scale,
                        clip.start,
                        clip.end(),
                    ));
                }
                ClipPayload::Subtitles { transcript_id, style, mode } => {
                    let Some(doc) =
                        project.transcripts.iter().find(|t| t.id == *transcript_id)
                    else {
                        continue;
                    };
                    use ue_core::model::SubtitleMode;
                    // karaoke: full phrase with the current word highlighted
                    // (progressive fill); needs font metrics
                    if *mode == SubtitleMode::Karaoke {
                        if let Some(chains) =
                            karaoke_overlays(seq, doc, clip, style, &font_part, scale)
                        {
                            parts.extend(chains);
                            continue;
                        }
                        // no metrics → falls back to word mode
                    }
                    // phrase mode: one line per segment; word mode:
                    // one big word at a time (shorts style)
                    let items: Vec<(&str, i64, i64)> = match mode {
                        SubtitleMode::Phrase => doc
                            .segments
                            .iter()
                            .map(|s| (s.text.as_str(), s.start_us, s.end_us))
                            .collect(),
                        SubtitleMode::Word | SubtitleMode::Karaoke => doc
                            .words
                            .iter()
                            .filter(|w| !w.rejected)
                            .map(|w| (w.text.as_str(), w.start_us, w.end_us))
                            .collect(),
                    };
                    let word_scale = match mode {
                        SubtitleMode::Phrase => 1.0,
                        _ => 1.6, // single words larger
                    };
                    let mut wstyle = style.clone();
                    wstyle.size *= word_scale as f32;
                    for (text, s_us, e_us) in items {
                        if text.trim().is_empty() {
                            continue;
                        }
                        let Some(tl_start) = asset_time_to_timeline(seq, doc.asset_id, s_us)
                        else {
                            continue; // that slice of the asset is not on the timeline
                        };
                        let tl_end = tl_start + (e_us - s_us);
                        let from = tl_start.max(clip.start);
                        let to = tl_end.min(clip.end());
                        if to <= from {
                            continue;
                        }
                        parts.push(drawtext_for(&font_part, text, &wstyle, scale, from, to));
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

fn xfade_kind(effect_id: &str) -> &'static str {
    TRANSITION_KINDS
        .iter()
        .find(|(id, _, _)| *id == effect_id)
        .map(|(_, kind, _)| *kind)
        .unwrap_or("fade")
}

/// Escaping the movie filter's filename (inside filter_complex).
fn escape_movie_path(p: &str) -> String {
    p.replace('\\', "/").replace(':', "\\\\:").replace('\'', "\\\\'")
}

/// Spans (emotion, from, to) of an Avatar clip: the transcript segments
/// mapped to the timeline, with gaps filled with the default emotion
/// (behavior of the toolkit's avatar_video_generation.py).
fn avatar_spans(
    project: &Project,
    seq: &ue_core::model::Sequence,
    clip: &ue_core::model::Clip,
    driver_asset: Id,
    default_emotion: &str,
) -> Vec<(String, TimeUs, TimeUs, f64)> {
    let doc = project.transcripts.iter().find(|t| t.asset_id == driver_asset);
    let (cs, ce) = (clip.start, clip.end());
    let mut spans: Vec<(String, TimeUs, TimeUs, f64)> = vec![];
    let mut cursor = cs;
    if let Some(doc) = doc {
        let avg = if doc.global_avg_volume > 1e-9 { doc.global_avg_volume } else { 1.0 };
        for seg in &doc.segments {
            let Some(tl_start) = asset_time_to_timeline(seq, driver_asset, seg.start_us) else {
                continue;
            };
            let tl_end = tl_start + (seg.end_us - seg.start_us);
            let from = tl_start.max(cs);
            let to = tl_end.min(ce);
            if to <= from {
                continue;
            }
            if from > cursor {
                spans.push((default_emotion.to_string(), cursor, from, 1.0));
            }
            let emotion = seg.emotion.clone().unwrap_or_else(|| default_emotion.to_string());
            spans.push((emotion, from, to, (seg.volume_rms / avg).clamp(0.0, 3.0)));
            cursor = to;
        }
    }
    if cursor < ce {
        spans.push((default_emotion.to_string(), cursor, ce, 1.0));
    }
    spans
}

/// movie+overlay chains for Avatar clips. Returns (chains, final label).
fn build_avatar_overlays(
    project: &Project,
    seq: &ue_core::model::Sequence,
    base_dir: &Path,
    in_label: &str,
    out_w: u32,
) -> (Vec<String>, String) {
    use ue_core::model::{ClipPayload, TrackKind};
    let mut fc: Vec<String> = vec![];
    let mut current = in_label.to_string();
    let mut n = 0usize;
    for track in seq.tracks.iter().filter(|t| t.kind == TrackKind::Video && !t.muted) {
        for clip in &track.clips {
            let ClipPayload::Avatar { driver_asset, avatars, shake_factor, scale } =
                &clip.payload
            else {
                continue;
            };
            let Some(default_emotion) = avatars.keys().next().cloned() else { continue };
            // avatar width in px (even)
            let aw = (((out_w as f64) * scale.clamp(0.05, 1.0)) as u32) & !1;
            let margin = 24;
            let base_x = out_w as i64 - aw as i64 - margin;
            for (emotion, from, to, vol_ratio) in
                avatar_spans(project, seq, clip, *driver_asset, &default_emotion)
            {
                let path_str = avatars
                    .get(&emotion)
                    .or_else(|| avatars.get(&default_emotion))
                    .cloned()
                    .unwrap_or_default();
                if path_str.is_empty() {
                    continue;
                }
                let abs = {
                    let p = Path::new(&path_str);
                    if p.is_absolute() { p.to_path_buf() } else { base_dir.join(p) }
                };
                if !abs.exists() {
                    continue; // missing avatar: skip that span without breaking
                }
                let amp = (shake_factor * vol_ratio * 8.0).round();
                let av = format!("av{n}");
                let nx = format!("avo{n}");
                fc.push(format!(
                    "movie=filename='{}':loop=999,setpts=PTS-STARTPTS+{}/TB,scale={aw}:-2[{av}]",
                    escape_movie_path(&abs.to_string_lossy()),
                    secs(from),
                ));
                fc.push(format!(
                    "[{current}][{av}]overlay=x='{base_x}+{amp}*sin(t*37)':\
                     y='H-h-{margin}+{amp}*cos(t*51)':enable='between(t,{},{})':\
                     eof_action=pass[{nx}]",
                    secs(from),
                    secs(to),
                ));
                current = nx;
                n += 1;
            }
        }
    }
    (fc, current)
}

pub fn build_ffmpeg_args(
    project: &Project,
    sequence_id: Id,
    base_dir: &Path,
    output: &Path,
    settings: &ExportSettings,
) -> ExportResult<FfmpegPlan> {
    let seq = project
        .sequence(sequence_id)
        .ok_or(ExportError::NoSequence(sequence_id))?;
    let audio_only = settings.format == crate::ExportFormat::M4a;
    if audio_only {
        return build_audio_only_args(project, sequence_id, base_dir, output, settings);
    }

    // MULTI-LAYER: the lowest video track (base) drives the EDL; media clips
    // on upper tracks are composited on top with overlay (+opacity).
    let video_tracks: Vec<&ue_core::model::Track> = seq
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Video && !t.muted)
        .collect();
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
    }
    let mut layer_clips: Vec<LayerClip> = vec![];
    for track in video_tracks.iter().skip(1) {
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
                    });
                }
                _ => {}
            }
        }
    }
    let multilayer = !layer_clips.is_empty();
    let edl = if multilayer {
        // EDL with only the base track: (visually) mute the upper ones
        let mut base_project = project.clone();
        if let Some(s) = base_project.sequence_mut(sequence_id) {
            let mut seen_base = false;
            for t in &mut s.tracks {
                if t.kind == TrackKind::Video && !t.muted {
                    if seen_base {
                        t.muted = true;
                    }
                    seen_base = true;
                }
            }
        }
        build_video_edl_with(&base_project, sequence_id, &settings.extra_packs)?
    } else {
        build_video_edl_with(project, sequence_id, &settings.extra_packs)?
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

    // unique inputs per asset
    let mut input_index: BTreeMap<Id, usize> = BTreeMap::new();
    let mut inputs: Vec<PathBuf> = vec![];
    let mut input_of = |asset_id: Id, project: &Project| -> usize {
        *input_index.entry(asset_id).or_insert_with(|| {
            let asset = project.asset(asset_id).expect("validated in the EDL");
            inputs.push(resolve_path(base_dir, &asset.path));
            inputs.len() - 1
        })
    };

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
                let idx = input_of(*asset_id, project);
                let effects = match vf {
                    Some(chain) => format!("{chain},"),
                    None => String::new(),
                };
                let setpts = if (*speed - 1.0).abs() > 1e-9 {
                    format!("setpts=(PTS-STARTPTS)/{speed}")
                } else {
                    "setpts=PTS-STARTPTS".to_string()
                };
                fc.push(format!(
                    "[{idx}:v]trim=start={}:end={},{setpts},{effects}{norm}[{label}]",
                    secs(*src_in),
                    secs(*src_out),
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

    // Sequential combination: concat on hard cuts, xfade on transitions.
    let mut current = "v0".to_string();
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
                fc.push(format!(
                    "[{current}][v{k}]xfade=transition={kind}:duration={}:offset={}[{out_label}]",
                    secs(d),
                    secs(offset),
                ));
                acc_dur += seg.duration() - d;
            }
            None => {
                fc.push(format!(
                    "[{current}][v{k}]concat=n=2:v=1:a=0[{out_label}]"
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
        match &layer.src {
            LayerSrc::Media { asset_id, src_in, src_out, speed } => {
                let idx = input_of(*asset_id, project);
                fc.push(format!(
                    "[{idx}:v]trim=start={}:end={},setpts=(PTS-STARTPTS)/{speed}+{}/TB,{effects}{fit},{alpha}fps={fps}[ly{k}]",
                    secs(*src_in),
                    secs(*src_out),
                    secs(start),
                ));
            }
            LayerSrc::Gen { source } => {
                fc.push(format!(
                    "{source},setpts=PTS-STARTPTS+{}/TB,{effects}{fit},{alpha}fps={fps}[ly{k}]",
                    secs(start),
                ));
            }
        }
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

    // reactive avatars (movie+overlay per segment)
    let (avatar_chains, after_avatars) =
        build_avatar_overlays(project, seq, base_dir, &current, out_w);
    fc.extend(avatar_chains);
    current = after_avatars;

    // burn titles and subtitles onto the combined video
    let text_chain = build_text_overlays(project, seq, out_h);
    match text_chain {
        Some(chain) => fc.push(format!("[{current}]{chain}[vout]")),
        None => fc.push(format!("[{current}]null[vout]")),
    }

    // ---- audio chains (the GIF has none) ----
    let is_gif = settings.format == crate::ExportFormat::Gif;
    let mut alabels: Vec<String> = vec![];
    for (k, item) in audio_items.iter().enumerate().filter(|_| !is_gif) {
        let idx = input_of(item.asset_id, project);
        let label = format!("a{k}");
        let dur_us = (((item.src_out - item.src_in) as f64) / item.speed).round() as TimeUs;
        let mut chain = format!(
            "[{idx}:a]atrim=start={}:end={},asetpts=PTS-STARTPTS,\
             aresample=48000,aformat=channel_layouts=stereo",
            secs(item.src_in),
            secs(item.src_out),
        );
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

    // ---- I-O range: trim the already-composited master ----
    let mut vlabel = "[vout]".to_string();
    let mut alabel = "[aout]".to_string();
    let mut duration_us = total_us;
    if let Some((r_in, r_out)) = settings.range {
        let a = r_in.clamp(0, total_us);
        let b = r_out.clamp(a, total_us);
        if b > a {
            fc.push(format!(
                "[vout]trim=start={}:end={},setpts=PTS-STARTPTS[voutr]",
                secs(a),
                secs(b)
            ));
            vlabel = "[voutr]".into();
            if has_audio && !is_gif {
                fc.push(format!(
                    "[aout]atrim=start={}:end={},asetpts=PTS-STARTPTS[aoutr]",
                    secs(a),
                    secs(b)
                ));
                alabel = "[aoutr]".into();
            }
            duration_us = b - a;
        }
    }

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
    for input in &inputs {
        args.push("-i".into());
        args.push(input.to_string_lossy().into_owned());
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

    Ok(FfmpegPlan { args, duration_us })
}
