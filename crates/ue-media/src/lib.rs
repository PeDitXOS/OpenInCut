//! ue-media: sondeo (ffprobe), hashing e importación de archivos, y extracción
//! de frames reales (ffmpeg) para el preview. Los binarios de FFmpeg se toman
//! del PATH en desarrollo (`UE_FFMPEG`/`UE_FFPROBE` los sobreescriben); en
//! producción serán sidecars empaquetados.

pub mod frame;
pub mod hashing;
pub mod probe;
pub mod stream;
pub mod thumbs;

use std::path::Path;

use thiserror::Error;
use ue_core::model::{MediaAsset, MediaKind};
use ulid::Ulid;

#[derive(Debug, Error)]
pub enum MediaError {
    #[error("no se pudo ejecutar {0}: {1}")]
    Spawn(String, String),
    #[error("{0} falló: {1}")]
    Tool(String, String),
    #[error("no se pudo interpretar la salida de ffprobe: {0}")]
    Parse(String),
    #[error("archivo no soportado o sin streams: {0}")]
    Unsupported(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

pub type MediaResult<T> = Result<T, MediaError>;

pub fn ffmpeg_bin() -> String {
    std::env::var("UE_FFMPEG").unwrap_or_else(|_| "ffmpeg".into())
}

pub fn ffprobe_bin() -> String {
    std::env::var("UE_FFPROBE").unwrap_or_else(|_| "ffprobe".into())
}

/// Importa un archivo: probe + hash → MediaAsset listo para el pool.
pub fn import_file(path: &Path) -> MediaResult<MediaAsset> {
    let (kind, probe_info) = probe::probe(path)?;
    let content_hash = hashing::content_hash(path)?;
    Ok(MediaAsset {
        id: Ulid::new(),
        kind,
        path: path.to_string_lossy().into_owned(),
        content_hash,
        probe: probe_info,
        proxy: None,
        audio_conform: None,
        peaks: None,
        thumbnails: None,
        transcript: None,
        offline: false,
    })
}

/// Conforma el audio de un archivo a WAV PCM s16le 48 kHz estéreo (PLAN §5.3).
/// Idempotente: si `out` ya existe no re-conforma.
pub fn conform_audio(src: &Path, out: &Path) -> MediaResult<()> {
    if out.exists() {
        return Ok(());
    }
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = out.with_extension("wav.tmp");
    let result = std::process::Command::new(ffmpeg_bin())
        .args(["-y", "-v", "error", "-i"])
        .arg(src)
        .args(["-vn", "-ac", "2", "-ar", "48000", "-c:a", "pcm_s16le", "-f", "wav"])
        .arg(&tmp)
        .output()
        .map_err(|e| MediaError::Spawn("ffmpeg".into(), e.to_string()))?;
    if !result.status.success() {
        let _ = std::fs::remove_file(&tmp);
        return Err(MediaError::Tool(
            "ffmpeg (conform)".into(),
            String::from_utf8_lossy(&result.stderr).trim().to_string(),
        ));
    }
    std::fs::rename(&tmp, out)?;
    Ok(())
}

/// Duración por defecto de un clip de imagen fija.
pub const IMAGE_CLIP_DURATION_US: i64 = 5_000_000;

pub fn default_clip_duration(asset: &MediaAsset) -> i64 {
    match asset.kind {
        MediaKind::Image => IMAGE_CLIP_DURATION_US,
        _ => asset.probe.duration_us,
    }
}
