//! Real frame extraction for the preview (v0: top video clip).
//! The Phase 2 wgpu engine will replace this path with full compositing;
//! the interface (sequence time → image) stays the same.

use std::path::Path;
use std::process::Command;

use ue_core::model::{ClipPayload, EffectInstance, Id, Project, TrackKind, Transform2D};
use ue_core::TimeUs;

use crate::{ffmpeg_bin, MediaError, MediaResult};

/// Info about the resolved active clip (for debug/tests).
#[derive(Debug, PartialEq)]
pub struct ResolvedFrame {
    pub asset_path: String,
    pub src_t_us: TimeUs,
    /// Time relative to the clip start (timeline µs): evaluates curves.
    pub clip_rel_us: TimeUs,
    /// Clip speed (to map the stream's t to clip time).
    pub speed: f64,
    /// Effects and transform of the resolved clip (the upper layer decides the chain).
    pub effects: Vec<EffectInstance>,
    pub transform: Transform2D,
}

/// Resolves which asset and which source instant correspond to sequence time
/// `t_us` (video clip from the topmost track, not muted).
pub fn resolve_top_video(project: &Project, sequence_id: Id, t_us: TimeUs) -> Option<ResolvedFrame> {
    let seq = project.sequence(sequence_id)?;
    // tracks compose from the bottom (index 0) upward → search from the top
    for track in seq.tracks.iter().rev() {
        if track.kind != TrackKind::Video || track.muted {
            continue;
        }
        for clip in &track.clips {
            if clip.start <= t_us && t_us < clip.end() {
                if let ClipPayload::Media { asset_id, src_in, .. } = &clip.payload {
                    let asset = project.asset(*asset_id)?;
                    let src_t = *src_in
                        + ((t_us - clip.start) as f64 * clip.speed).round() as TimeUs;
                    // preview: prefer the proxy (lighter to decode);
                    // export always uses the original
                    let path = asset
                        .proxy
                        .clone()
                        .filter(|p| Path::new(p).exists())
                        .unwrap_or_else(|| asset.path.clone());
                    return Some(ResolvedFrame {
                        asset_path: path,
                        src_t_us: src_t,
                        clip_rel_us: t_us - clip.start,
                        speed: clip.speed,
                        effects: clip.effects.clone(),
                        transform: clip.transform.clone(),
                    });
                }
            }
        }
    }
    None
}

/// Extracts a JPEG frame at sequence time `t_us`. `None` if there is no active
/// video clip. `base_dir` resolves relative asset paths.
/// `extra_vf` is the clip's effects chain (applied before the rescale).
pub fn render_frame(
    project: &Project,
    sequence_id: Id,
    t_us: TimeUs,
    max_width: u32,
    base_dir: &Path,
    extra_vf: Option<&str>,
) -> MediaResult<Option<Vec<u8>>> {
    let Some(resolved) = resolve_top_video(project, sequence_id, t_us) else {
        return Ok(None);
    };
    let path = {
        let p = Path::new(&resolved.asset_path);
        if p.is_absolute() { p.to_path_buf() } else { base_dir.join(p) }
    };
    let ss = format!("{:.6}", resolved.src_t_us as f64 / 1_000_000.0);
    let scale = format!("scale='min({max_width},iw)':-2");
    let vf = match extra_vf {
        Some(chain) if !chain.is_empty() => format!("{chain},{scale}"),
        _ => scale,
    };
    let out = Command::new(ffmpeg_bin())
        .args(["-v", "error", "-ss", &ss, "-i"])
        .arg(&path)
        .args(["-frames:v", "1", "-vf", &vf, "-f", "image2", "-c:v", "mjpeg", "-q:v", "4", "pipe:1"])
        .output()
        .map_err(|e| MediaError::Spawn("ffmpeg".into(), e.to_string()))?;
    if !out.status.success() {
        return Err(MediaError::Tool(
            "ffmpeg".into(),
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    if out.stdout.is_empty() {
        // -ss past the end of the file produces empty output
        return Ok(None);
    }
    Ok(Some(out.stdout))
}
