//! Tests de integración con FFmpeg real: generan media sintética (testsrc con
//! contador quemado, tonos, png) y verifican probe, hash, import y frames.
//! Si no hay ffmpeg en PATH, los tests se saltan con aviso.

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

/// Genera la media de prueba una sola vez en target/ue-test-media.
fn demo_dir() -> Option<&'static PathBuf> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        if !ffmpeg_available() {
            eprintln!("AVISO: ffmpeg no disponible; tests de media saltados");
            return None;
        }
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-test-media");
        std::fs::create_dir_all(&dir).unwrap();
        let ff = ue_media::ffmpeg_bin();
        let runs: Vec<Vec<String>> = vec![
            // video con contador de tiempo quemado (testsrc) + tono
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
            // audio puro
            vec![
                "-y".into(), "-v".into(), "error".into(),
                "-f".into(), "lavfi".into(), "-i".into(),
                "sine=frequency=220:duration=3".into(),
                dir.join("tone.wav").to_string_lossy().into_owned(),
            ],
            // imagen
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
            assert!(st.success(), "generación de media falló: {args:?}");
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
    assert!((5.8..=6.3).contains(&dur_s), "duración ≈ 6 s, fue {dur_s}");
    assert_eq!(asset.probe.vcodec.as_deref(), Some("h264"));
    assert_eq!(asset.probe.acodec.as_deref(), Some("aac"));
    assert!(asset.content_hash.starts_with("xxh3:"));
    assert!(!asset.probe.vfr, "media sintética es CFR");
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
    std::fs::write(&bogus, "esto no es media").unwrap();
    assert!(import_file(&bogus).is_err());
}

/// Proyecto: clip en timeline t=10s usando la fuente desde 3s.
/// El mapeo timeline→fuente debe dar src_t = 3 + (t-10).
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

    // fuera del clip → None
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
        let frame = session.next_frame().unwrap().expect("frame disponible");
        assert_eq!(&frame[0..2], &[0xFF, 0xD8]);
        assert_eq!(&frame[frame.len() - 2..], &[0xFF, 0xD9]);
        frames += 1;
    }
    assert_eq!(frames, 12);
    // 12 frames a 24 fps = 0.5 s avanzados desde el inicio de la sesión
    assert_eq!(session.next_src_us(), 2_500_000);
    // el video dura 6 s: desde 2 s quedan ~4 s * 24 fps ≈ 96 frames; agotarlo
    let mut rest = 0;
    while session.next_frame().unwrap().is_some() {
        rest += 1;
        assert!(rest < 200, "no debe ser infinito");
    }
    assert!((80..=110).contains(&rest), "quedaban ≈96 frames, fueron {rest}");
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
    // clip completo (0..6s de la fuente) colocado en t=0
    let clip = Clip::new_media(asset_id, 0, 6 * SEC, 0);
    store.insert_clip(vtrack, clip, InsertMode::Strict).unwrap();

    let out_dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-test-frames");
    std::fs::create_dir_all(&out_dir).unwrap();

    for (name, t) in [("frame_0s", 0i64), ("frame_2s", 2 * SEC), ("frame_5s", 5 * SEC)] {
        let jpeg = render_frame(&store.project, seq_id, t, 640, dir, None)
            .unwrap()
            .unwrap_or_else(|| panic!("frame en {name} debe existir"));
        assert!(jpeg.len() > 1000, "jpeg razonable en {name}");
        assert_eq!(&jpeg[0..2], &[0xFF, 0xD8], "cabecera JPEG en {name}");
        std::fs::write(out_dir.join(format!("{name}.jpg")), &jpeg).unwrap();
    }
    // sin clip activo → None
    let none = render_frame(&store.project, seq_id, 30 * SEC, 640, dir, None).unwrap();
    assert!(none.is_none());
    eprintln!("frames de verificación visual en: {}", out_dir.display());
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
    // dimensiones del JPEG = count*tile_w × tile_h
    let (kind, info) = ue_media::probe::probe(&strip.path).unwrap();
    assert_eq!(kind, MediaKind::Image);
    assert_eq!(info.width, strip.tile_w * strip.count);
    assert_eq!(info.height, strip.tile_h);
    // segunda llamada: reutiliza el caché (mismo mtime)
    let m1 = std::fs::metadata(&strip.path).unwrap().modified().unwrap();
    let again =
        ue_media::thumbs::generate_thumb_strip(&src, 6_000_000, &cache, "hash-thumbs-test")
            .unwrap();
    let m2 = std::fs::metadata(&again.path).unwrap().modified().unwrap();
    assert_eq!(m1, m2, "no regenera si ya existe");
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
    assert!(info.width <= ue_media::proxy::PROXY_MAX_W, "ancho {} ≤ 960", info.width);
    assert_eq!(info.audio_channels, 0, "el proxy va sin audio");

    // resolve_top_video prefiere el proxy cuando existe
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
    assert_eq!(resolved.asset_path, proxy.to_string_lossy(), "preview usa el proxy");
    // si el proxy no existe en disco, cae al original
    let seq = store.project.sequences.first().unwrap().id;
    let mut p2 = store.project.clone();
    p2.assets[0].proxy = Some("/no/existe.mp4".into());
    let r2 = resolve_top_video(&p2, seq, 1 * SEC).unwrap();
    assert_eq!(r2.asset_path, src.to_string_lossy(), "proxy roto → original");
}
