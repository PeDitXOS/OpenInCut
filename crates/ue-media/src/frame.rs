//! Extracción de frames reales para el preview (v0: clip de video superior).
//! El motor wgpu de la Fase 2 sustituirá esta ruta por composición completa;
//! la interfaz (tiempo de secuencia → imagen) se mantiene.

use std::path::Path;
use std::process::Command;

use ue_core::model::{ClipPayload, EffectInstance, Id, Project, TrackKind, Transform2D};
use ue_core::TimeUs;

use crate::{ffmpeg_bin, MediaError, MediaResult};

/// Información del clip activo resuelto (para debug/tests).
#[derive(Debug, PartialEq)]
pub struct ResolvedFrame {
    pub asset_path: String,
    pub src_t_us: TimeUs,
    /// Tiempo relativo al inicio del clip (µs de timeline): evalúa curvas.
    pub clip_rel_us: TimeUs,
    /// Efectos y transform del clip resuelto (la capa superior decide la cadena).
    pub effects: Vec<EffectInstance>,
    pub transform: Transform2D,
}

/// Resuelve qué asset y qué instante fuente corresponden al tiempo `t_us`
/// de la secuencia (clip de video de la pista más alta, sin mute).
pub fn resolve_top_video(project: &Project, sequence_id: Id, t_us: TimeUs) -> Option<ResolvedFrame> {
    let seq = project.sequence(sequence_id)?;
    // las pistas se componen de abajo (índice 0) hacia arriba → buscar desde arriba
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
                    // preview: preferir el proxy (más ligero de decodificar);
                    // el export usa siempre el original
                    let path = asset
                        .proxy
                        .clone()
                        .filter(|p| Path::new(p).exists())
                        .unwrap_or_else(|| asset.path.clone());
                    return Some(ResolvedFrame {
                        asset_path: path,
                        src_t_us: src_t,
                        clip_rel_us: t_us - clip.start,
                        effects: clip.effects.clone(),
                        transform: clip.transform.clone(),
                    });
                }
            }
        }
    }
    None
}

/// Extrae un frame JPEG del tiempo `t_us` de la secuencia. `None` si no hay
/// clip de video activo. `base_dir` resuelve rutas relativas de assets.
/// `extra_vf` es la cadena de efectos del clip (se aplica antes del reescalado).
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
        // -ss más allá del final del archivo produce salida vacía
        return Ok(None);
    }
    Ok(Some(out.stdout))
}
