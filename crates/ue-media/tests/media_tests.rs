//! Integration tests with real FFmpeg: generate synthetic media (testsrc with
//! a burned-in counter, tones, png) and verify probe, hash, import and frames.
//! If there's no ffmpeg on PATH, the tests are skipped with a notice.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use ue_core::model::*;
use ue_core::ops::InsertMode;
use ue_core::ProjectStore;
use ue_media::frame::{render_frame, resolve_top_video};
use ue_media::{default_clip_duration, import_file};

const SEC: i64 = 1_000_000;

fn ffmpeg_available() -> bool {
    Command::new(ue_media::ffmpeg_bin())
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Generates the test media once in target/ue-test-media.
fn demo_dir() -> Option<&'static PathBuf> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        if !ffmpeg_available() {
            eprintln!("NOTICE: ffmpeg not available; media tests skipped");
            return None;
        }
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-test-media");
        std::fs::create_dir_all(&dir).unwrap();
        let ff = ue_media::ffmpeg_bin();
        let runs: Vec<Vec<String>> = vec![
            // video with a burned-in time counter (testsrc) + tone
            vec![
                "-y".into(), "-v".into(), "error".into(),
                "-f".into(), "lavfi".into(), "-i".into(),
                "testsrc=duration=6:size=640x360:rate=30".into(),
                "-f".into(), "lavfi".into(), "-i".into(),
                "sine=frequency=440:duration=6".into(),
                "-c:v".into(), "libx264".into(), "-preset".into(), "ultrafast".into(),
                "-pix_fmt".into(), "yuv420p".into(), "-c:a".into(), "aac".into(),
                "-shortest".into(),
                dir.join("video_a.mp4").to_string_lossy().into_owned(),
            ],
            // audio only
            vec![
                "-y".into(), "-v".into(), "error".into(),
                "-f".into(), "lavfi".into(), "-i".into(),
                "sine=frequency=220:duration=3".into(),
                dir.join("tone.wav").to_string_lossy().into_owned(),
            ],
            // image
            vec![
                "-y".into(), "-v".into(), "error".into(),
                "-f".into(), "lavfi".into(), "-i".into(),
                "color=c=0x336699:s=320x240,format=rgba".into(),
                "-frames:v".into(), "1".into(),
                dir.join("img.png").to_string_lossy().into_owned(),
            ],
        ];
        for args in runs {
            let st = Command::new(&ff).args(&args).status().expect("ffmpeg corre");
            assert!(st.success(), "media generation failed: {args:?}");
        }
        Some(dir)
    })
    .as_ref()
}

macro_rules! require_media {
    () => {
        match demo_dir() {
            Some(d) => d,
            None => return,
        }
    };
}

#[test]
fn probe_video_kind_duration_fps() {
    let dir = require_media!();
    let asset = import_file(&dir.join("video_a.mp4")).unwrap();
    assert_eq!(asset.kind, MediaKind::Video);
    assert_eq!(asset.probe.width, 640);
    assert_eq!(asset.probe.height, 360);
    assert_eq!(asset.probe.fps, Some((30, 1)));
    let dur_s = asset.probe.duration_us as f64 / 1e6;
    assert!((5.8..=6.3).contains(&dur_s), "duration ≈ 6 s, was {dur_s}");
    assert_eq!(asset.probe.vcodec.as_deref(), Some("h264"));
    assert_eq!(asset.probe.acodec.as_deref(), Some("aac"));
    assert!(asset.content_hash.starts_with("xxh3:"));
    assert!(!asset.probe.vfr, "synthetic media is CFR");
}

#[test]
fn probe_audio_and_image_kinds() {
    let dir = require_media!();
    let audio = import_file(&dir.join("tone.wav")).unwrap();
    assert_eq!(audio.kind, MediaKind::Audio);
    assert_eq!(audio.probe.audio_channels, 1);
    let dur_s = audio.probe.duration_us as f64 / 1e6;
    assert!((2.9..=3.1).contains(&dur_s));

    let img = import_file(&dir.join("img.png")).unwrap();
    assert_eq!(img.kind, MediaKind::Image);
    assert_eq!((img.probe.width, img.probe.height), (320, 240));
    assert_eq!(default_clip_duration(&img), 5 * SEC);
}

#[test]
fn probe_unsupported_file_errors() {
    let dir = require_media!();
    let bogus = dir.join("bogus.txt");
    std::fs::write(&bogus, "this is not media").unwrap();
    assert!(import_file(&bogus).is_err());
}

