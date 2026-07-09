//! Tests de ue-export: EDL (unitarios) y exportación real con ffmpeg
//! (integración, verificable visualmente con el contador de testsrc).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use std::sync::atomic::{AtomicBool, Ordering};

use ue_core::model::*;
use ue_core::ops::InsertMode;
use ue_core::ProjectStore;
use ue_export::edl::{build_video_edl, edl_duration, Segment};
use ue_export::{
    export_sequence, export_sequence_with_progress, ExportError, ExportSettings,
};

const SEC: i64 = 1_000_000;

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn fake_asset(kind: MediaKind, path: &str, dur_s: i64, with_audio: bool) -> MediaAsset {
    MediaAsset {
        id: Id::new(),
        kind,
        path: path.into(),
        content_hash: format!("xxh3:{path}"),
        probe: ProbeInfo {
            duration_us: dur_s * SEC,
            fps: if kind == MediaKind::Video { Some((30, 1)) } else { None },
            width: if kind == MediaKind::Video { 640 } else { 0 },
            height: if kind == MediaKind::Video { 360 } else { 0 },
            rotation: 0,
            vcodec: None,
            acodec: None,
            audio_channels: if with_audio { 2 } else { 0 },
            vfr: false,
        },
        proxy: None,
        audio_conform: None,
        peaks: None,
        thumbnails: None,
        transcript: None,
        offline: false,
    }
}

/// (store, seq, v1, v2, a1, asset_a, asset_b)
fn project_two_video_tracks() -> (ProjectStore, Id, Id, Id, Id, Id, Id) {
    let mut p = Project::new("t");
    let seq_id = p.active_sequence;
    let a = fake_asset(MediaKind::Video, "a.mp4", 10, false);
    let b = fake_asset(MediaKind::Video, "b.mp4", 10, false);
    let (aid, bid) = (a.id, b.id);
    p.assets.push(a);
    p.assets.push(b);
    // añadir V2 sobre V1
    let seq = p.sequence_mut(seq_id).unwrap();
    seq.tracks.push(Track::new(TrackKind::Video, "V2"));
    let v2 = seq.tracks.last().unwrap().id;
    let v1 = seq.tracks.iter().find(|t| t.kind == TrackKind::Video && t.name == "V1").unwrap().id;
    let a1 = seq.tracks.iter().find(|t| t.kind == TrackKind::Audio).unwrap().id;
    (ProjectStore::new(p), seq_id, v1, v2, a1, aid, bid)
}

// ---------------------------------------------------------------------------
// EDL
// ---------------------------------------------------------------------------

#[test]
fn edl_with_gap_and_two_sources() {
    let (mut store, seq, v1, _v2, _a1, a, b) = project_two_video_tracks();
    store.insert_clip(v1, Clip::new_media(a, 1 * SEC, 3 * SEC, 0), InsertMode::Strict).unwrap();
    store.insert_clip(v1, Clip::new_media(b, 4 * SEC, 6 * SEC, 2 * SEC), InsertMode::Strict).unwrap();
    // hueco [4, 5) y luego otra vez A
    store.insert_clip(v1, Clip::new_media(a, 8 * SEC, 9 * SEC, 5 * SEC), InsertMode::Strict).unwrap();

    let edl = build_video_edl(&store.project, seq).unwrap();
    assert_eq!(
        edl,
        vec![
            Segment::Source { asset_id: a, src_in: 1 * SEC, src_out: 3 * SEC, vf: None },
            Segment::Source { asset_id: b, src_in: 4 * SEC, src_out: 6 * SEC, vf: None },
            Segment::Black { duration: 1 * SEC },
            Segment::Source { asset_id: a, src_in: 8 * SEC, src_out: 9 * SEC, vf: None },
        ]
    );
    assert_eq!(edl_duration(&edl), 6 * SEC);
}

