//! ue-media: probing (ffprobe), hashing and file import, and real frame
//! extraction (ffmpeg) for the preview. FFmpeg binaries are taken from the
//! PATH in development (`UE_FFMPEG`/`UE_FFPROBE` override them); in production
//! they will be packaged sidecars.

pub mod frame;
pub mod hashing;
pub mod denoise;
pub mod probe;
pub mod proxy;
pub mod thumbs;

use std::path::Path;

use thiserror::Error;
use ue_core::model::{MediaAsset, MediaKind};
use ulid::Ulid;

#[derive(Debug, Error)]
pub enum MediaError {
    #[error("could not run {0}: {1}")]
    Spawn(String, String),
    #[error("{0} failed: {1}")]
    Tool(String, String),
    #[error("could not parse ffprobe output: {0}")]
    Parse(String),
    #[error("unsupported file or no streams: {0}")]
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

/// Imports a file: probe + hash → MediaAsset ready for the pool.
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

/// Conforms a file's audio to WAV PCM s16le 48 kHz stereo (PLAN §5.3).
/// Idempotent: if `out` already exists it does not re-conform.
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

/// Default duration of a still-image clip.
pub const IMAGE_CLIP_DURATION_US: i64 = 5_000_000;

/// Is this path a still image (by extension)? Stills need `-loop 1` and no
/// seeking in the decoders.
pub fn is_image_path(path: &std::path::Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(),
        Some("png" | "jpg" | "jpeg" | "webp" | "bmp" | "tiff" | "tif" | "gif" | "heic")
    )
}

pub fn default_clip_duration(asset: &MediaAsset) -> i64 {
    match asset.kind {
        MediaKind::Image => IMAGE_CLIP_DURATION_US,
        _ => asset.probe.duration_us,
    }
}
