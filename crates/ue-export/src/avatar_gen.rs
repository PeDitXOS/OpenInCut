//! Standalone avatar video generation (port of the toolkit's
//! avatar_video_generation.py, but ffmpeg-native).
//!
//! Given a transcript with per-segment emotions and an `AvatarConfig`, it
//! renders a transparent-background video (or a keyed one over black) where
//! the right expression plays for each segment and the avatar shakes with the
//! speaker's volume. The result is imported as a normal media asset, so the
//! user can place, scale, crop, or key it however they like.

use std::path::{Path, PathBuf};
use std::process::Command;

use ue_core::model::{AvatarConfig, TranscriptDoc};
use ue_core::TimeUs;

use crate::{ExportError, ExportResult};

/// One rendered stretch: which expression, when, and how loud.
#[derive(Debug, Clone, PartialEq)]
pub struct AvatarSpan {
    pub expression: String,
    pub from_us: TimeUs,
    pub to_us: TimeUs,
    /// Segment RMS / average RMS: drives the shake amplitude.
    pub volume_ratio: f64,
}

/// Contiguous spans covering [0, duration): every gap is filled with the
/// default expression, so the avatar is always on screen.
pub fn plan_spans(doc: &TranscriptDoc, config: &AvatarConfig, duration_us: TimeUs) -> Vec<AvatarSpan> {
    let Some(default) = config.default_expression().map(|e| e.name.clone()) else {
        return vec![];
    };
    let avg = if doc.global_avg_volume > 1e-9 { doc.global_avg_volume } else { 1.0 };
    let known: Vec<&str> = config.expressions.iter().map(|e| e.name.as_str()).collect();

    let mut spans: Vec<AvatarSpan> = vec![];
    let mut cursor: TimeUs = 0;
    for seg in &doc.segments {
        let from = seg.start_us.clamp(0, duration_us);
        let to = seg.end_us.clamp(from, duration_us);
        if to <= from {
            continue;
        }
        if from > cursor {
            spans.push(AvatarSpan {
                expression: default.clone(),
                from_us: cursor,
                to_us: from,
                volume_ratio: 1.0,
            });
        }
        let emotion = seg
            .emotion
            .as_deref()
            .filter(|e| known.contains(e))
            .unwrap_or(&default)
            .to_string();
        spans.push(AvatarSpan {
            expression: emotion,
            from_us: from,
            to_us: to,
            volume_ratio: (seg.volume_rms / avg).clamp(0.0, 3.0),
        });
        cursor = to;
    }
    if cursor < duration_us {
        spans.push(AvatarSpan {
            expression: default,
            from_us: cursor,
            to_us: duration_us,
            volume_ratio: 1.0,
        });
    }
    spans
}

fn secs(us: TimeUs) -> String {
    format!("{:.6}", us as f64 / 1_000_000.0)
}

/// Is the file a still image? (images loop as a single frame)
fn is_image(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()).as_deref(),
        Some("png" | "jpg" | "jpeg" | "webp" | "bmp" | "tiff" | "gif")
    )
}

