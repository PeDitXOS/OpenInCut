//! Tiras de miniaturas (filmstrip) por asset para el timeline: un solo JPEG
//! horizontal de N tiles a intervalos regulares, cacheado por hash de contenido.

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

/// Genera (o reutiliza del caché) la tira de miniaturas de un video.
/// `hash` clava el nombre del archivo en caché; regenera solo si no existe.
pub fn generate_thumb_strip(
    src: &Path,
    duration_us: i64,
    cache_dir: &Path,
    hash: &str,
) -> MediaResult<ThumbStrip> {
    let dur_s = (duration_us as f64 / 1e6).max(0.5);
    // ≤60 tiles; para videos cortos ~1 tile por segundo
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
    // en archivos largos, decodificar solo keyframes (mucho más rápido);
    // en cortos hay pocos keyframes y saldrían tiles repetidos
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
        return Err(MediaError::Tool("ffmpeg".into(), "tira de miniaturas falló".into()));
    }
    std::fs::rename(&tmp, &out)?;
    Ok(strip)
}