/// Project: clip on the timeline at t=10s using the source from 3s.
/// The timeline→source mapping must give src_t = 3 + (t-10).
#[test]
fn resolve_maps_timeline_to_source_time() {
    let dir = require_media!();
    let mut project = Project::new("t");
    let seq_id = project.active_sequence;
    let asset = import_file(&dir.join("video_a.mp4")).unwrap();
    let asset_id = asset.id;
    project.assets.push(asset);
    let vtrack = project
        .sequence(seq_id)
        .unwrap()
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Video)
        .unwrap()
        .id;
    let mut store = ProjectStore::new(project);
    let clip = Clip::new_media(asset_id, 3 * SEC, 6 * SEC, 10 * SEC);
    store.insert_clip(vtrack, clip, InsertMode::Strict).unwrap();

    let r = resolve_top_video(&store.project, seq_id, 12 * SEC).unwrap();
    assert_eq!(r.src_t_us, 5 * SEC);
    assert!(r.asset_path.ends_with("video_a.mp4"));

    // outside the clip → None
    assert!(resolve_top_video(&store.project, seq_id, 1 * SEC).is_none());
    assert!(resolve_top_video(&store.project, seq_id, 20 * SEC).is_none());
}

#[test]
fn mjpeg_session_reads_sequential_frames() {
    let dir = require_media!();
    let mut session =
        ue_media::stream::MjpegSession::open(&dir.join("video_a.mp4"), 2_000_000, 480, 24, None)
            .unwrap();
    assert_eq!(session.next_src_us(), 2_000_000);
    let mut frames = 0;
    for _ in 0..12 {
        let frame = session.next_frame().unwrap().expect("frame available");
        assert_eq!(&frame[0..2], &[0xFF, 0xD8]);
        assert_eq!(&frame[frame.len() - 2..], &[0xFF, 0xD9]);
        frames += 1;
    }
    assert_eq!(frames, 12);
    // 12 frames at 24 fps = 0.5 s advanced from the session start
    assert_eq!(session.next_src_us(), 2_500_000);
    // the video lasts 6 s: from 2 s ~4 s * 24 fps ≈ 96 frames remain; exhaust it
    let mut rest = 0;
    while session.next_frame().unwrap().is_some() {
        rest += 1;
        assert!(rest < 200, "must not be infinite");
    }
    assert!((80..=110).contains(&rest), "≈96 frames remained, got {rest}");
}

#[test]
fn render_frame_produces_jpegs_for_visual_check() {
    let dir = require_media!();
    let mut project = Project::new("t");
    let seq_id = project.active_sequence;
    let asset = import_file(&dir.join("video_a.mp4")).unwrap();
    let asset_id = asset.id;
    project.assets.push(asset);
    let vtrack = project
        .sequence(seq_id)
        .unwrap()
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Video)
        .unwrap()
        .id;
    let mut store = ProjectStore::new(project);
    // full clip (0..6s of the source) placed at t=0
    let clip = Clip::new_media(asset_id, 0, 6 * SEC, 0);
    store.insert_clip(vtrack, clip, InsertMode::Strict).unwrap();

    let out_dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-test-frames");
    std::fs::create_dir_all(&out_dir).unwrap();

    for (name, t) in [("frame_0s", 0i64), ("frame_2s", 2 * SEC), ("frame_5s", 5 * SEC)] {
        let jpeg = render_frame(&store.project, seq_id, t, 640, dir, None)
            .unwrap()
            .unwrap_or_else(|| panic!("frame at {name} must exist"));
        assert!(jpeg.len() > 1000, "reasonable jpeg at {name}");
        assert_eq!(&jpeg[0..2], &[0xFF, 0xD8], "JPEG header at {name}");
        std::fs::write(out_dir.join(format!("{name}.jpg")), &jpeg).unwrap();
    }
    // no active clip → None
    let none = render_frame(&store.project, seq_id, 30 * SEC, 640, dir, None).unwrap();
    assert!(none.is_none());
    eprintln!("visual-check frames at: {}", out_dir.display());
}