#[test]
fn edl_top_track_wins() {
    let (mut store, seq, v1, v2, _a1, a, b) = project_two_video_tracks();
    // V1: A cubre [0, 6); V2: B cubre [2, 4) → B debe ganar en el medio
    store.insert_clip(v1, Clip::new_media(a, 0, 6 * SEC, 0), InsertMode::Strict).unwrap();
    store.insert_clip(v2, Clip::new_media(b, 0, 2 * SEC, 2 * SEC), InsertMode::Strict).unwrap();

    let edl = build_video_edl(&store.project, seq).unwrap();
    assert_eq!(
        edl,
        vec![
            Segment::Source { asset_id: a, src_in: 0, src_out: 2 * SEC, vf: None },
            Segment::Source { asset_id: b, src_in: 0, src_out: 2 * SEC, vf: None },
            Segment::Source { asset_id: a, src_in: 4 * SEC, src_out: 6 * SEC, vf: None },
        ]
    );
}

#[test]
fn edl_merges_contiguous_and_trims_trailing_black() {
    let (mut store, seq, v1, _v2, _a1, a, _b) = project_two_video_tracks();
    // dos clips contiguos con fuente continua → se fusionan
    store.insert_clip(v1, Clip::new_media(a, 1 * SEC, 3 * SEC, 0), InsertMode::Strict).unwrap();
    store.insert_clip(v1, Clip::new_media(a, 3 * SEC, 5 * SEC, 2 * SEC), InsertMode::Strict).unwrap();
    let edl = build_video_edl(&store.project, seq).unwrap();
    assert_eq!(
        edl,
        vec![Segment::Source { asset_id: a, src_in: 1 * SEC, src_out: 5 * SEC, vf: None }]
    );
}

#[test]
fn edl_rejects_speed_and_empty() {
    let (mut store, seq, v1, _v2, _a1, a, _b) = project_two_video_tracks();
    assert!(matches!(
        build_video_edl(&store.project, seq),
        Err(ExportError::EmptyTimeline)
    ));
    let mut clip = Clip::new_media(a, 0, 2 * SEC, 0);
    clip.speed = 2.0;
    clip.duration = 1 * SEC;
    store.insert_clip(v1, clip, InsertMode::Strict).unwrap();
    assert!(matches!(
        build_video_edl(&store.project, seq),
        Err(ExportError::SpeedUnsupported(_))
    ));
}

#[test]
fn muted_track_is_invisible() {
    let (mut store, seq, v1, v2, _a1, a, b) = project_two_video_tracks();
    store.insert_clip(v1, Clip::new_media(a, 0, 2 * SEC, 0), InsertMode::Strict).unwrap();
    store.insert_clip(v2, Clip::new_media(b, 0, 2 * SEC, 0), InsertMode::Strict).unwrap();
    // sin mute gana V2 (B)
    let edl = build_video_edl(&store.project, seq).unwrap();
    assert!(matches!(edl[0], Segment::Source { asset_id, .. } if asset_id == b));
    // con V2 muteada gana V1 (A)
    store
        .dispatch(
            "mute",
            vec![ue_core::Action::SetTrackProp {
                track_id: v2,
                prop: ue_core::action::TrackProp::Muted(true),
            }],
        )
        .unwrap();
    let edl = build_video_edl(&store.project, seq).unwrap();
    assert!(matches!(edl[0], Segment::Source { asset_id, .. } if asset_id == a));
}

// ---------------------------------------------------------------------------
// Exportación real (ffmpeg)
// ---------------------------------------------------------------------------

fn ffmpeg_available() -> bool {
    Command::new(ue_media::ffmpeg_bin())
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn media_dir() -> Option<&'static PathBuf> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        if !ffmpeg_available() {
            eprintln!("AVISO: ffmpeg no disponible; test de export saltado");
            return None;
        }
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-export-media");
        std::fs::create_dir_all(&dir).unwrap();
        let st = Command::new(ue_media::ffmpeg_bin())
            .args([
                "-y", "-v", "error",
                "-f", "lavfi", "-i", "testsrc=duration=8:size=640x360:rate=30",
                "-f", "lavfi", "-i", "sine=frequency=440:duration=8",
                "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p",
                "-c:a", "aac", "-shortest",
            ])
            .arg(dir.join("counter.mp4"))
            .status()
            .unwrap();
        assert!(st.success());
        Some(dir)
    })
    .as_ref()
}

fn ffprobe_json(path: &Path) -> serde_json::Value {
    let out = Command::new(ue_media::ffprobe_bin())
        .args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams"])
        .arg(path)
        .output()
        .unwrap();
    serde_json::from_slice(&out.stdout).unwrap()
}

