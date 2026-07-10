//! Thumbnail strips (filmstrip) per asset for the timeline: a single
//! horizontal JPEG of N tiles at regular intervals, cached by content hash.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

use crate::{ffmpeg_bin, MediaError, MediaResult};

pub const TILE_W: u32 = 96;
pub const TILE_H: u32 = 54;

#[derive(Debug, Clone, Serialize)]
pub struct ThumbStrip {
    pub path: PathBuf,
    pub tile_w: u32,
    pub tile_h: u32,
    pub count: u32,
    pub interval_us: i64,
}

/// Generates (or reuses from the cache) a video's thumbnail strip.
/// `hash` pins the cache file name; regenerates only if it doesn't exist.
pub fn generate_thumb_strip(
    src: &Path,
    duration_us: i64,
    cache_dir: &Path,
    hash: &str,
) -> MediaResult<ThumbStrip> {
    let dur_s = (duration_us as f64 / 1e6).max(0.5);
    // ≤60 tiles; for short videos ~1 tile per second
    let count = (dur_s.ceil() as u32).clamp(4, 60);
    let interval_us = duration_us / count as i64;
    let out = cache_dir.join(format!("{hash}.thumbs.jpg"));
    let strip = ThumbStrip {
        path: out.clone(),
        tile_w: TILE_W,
        tile_h: TILE_H,
        count,
        interval_us,
    };
    if out.exists() {
        return Ok(strip);
    }
    std::fs::create_dir_all(cache_dir)?;
    let tmp = cache_dir.join(format!("{hash}.thumbs.part.jpg"));
    let fps = count as f64 / dur_s;
    let mut cmd = Command::new(ffmpeg_bin());
    cmd.args(["-y", "-v", "error"]);
    // on long files, decode only keyframes (much faster);
    // on short ones there are few keyframes and tiles would come out repeated
    if dur_s > 300.0 {
        cmd.args(["-skip_frame", "nokey"]);
    }
    let status = cmd
        .arg("-i")
        .arg(src)
        .args([
            "-vf",
            &format!("fps={fps:.6},scale={TILE_W}:{TILE_H},tile={count}x1"),
            "-frames:v",
            "1",
            "-q:v",
            "5",
        ])
        .arg(&tmp)
        .status()
        .map_err(|e| MediaError::Spawn("ffmpeg".into(), e.to_string()))?;
    if !status.success() || !tmp.exists() {
        let _ = std::fs::remove_file(&tmp);
        return Err(MediaError::Tool("ffmpeg".into(), "thumbnail strip failed".into()));
    }
    std::fs::rename(&tmp, &out)?;
    Ok(strip)
}
