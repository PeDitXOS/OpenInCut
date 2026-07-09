//! Proxies de preview: copia h264 ligera (≤960 px de ancho, GOP corto) que el
//! FrameService y render_frame decodifican en lugar del original. El export
//! usa siempre el archivo original.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{ffmpeg_bin, MediaError, MediaResult};

/// Ancho máximo del proxy; por debajo de esto no vale la pena generar uno.
pub const PROXY_MAX_W: u32 = 960;

/// Genera (o reutiliza del caché por hash) el proxy de un video.
pub fn generate_proxy(src: &Path, cache_dir: &Path, hash: &str) -> MediaResult<PathBuf> {
    let out = cache_dir.join(format!("{hash}.proxy.mp4"));
    if out.exists() {
        return Ok(out);
    }
    std::fs::create_dir_all(cache_dir)?;
    let tmp = cache_dir.join(format!("{hash}.proxy.part.mp4"));
    let status = Command::new(ffmpeg_bin())
        .args(["-y", "-v", "error", "-i"])
        .arg(src)
        .args([
            "-vf",
            &format!("scale='min({PROXY_MAX_W},iw)':-2"),
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-crf",
            "28",
            // GOP corto: seeks del preview casi instantáneos
            "-g",
            "12",
            "-an",
            "-movflags",
            "+faststart",
        ])
        .arg(&tmp)
        .status()
        .map_err(|e| MediaError::Spawn("ffmpeg".into(), e.to_string()))?;
    if !status.success() || !tmp.exists() {
        let _ = std::fs::remove_file(&tmp);
        return Err(MediaError::Tool("ffmpeg".into(), "proxy falló".into()));
    }
    std::fs::rename(&tmp, &out)?;
    Ok(out)
}