/// Edición: [1..3s) + [4..6s) del mismo archivo, pegadas. El resultado dura 4 s
/// y el contador quemado debe saltar 1,2 → 4,5. Se validan metadatos con
/// ffprobe y se extraen frames para verificación visual.
#[test]
fn export_cut_and_reordered_timeline() {
    let Some(dir) = media_dir() else { return };
    let mut project = Project::new("export-test");
    let seq_id = project.active_sequence;
    let asset = ue_media::import_file(&dir.join("counter.mp4")).unwrap();
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
    store.insert_clip(v1, Clip::new_media(asset_id, 1 * SEC, 3 * SEC, 0), InsertMode::Strict).unwrap();
    store.insert_clip(v1, Clip::new_media(asset_id, 4 * SEC, 6 * SEC, 2 * SEC), InsertMode::Strict).unwrap();

    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-export-out.mp4");
    export_sequence(&store.project, seq_id, dir, &out, &ExportSettings::default()).unwrap();

    // metadatos
    let meta = ffprobe_json(&out);
    let dur: f64 = meta["format"]["duration"].as_str().unwrap().parse().unwrap();
    assert!((3.9..=4.2).contains(&dur), "duración ≈ 4 s, fue {dur}");
    let streams = meta["streams"].as_array().unwrap();
    let v = streams.iter().find(|s| s["codec_type"] == "video").unwrap();
    assert_eq!(v["codec_name"], "h264");
    assert_eq!(v["width"], 1920); // resolución de la secuencia (con pad)
    assert!(streams.iter().any(|s| s["codec_type"] == "audio"), "lleva el audio del clip");

    // frames para verificación visual: 0.5s → contador "1"; 2.5s → contador "4"
    let frames_dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-export-frames");
    std::fs::create_dir_all(&frames_dir).unwrap();
    for (name, t) in [("salida_0.5s", 0.5f64), ("salida_2.5s", 2.5f64)] {
        let st = Command::new(ue_media::ffmpeg_bin())
            .args(["-y", "-v", "error", "-ss", &t.to_string(), "-i"])
            .arg(&out)
            .args(["-frames:v", "1"])
            .arg(frames_dir.join(format!("{name}.jpg")))
            .status()
            .unwrap();
        assert!(st.success());
    }
    eprintln!("export verificable en {} y frames en {}", out.display(), frames_dir.display());
}

// ---------------------------------------------------------------------------
// Progreso, cancelación y efectos (chroma key de punta a punta)
// ---------------------------------------------------------------------------

/// Timeline mínimo listo para exportar sobre counter.mp4.
fn simple_store(dir: &Path) -> (ProjectStore, Id) {
    let mut project = Project::new("fx-test");
    let seq_id = project.active_sequence;
    let asset = ue_media::import_file(&dir.join("counter.mp4")).unwrap();
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
    (store, seq_id)
}

#[test]
fn export_reports_monotonic_progress() {
    let Some(dir) = media_dir() else { return };
    let (store, seq_id) = simple_store(dir);
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-progress-out.mp4");
    let mut values: Vec<f32> = vec![];
    let never = AtomicBool::new(false);
    export_sequence_with_progress(
        &store.project,
        seq_id,
        dir,
        &out,
        &ExportSettings::default(),
        |p| values.push(p),
        &never,
    )
    .unwrap();
    assert!(!values.is_empty(), "hubo reportes de progreso");
    assert!(values.windows(2).all(|w| w[0] <= w[1]), "monótono: {values:?}");
    assert_eq!(*values.last().unwrap(), 1.0, "termina en 1.0");
}

#[test]
fn export_cancellation_kills_and_cleans() {
    let Some(dir) = media_dir() else { return };
    let (store, seq_id) = simple_store(dir);
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-cancel-out.mp4");
    let cancel = AtomicBool::new(true); // cancelado desde el arranque
    let result = export_sequence_with_progress(
        &store.project,
        seq_id,
        dir,
        &out,
        &ExportSettings::default(),
        |_| {},
        &cancel,
    );
    assert!(matches!(result, Err(ExportError::Cancelled)));
    assert!(!out.exists(), "el parcial se borra al cancelar");
}