/// ffmpeg plan for the avatar video. Public so it can be unit-tested without
/// running ffmpeg.
pub fn build_args(
    config: &AvatarConfig,
    spans: &[AvatarSpan],
    duration_us: TimeUs,
    size: (u32, u32),
    fps: (u32, u32),
    output: &Path,
) -> ExportResult<Vec<String>> {
    if spans.is_empty() {
        return Err(ExportError::EmptyTimeline);
    }
    let (w, h) = size;
    let fps_s = format!("{}/{}", fps.0, fps.1);
    let mut args: Vec<String> = vec!["-y".into(), "-v".into(), "error".into()];

    // one input per expression actually used (images loop, videos loop too)
    let mut inputs: Vec<(String, PathBuf)> = vec![];
    for span in spans {
        if inputs.iter().any(|(name, _)| *name == span.expression) {
            continue;
        }
        let expr = config
            .expressions
            .iter()
            .find(|e| e.name == span.expression)
            .ok_or_else(|| ExportError::Ffmpeg(format!("unknown expression {}", span.expression)))?;
        inputs.push((expr.name.clone(), PathBuf::from(&expr.path)));
    }
    for (_, path) in &inputs {
        if is_image(path) {
            args.extend(["-loop".into(), "1".into()]);
        } else {
            args.extend(["-stream_loop".into(), "-1".into()]);
        }
        args.push("-i".into());
        args.push(path.to_string_lossy().into_owned());
    }

    let mut fc: Vec<String> = vec![];
    // transparent canvas for the whole duration
    fc.push(format!(
        "color=c=black@0.0:s={w}x{h}:rate={fps_s}:duration={},format=rgba[base]",
        secs(duration_us)
    ));
    // each expression, fitted and alpha-preserving
    // the avatar occupies `scale` of the canvas width (the rest stays transparent)
    let aw = (((w as f64) * config.scale.clamp(0.05, 1.0)) as u32) & !1;
    for (i, (name, _)) in inputs.iter().enumerate() {
        fc.push(format!(
            "[{i}:v]format=rgba,scale={aw}:-2,fps={fps_s},setpts=PTS-STARTPTS[e{}]",
            sanitize(name)
        ));
        // one copy per span that uses it
        let uses = spans.iter().filter(|s| s.expression == *name).count();
        if uses > 1 {
            let outs: String = (0..uses).map(|k| format!("[e{}_{k}]", sanitize(name))).collect();
            fc.push(format!("[e{}]split={uses}{outs}", sanitize(name)));
        }
    }

    // overlay each span with its shake, in order
    let mut counters: std::collections::BTreeMap<String, usize> = Default::default();
    let mut current = "base".to_string();
    for (i, span) in spans.iter().enumerate() {
        let key = sanitize(&span.expression);
        let uses = spans.iter().filter(|s| s.expression == span.expression).count();
        let label = if uses > 1 {
            let c = counters.entry(key.clone()).or_insert(0);
            let l = format!("e{key}_{c}");
            *c += 1;
            l
        } else {
            format!("e{key}")
        };
        // deterministic sinusoidal shake, amplitude scaled by the segment volume
        let amp = (config.shake_factor * span.volume_ratio * 6.0).clamp(0.0, 40.0);
        let (x, y) = if amp > 0.05 {
            (
                format!("'(W-w)/2+{amp:.2}*sin(t*37)'"),
                format!("'(H-h)/2+{amp:.2}*sin(t*23)'"),
            )
        } else {
            ("(W-w)/2".to_string(), "(H-h)/2".to_string())
        };
        let out = format!("ov{i}");
        fc.push(format!(
            "[{current}][{label}]overlay=x={x}:y={y}:eof_action=pass:format=auto:\
             enable='between(t,{},{})'[{out}]",
            secs(span.from_us),
            secs(span.to_us),
        ));
        current = out;
    }

    fc.push(format!("[{current}]format=rgba[out]"));
    args.push("-filter_complex".into());
    args.push(fc.join(";"));
    args.extend(["-map".into(), "[out]".into()]);
    args.extend(["-t".into(), secs(duration_us)]);
    // qtrle in a .mov keeps a REAL alpha channel (verified: libvpx-vp9 quietly
    // drops it to yuv420p even with -pix_fmt yuva420p), so the generated
    // avatar can simply be overlaid on any track.
    args.extend(["-c:v".into(), "qtrle".into(), "-an".into()]);
    args.push(output.to_string_lossy().into_owned());
    Ok(args)
}

/// ffmpeg-safe label fragment.
fn sanitize(name: &str) -> String {
    name.chars().filter(|c| c.is_ascii_alphanumeric()).collect()
}

/// Renders the avatar video. Blocking; `progress` receives 0..1.
pub fn generate(
    config: &AvatarConfig,
    doc: &TranscriptDoc,
    duration_us: TimeUs,
    size: (u32, u32),
    fps: (u32, u32),
    output: &Path,
    mut progress: impl FnMut(f64),
) -> ExportResult<Vec<AvatarSpan>> {
    let spans = plan_spans(doc, config, duration_us);
    let args = build_args(config, &spans, duration_us, size, fps, output)?;
    progress(0.05);
    let out = Command::new(ue_media::ffmpeg_bin())
        .args(&args)
        .output()
        .map_err(|e| ExportError::Ffmpeg(e.to_string()))?;
    if !out.status.success() {
        return Err(ExportError::Ffmpeg(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    progress(1.0);
    Ok(spans)
}
