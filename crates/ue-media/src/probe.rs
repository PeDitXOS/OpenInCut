//! File probing with ffprobe (-print_format json).

use std::path::Path;
use std::process::Command;

use serde::Deserialize;
use ue_core::model::{MediaKind, ProbeInfo};

use crate::{ffprobe_bin, MediaError, MediaResult};

#[derive(Deserialize)]
struct FfprobeOut {
    #[serde(default)]
    format: Option<FfFormat>,
    #[serde(default)]
    streams: Vec<FfStream>,
}

#[derive(Deserialize)]
struct FfFormat {
    #[serde(default)]
    format_name: String,
    #[serde(default)]
    duration: Option<String>,
}

#[derive(Deserialize)]
struct FfStream {
    codec_type: String,
    #[serde(default)]
    codec_name: Option<String>,
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
    #[serde(default)]
    avg_frame_rate: Option<String>,
    #[serde(default)]
    r_frame_rate: Option<String>,
    #[serde(default)]
    channels: Option<u32>,
    #[serde(default)]
    duration: Option<String>,
    #[serde(default)]
    side_data_list: Option<Vec<serde_json::Value>>,
}

const IMAGE_FORMATS: &[&str] = &[
    "image2", "png_pipe", "webp_pipe", "bmp_pipe", "tiff_pipe", "jpeg_pipe", "gif_pipe",
];

fn parse_rate(s: &str) -> Option<(u32, u32)> {
    let (n, d) = s.split_once('/')?;
    let n: u32 = n.parse().ok()?;
    let d: u32 = d.parse().ok()?;
    if n == 0 || d == 0 {
        return None;
    }
    Some((n, d))
}

fn parse_secs(s: &str) -> Option<i64> {
    let f: f64 = s.parse().ok()?;
    Some((f * 1_000_000.0).round() as i64)
}

pub fn probe(path: &Path) -> MediaResult<(MediaKind, ProbeInfo)> {
    let out = Command::new(ffprobe_bin())
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(path)
        .output()
        .map_err(|e| MediaError::Spawn("ffprobe".into(), e.to_string()))?;
    if !out.status.success() {
        return Err(MediaError::Tool(
            "ffprobe".into(),
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    let parsed: FfprobeOut = serde_json::from_slice(&out.stdout)
        .map_err(|e| MediaError::Parse(e.to_string()))?;

    let video = parsed.streams.iter().find(|s| s.codec_type == "video");
    let audio = parsed.streams.iter().find(|s| s.codec_type == "audio");
    let format_name = parsed
        .format
        .as_ref()
        .map(|f| f.format_name.clone())
        .unwrap_or_default();

    let kind = if video.is_some()
        && IMAGE_FORMATS.iter().any(|f| format_name.split(',').any(|p| p == *f))
    {
        MediaKind::Image
    } else if video.is_some() {
        MediaKind::Video
    } else if audio.is_some() {
        MediaKind::Audio
    } else {
        return Err(MediaError::Unsupported(path.display().to_string()));
    };

    // duration: format → video stream → audio stream
    let duration_us = parsed
        .format
        .as_ref()
        .and_then(|f| f.duration.as_deref())
        .and_then(parse_secs)
        .or_else(|| video.and_then(|s| s.duration.as_deref()).and_then(parse_secs))
        .or_else(|| audio.and_then(|s| s.duration.as_deref()).and_then(parse_secs))
        .unwrap_or(0);

    // rotation from side_data (displaymatrix)
    let rotation = video
        .and_then(|s| s.side_data_list.as_ref())
        .and_then(|list| {
            list.iter().find_map(|sd| {
                sd.get("rotation").and_then(|r| r.as_f64()).map(|r| r as i32)
            })
        })
        .unwrap_or(0);

    // VFR: avg and r really differ (not just by representation)
    let fps = video.and_then(|s| s.avg_frame_rate.as_deref()).and_then(parse_rate);
    let r_fps = video.and_then(|s| s.r_frame_rate.as_deref()).and_then(parse_rate);
    let vfr = match (fps, r_fps) {
        (Some((an, ad)), Some((rn, rd))) => (an as u64) * (rd as u64) != (rn as u64) * (ad as u64),
        _ => false,
    };

    let info = ProbeInfo {
        duration_us,
        fps: if kind == MediaKind::Video { fps } else { None },
        width: video.and_then(|s| s.width).unwrap_or(0),
        height: video.and_then(|s| s.height).unwrap_or(0),
        rotation,
        vcodec: video.and_then(|s| s.codec_name.clone()),
        acodec: audio.and_then(|s| s.codec_name.clone()),
        audio_channels: audio.and_then(|s| s.channels).unwrap_or(0),
        vfr,
    };
    Ok((kind, info))
}