#[test]
fn thumb_strip_generates_tiled_jpeg() {
    let Some(dir) = demo_dir() else { return };
    let cache = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-thumbs-cache");
    let src = dir.join("video_a.mp4");
    let strip =
        ue_media::thumbs::generate_thumb_strip(&src, 6_000_000, &cache, "hash-thumbs-test")
            .unwrap();
    assert!(strip.path.exists());
    assert_eq!(strip.count, 6, "6 s → 6 tiles");
    // JPEG dimensions = count*tile_w × tile_h
    let (kind, info) = ue_media::probe::probe(&strip.path).unwrap();
    assert_eq!(kind, MediaKind::Image);
    assert_eq!(info.width, strip.tile_w * strip.count);
    assert_eq!(info.height, strip.tile_h);
    // second call: reuses the cache (same mtime)
    let m1 = std::fs::metadata(&strip.path).unwrap().modified().unwrap();
    let again =
        ue_media::thumbs::generate_thumb_strip(&src, 6_000_000, &cache, "hash-thumbs-test")
            .unwrap();
    let m2 = std::fs::metadata(&again.path).unwrap().modified().unwrap();
    assert_eq!(m1, m2, "does not regenerate if it already exists");
}

#[test]
fn proxy_generates_light_h264_and_preview_prefers_it() {
    let Some(dir) = demo_dir() else { return };
    let cache = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-proxy-cache");
    let src = dir.join("video_a.mp4");
    let proxy = ue_media::proxy::generate_proxy(&src, &cache, "hash-proxy-test").unwrap();
    assert!(proxy.exists());
    let (kind, info) = ue_media::probe::probe(&proxy).unwrap();
    assert_eq!(kind, MediaKind::Video);
    assert!(info.width <= ue_media::proxy::PROXY_MAX_W, "width {} ≤ 960", info.width);
    assert_eq!(info.audio_channels, 0, "the proxy has no audio");

    // resolve_top_video prefers the proxy when it exists
    let mut project = Project::new("proxy-test");
    let seq_id = project.active_sequence;
    let mut asset = import_file(&src).unwrap();
    asset.proxy = Some(proxy.to_string_lossy().into_owned());
    let asset_id = asset.id;
    project.assets.push(asset);
    let v1 = project
        .sequence(seq_id)
        .unwrap()
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Video)
        .unwrap()
        .id;
    let mut store = ProjectStore::new(project);
    store
        .insert_clip(v1, Clip::new_media(asset_id, 0, 4 * SEC, 0), InsertMode::Strict)
        .unwrap();
    let resolved = resolve_top_video(&store.project, seq_id, 1 * SEC).unwrap();
    assert_eq!(resolved.asset_path, proxy.to_string_lossy(), "preview uses the proxy");
    // if the proxy doesn't exist on disk, fall back to the original
    let seq = store.project.sequences.first().unwrap().id;
    let mut p2 = store.project.clone();
    p2.assets[0].proxy = Some("/no/existe.mp4".into());
    let r2 = resolve_top_video(&p2, seq, 1 * SEC).unwrap();
    assert_eq!(r2.asset_path, src.to_string_lossy(), "broken proxy → original");
}

/// afftdn drops the noise floor: a noise-only region gets much quieter while
/// the voice-band tone survives.
#[test]
fn denoise_wav_lowers_noise_floor() {
    let Some(dir) = demo_dir() else { return };
    let noisy = dir.join("noisy_v2.wav");
    if !noisy.exists() {
        // 2 s: 300 Hz tone + white noise, then 1 s of noise only
        let st = std::process::Command::new(ue_media::ffmpeg_bin())
            .args([
                "-y", "-v", "error",
                "-f", "lavfi", "-i", "sine=frequency=300:duration=3",
                "-f", "lavfi", "-i", "anoisesrc=colour=white:amplitude=0.1:duration=3",
                "-filter_complex",
                "[0:a]volume='if(lt(t,2),1,0)':eval=frame[s];[s][1:a]amix=inputs=2:normalize=0,aformat=channel_layouts=stereo",
                "-ar", "48000", "-c:a", "pcm_s16le",
            ])
            .arg(&noisy)
            .status()
            .unwrap();
        assert!(st.success());
    }
    let out = ue_media::denoise::denoise_wav(&noisy).unwrap();
    let rms_tail = |p: &Path| -> f64 {
        let o = std::process::Command::new(ue_media::ffmpeg_bin())
            .args(["-v", "error", "-ss", "2.3", "-i"])
            .arg(p)
            .args(["-t", "0.6", "-f", "s16le", "-ac", "1", "-"])
            .output()
            .unwrap();
        let samples: Vec<i16> = o
            .stdout
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();
        let sq: f64 = samples.iter().map(|s| (*s as f64 / 32768.0).powi(2)).sum();
        (sq / samples.len().max(1) as f64).sqrt()
    };
    let before = rms_tail(&noisy);
    let after = rms_tail(&out);
    assert!(before > 0.02, "the fixture really is noisy: {before}");
    assert!(
        after < before * 0.35,
        "noise floor drops ≥ ~9 dB: before {before}, after {after}"
    );
}