/// Lee el píxel RGB (x, y) del frame en `t` segundos de un video.
fn pixel_at(video: &Path, t: f64, x: u32, y: u32) -> (u8, u8, u8) {
    let out = Command::new(ue_media::ffmpeg_bin())
        .args(["-v", "error", "-ss", &t.to_string(), "-i"])
        .arg(video)
        .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "pipe:1"])
        .output()
        .unwrap();
    assert!(out.status.success());
    // resolución de la secuencia por defecto: 1920x1080
    let w = 1920usize;
    let idx = (y as usize * w + x as usize) * 3;
    (out.stdout[idx], out.stdout[idx + 1], out.stdout[idx + 2])
}

/// Chroma key de punta a punta: fondo verde con caja roja → export con el
/// efecto → el verde desaparece (negro v0) y el rojo sobrevive.
#[test]
fn chroma_key_effect_applies_in_export() {
    let Some(dir) = media_dir() else { return };
    // fuente: fondo verde puro con caja roja centrada
    let src = dir.join("greenscreen.mp4");
    let st = Command::new(ue_media::ffmpeg_bin())
        .args([
            "-y", "-v", "error",
            "-f", "lavfi", "-i",
            "color=c=0x00FF00:s=640x360:d=2,drawbox=x=220:y=100:w=200:h=160:color=red:t=fill",
            "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p",
        ])
        .arg(&src)
        .status()
        .unwrap();
    assert!(st.success());

    let mut project = Project::new("chroma-test");
    let seq_id = project.active_sequence;
    let asset = ue_media::import_file(&src).unwrap();
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
    let mut clip = Clip::new_media(asset_id, 0, 2 * SEC, 0);
    clip.effects.push(EffectInstance {
        effect_id: "core.chroma_key".into(),
        enabled: true,
        params: [("similarity".to_string(), ue_core::keyframe::Param::Const(0.25))]
            .into_iter()
            .collect(),
        color_params: [("key_color".to_string(), "#00ff00".to_string())]
            .into_iter()
            .collect(),
    });
    store.insert_clip(v1, clip, InsertMode::Strict).unwrap();

    // la EDL lleva la cadena renderizada
    let edl = build_video_edl(&store.project, seq_id).unwrap();
    match &edl[0] {
        Segment::Source { vf: Some(vf), .. } => {
            assert!(vf.contains("chromakey=color=0x00FF00"), "cadena: {vf}");
            assert!(vf.contains("despill"));
        }
        other => panic!("se esperaba Source con vf, fue {other:?}"),
    }

    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-chroma-out.mp4");
    export_sequence(&store.project, seq_id, dir, &out, &ExportSettings::default()).unwrap();

    // El video fuente 640x360 se letterboxea a 1920x1080 (escala ×3):
    // fondo verde en (100,540)→fuente(33,180); caja roja en (960,540)→fuente(320,180).
    let (r, g, b) = pixel_at(&out, 1.0, 100, 540);
    assert!(
        r < 40 && g < 40 && b < 40,
        "el fondo verde quedó keyeado (negro v0), fue rgb({r},{g},{b})"
    );
    let (r, g, b) = pixel_at(&out, 1.0, 960, 540);
    assert!(
        r > 150 && g < 90 && b < 90,
        "la caja roja sobrevive al keying, fue rgb({r},{g},{b})"
    );

    // y sin el efecto, el fondo sigue verde (control)
    let mut store2 = {
        let mut project = Project::new("control");
        let seq_id2 = project.active_sequence;
        let asset = ue_media::import_file(&src).unwrap();
        let aid = asset.id;
        project.assets.push(asset);
        let v1 = project
            .sequence(seq_id2)
            .unwrap()
            .tracks
            .iter()
            .find(|t| t.kind == TrackKind::Video)
            .unwrap()
            .id;
        let mut s = ProjectStore::new(project);
        s.insert_clip(v1, Clip::new_media(aid, 0, 2 * SEC, 0), InsertMode::Strict).unwrap();
        (s, seq_id2)
    };
    let out2 = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-chroma-control.mp4");
    export_sequence(&store2.0.project, store2.1, dir, &out2, &ExportSettings::default()).unwrap();
    let (r, g, _b) = pixel_at(&out2, 1.0, 100, 540);
    assert!(g > 150 && r < 90, "control sin efecto: sigue verde");
    let _ = &mut store2;
}
