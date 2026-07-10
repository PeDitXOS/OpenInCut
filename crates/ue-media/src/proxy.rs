//! Preview proxies: a light h264 copy (≤960 px wide, short GOP) that the
//! FrameService and render_frame decode instead of the original. Export
//! always uses the original file.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{ffmpeg_bin, MediaError, MediaResult};

/// Maximum proxy width; below this it's not worth generating one.
pub const PROXY_MAX_W: u32 = 960;

/// Generates (or reuses from the hash cache) a video's proxy.
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
            // short GOP: preview seeks are nearly instant
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
        return Err(MediaError::Tool("ffmpeg".into(), "proxy failed".into()));
    }
    std::fs::rename(&tmp, &out)?;
    Ok(out)
}
