//! ue-export tests: EDL (unit) and real ffmpeg export
//! (integration, visually verifiable with the testsrc counter).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use std::sync::atomic::AtomicBool;

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
    // add V2 above V1
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
    // gap [4, 5) then A again
    store.insert_clip(v1, Clip::new_media(a, 8 * SEC, 9 * SEC, 5 * SEC), InsertMode::Strict).unwrap();

    let edl = build_video_edl(&store.project, seq).unwrap();
    assert_eq!(
        edl,
        vec![
            Segment::Source { asset_id: a, src_in: 1 * SEC, src_out: 3 * SEC, speed: 1.0, vf: None, transition_in: None, entrance: None, exit: None },
            Segment::Source { asset_id: b, src_in: 4 * SEC, src_out: 6 * SEC, speed: 1.0, vf: None, transition_in: None, entrance: None, exit: None },
            Segment::Black { duration: 1 * SEC },
            Segment::Source { asset_id: a, src_in: 8 * SEC, src_out: 9 * SEC, speed: 1.0, vf: None, transition_in: None, entrance: None, exit: None },
        ]
    );
    assert_eq!(edl_duration(&edl), 6 * SEC);
}

#[test]
fn edl_top_track_wins() {
    let (mut store, seq, v1, v2, _a1, a, b) = project_two_video_tracks();
    // V1: A covers [0, 6); V2: B covers [2, 4) → B must win in the middle
    store.insert_clip(v1, Clip::new_media(a, 0, 6 * SEC, 0), InsertMode::Strict).unwrap();
    store.insert_clip(v2, Clip::new_media(b, 0, 2 * SEC, 2 * SEC), InsertMode::Strict).unwrap();

    let edl = build_video_edl(&store.project, seq).unwrap();
    assert_eq!(
        edl,
        vec![
            Segment::Source { asset_id: a, src_in: 0, src_out: 2 * SEC, speed: 1.0, vf: None, transition_in: None, entrance: None, exit: None },
            Segment::Source { asset_id: b, src_in: 0, src_out: 2 * SEC, speed: 1.0, vf: None, transition_in: None, entrance: None, exit: None },
            Segment::Source { asset_id: a, src_in: 4 * SEC, src_out: 6 * SEC, speed: 1.0, vf: None, transition_in: None, entrance: None, exit: None },
        ]
    );
}

#[test]
fn edl_merges_contiguous_and_trims_trailing_black() {
    let (mut store, seq, v1, _v2, _a1, a, _b) = project_two_video_tracks();
    // two contiguous clips with continuous source → they merge
    store.insert_clip(v1, Clip::new_media(a, 1 * SEC, 3 * SEC, 0), InsertMode::Strict).unwrap();
    store.insert_clip(v1, Clip::new_media(a, 3 * SEC, 5 * SEC, 2 * SEC), InsertMode::Strict).unwrap();
    let edl = build_video_edl(&store.project, seq).unwrap();
    assert_eq!(
        edl,
        vec![Segment::Source { asset_id: a, src_in: 1 * SEC, src_out: 5 * SEC, speed: 1.0, vf: None, transition_in: None, entrance: None, exit: None }]
    );
}

#[test]
fn edl_supports_speed_and_rejects_empty() {
    let (mut store, seq, v1, _v2, _a1, a, _b) = project_two_video_tracks();
    assert!(matches!(
        build_video_edl(&store.project, seq),
        Err(ExportError::EmptyTimeline)
    ));
    // clip at 2×: uses [0..2s) of the source in 1 s of timeline
    let mut clip = Clip::new_media(a, 0, 2 * SEC, 0);
    clip.speed = 2.0;
    clip.duration = 1 * SEC;
    store.insert_clip(v1, clip, InsertMode::Strict).unwrap();
    let edl = build_video_edl(&store.project, seq).unwrap();
    match &edl[0] {
        Segment::Source { src_in, src_out, speed, .. } => {
            assert_eq!((*src_in, *src_out), (0, 2 * SEC), "consumes the whole source");
            assert_eq!(*speed, 2.0);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(edl_duration(&edl), 1 * SEC, "1 s of output at 2×");
}

#[test]
fn muted_track_is_invisible() {
    let (mut store, seq, v1, v2, _a1, a, b) = project_two_video_tracks();
    store.insert_clip(v1, Clip::new_media(a, 0, 2 * SEC, 0), InsertMode::Strict).unwrap();
    store.insert_clip(v2, Clip::new_media(b, 0, 2 * SEC, 0), InsertMode::Strict).unwrap();
    // without mute V2 (B) wins
    let edl = build_video_edl(&store.project, seq).unwrap();
    assert!(matches!(edl[0], Segment::Source { asset_id, .. } if asset_id == b));
    // with V2 muted V1 (A) wins
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
// Real export (ffmpeg)
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
            eprintln!("NOTE: ffmpeg not available; export test skipped");
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

/// Edit: [1..3s) + [4..6s) of the same file, spliced together. The result lasts 4 s
/// and the burned-in counter must jump 1,2 → 4,5. Metadata is validated with
/// ffprobe and frames are extracted for visual check.
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

    // metadata
    let meta = ffprobe_json(&out);
    let dur: f64 = meta["format"]["duration"].as_str().unwrap().parse().unwrap();
    assert!((3.9..=4.2).contains(&dur), "duration ≈ 4 s, was {dur}");
    let streams = meta["streams"].as_array().unwrap();
    let v = streams.iter().find(|s| s["codec_type"] == "video").unwrap();
    assert_eq!(v["codec_name"], "h264");
    assert_eq!(v["width"], 1920); // sequence resolution (with pad)
    assert!(streams.iter().any(|s| s["codec_type"] == "audio"), "carries the clip audio");

    // frames for visual check: 0.5s → counter "1"; 2.5s → counter "4"
    let frames_dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-export-frames");
    std::fs::create_dir_all(&frames_dir).unwrap();
    for (name, t) in [("output_0.5s", 0.5f64), ("output_2.5s", 2.5f64)] {
        let st = Command::new(ue_media::ffmpeg_bin())
            .args(["-y", "-v", "error", "-ss", &t.to_string(), "-i"])
            .arg(&out)
            .args(["-frames:v", "1"])
            .arg(frames_dir.join(format!("{name}.jpg")))
            .status()
            .unwrap();
        assert!(st.success());
    }
    eprintln!("export verifiable at {} and frames at {}", out.display(), frames_dir.display());
}

// ---------------------------------------------------------------------------
// Multi-layer: overlay of upper tracks with opacity
// ---------------------------------------------------------------------------

/// V1 red 4s + V2 blue [1s,3s) with opacity 0.5 → center: ~50/50 blend.
#[test]
fn multilayer_overlay_blends_with_opacity() {
    let Some(dir) = media_dir() else { return };
    // solid color sources
    for (name, color) in [("solid_red.mp4", "red"), ("solid_blue.mp4", "blue")] {
        let out = dir.join(name);
        if !out.exists() {
            let st = Command::new(ue_media::ffmpeg_bin())
                .args([
                    "-y", "-v", "error",
                    "-f", "lavfi", "-i",
                    &format!("color={color}:size=640x360:rate=30:duration=4"),
                    "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p",
                ])
                .arg(&out)
                .status()
                .unwrap();
            assert!(st.success());
        }
    }
    let mut project = Project::new("multilayer");
    let seq_id = project.active_sequence;
    let red = ue_media::import_file(&dir.join("solid_red.mp4")).unwrap();
    let blue = ue_media::import_file(&dir.join("solid_blue.mp4")).unwrap();
    let (red_id, blue_id) = (red.id, blue.id);
    project.assets.push(red);
    project.assets.push(blue);
    let mut store = ProjectStore::new(project);
    // add V2 above V1
    let (seq_idx_len, v2) = {
        let seq = store.project.sequence(seq_id).unwrap();
        (seq.tracks.len(), ue_core::model::Track::new(TrackKind::Video, "V2"))
    };
    let v2_id = v2.id;
    store
        .dispatch(
            "V2",
            vec![ue_core::Action::AddTrack { sequence_id: seq_id, index: seq_idx_len, track: v2 }],
        )
        .unwrap();
    let vids: Vec<Id> = store
        .project
        .sequence(seq_id)
        .unwrap()
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Video)
        .map(|t| t.id)
        .collect();
    assert_eq!(vids[1], v2_id);
    store.insert_clip(vids[0], Clip::new_media(red_id, 0, 4 * SEC, 0), InsertMode::Strict).unwrap();
    let mut top = Clip::new_media(blue_id, 0, 2 * SEC, 1 * SEC);
    top.transform.opacity = 0.5.into();
    store.insert_clip(vids[1], top, InsertMode::Strict).unwrap();

    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-multicapa.mp4");
    export_sequence(&store.project, seq_id, dir, &out, &ExportSettings::default()).unwrap();

    let meta = ffprobe_json(&out);
    let dur: f64 = meta["format"]["duration"].as_str().unwrap().parse().unwrap();
    assert!((3.9..=4.2).contains(&dur), "lasts as long as the base (4 s), was {dur}");
    let (r, _g, b) = pixel_at(&out, 0.5, 960, 540);
    assert!(r > 200 && b < 60, "0.5s: pure red, was ({r},{b})");
    let (r, _g, b) = pixel_at(&out, 2.0, 960, 540);
    assert!(
        (80..=180).contains(&(r as i32)) && (80..=180).contains(&(b as i32)),
        "2.0s: red+blue blend at 50%, was ({r},{b})"
    );
    let (r, _g, b) = pixel_at(&out, 3.5, 960, 540);
    assert!(r > 200 && b < 60, "3.5s: red again, was ({r},{b})");
}

/// Generators: gradient base + solid rectangle as a positioned layer.
#[test]
fn generators_render_solid_and_gradient() {
    let Some(dir) = media_dir() else { return };
    let mut project = Project::new("generators");
    let seq_id = project.active_sequence;
    let seq = project.sequence_mut(seq_id).unwrap();
    seq.tracks.push(Track::new(TrackKind::Video, "V2"));
    let v2 = seq.tracks.last().unwrap().id;
    let v1 = seq.tracks.iter().find(|t| t.kind == TrackKind::Video && t.name == "V1").unwrap().id;
    let mut store = ProjectStore::new(project);

    // base: gradient across the whole canvas (amber → near black, diagonal)
    let mut grad = Clip::new_generator("core.gradient", 0, 2 * SEC);
    if let ClipPayload::Generator { color_params, .. } = &mut grad.payload {
        color_params.insert("color_a".into(), "#ffb224".into());
        color_params.insert("color_b".into(), "#000000".into());
    }
    store.insert_clip(v1, grad, InsertMode::Strict).unwrap();

    // layer: green 400x300 rectangle shifted to the right
    let mut rect = Clip::new_generator("core.solid", 0, 2 * SEC);
    if let ClipPayload::Generator { params, color_params, .. } = &mut rect.payload {
        color_params.insert("color".into(), "#00cc44".into());
        params.insert("width".into(), 400.0.into());
        params.insert("height".into(), 300.0.into());
    }
    rect.transform.position.0 = 500.0.into();
    store.insert_clip(v2, rect, InsertMode::Strict).unwrap();

    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-generadores.mp4");
    export_sequence(&store.project, seq_id, dir, &out, &ExportSettings::default()).unwrap();

    let meta = ffprobe_json(&out);
    let dur: f64 = meta["format"]["duration"].as_str().unwrap().parse().unwrap();
    assert!((1.9..=2.2).contains(&dur), "≈2 s, was {dur}");
    // top-left corner: gradient amber (color_a)
    let (r, g, b) = pixel_at(&out, 1.0, 60, 60);
    assert!(r > 180 && g > 100 && b < 90, "amber corner, was ({r},{g},{b})");
    // bottom-right corner: toward black (color_b)
    let (r, g, b) = pixel_at(&out, 1.0, 1860, 1020);
    assert!(r < 80 && g < 80 && b < 80, "dark corner, was ({r},{g},{b})");
    // the green rectangle: canvas center + 500px
    let (r, g, b) = pixel_at(&out, 1.0, 1460, 540);
    assert!(g > 140 && r < 90 && b < 120, "green rectangle at +500px, was ({r},{g},{b})");
    // to the left of center there is NO rectangle (the gradient shows)
    let (_r, g2, _b) = pixel_at(&out, 1.0, 500, 540);
    assert!(g2 < 200, "no solid green on the left");
}

/// Position animated by keyframes: the red block travels from left to right.
#[test]
fn animated_position_moves_in_export() {
    use ue_core::keyframe::{Interp, Keyframe, KeyframeCurve, Param};
    let Some(dir) = media_dir() else { return };
    let src = dir.join("solid_red.mp4");
    if !src.exists() {
        let st = Command::new(ue_media::ffmpeg_bin())
            .args([
                "-y", "-v", "error", "-f", "lavfi", "-i",
                "color=red:size=640x360:rate=30:duration=4",
                "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p",
            ])
            .arg(&src)
            .status()
            .unwrap();
        assert!(st.success());
    }
    let mut project = Project::new("anim-pos");
    let seq_id = project.active_sequence;
    let asset = ue_media::import_file(&src).unwrap();
    let aid = asset.id;
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
    let mut clip = Clip::new_media(aid, 0, 4 * SEC, 0);
    clip.transform.position.0 = Param::Curve(KeyframeCurve::new(vec![
        Keyframe { t: 0, value: -600.0, interp: Interp::Linear },
        Keyframe { t: 4 * SEC, value: 600.0, interp: Interp::Linear },
    ]));
    store.insert_clip(v1, clip, InsertMode::Strict).unwrap();

    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-anim-pos.mp4");
    export_sequence(&store.project, seq_id, dir, &out, &ExportSettings::default()).unwrap();

    // early: block on the left; late: on the right
    let (r, _g, _b) = pixel_at(&out, 0.3, 300, 540);
    assert!(r > 180, "t=0.3: red on the left, was r={r}");
    let (r, _g, _b) = pixel_at(&out, 0.3, 1600, 540);
    assert!(r < 60, "t=0.3: right side empty, was r={r}");
    let (r, _g, _b) = pixel_at(&out, 3.7, 1600, 540);
    assert!(r > 180, "t=3.7: red on the right, was r={r}");
    let (r, _g, _b) = pixel_at(&out, 3.7, 300, 540);
    assert!(r < 60, "t=3.7: left side empty, was r={r}");
}

/// Animated scale: the red block grows 0.2→1.0 (eval=frame + canvas).
#[test]
fn animated_scale_grows_in_export() {
    use ue_core::keyframe::{Interp, Keyframe, KeyframeCurve, Param};
    let Some(dir) = media_dir() else { return };
    let src = dir.join("solid_red.mp4");
    assert!(src.exists(), "generated by other tests or by this one");
    let mut project = Project::new("anim-scale");
    let seq_id = project.active_sequence;
    let asset = ue_media::import_file(&src).unwrap();
    let aid = asset.id;
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
    let mut clip = Clip::new_media(aid, 0, 4 * SEC, 0);
    let curve = Param::Curve(KeyframeCurve::new(vec![
        Keyframe { t: 0, value: 0.2, interp: Interp::Linear },
        Keyframe { t: 4 * SEC, value: 1.0, interp: Interp::Linear },
    ]));
    clip.transform.scale = (curve.clone(), curve);
    store.insert_clip(v1, clip, InsertMode::Strict).unwrap();

    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-anim-scale.mp4");
    export_sequence(&store.project, seq_id, dir, &out, &ExportSettings::default()).unwrap();

    // the native red is 640x360; at scale 0.2 it does NOT reach x=700, at 1.0 it does
    let (r, _g, _b) = pixel_at(&out, 0.3, 700, 540);
    assert!(r < 60, "t=0.3: small scale, x=700 black, was r={r}");
    let (r, _g, _b) = pixel_at(&out, 0.3, 960, 540);
    assert!(r > 180, "t=0.3: the center is red, was r={r}");
    let (r, _g, _b) = pixel_at(&out, 3.8, 700, 540);
    assert!(r > 180, "t=3.8: scale 1.0, x=700 red, was r={r}");
}

/// Animated opacity 0→1 over black: the center goes from dark to red.
#[test]
fn animated_opacity_fades_in_export() {
    use ue_core::keyframe::{Interp, Keyframe, KeyframeCurve, Param};
    let Some(dir) = media_dir() else { return };
    let src = dir.join("solid_red.mp4");
    assert!(src.exists());
    let mut project = Project::new("anim-op");
    let seq_id = project.active_sequence;
    let asset = ue_media::import_file(&src).unwrap();
    let aid = asset.id;
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
    let mut clip = Clip::new_media(aid, 0, 4 * SEC, 0);
    clip.transform.opacity = Param::Curve(KeyframeCurve::new(vec![
        Keyframe { t: 0, value: 0.0, interp: Interp::Linear },
        Keyframe { t: 4 * SEC, value: 1.0, interp: Interp::Linear },
    ]));
    store.insert_clip(v1, clip, InsertMode::Strict).unwrap();

    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-anim-op.mp4");
    export_sequence(&store.project, seq_id, dir, &out, &ExportSettings::default()).unwrap();

    let (r0, _g, _b) = pixel_at(&out, 0.2, 960, 540);
    let (r1, _g, _b) = pixel_at(&out, 2.0, 960, 540);
    let (r2, _g, _b) = pixel_at(&out, 3.8, 960, 540);
    assert!(r0 < 40, "t=0.2: nearly transparent over black, was r={r0}");
    assert!((80..190).contains(&(r1 as i32)), "t=2.0: intermediate blend, was r={r1}");
    assert!(r2 > 200, "t=3.8: opaque, was r={r2}");
    assert!(r0 < r1 && r1 < r2, "monotonic: {r0} < {r1} < {r2}");
}

// ---------------------------------------------------------------------------
// Audio: pan, gain curves and loudnorm
// ---------------------------------------------------------------------------

/// Interleaved s16le PCM of the exported audio (48k stereo).
fn decode_pcm(path: &Path) -> Vec<i16> {
    let out = Command::new(ue_media::ffmpeg_bin())
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-map", "0:a", "-ac", "2", "-ar", "48000", "-f", "s16le", "-"])
        .output()
        .unwrap();
    assert!(out.status.success());
    out.stdout
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect()
}

fn rms(samples: impl Iterator<Item = i16>) -> f64 {
    let (mut sq, mut n) = (0.0f64, 0u64);
    for s in samples {
        let v = s as f64 / 32768.0;
        sq += v * v;
        n += 1;
    }
    if n == 0 { 0.0 } else { (sq / n as f64).sqrt() }
}

/// pan=1 silences the left; a 0→-40 dB curve mutes the second half;
/// loudnorm appears in the graph when requested.
#[test]
fn audio_pan_and_gain_curve_apply_in_export() {
    use ue_core::keyframe::{Interp, Keyframe, KeyframeCurve, Param};
    let Some(dir) = media_dir() else { return };
    let (mut store, seq_id) = simple_store(dir);
    let seq = store.project.sequences.iter_mut().find(|s| s.id == seq_id).unwrap();
    let clip = seq
        .tracks
        .iter_mut()
        .find(|t| t.kind == TrackKind::Video)
        .and_then(|t| t.clips.first_mut())
        .unwrap();
    clip.audio.pan = 1.0.into();
    clip.audio.gain_db = Param::Curve(KeyframeCurve::new(vec![
        Keyframe { t: 0, value: 0.0, interp: Interp::Linear },
        Keyframe { t: 2 * SEC, value: 0.0, interp: Interp::Linear },
        Keyframe { t: 2 * SEC + 1, value: -40.0, interp: Interp::Hold },
    ]));

    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-audio-pan-curve.mp4");
    export_sequence(&store.project, seq_id, dir, &out, &ExportSettings::default()).unwrap();

    let pcm = decode_pcm(&out);
    let left = rms(pcm.iter().step_by(2).copied());
    let right_first = rms(pcm.chunks_exact(2).take(48000 * 2).map(|c| c[1]));
    let right_second = rms(pcm.chunks_exact(2).skip(48000 * 2 + 24000).map(|c| c[1]));
    assert!(left < 0.005, "pan=1 must silence the left, RMS was {left}");
    // the lavfi sine is low amplitude: it just needs to be clearly audible
    assert!(right_first > 0.03, "right plays in the first half, RMS was {right_first}");
    assert!(
        right_second < right_first * 0.05,
        "the curve mutes the second half: {right_second} vs {right_first}"
    );
}

/// range=[1s,3s) trims the already-composed master: lasts ≈2 s.
#[test]
fn range_export_trims_master() {
    let Some(dir) = media_dir() else { return };
    let (store, seq_id) = simple_store(dir); // 4 s clip
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-range.mp4");
    let settings = ExportSettings { range: Some((1 * SEC, 3 * SEC)), ..Default::default() };
    export_sequence(&store.project, seq_id, dir, &out, &settings).unwrap();
    let meta = ffprobe_json(&out);
    let dur: f64 = meta["format"]["duration"].as_str().unwrap().parse().unwrap();
    assert!((1.9..=2.2).contains(&dur), "2 s range, was {dur}");
    assert!(meta["streams"].as_array().unwrap().iter().any(|s| s["codec_type"] == "audio"));
}

/// M4A: no video stream, with audio, duration of the mix.
#[test]
fn audio_only_export_m4a() {
    let Some(dir) = media_dir() else { return };
    let (store, seq_id) = simple_store(dir); // 4s clip with tone
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-audio-only.m4a");
    let settings =
        ExportSettings { format: ue_export::ExportFormat::M4a, ..Default::default() };
    export_sequence(&store.project, seq_id, dir, &out, &settings).unwrap();
    let meta = ffprobe_json(&out);
    let streams = meta["streams"].as_array().unwrap();
    assert!(streams.iter().all(|s| s["codec_type"] != "video"), "no video");
    assert!(streams.iter().any(|s| s["codec_type"] == "audio"), "with audio");
    let dur: f64 = meta["format"]["duration"].as_str().unwrap().parse().unwrap();
    assert!((3.8..=4.3).contains(&dur), "≈4 s, was {dur}");
}

/// GIF: gif container, ≤480 px wide, no audio.
#[test]
fn gif_export_palette() {
    let Some(dir) = media_dir() else { return };
    let (store, seq_id) = simple_store(dir);
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-out.gif");
    let settings = ExportSettings {
        format: ue_export::ExportFormat::Gif,
        range: Some((0, 2 * SEC)), // short GIF
        ..Default::default()
    };
    export_sequence(&store.project, seq_id, dir, &out, &settings).unwrap();
    let meta = ffprobe_json(&out);
    assert_eq!(meta["format"]["format_name"], "gif");
    let v = meta["streams"].as_array().unwrap().iter().find(|s| s["codec_type"] == "video").unwrap();
    assert!(v["width"].as_u64().unwrap() <= 480);
    assert!(meta["streams"].as_array().unwrap().iter().all(|s| s["codec_type"] != "audio"));
}

#[test]
fn loudnorm_flag_appends_master_filter() {
    let Some(dir) = media_dir() else { return };
    let (store, seq_id) = simple_store(dir);
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-audio-loudnorm.mp4");
    let settings = ExportSettings { loudnorm: true, ..Default::default() };
    let plan =
        ue_export::graph::build_ffmpeg_args(&store.project, seq_id, dir, &out, &settings).unwrap();
    let fc = plan.args.iter().find(|a| a.contains("amix")).unwrap();
    assert!(fc.contains("loudnorm=I=-14"), "graph with loudnorm: {fc}");
    let settings = ExportSettings::default();
    let plan =
        ue_export::graph::build_ffmpeg_args(&store.project, seq_id, dir, &out, &settings).unwrap();
    let fc = plan.args.iter().find(|a| a.contains("amix")).unwrap();
    assert!(!fc.contains("loudnorm"), "without the flag there is no loudnorm: {fc}");
}

// ---------------------------------------------------------------------------
// Progress, cancellation and effects (end-to-end chroma key)
// ---------------------------------------------------------------------------

/// Minimal timeline ready to export over counter.mp4.
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
    assert!(!values.is_empty(), "there were progress reports");
    assert!(values.windows(2).all(|w| w[0] <= w[1]), "monotonic: {values:?}");
    assert_eq!(*values.last().unwrap(), 1.0, "ends at 1.0");
}

#[test]
fn export_cancellation_kills_and_cleans() {
    let Some(dir) = media_dir() else { return };
    let (store, seq_id) = simple_store(dir);
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-cancel-out.mp4");
    let cancel = AtomicBool::new(true); // cancelled from the start
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
    assert!(!out.exists(), "the partial file is deleted on cancel");
}

/// Reads the RGB pixel (x, y) of the frame at `t` seconds from a 1920-wide video.
fn pixel_at(video: &Path, t: f64, x: u32, y: u32) -> (u8, u8, u8) {
    pixel_at_w(video, t, x, y, 1920)
}

/// Same as pixel_at but for arbitrary widths.
fn pixel_at_w(video: &Path, t: f64, x: u32, y: u32, w: usize) -> (u8, u8, u8) {
    let out = Command::new(ue_media::ffmpeg_bin())
        .args(["-v", "error", "-ss", &t.to_string(), "-i"])
        .arg(video)
        .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "pipe:1"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let idx = (y as usize * w + x as usize) * 3;
    (out.stdout[idx], out.stdout[idx + 1], out.stdout[idx + 2])
}

/// End-to-end chroma key: green background with a red box → export with the
/// effect → the green disappears (black v0) and the red survives.
#[test]
fn chroma_key_effect_applies_in_export() {
    let Some(dir) = media_dir() else { return };
    // source: pure green background with a centered red box
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

    // the EDL carries the rendered chain
    let edl = build_video_edl(&store.project, seq_id).unwrap();
    match &edl[0] {
        Segment::Source { vf: Some(vf), .. } => {
            assert!(vf.contains("chromakey=color=0x00FF00"), "chain: {vf}");
            assert!(vf.contains("despill"));
        }
        other => panic!("expected Source with vf, was {other:?}"),
    }

    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-chroma-out.mp4");
    export_sequence(&store.project, seq_id, dir, &out, &ExportSettings::default()).unwrap();

    // The 640x360 source video is letterboxed to 1920x1080 (scale ×3):
    // green background at (100,540)→source(33,180); red box at (960,540)→source(320,180).
    let (r, g, b) = pixel_at(&out, 1.0, 100, 540);
    assert!(
        r < 40 && g < 40 && b < 40,
        "the green background got keyed out (black v0), was rgb({r},{g},{b})"
    );
    let (r, g, b) = pixel_at(&out, 1.0, 960, 540);
    assert!(
        r > 150 && g < 90 && b < 90,
        "the red box survives the keying, was rgb({r},{g},{b})"
    );

    // and without the effect, the background stays green (control)
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
    assert!(g > 150 && r < 90, "control without effect: still green");
    let _ = &mut store2;
}

// ---------------------------------------------------------------------------
// Transitions (crossfade)
// ---------------------------------------------------------------------------

#[test]
fn transition_extends_handles_and_survives_edl() {
    let (mut store, seq, v1, _v2, _a1, a, b) = project_two_video_tracks();
    // A uses [1..3) of the file (there is material after); B uses [4..6) (there is before)
    store.insert_clip(v1, Clip::new_media(a, 1 * SEC, 3 * SEC, 0), InsertMode::Strict).unwrap();
    let clip_b = Clip::new_media(b, 4 * SEC, 6 * SEC, 2 * SEC);
    let b_id = clip_b.id;
    store.insert_clip(v1, clip_b, InsertMode::Strict).unwrap();
    store
        .dispatch(
            "transition",
            vec![ue_core::Action::SetClipTransition {
                clip_id: b_id,
                transition: Some(TransitionRef {
                    effect_id: "core.crossfade".into(),
                    duration: 1 * SEC,
                    params: Default::default() }),
                out: false,
            }],
        )
        .unwrap();

    let edl = build_video_edl(&store.project, seq).unwrap();
    assert_eq!(edl.len(), 2);
    // handles: A extended +0.5s, B pulled forward -0.5s, effective transition 1s
    match (&edl[0], &edl[1]) {
        (
            Segment::Source { src_in: a_in, src_out: a_out, .. },
            Segment::Source { src_in: b_in, src_out: b_out, transition_in, .. },
        ) => {
            assert_eq!((*a_in, *a_out), (1 * SEC, 3 * SEC + 500_000));
            assert_eq!((*b_in, *b_out), (4 * SEC - 500_000, 6 * SEC));
            assert_eq!(*transition_in, Some((1 * SEC, "core.crossfade".to_string())));
        }
        other => panic!("unexpected EDL: {other:?}"),
    }
    // the output duration does not change: 4 s
    assert_eq!(edl_duration(&edl), 4 * SEC);

    // not enough material (clip A pinned to the end of the file) → it shrinks
    let (mut store2, seq2, v1b, _v, _a, a2, b2) = project_two_video_tracks();
    store2.insert_clip(v1b, Clip::new_media(a2, 8 * SEC, 10 * SEC, 0), InsertMode::Strict).unwrap();
    let cb = Clip::new_media(b2, 4 * SEC, 6 * SEC, 2 * SEC);
    let cb_id = cb.id;
    store2.insert_clip(v1b, cb, InsertMode::Strict).unwrap();
    store2
        .dispatch(
            "transition",
            vec![ue_core::Action::SetClipTransition {
                clip_id: cb_id,
                transition: Some(TransitionRef {
                    effect_id: "core.crossfade".into(),
                    duration: 1 * SEC,
                    params: Default::default() }),
                out: false,
            }],
        )
        .unwrap();
    let edl2 = build_video_edl(&store2.project, seq2).unwrap();
    match &edl2[1] {
        Segment::Source { transition_in, .. } => {
            assert_eq!(*transition_in, None, "no handle on the left → no transition");
        }
        other => panic!("{other:?}"),
    }
}

/// Red→blue with a 1 s crossfade: the total duration is preserved and the
/// midpoint of the transition is a blend of both.
#[test]
fn crossfade_export_blends_and_keeps_duration() {
    let Some(dir) = media_dir() else { return };
    for (name, color) in [("red.mp4", "red"), ("blue.mp4", "blue")] {
        let st = Command::new(ue_media::ffmpeg_bin())
            .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
            .arg(format!("color=c={color}:s=640x360:d=4:r=30"))
            .args(["-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p"])
            .arg(dir.join(name))
            .status()
            .unwrap();
        assert!(st.success());
    }
    let mut project = Project::new("xfade-test");
    let seq_id = project.active_sequence;
    let red = ue_media::import_file(&dir.join("red.mp4")).unwrap();
    let blue = ue_media::import_file(&dir.join("blue.mp4")).unwrap();
    let (rid, bid) = (red.id, blue.id);
    project.assets.push(red);
    project.assets.push(blue);
    let v1 = project
        .sequence(seq_id)
        .unwrap()
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Video)
        .unwrap()
        .id;
    let mut store = ProjectStore::new(project);
    // red [0.5..3.5) at t=0; blue [0.5..3.5) at t=3 → there are handles on both sides
    store.insert_clip(v1, Clip::new_media(rid, 500_000, 3_500_000, 0), InsertMode::Strict).unwrap();
    let cb = Clip::new_media(bid, 500_000, 3_500_000, 3 * SEC);
    let cb_id = cb.id;
    store.insert_clip(v1, cb, InsertMode::Strict).unwrap();
    store
        .dispatch(
            "transition",
            vec![ue_core::Action::SetClipTransition {
                clip_id: cb_id,
                transition: Some(TransitionRef {
                    effect_id: "core.crossfade".into(),
                    duration: 1 * SEC,
                    params: Default::default() }),
                out: false,
            }],
        )
        .unwrap();

    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-xfade-out.mp4");
    export_sequence(&store.project, seq_id, dir, &out, &ExportSettings::default()).unwrap();

    let meta = ffprobe_json(&out);
    let dur: f64 = meta["format"]["duration"].as_str().unwrap().parse().unwrap();
    assert!((5.9..=6.2).contains(&dur), "exactly 6 s despite the crossfade, was {dur}");

    // t=1.5: pure red; t=4.5: pure blue; t=3.0 (center of the transition): blend
    let (r, _g, b) = pixel_at(&out, 1.5, 960, 540);
    assert!(r > 180 && b < 60, "pure red, was r={r} b={b}");
    let (r, _g, b) = pixel_at(&out, 4.5, 960, 540);
    assert!(b > 180 && r < 60, "pure blue, was r={r} b={b}");
    let (r, _g, b) = pixel_at(&out, 3.0, 960, 540);
    assert!(
        (50..=200).contains(&(r as i32)) && (50..=200).contains(&(b as i32)),
        "blend at the center of the fade, was r={r} b={b}"
    );
}

// ---------------------------------------------------------------------------
// Titles burned into the export (drawtext)
// ---------------------------------------------------------------------------

#[test]
fn text_clips_burn_into_export() {
    let Some(dir) = media_dir() else { return };
    // base: 3 s black video
    let src = dir.join("black.mp4");
    let st = Command::new(ue_media::ffmpeg_bin())
        .args(["-y", "-v", "error", "-f", "lavfi", "-i", "color=c=black:s=640x360:d=3:r=30"])
        .args(["-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p"])
        .arg(&src)
        .status()
        .unwrap();
    assert!(st.success());

    let mut project = Project::new("text-test");
    let seq_id = project.active_sequence;
    let asset = ue_media::import_file(&src).unwrap();
    let aid = asset.id;
    project.assets.push(asset);
    // V2 above for the title
    let seq = project.sequence_mut(seq_id).unwrap();
    seq.tracks.push(Track::new(TrackKind::Video, "V2"));
    let v2 = seq.tracks.last().unwrap().id;
    let v1 = seq
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Video && t.name == "V1")
        .unwrap()
        .id;
    let mut store = ProjectStore::new(project);
    store.insert_clip(v1, Clip::new_media(aid, 0, 3 * SEC, 0), InsertMode::Strict).unwrap();
    // centered title, white, size 120 (large for sampling), 1..3 s
    let mut title = Clip::new_text("HELLO WORLD", 1 * SEC, 2 * SEC);
    if let ClipPayload::Text { style, .. } = &mut title.payload {
        style.size = 120.0;
    }
    store.insert_clip(v2, title, InsertMode::Strict).unwrap();

    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-text-out.mp4");
    export_sequence(&store.project, seq_id, dir, &out, &ExportSettings::default()).unwrap();

    // sample a central band at t=2 (title active) looking for bright pixels
    let bright_at = |t: f64| -> usize {
        let mut bright = 0;
        for x in (600..1300).step_by(20) {
            let (r, g, b) = pixel_at(&out, t, x, 540);
            if r as u32 + g as u32 + b as u32 > 380 {
                bright += 1;
            }
        }
        bright
    };
    assert!(bright_at(2.0) >= 3, "the title is visible at t=2 ({} bright samples)", bright_at(2.0));
    assert_eq!(bright_at(0.5), 0, "before the title everything is black");
}

/// Auto subtitles: a TranscriptDoc with two phrases → Subtitles clip
/// over black video → each phrase appears in its range (bottom band) and not outside.
#[test]
fn auto_subtitles_burn_per_segment() {
    let Some(dir) = media_dir() else { return };
    let src = dir.join("black_subs.mp4"); // own file: avoids races with other tests
    let st = Command::new(ue_media::ffmpeg_bin())
        .args(["-y", "-v", "error", "-f", "lavfi", "-i", "color=c=black:s=640x360:d=3:r=30"])
        .args(["-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p"])
        .arg(&src)
        .status()
        .unwrap();
    assert!(st.success());

    let mut project = Project::new("subs-test");
    let seq_id = project.active_sequence;
    let asset = ue_media::import_file(&src).unwrap();
    let aid = asset.id;
    project.assets.push(asset);
    // synthetic transcript: "first phrase" [0.2..1.2s), "second" [1.8..2.6s)
    let doc = TranscriptDoc {
        id: Id::new(),
        asset_id: aid,
        language: "es".into(),
        model: "test".into(),
        words: vec![],
        segments: vec![
            ue_core::model::Segment {
                text: "first phrase".into(),
                start_us: 200_000,
                end_us: 1_200_000,
                word_range: (0, 0),
                emotion: None,
                volume_rms: 0.0,
            },
            ue_core::model::Segment {
                text: "second".into(),
                start_us: 1_800_000,
                end_us: 2_600_000,
                word_range: (0, 0),
                emotion: None,
                volume_rms: 0.0,
            },
        ],
        global_avg_volume: 0.0,
    };
    let doc_id = doc.id;
    project.transcripts.push(doc);
    let seq = project.sequence_mut(seq_id).unwrap();
    seq.tracks.push(Track::new(TrackKind::Video, "V2"));
    let v2 = seq.tracks.last().unwrap().id;
    let v1 = seq
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Video && t.name == "V1")
        .unwrap()
        .id;
    let mut store = ProjectStore::new(project);
    store.insert_clip(v1, Clip::new_media(aid, 0, 3 * SEC, 0), InsertMode::Strict).unwrap();
    let mut style = TextStyle::default();
    style.size = 90.0;
    style.y_offset = 380.0;
    let subs = Clip {
        id: Id::new(),
        payload: ClipPayload::Subtitles {
            transcript_id: doc_id,
            style,
            mode: SubtitleMode::Phrase,
            max_words: None,
        },
        start: 0,
        duration: 3 * SEC,
        speed: 1.0,
        effects: vec![],
        transform: Default::default(),
        audio: Default::default(),
        transition_in: None,
        transition_out: None,
        label_color: None,
        name: None,
        group: None,
    };
    store.insert_clip(v2, subs, InsertMode::Strict).unwrap();

    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-subs-out.mp4");
    export_sequence(&store.project, seq_id, dir, &out, &ExportSettings::default()).unwrap();

    // subtitle band: y = 540 + 380 = 920 (center of the text)
    let bright_at = |t: f64| -> usize {
        let mut n = 0;
        for x in (500..1400).step_by(25) {
            let (r, g, b) = pixel_at(&out, t, x, 920);
            if r as u32 + g as u32 + b as u32 > 380 {
                n += 1;
            }
        }
        n
    };
    assert!(bright_at(0.7) >= 3, "first phrase visible at t=0.7 ({})", bright_at(0.7));
    assert_eq!(bright_at(1.5), 0, "gap between phrases with no text");
    assert!(bright_at(2.2) >= 2, "second phrase visible at t=2.2 ({})", bright_at(2.2));
}

// ---------------------------------------------------------------------------
// Modo vertical (core.vertical_fill)
// ---------------------------------------------------------------------------

/// The vertical sequence exports at 1080x1920 and the blurred background fills the
/// top part (no black letterbox), while the center is the video.
#[test]
fn vertical_fill_export_has_blurred_background() {
    let Some(dir) = media_dir() else { return };
    let mut project = Project::new("vertical-test");
    let seq_id = project.active_sequence;
    let asset = ue_media::import_file(&dir.join("counter.mp4")).unwrap();
    let aid = asset.id;
    project.assets.push(asset);
    // vertical sequence with the effect on the clip (what the wizard produces)
    let seq = project.sequence_mut(seq_id).unwrap();
    seq.resolution = (1080, 1920);
    let v1 = seq
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Video)
        .unwrap()
        .id;
    let mut store = ProjectStore::new(project);
    let mut clip = Clip::new_media(aid, 0, 3 * SEC, 0);
    clip.effects.push(EffectInstance {
        effect_id: "core.vertical_fill".into(),
        enabled: true,
        params: Default::default(),
        color_params: Default::default(),
    });
    store.insert_clip(v1, clip, InsertMode::Strict).unwrap();

    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-vertical-out.mp4");
    export_sequence(&store.project, seq_id, dir, &out, &ExportSettings::default()).unwrap();

    let meta = ffprobe_json(&out);
    let v = meta["streams"].as_array().unwrap().iter().find(|s| s["codec_type"] == "video").unwrap();
    assert_eq!((v["width"].as_i64(), v["height"].as_i64()), (Some(1080), Some(1920)));

    // top band (y=200): blurred background ⇒ NOT black
    let mut top_bright = 0;
    for x in (100..1000).step_by(80) {
        let (r, g, b) = pixel_at_w(&out, 1.0, x, 200, 1080);
        if r as u32 + g as u32 + b as u32 > 90 {
            top_bright += 1;
        }
    }
    assert!(top_bright >= 6, "the blurred background fills the top ({top_bright}/12)");
    // center (y=960): the real video (testsrc has saturated colors)
    let (r, g, b) = pixel_at_w(&out, 1.0, 540, 960, 1080);
    assert!(r as u32 + g as u32 + b as u32 > 120, "center has content: rgb({r},{g},{b})");
}

/// Real 2× speed: the testsrc counter advances twice as fast per second of
/// output, the duration is halved and the audio carries atempo (pitch OK).
#[test]
fn speed_2x_export_halves_duration_and_doubles_counter() {
    let Some(dir) = media_dir() else { return };
    let mut project = Project::new("speed-test");
    let seq_id = project.active_sequence;
    let asset = ue_media::import_file(&dir.join("counter.mp4")).unwrap();
    let aid = asset.id;
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
    // source [0..6s) at 2× → 3 s of output
    let clip_id = store
        .insert_clip(v1, Clip::new_media(aid, 0, 6 * SEC, 0), InsertMode::Strict)
        .unwrap();
    store.set_clip_speed(clip_id, 2.0).unwrap();
    assert_eq!(store.project.clip(clip_id).unwrap().duration, 3 * SEC);

    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-speed-out.mp4");
    export_sequence(&store.project, seq_id, dir, &out, &ExportSettings::default()).unwrap();

    let meta = ffprobe_json(&out);
    let dur: f64 = meta["format"]["duration"].as_str().unwrap().parse().unwrap();
    assert!((2.9..=3.2).contains(&dur), "≈3 s of output, was {dur}");
    assert!(
        meta["streams"].as_array().unwrap().iter().any(|s| s["codec_type"] == "audio"),
        "the audio survives with atempo"
    );

    // the counter (big digit of the testsrc) at t=1 of output must be "2"
    // (source 2s), and at t=2.5 must be "5". Visual check: extract frames.
    let frames_dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-speed-frames");
    std::fs::create_dir_all(&frames_dir).unwrap();
    for (name, t) in [("output2x_1s", 1.0f64), ("output2x_2.5s", 2.5)] {
        let st = Command::new(ue_media::ffmpeg_bin())
            .args(["-y", "-v", "error", "-ss", &t.to_string(), "-i"])
            .arg(&out)
            .args(["-frames:v", "1"])
            .arg(frames_dir.join(format!("{name}.jpg")))
            .status()
            .unwrap();
        assert!(st.success());
    }
    eprintln!("speed frames at {}", frames_dir.display());
}

/// Word-by-word mode: each word appears alone at its exact moment.
#[test]
fn word_mode_subtitles_burn_per_word() {
    let Some(dir) = media_dir() else { return };
    let src = dir.join("black_words.mp4");
    let st = Command::new(ue_media::ffmpeg_bin())
        .args(["-y", "-v", "error", "-f", "lavfi", "-i", "color=c=black:s=640x360:d=3:r=30"])
        .args(["-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p"])
        .arg(&src)
        .status()
        .unwrap();
    assert!(st.success());

    let mut project = Project::new("word-subs");
    let seq_id = project.active_sequence;
    let asset = ue_media::import_file(&src).unwrap();
    let aid = asset.id;
    project.assets.push(asset);
    let doc_id = Id::new();
    project.transcripts.push(TranscriptDoc {
        id: doc_id,
        asset_id: aid,
        language: "es".into(),
        model: "t".into(),
        words: vec![
            Word { text: "ONE".into(), start_us: 300_000, end_us: 800_000, confidence: 1.0, rejected: false, display: None },
            Word { text: "TWO".into(), start_us: 1_500_000, end_us: 2_000_000, confidence: 1.0, rejected: false, display: None },
            Word { text: "IGNORED".into(), start_us: 2_300_000, end_us: 2_600_000, confidence: 1.0, rejected: true, display: None },
        ],
        segments: vec![],
        global_avg_volume: 0.0,
    });
    let seq = project.sequence_mut(seq_id).unwrap();
    seq.tracks.push(Track::new(TrackKind::Video, "V2"));
    let v2 = seq.tracks.last().unwrap().id;
    let v1 = seq.tracks.iter().find(|t| t.kind == TrackKind::Video && t.name == "V1").unwrap().id;
    let mut store = ProjectStore::new(project);
    store.insert_clip(v1, Clip::new_media(aid, 0, 3 * SEC, 0), InsertMode::Strict).unwrap();
    let subs = Clip {
        id: Id::new(),
        payload: ClipPayload::Subtitles {
            transcript_id: doc_id,
            style: TextStyle { size: 80.0, y_offset: 380.0, ..Default::default() },
            mode: SubtitleMode::Word,
            max_words: None,
        },
        start: 0,
        duration: 3 * SEC,
        speed: 1.0,
        effects: vec![],
        transform: Default::default(),
        audio: Default::default(),
        transition_in: None,
        transition_out: None,
        label_color: None,
        name: None,
        group: None,
    };
    store.insert_clip(v2, subs, InsertMode::Strict).unwrap();

    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-wordsubs-out.mp4");
    export_sequence(&store.project, seq_id, dir, &out, &ExportSettings::default()).unwrap();

    let bright_at = |t: f64| -> usize {
        let mut n = 0;
        for y in [890u32, 920, 950] {
            for x in (700..1300).step_by(20) {
                let (r, g, b) = pixel_at(&out, t, x, y);
                if r as u32 + g as u32 + b as u32 > 380 { n += 1; }
            }
        }
        n
    };
    assert!(bright_at(0.5) >= 2, "ONE visible at t=0.5 ({})", bright_at(0.5));
    assert_eq!(bright_at(1.1), 0, "clean gap between words");
    assert!(bright_at(1.7) >= 2, "TWO visible at t=1.7 ({})", bright_at(1.7));
    assert_eq!(bright_at(2.4), 0, "rejected words are not burned in");
}

/// Karaoke: the whole phrase visible and the words light up as they are spoken.
#[test]
fn karaoke_mode_highlights_words_progressively() {
    let Some(dir) = media_dir() else { return };
    let src = dir.join("black_words.mp4");
    if !src.exists() {
        let st = Command::new(ue_media::ffmpeg_bin())
            .args(["-y", "-v", "error", "-f", "lavfi", "-i", "color=c=black:s=640x360:d=3:r=30"])
            .args(["-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p"])
            .arg(&src)
            .status()
            .unwrap();
        assert!(st.success());
    }

    let mut project = Project::new("karaoke-subs");
    let seq_id = project.active_sequence;
    let asset = ue_media::import_file(&src).unwrap();
    let aid = asset.id;
    project.assets.push(asset);
    let doc_id = Id::new();
    project.transcripts.push(TranscriptDoc {
        id: doc_id,
        asset_id: aid,
        language: "es".into(),
        model: "t".into(),
        words: vec![
            Word { text: "ONE".into(), start_us: 300_000, end_us: 800_000, confidence: 1.0, rejected: false, display: None },
            Word { text: "TWO".into(), start_us: 1_500_000, end_us: 2_000_000, confidence: 1.0, rejected: false, display: None },
        ],
        segments: vec![ue_core::model::Segment {
            text: "ONE TWO".into(),
            start_us: 200_000,
            end_us: 2_600_000,
            word_range: (0, 2),
            emotion: None,
            volume_rms: 0.0,
        }],
        global_avg_volume: 0.0,
    });
    let seq = project.sequence_mut(seq_id).unwrap();
    seq.tracks.push(Track::new(TrackKind::Video, "V2"));
    let v2 = seq.tracks.last().unwrap().id;
    let v1 = seq.tracks.iter().find(|t| t.kind == TrackKind::Video && t.name == "V1").unwrap().id;
    let mut store = ProjectStore::new(project);
    store.insert_clip(v1, Clip::new_media(aid, 0, 3 * SEC, 0), InsertMode::Strict).unwrap();
    let subs = Clip {
        id: Id::new(),
        payload: ClipPayload::Subtitles {
            transcript_id: doc_id,
            style: TextStyle { size: 90.0, y_offset: 380.0, ..Default::default() },
            mode: SubtitleMode::Karaoke,
            max_words: None,
        },
        start: 0,
        duration: 3 * SEC,
        speed: 1.0,
        effects: vec![],
        transform: Default::default(),
        audio: Default::default(),
        transition_in: None,
        transition_out: None,
        label_color: None,
        name: None,
        group: None,
    };
    store.insert_clip(v2, subs, InsertMode::Strict).unwrap();

    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-karaoke-out.mp4");
    export_sequence(&store.project, seq_id, dir, &out, &ExportSettings::default()).unwrap();

    // highlight amber (#FFB224) in the subtitle band
    let accent_at = |t: f64| -> usize {
        let mut n = 0;
        for y in [880u32, 910, 940, 970] {
            for x in (600..1400).step_by(12) {
                let (r, g, b) = pixel_at(&out, t, x, y);
                if r > 200 && (100..230).contains(&(g as i32)) && b < 100 {
                    n += 1;
                }
            }
        }
        n
    };
    let early = accent_at(0.5); // ONE playing, TWO still dim
    let late = accent_at(1.7); // both lit
    assert!(early >= 1, "ONE highlighted at t=0.5 (n={early})");
    assert!(late > early, "the highlight advances: t=1.7 (n={late}) > t=0.5 (n={early})");
    // before the segment nothing is lit
    assert_eq!(accent_at(0.05), 0, "no highlight before the segment");
}

/// System font enumeration and family resolution → fontfile.
#[test]
fn system_fonts_enumerate_and_resolve() {
    let fonts = ue_export::graph::list_system_fonts();
    eprintln!("fonts found: {}", fonts.len());
    if fonts.is_empty() {
        eprintln!("NOTE: no system fonts (CI without fonts?); lax test");
        return;
    }
    assert!(fonts.len() > 5, "a desktop system has fonts");
    // resolving the first listed family must yield an existing path
    let (family, path) = &fonts[0];
    let resolved = ue_export::graph::resolve_font_family(family);
    assert!(resolved.is_some(), "family {family} resolvable");
    assert!(std::path::Path::new(path).exists(), "the path exists: {path}");
    // a nonexistent family falls to None (and drawtext will use the default)
    assert!(ue_export::graph::resolve_font_family("NoSuchFontFamily9999").is_none());
}

/// The default font "sans-serif" (and the other CSS generics) MUST resolve to
/// a real installed font, or a clip draws NOTHING silently (field bug: a
/// subtitles clip with the default font rendered empty).
#[test]
fn generic_font_families_resolve_to_real_files() {
    if ue_export::graph::list_system_fonts().is_empty() {
        eprintln!("NOTE: no system fonts; skipped");
        return;
    }
    for generic in ["sans-serif", "sans", "serif", "monospace", "mono", ""] {
        let resolved = ue_export::graph::resolve_font_family(generic);
        assert!(resolved.is_some(), "generic '{generic}' must map to a real font");
        assert!(
            std::path::Path::new(&resolved.unwrap()).exists(),
            "the mapped font file exists for '{generic}'"
        );
        assert!(ue_export::graph::font_is_available(generic), "'{generic}' reports available");
    }
    // the default TextStyle uses one of these — it must be renderable
    let default_font = ue_core::model::TextStyle::default().font;
    assert!(
        ue_export::graph::font_is_available(&default_font),
        "the DEFAULT font '{default_font}' must render, not draw empty"
    );
}

/// Continuous speech must be chunked into caption-sized phrases: one giant
/// Whisper segment can NOT become one giant drawtext (the reported bug).
#[test]
fn continuous_speech_chunks_into_multiple_captions() {
    let Some(dir) = media_dir() else { return };
    let (mut store, seq_id) = simple_store(dir); // 4s clip
    let vtrack = store
        .project
        .sequence(seq_id)
        .unwrap()
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Video)
        .unwrap();
    let aid = match &vtrack.clips[0].payload {
        ClipPayload::Media { asset_id, .. } => *asset_id,
        _ => panic!(),
    };
    // 20 words with no pauses inside ONE segment spanning the whole clip
    let words: Vec<Word> = (0..20)
        .map(|i| Word {
            text: format!("palabra{i:02}"),
            start_us: i * 200_000,
            end_us: i * 200_000 + 180_000,
            confidence: 1.0,
            rejected: false,
            display: None,
        })
        .collect();
    let doc_id = Id::new();
    store.project.transcripts.push(TranscriptDoc {
        id: doc_id,
        asset_id: aid,
        language: "es".into(),
        model: "t".into(),
        words,
        segments: vec![ue_core::model::Segment {
            text: "todo junto".into(),
            start_us: 0,
            end_us: 4 * SEC,
            word_range: (0, 20),
            emotion: None,
            volume_rms: 0.0,
        }],
        global_avg_volume: 0.0,
    });
    let seq = store.project.sequence_mut(seq_id).unwrap();
    seq.tracks.push(Track::new(TrackKind::Video, "V2"));
    let v2 = seq.tracks.last().unwrap().id;
    let subs = Clip {
        id: Id::new(),
        payload: ClipPayload::Subtitles {
            transcript_id: doc_id,
            style: TextStyle { size: 60.0, y_offset: 380.0, ..Default::default() },
            mode: SubtitleMode::Phrase,
            max_words: None,
        },
        start: 0,
        duration: 4 * SEC,
        speed: 1.0,
        effects: vec![],
        transform: Default::default(),
        audio: Default::default(),
        transition_in: None,
        transition_out: None,
        label_color: None,
        name: None,
        group: None,
    };
    store.insert_clip(v2, subs, InsertMode::Strict).unwrap();

    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-chunked-subs.mp4");
    let plan = ue_export::graph::build_ffmpeg_args(
        &store.project,
        seq_id,
        dir,
        &out,
        &ExportSettings::default(),
    )
    .unwrap();
    let fc = plan.args.iter().find(|a| a.contains("drawtext")).unwrap();
    let n = fc.matches("drawtext").count();
    assert!(n >= 3, "20 continuous words must yield several captions, got {n}");
    assert!(!fc.contains("todo junto"), "the giant segment text is not used");
}

/// denoise=true inserts the afftdn filter into the clip's audio chain.
#[test]
fn denoise_flag_inserts_afftdn() {
    let Some(dir) = media_dir() else { return };
    let (mut store, seq_id) = simple_store(dir);
    let seq = store.project.sequences.iter_mut().find(|s| s.id == seq_id).unwrap();
    let clip = seq
        .tracks
        .iter_mut()
        .find(|t| t.kind == TrackKind::Video)
        .and_then(|t| t.clips.first_mut())
        .unwrap();
    clip.audio.denoise = true;
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-denoise.mp4");
    let plan = ue_export::graph::build_ffmpeg_args(
        &store.project,
        seq_id,
        dir,
        &out,
        &ExportSettings::default(),
    )
    .unwrap();
    let fc = plan.args.iter().find(|a| a.contains("amix")).unwrap();
    assert!(fc.contains("afftdn"), "denoise filter present: {fc}");
}

/// With the denoised conform rendered (DNS64 output), the export reads audio
/// FROM that wav (exact live parity) and skips the inline afftdn fallback.
#[test]
fn denoised_wav_becomes_the_audio_input() {
    let Some(dir) = media_dir() else { return };
    let (mut store, seq_id) = simple_store(dir);
    // fake conform + denoised sibling
    let conform = Path::new(env!("CARGO_TARGET_TMPDIR")).join("fake-conform.wav");
    let denoised = ue_media::denoise::denoised_path(&conform);
    let st = Command::new(ue_media::ffmpeg_bin())
        .args(["-y", "-v", "error", "-f", "lavfi", "-i", "sine=frequency=200:duration=4",
               "-ar", "48000", "-ac", "2", "-c:a", "pcm_s16le"])
        .arg(&denoised)
        .status()
        .unwrap();
    assert!(st.success());
    let aid = {
        let seq = store.project.sequences.iter_mut().find(|s| s.id == seq_id).unwrap();
        let clip = seq
            .tracks
            .iter_mut()
            .find(|t| t.kind == TrackKind::Video)
            .and_then(|t| t.clips.first_mut())
            .unwrap();
        clip.audio.denoise = true;
        match &clip.payload {
            ClipPayload::Media { asset_id, .. } => *asset_id,
            _ => panic!(),
        }
    };
    store
        .project
        .assets
        .iter_mut()
        .find(|a| a.id == aid)
        .unwrap()
        .audio_conform = Some(conform.to_string_lossy().into_owned());

    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-denoise-input.mp4");
    let plan = ue_export::graph::build_ffmpeg_args(
        &store.project,
        seq_id,
        dir,
        &out,
        &ExportSettings::default(),
    )
    .unwrap();
    let args_s = plan.args.join(" ");
    assert!(
        args_s.contains("denoise.wav"),
        "the denoised wav is an ffmpeg input: {args_s}"
    );
    let fc = plan.args.iter().find(|a| a.contains("amix")).unwrap();
    assert!(!fc.contains("afftdn"), "no inline fallback when the wav exists: {fc}");
}

/// Word corrections ("godo" → "godot") show up in the caption drawtexts.
#[test]
fn corrected_words_appear_in_captions() {
    let Some(dir) = media_dir() else { return };
    let (mut store, seq_id) = simple_store(dir);
    let vtrack = store
        .project
        .sequence(seq_id)
        .unwrap()
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Video)
        .unwrap();
    let aid = match &vtrack.clips[0].payload {
        ClipPayload::Media { asset_id, .. } => *asset_id,
        _ => panic!(),
    };
    let doc_id = Id::new();
    store.project.transcripts.push(TranscriptDoc {
        id: doc_id,
        asset_id: aid,
        language: "es".into(),
        model: "t".into(),
        words: vec![
            Word { text: "uso".into(), start_us: 100_000, end_us: 400_000, confidence: 1.0, rejected: false, display: None },
            Word { text: "godo".into(), start_us: 450_000, end_us: 800_000, confidence: 1.0, rejected: false, display: None },
        ],
        segments: vec![],
        global_avg_volume: 0.0,
    });
    // correct via the action (undoable), like the command does
    store
        .dispatch(
            "Correct word",
            vec![ue_core::Action::SetWordText {
                transcript_id: doc_id,
                index: 1,
                display: Some("godot".into()),
            }],
        )
        .unwrap();
    let seq = store.project.sequence_mut(seq_id).unwrap();
    seq.tracks.push(Track::new(TrackKind::Video, "V2"));
    let v2 = seq.tracks.last().unwrap().id;
    let subs = Clip {
        id: Id::new(),
        payload: ClipPayload::Subtitles {
            transcript_id: doc_id,
            style: TextStyle::default(),
            mode: SubtitleMode::Phrase,
            max_words: None,
        },
        start: 0,
        duration: 2 * SEC,
        speed: 1.0,
        effects: vec![],
        transform: Default::default(),
        audio: Default::default(),
        transition_in: None,
        transition_out: None,
        label_color: None,
        name: None,
        group: None,
    };
    store.insert_clip(v2, subs, InsertMode::Strict).unwrap();
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-corrected.mp4");
    let plan = ue_export::graph::build_ffmpeg_args(
        &store.project,
        seq_id,
        dir,
        &out,
        &ExportSettings::default(),
    )
    .unwrap();
    let fc = plan.args.iter().find(|a| a.contains("drawtext")).unwrap();
    assert!(fc.contains("uso godot"), "corrected label burned: {fc}");
    // undo restores the original (first undo = the subs clip insert)
    store.undo().unwrap();
    store.undo().unwrap();
    assert_eq!(store.project.transcripts[0].words[1].label(), "godo");
}

/// THE foundational compositing rule (field bug dragged since the start):
/// a transform must NEVER change the apparent size of a clip. Frames are
/// fitted to the sequence canvas before compositing, so a small source (or
/// the preview's half-size proxy) renders identically with and without a
/// position offset.
#[test]
fn position_offset_does_not_change_apparent_size() {
    let Some(dir) = media_dir() else { return };
    let src = dir.join("solid_red.mp4"); // 640x360 in a 1920x1080 sequence
    assert!(src.exists());
    let mut project = Project::new("fit-canvas");
    let seq_id = project.active_sequence;
    let asset = ue_media::import_file(&src).unwrap();
    let aid = asset.id;
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
    let mut clip = Clip::new_media(aid, 0, 2 * SEC, 0);
    clip.transform.position.0 = 110.0.into();
    store.insert_clip(v1, clip, InsertMode::Strict).unwrap();

    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-fit-canvas.mp4");
    export_sequence(&store.project, seq_id, dir, &out, &ExportSettings::default()).unwrap();

    // fitted to canvas then shifted: red spans [110, 1920), black strip on the left
    let (r, _g, _b) = pixel_at(&out, 1.0, 50, 540);
    assert!(r < 60, "left strip is background, got r={r}");
    let (r, _g, _b) = pixel_at(&out, 1.0, 300, 540);
    assert!(r > 180, "content starts right after the offset, got r={r}");
    let (r, _g, _b) = pixel_at(&out, 1.0, 1800, 540);
    assert!(
        r > 180,
        "content still fills the canvas far right: the clip must NOT shrink \
         to its native size when positioned (got r={r})"
    );
    let (r, _g, _b) = pixel_at(&out, 1.0, 960, 100);
    assert!(r > 180, "vertical extent fills the canvas too, got r={r}");
}

/// Paused preview at a FRACTIONAL seek time with a position transform must
/// not be black. Field bug: -ss lands between keyframes so the fg PTS is
/// large while the canvas `color` source starts at 0 → overlay emitted only
/// the background. clip_vf_sampled rebases the fg PTS (single-frame path).
#[test]
fn paused_frame_with_position_is_not_black_at_fractional_seeks() {
    let Some(dir) = media_dir() else { return };
    let src = dir.join("gop_video.mp4"); // short GOP, like the app's proxies
    if !src.exists() {
        let st = Command::new(ue_media::ffmpeg_bin())
            .args(["-y", "-v", "error", "-f", "lavfi", "-i",
                   "testsrc=duration=25:size=640x360:rate=30"])
            .args(["-c:v", "libx264", "-preset", "ultrafast", "-g", "12", "-pix_fmt", "yuv420p"])
            .arg(&src)
            .status()
            .unwrap();
        assert!(st.success());
    }

    let mut project = Project::new("paused-black");
    let seq_id = project.active_sequence;
    let asset = ue_media::import_file(&src).unwrap();
    let aid = asset.id;
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
    let mut clip = Clip::new_media(aid, 0, 20 * SEC, 0);
    clip.transform.position.0 = 454.0.into(); // his exact value
    store.insert_clip(v1, clip, InsertMode::Strict).unwrap();
    let dur = store.project.sequence(seq_id).unwrap().tracks
        .iter().find(|t| t.id == v1).unwrap().clips[0].duration;
    assert_eq!(dur, 20 * SEC, "clip inserted with the expected duration");

    let reg = ue_render::core_registry();
    // the exact times his app rendered black
    for t_us in [12_400_000i64, 15_154_000, 19_115_000] {
        let r = ue_media::frame::resolve_top_video(&store.project, seq_id, t_us).unwrap();
        let vf = ue_render::clip_vf_sampled(
            &reg,
            &r.effects,
            &r.transform,
            Some((1920, 1080)),
            r.clip_rel_us,
        );
        let bytes =
            ue_media::frame::render_frame(&store.project, seq_id, t_us, 1280, dir, vf.as_deref())
                .unwrap()
                .expect("a frame");
        assert!(
            bytes.len() > 20_000,
            "paused frame at {:.3}s is essentially black ({} bytes)",
            t_us as f64 / 1e6,
            bytes.len()
        );
    }
}

/// Multi-range export: several pieces of the finished master, concatenated
/// in order, with audio. Duration = sum of the pieces, and the content of
/// each piece matches the corresponding master time (burned counter check).
#[test]
fn multi_range_export_concatenates_pieces() {
    let Some(dir) = media_dir() else { return };
    let (store, seq_id) = simple_store(dir); // counter.mp4, 4 s, with audio
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-multi-range.mp4");
    let settings = ExportSettings {
        ranges: vec![(0, 1 * SEC), (2 * SEC, 3 * SEC), (3 * SEC, 4 * SEC)],
        ..Default::default()
    };
    export_sequence(&store.project, seq_id, dir, &out, &settings).unwrap();

    let meta = ffprobe_json(&out);
    let dur: f64 = meta["format"]["duration"].as_str().unwrap().parse().unwrap();
    assert!((2.9..=3.2).contains(&dur), "3 pieces of 1 s each, got {dur}");
    let streams = meta["streams"].as_array().unwrap();
    assert!(streams.iter().any(|s| s["codec_type"] == "audio"), "audio survived the concat");
    assert!(streams.iter().any(|s| s["codec_type"] == "video"));

    // the second output second must show master t≈2s, not t≈1s
    let frames_dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-multi-range-frames");
    std::fs::create_dir_all(&frames_dir).unwrap();
    for (name, t) in [("piece1_0.5s", 0.5f64), ("piece2_1.5s", 1.5), ("piece3_2.5s", 2.5)] {
        let st = Command::new(ue_media::ffmpeg_bin())
            .args(["-y", "-v", "error", "-ss", &t.to_string(), "-i"])
            .arg(&out)
            .args(["-frames:v", "1"])
            .arg(frames_dir.join(format!("{name}.jpg")))
            .status()
            .unwrap();
        assert!(st.success());
    }
    eprintln!("multi-range frames in {}", frames_dir.display());
}

/// A single entry in `ranges` behaves exactly like the `range` shorthand.
#[test]
fn single_range_in_ranges_matches_shorthand() {
    let Some(dir) = media_dir() else { return };
    let (store, seq_id) = simple_store(dir);
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-one-of-ranges.mp4");
    let settings = ExportSettings { ranges: vec![(1 * SEC, 3 * SEC)], ..Default::default() };
    export_sequence(&store.project, seq_id, dir, &out, &settings).unwrap();
    let meta = ffprobe_json(&out);
    let dur: f64 = meta["format"]["duration"].as_str().unwrap().parse().unwrap();
    assert!((1.9..=2.2).contains(&dur), "2 s piece, got {dur}");
}

// ---------------------------------------------------------------------------
// Avatar generation (standalone media asset)
// ---------------------------------------------------------------------------

/// Spans cover the whole duration: gaps are filled with the default
/// expression and unknown emotions fall back to it.
#[test]
fn avatar_spans_cover_the_timeline() {
    use ue_core::model::{AvatarConfig, AvatarExpression};
    let mut cfg = AvatarConfig::new("me");
    cfg.expressions = vec![
        AvatarExpression { name: "calm".into(), path: "c.png".into() },
        AvatarExpression { name: "angry".into(), path: "a.png".into() },
    ];
    let doc = TranscriptDoc {
        id: Id::new(),
        asset_id: Id::new(),
        language: "es".into(),
        model: "t".into(),
        words: vec![],
        segments: vec![
            ue_core::model::Segment {
                text: "one".into(),
                start_us: 1 * SEC,
                end_us: 2 * SEC,
                word_range: (0, 0),
                emotion: Some("angry".into()),
                volume_rms: 2.0,
            },
            ue_core::model::Segment {
                text: "two".into(),
                start_us: 3 * SEC,
                end_us: 4 * SEC,
                word_range: (0, 0),
                emotion: Some("unknown-emotion".into()),
                volume_rms: 1.0,
            },
        ],
        global_avg_volume: 1.0,
    };
    let spans = ue_export::avatar_gen::plan_spans(&doc, &cfg, 5 * SEC);
    assert_eq!(spans.len(), 5, "gap-filled: [0,1) [1,2) [2,3) [3,4) [4,5)");
    assert_eq!(spans[0].expression, "calm");
    assert_eq!(spans[1].expression, "angry");
    assert!((spans[1].volume_ratio - 2.0).abs() < 1e-9, "loud segment shakes more");
    assert_eq!(spans[2].expression, "calm", "gap uses the default");
    assert_eq!(spans[3].expression, "calm", "unknown emotion falls back");
    assert_eq!(spans.last().unwrap().to_us, 5 * SEC, "covers to the end");
}

/// End-to-end: an avatar video built from a PNG and an MP4 expression, with a
/// transparent background, correct duration, and the right expression visible
/// in each span.
#[test]
fn avatar_video_generates_from_images_and_videos() {
    let Some(dir) = media_dir() else { return };
    use ue_core::model::{AvatarConfig, AvatarExpression};
    // calm = solid green PNG; angry = solid red MP4
    let calm = dir.join("av_calm.png");
    let angry = dir.join("av_angry.mp4");
    if !calm.exists() {
        assert!(Command::new(ue_media::ffmpeg_bin())
            .args(["-y", "-v", "error", "-f", "lavfi", "-i", "color=c=green:s=200x200", "-frames:v", "1"])
            .arg(&calm).status().unwrap().success());
    }
    if !angry.exists() {
        assert!(Command::new(ue_media::ffmpeg_bin())
            .args(["-y", "-v", "error", "-f", "lavfi", "-i", "color=c=red:s=200x200:d=1:r=30"])
            .args(["-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p"])
            .arg(&angry).status().unwrap().success());
    }

    let mut cfg = AvatarConfig::new("me");
    cfg.shake_factor = 0.0; // deterministic pixel positions
    cfg.scale = 0.5; // 200px avatar on a 400px canvas → corners transparent
    cfg.expressions = vec![
        AvatarExpression { name: "calm".into(), path: calm.to_string_lossy().into_owned() },
        AvatarExpression { name: "angry".into(), path: angry.to_string_lossy().into_owned() },
    ];
    let doc = TranscriptDoc {
        id: Id::new(),
        asset_id: Id::new(),
        language: "es".into(),
        model: "t".into(),
        words: vec![],
        segments: vec![ue_core::model::Segment {
            text: "grr".into(),
            start_us: 1 * SEC,
            end_us: 2 * SEC,
            word_range: (0, 0),
            emotion: Some("angry".into()),
            volume_rms: 1.0,
        }],
        global_avg_volume: 1.0,
    };

    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-avatar.mov");
    let spans = ue_export::avatar_gen::generate(
        &cfg,
        &doc,
        3 * SEC,
        (400, 400),
        (30, 1),
        &out,
        |_| {},
    )
    .unwrap();
    assert_eq!(spans.len(), 3);

    let meta = ffprobe_json(&out);
    let dur: f64 = meta["format"]["duration"].as_str().unwrap().parse().unwrap();
    assert!((2.8..=3.3).contains(&dur), "3 s avatar video, got {dur}");
    let v = meta["streams"].as_array().unwrap().iter().find(|s| s["codec_type"] == "video").unwrap();
    assert_eq!(v["codec_name"], "qtrle", "alpha-capable codec");

    // RGBA probe: corner transparent, center shows the current expression
    let rgba = |t: f64, x: u32, y: u32| -> [u8; 4] {
        let o = Command::new(ue_media::ffmpeg_bin())
            .args(["-v", "error", "-ss", &t.to_string(), "-i"])
            .arg(&out)
            .args(["-frames:v", "1", "-vf", &format!("crop=1:1:{x}:{y}"), "-pix_fmt", "rgba",
                   "-f", "rawvideo", "-"])
            .output()
            .unwrap();
        [o.stdout[0], o.stdout[1], o.stdout[2], o.stdout[3]]
    };
    let corner = rgba(0.5, 5, 5);
    assert_eq!(corner[3], 0, "background is transparent (alpha=0), got {corner:?}");
    let calm_px = rgba(0.5, 200, 200);
    assert!(calm_px[1] > 90 && calm_px[0] < 90 && calm_px[3] == 255, "calm=green opaque: {calm_px:?}");
    let angry_px = rgba(1.5, 200, 200);
    assert!(angry_px[0] > 90 && angry_px[1] < 90, "angry=red: {angry_px:?}");
    let back_px = rgba(2.5, 200, 200);
    assert!(back_px[1] > 90 && back_px[0] < 90, "back to calm: {back_px:?}");
}


/// BUG 2: karaoke paints the active word in `highlight_color`. Realistic
/// density (3 words/s); exports and checks for the yellow highlight.
#[test]
fn karaoke_highlights_the_active_word() {
    let Some(dir) = media_dir() else { return };
    let base = dir.join("karaoke_hl.mp4");
    let st = Command::new(ue_media::ffmpeg_bin())
        .args(["-y","-v","error","-f","lavfi","-i","color=c=navy:s=1080x1920:d=10:r=30"])
        .args(["-c:v","libx264","-preset","ultrafast","-pix_fmt","yuv420p"]).arg(&base).status().unwrap();
    assert!(st.success());
    let mut project = Project::new("k");
    let seq_id = project.active_sequence;
    project.sequence_mut(seq_id).unwrap().resolution = (1080, 1920);
    let asset = ue_media::import_file(&base).unwrap();
    let aid = asset.id;
    project.assets.push(asset);
    // 30 words over 10s (3/s), no gaps so they chunk into a couple of phrases
    let words: Vec<Word> = (0..30).map(|i| {
        let t = i as i64 * 333_000;
        Word { text: format!("palabra{i}"), start_us: t, end_us: t + 320_000, confidence: 1.0, rejected: false, display: None }
    }).collect();
    let doc = TranscriptDoc {
        id: Id::new(), asset_id: aid, language: "es".into(), model: "t".into(), words,
        segments: vec![ue_core::model::Segment { text: "x".into(), start_us: 0, end_us: 10 * SEC, word_range: (0, 30), emotion: None, volume_rms: 0.0 }],
        global_avg_volume: 0.0,
    };
    let doc_id = doc.id;
    project.transcripts.push(doc);
    let seq = project.sequence_mut(seq_id).unwrap();
    seq.tracks.push(Track::new(TrackKind::Video, "V2"));
    let v1 = seq.tracks.iter().find(|t| t.name == "V1").unwrap().id;
    let v2 = seq.tracks.iter().find(|t| t.name == "V2").unwrap().id;
    let mut store = ProjectStore::new(project);
    store.insert_clip(v1, Clip::new_media(aid, 0, 10 * SEC, 0), InsertMode::Strict).unwrap();
    let style = TextStyle { size: 80.0, y_offset: 380.0, highlight_color: Some("#ffcc00".into()), ..Default::default() };
    store.insert_clip(v2, Clip {
        id: Id::new(),
        payload: ClipPayload::Subtitles { transcript_id: doc_id, style, mode: SubtitleMode::Karaoke, max_words: None },
        start: 0, duration: 10 * SEC, speed: 1.0, effects: vec![], transform: Default::default(),
        audio: Default::default(), transition_in: None, transition_out: None, label_color: None, name: None, group: None,
    }, InsertMode::Strict).unwrap();

    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-karaoke-hl.mp4");
    let _ = std::fs::remove_file(&out);
    export_sequence(&store.project, seq_id, dir, &out, &ExportSettings::default()).unwrap();
    assert!(out.exists());
    // at t=3 s several words have played → yellow highlight (#ffcc00) somewhere
    // in the caption band (y≈1300 at 1080p→1920)
    let mut yellow = 0;
    for y in (1500..1720).step_by(5) {
        for x in (150..950).step_by(5) {
            let (r, g, b) = pixel_at_w(&out, 3.0, x, y, 1080);
            if r > 190 && g > 140 && b < 90 { yellow += 1; }
        }
    }
    assert!(yellow >= 5, "the active word is painted in highlight_color (yellow px={yellow})");
}

/// BUG 1: a long dense karaoke that used to crash ffmpeg (thousands of
/// drawtext) now exports a RANGE fine (overlays bounded to the range), and a
/// FULL export (2000 words) works too: the ASS script lives in a file.
#[test]
fn karaoke_bounds_to_range_and_guards_the_rest() {
    let Some(dir) = media_dir() else { return };
    let base = dir.join("karaoke_dense.mp4");
    let st = Command::new(ue_media::ffmpeg_bin())
        .args(["-y","-v","error","-f","lavfi","-i","color=c=black:s=1080x1920:d=30:r=30"])
        .args(["-c:v","libx264","-preset","ultrafast","-pix_fmt","yuv420p"]).arg(&base).status().unwrap();
    assert!(st.success());
    let mut project = Project::new("k");
    let seq_id = project.active_sequence;
    project.sequence_mut(seq_id).unwrap().resolution = (1080, 1920);
    let asset = ue_media::import_file(&base).unwrap();
    let aid = asset.id;
    project.assets.push(asset);
    // 2000 words over 30s — the density that produced ~4000 drawtext and crashed
    let words: Vec<Word> = (0..2000).map(|i| {
        let t = i as i64 * 15_000;
        Word { text: format!("w{i}"), start_us: t, end_us: t + 14_000, confidence: 1.0, rejected: false, display: None }
    }).collect();
    let doc = TranscriptDoc {
        id: Id::new(), asset_id: aid, language: "es".into(), model: "t".into(), words,
        segments: vec![ue_core::model::Segment { text: "x".into(), start_us: 0, end_us: 30 * SEC, word_range: (0, 2000), emotion: None, volume_rms: 0.0 }],
        global_avg_volume: 0.0,
    };
    let doc_id = doc.id;
    project.transcripts.push(doc);
    let seq = project.sequence_mut(seq_id).unwrap();
    seq.tracks.push(Track::new(TrackKind::Video, "V2"));
    let v1 = seq.tracks.iter().find(|t| t.name == "V1").unwrap().id;
    let v2 = seq.tracks.iter().find(|t| t.name == "V2").unwrap().id;
    let mut store = ProjectStore::new(project);
    store.insert_clip(v1, Clip::new_media(aid, 0, 30 * SEC, 0), InsertMode::Strict).unwrap();
    let style = TextStyle { size: 90.0, y_offset: 380.0, highlight_color: Some("#ffcc00".into()), ..Default::default() };
    store.insert_clip(v2, Clip {
        id: Id::new(),
        payload: ClipPayload::Subtitles { transcript_id: doc_id, style, mode: SubtitleMode::Karaoke, max_words: None },
        start: 0, duration: 30 * SEC, speed: 1.0, effects: vec![], transform: Default::default(),
        audio: Default::default(), transition_in: None, transition_out: None, label_color: None, name: None, group: None,
    }, InsertMode::Strict).unwrap();

    // a short range is bounded → only ~33 words → succeeds (used to crash)
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-karaoke-bounded.mp4");
    let _ = std::fs::remove_file(&out);
    let ranged = ExportSettings { ranges: vec![(0, 500_000)], ..Default::default() };
    let r = export_sequence(&store.project, seq_id, dir, &out, &ranged);
    assert!(r.is_ok(), "bounded karaoke range must succeed, got: {r:?}");
    assert!(out.exists());

    // the FULL 2000-word export also succeeds now: the script lives in a FILE
    // (ass=), so the filtergraph no longer grows with the transcript — the old
    // drawtext path emitted ~4000 filters and crashed ffmpeg, and a size guard
    // rejected it; both went away when subtitles moved to libass
    let full = ExportSettings::default();
    let out2 = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-karaoke-full.mp4");
    let r2 = export_sequence(&store.project, seq_id, dir, &out2, &full);
    assert!(r2.is_ok(), "full dense karaoke exports via the ASS file, got: {r2:?}");
    assert!(out2.exists());
}


/// An image clip is a still: the paused preview shows it at ANY playhead
/// position (seeking into a one-frame file used to yield black).
#[test]
fn preview_shows_image_clip_at_any_time() {
    let Some(dir) = media_dir() else { return };
    let img = dir.join("still_red.png");
    Command::new(ue_media::ffmpeg_bin())
        .args(["-y","-v","error","-f","lavfi","-i","color=c=red:s=800x600","-frames:v","1"]).arg(&img).status().unwrap();
    let mut project = Project::new("img");
    let seq_id = project.active_sequence;
    project.sequence_mut(seq_id).unwrap().resolution = (1280, 720);
    let mut asset = ue_media::import_file(&img).unwrap();
    assert_eq!(asset.kind, MediaKind::Image, "png imports as an image");
    asset.probe.duration_us = 40_000; // a still reports a tiny probe duration
    let aid = asset.id;
    project.assets.push(asset);
    let v1 = project.sequence(seq_id).unwrap().tracks.iter().find(|t| t.kind == TrackKind::Video).unwrap().id;
    let mut store = ProjectStore::new(project);
    // 5 s image clip
    store.insert_clip(v1, Clip::new_media(aid, 0, 5_000_000, 0), InsertMode::Strict).unwrap();
    // preview at t=3 s (deep inside the clip, past the "one frame") must be red
    let jpeg = ue_export::preview::render_preview_frame(&store.project, seq_id, dir, 3_000_000, 640, &[])
        .expect("compositor ok")
        .expect("a frame, not None");
    let f = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-img-preview.jpg");
    std::fs::write(&f, &jpeg).unwrap();
    // read a still JPEG WITHOUT -ss (seeking a one-frame file yields nothing);
    // the preview is 640 wide (canvas 1280x720 → half), centre is red
    let out = Command::new(ue_media::ffmpeg_bin())
        .args(["-v", "error", "-i"]).arg(&f)
        .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .output().unwrap();
    let p = out.stdout;
    let idx = (180usize * 640 + 320) * 3;
    let (r, g, b) = (p[idx], p[idx + 1], p[idx + 2]);
    assert!(r > 150 && g < 100 && b < 100, "the image (red) shows at t=3s, not black: rgb({r},{g},{b})");
}

/// The drop_shadow effect is in the catalog and its split/geq/overlay
/// filtergraph composes into a real export layer chain without breaking ffmpeg.
#[test]
fn drop_shadow_effect_exports_without_error() {
    let reg = ue_render::core_registry();
    let def = reg.iter().find(|e| e.id == "core.drop_shadow").expect("drop_shadow is a core effect");
    assert!(def.params.iter().any(|p| p.key == "margin"), "has the room/margin param");
    // render_chain emits the split/geq/overlay filtergraph with a UNIQUE label
    let inst = EffectInstance { effect_id: "core.drop_shadow".into(), enabled: true, params: Default::default(), color_params: Default::default() };
    let vf = ue_render::render_effect(def, &inst);
    for f in ["split", "colorchannelmixer", "gblur", "pad", "overlay"] {
        assert!(vf.contains(f), "the shadow chain uses {f}: {vf}");
    }
    assert!(vf.contains("sh") && vf.contains("];["), "unique labelled filtergraph");

    // integration: a PiP clip with the effect exports (the ;-graph composes)
    let Some(dir) = media_dir() else { return };
    let base = dir.join("ds_base.mp4");
    let pip = dir.join("ds_pip.mp4");
    for (p, c, d) in [(&base, "white", "1"), (&pip, "green", "1")] {
        Command::new(ue_media::ffmpeg_bin())
            .args(["-y","-v","error","-f","lavfi","-i",&format!("color=c={c}:s=320x240:d={d}:r=15")])
            .args(["-c:v","libx264","-preset","ultrafast","-pix_fmt","yuv420p"]).arg(p).status().unwrap();
    }
    let mut project = Project::new("ds");
    let seq_id = project.active_sequence;
    project.sequence_mut(seq_id).unwrap().resolution = (640, 480);
    let ba = ue_media::import_file(&base).unwrap(); let baid = ba.id; project.assets.push(ba);
    let pa = ue_media::import_file(&pip).unwrap(); let paid = pa.id; project.assets.push(pa);
    let seq = project.sequence_mut(seq_id).unwrap();
    seq.tracks.push(Track::new(TrackKind::Video, "V2"));
    let v1 = seq.tracks.iter().find(|t| t.name == "V1").unwrap().id;
    let v2 = seq.tracks.iter().find(|t| t.name == "V2").unwrap().id;
    let mut store = ProjectStore::new(project);
    store.insert_clip(v1, Clip::new_media(baid, 0, SEC, 0), InsertMode::Strict).unwrap();
    let mut clip = Clip::new_media(paid, 0, SEC, 0);
    clip.transform.scale = (0.5.into(), 0.5.into());
    clip.effects.push(inst);
    store.insert_clip(v2, clip, InsertMode::Strict).unwrap();
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-ds-export.mp4");
    let _ = std::fs::remove_file(&out);
    export_sequence(&store.project, seq_id, dir, &out, &ExportSettings::default())
        .expect("clip with drop_shadow exports");
    assert!(ue_media::import_file(&out).is_ok(), "the export is a valid video");
}

/// AUDIT — image over video: the compositor frame (now used by BOTH the paused
/// preview and live playback) must equal the export frame, pixel for pixel.
/// Also times the compositor so the playback fps is a known quantity.
#[test]
fn audit_image_over_video_preview_equals_export() {
    let Some(dir) = media_dir() else { return };
    let vid = dir.join("audit_base.mp4");
    Command::new(ue_media::ffmpeg_bin())
        .args(["-y","-v","error","-f","lavfi","-i","color=c=blue:s=1280x720:d=2:r=30"])
        .args(["-c:v","libx264","-preset","ultrafast","-pix_fmt","yuv420p"]).arg(&vid).status().unwrap();
    let imgp = dir.join("audit_overlay.png");
    Command::new(ue_media::ffmpeg_bin())
        .args(["-y","-v","error","-f","lavfi","-i","color=c=red:s=400x400","-frames:v","1"]).arg(&imgp).status().unwrap();

    let mut project = Project::new("audit");
    let seq_id = project.active_sequence;
    project.sequence_mut(seq_id).unwrap().resolution = (1280, 720);
    let va = ue_media::import_file(&vid).unwrap(); let vaid = va.id; project.assets.push(va);
    let mut ia = ue_media::import_file(&imgp).unwrap();
    assert_eq!(ia.kind, MediaKind::Image);
    ia.probe.duration_us = 40_000;
    let iaid = ia.id; project.assets.push(ia);
    let seq = project.sequence_mut(seq_id).unwrap();
    seq.tracks.push(Track::new(TrackKind::Video, "V2"));
    let v1 = seq.tracks.iter().find(|t| t.name == "V1").unwrap().id;
    let v2 = seq.tracks.iter().find(|t| t.name == "V2").unwrap().id;
    let mut store = ProjectStore::new(project);
    store.insert_clip(v1, Clip::new_media(vaid, 0, 2 * SEC, 0), InsertMode::Strict).unwrap();
    let mut img = Clip::new_media(iaid, 0, 2 * SEC, 0);
    img.transform.scale = (0.4.into(), 0.4.into());
    store.insert_clip(v2, img, InsertMode::Strict).unwrap();

    // export → frame at t=1s
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-audit-export.mp4");
    let _ = std::fs::remove_file(&out);
    export_sequence(&store.project, seq_id, dir, &out, &ExportSettings::default()).unwrap();
    let exp = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-audit-exp.png");
    Command::new(ue_media::ffmpeg_bin()).args(["-y","-v","error","-ss","1","-i"]).arg(&out).args(["-frames:v","1"]).arg(&exp).status().unwrap();

    // compositor (the playback/paused path) → frame at t=1s, TIMED
    let t0 = std::time::Instant::now();
    let jpeg = ue_export::preview::render_preview_frame(&store.project, seq_id, dir, SEC, 1280, &[]).unwrap().unwrap();
    eprintln!("compositor frame: {} ms (playback ≈ {} fps)", t0.elapsed().as_millis(), 1000 / t0.elapsed().as_millis().max(1));
    let prev = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-audit-prev.png");
    std::fs::write(dir.join("audit-prev.jpg"), &jpeg).unwrap();
    Command::new(ue_media::ffmpeg_bin()).args(["-y","-v","error","-i"]).arg(dir.join("audit-prev.jpg")).args(["-vf","scale=1280:720"]).arg(&prev).status().unwrap();

    let rgb = |f: &Path, x: u32, y: u32| pixel_at_w(f, 0.0, x, y, 1280);
    // corners = blue base; centre = red image
    for (x, y, what) in [(40u32,40u32,"corner"),(1240,40,"corner"),(640,360,"centre"),(40,680,"corner")] {
        let (er,eg,eb) = rgb(&exp,x,y);
        let (pr,pg,pb) = rgb(&prev,x,y);
        let d = (er as i32-pr as i32).abs().max((eg as i32-pg as i32).abs()).max((eb as i32-pb as i32).abs());
        eprintln!("{what} ({x},{y}) export=({er},{eg},{eb}) preview=({pr},{pg},{pb}) d={d}");
        assert!(d <= 40, "{what} diverges: export=({er},{eg},{eb}) preview=({pr},{pg},{pb})");
    }
    // sanity: the composite really has BOTH (blue corner, red centre)
    let (cr,_,cb) = rgb(&prev,40,40); assert!(cb > 150 && cr < 90, "blue base");
    let (mr,_,mb) = rgb(&prev,640,360); assert!(mr > 150 && mb < 90, "red image on top");
}

/// AUDIT — a generator clip (solid) composites identically in the compositor
/// and the export.
#[test]
fn audit_generator_preview_equals_export() {
    let Some(dir) = media_dir() else { return };
    let mut project = Project::new("gen");
    let seq_id = project.active_sequence;
    project.sequence_mut(seq_id).unwrap().resolution = (960, 540);
    let v1 = project.sequence(seq_id).unwrap().tracks.iter().find(|t| t.kind == TrackKind::Video).unwrap().id;
    let mut store = ProjectStore::new(project);
    // a solid gradient generator filling the frame
    let mut clip = Clip::new_generator("core.solid", 0, 2 * SEC);
    if let ClipPayload::Generator { color_params, .. } = &mut clip.payload {
        color_params.insert("color".into(), "#33cc66".into());
    }
    store.insert_clip(v1, clip, InsertMode::Strict).unwrap();

    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-audit-gen.mp4");
    let _ = std::fs::remove_file(&out);
    export_sequence(&store.project, seq_id, dir, &out, &ExportSettings::default()).unwrap();
    let exp = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-audit-gen-exp.png");
    Command::new(ue_media::ffmpeg_bin()).args(["-y","-v","error","-ss","1","-i"]).arg(&out).args(["-frames:v","1"]).arg(&exp).status().unwrap();

    let jpeg = ue_export::preview::render_preview_frame(&store.project, seq_id, dir, SEC, 960, &[]).unwrap().unwrap();
    let prev = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-audit-gen-prev.png");
    std::fs::write(dir.join("audit-gen-prev.jpg"), &jpeg).unwrap();
    Command::new(ue_media::ffmpeg_bin()).args(["-y","-v","error","-i"]).arg(dir.join("audit-gen-prev.jpg")).args(["-vf","scale=960:540"]).arg(&prev).status().unwrap();

    let (er,eg,eb) = pixel_at_w(&exp, 0.0, 480, 270, 960);
    let (pr,pg,pb) = pixel_at_w(&prev, 0.0, 480, 270, 960);
    eprintln!("generator export=({er},{eg},{eb}) preview=({pr},{pg},{pb})");
    let d = (er as i32-pr as i32).abs().max((eg as i32-pg as i32).abs()).max((eb as i32-pb as i32).abs());
    assert!(d <= 30, "generator diverges: export=({er},{eg},{eb}) preview=({pr},{pg},{pb})");
    assert!(eg > 150 && er < 120, "the green generator actually rendered");
}

/// Material living only on V2 (V1 empty) is the export's base: it fills the
/// canvas instead of failing with "empty timeline".
#[test]
fn clips_only_on_upper_track_export_as_base() {
    let Some(dir) = media_dir() else { return };
    let src = dir.join("upper_only_blue.mp4");
    if !src.exists() {
        let st = Command::new(ue_media::ffmpeg_bin())
            .args([
                "-y", "-v", "error",
                "-f", "lavfi", "-i", "color=blue:size=640x360:rate=30:duration=2",
                "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p",
            ])
            .arg(&src)
            .status()
            .unwrap();
        assert!(st.success());
    }
    let mut project = Project::new("upper-only");
    let seq_id = project.active_sequence;
    let asset = ue_media::import_file(&src).unwrap();
    let aid = asset.id;
    project.assets.push(asset);
    let seq = project.sequence_mut(seq_id).unwrap();
    seq.tracks.push(Track::new(TrackKind::Video, "V2"));
    let v2 = seq.tracks.last().unwrap().id;
    let mut store = ProjectStore::new(project);
    store.insert_clip(v2, Clip::new_media(aid, 0, 2 * SEC, 0), InsertMode::Strict).unwrap();

    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-upper-only.mp4");
    let _ = std::fs::remove_file(&out);
    export_sequence(&store.project, seq_id, dir, &out, &ExportSettings::default()).unwrap();
    let meta = ffprobe_json(&out);
    let dur: f64 = meta["format"]["duration"].as_str().unwrap().parse().unwrap();
    assert!((1.9..=2.2).contains(&dur), "lasts 2 s, was {dur}");
    let (r, _g, b) = pixel_at(&out, 1.0, 960, 540);
    assert!(b > 150 && r < 90, "centre is blue, was ({r},{b})");
    let (r2, _g2, b2) = pixel_at(&out, 1.0, 100, 100);
    assert!(b2 > 150 && r2 < 90, "fills the canvas as the base, was ({r2},{b2})");
}

/// A timeline with only a title exports over black instead of failing.
#[test]
fn text_only_timeline_exports_over_black() {
    let Some(dir) = media_dir() else { return };
    let project = Project::new("titles");
    let seq_id = project.active_sequence;
    let v1 = project
        .sequence(seq_id)
        .unwrap()
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Video)
        .unwrap()
        .id;
    let mut store = ProjectStore::new(project);
    store.insert_clip(v1, Clip::new_text("TITLE CARD", 0, 3 * SEC), InsertMode::Strict).unwrap();

    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-text-only.mp4");
    let _ = std::fs::remove_file(&out);
    export_sequence(&store.project, seq_id, dir, &out, &ExportSettings::default()).unwrap();
    let meta = ffprobe_json(&out);
    let dur: f64 = meta["format"]["duration"].as_str().unwrap().parse().unwrap();
    assert!((2.9..=3.2).contains(&dur), "lasts 3 s, was {dur}");
    let (r, g, b) = pixel_at(&out, 1.0, 100, 100);
    assert!(r < 30 && g < 30 && b < 30, "background is black, was ({r},{g},{b})");
}

/// Preview compositor parity: a gap in the base track keeps the upper layer
/// PiP-sized over black (never stretched to fill), exactly like the export.
#[test]
fn preview_gap_in_base_keeps_layer_pip() {
    let Some(dir) = media_dir() else { return };
    for (name, color) in [("gap_red.mp4", "red"), ("gap_blue.mp4", "blue")] {
        let out = dir.join(name);
        if !out.exists() {
            let st = Command::new(ue_media::ffmpeg_bin())
                .args([
                    "-y", "-v", "error",
                    "-f", "lavfi", "-i",
                    &format!("color={color}:size=640x360:rate=30:duration=3"),
                    "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p",
                ])
                .arg(&out)
                .status()
                .unwrap();
            assert!(st.success());
        }
    }
    let mut project = Project::new("gap");
    let seq_id = project.active_sequence;
    let red = ue_media::import_file(&dir.join("gap_red.mp4")).unwrap();
    let blue = ue_media::import_file(&dir.join("gap_blue.mp4")).unwrap();
    let (rid, bid) = (red.id, blue.id);
    project.assets.push(red);
    project.assets.push(blue);
    let seq = project.sequence_mut(seq_id).unwrap();
    seq.tracks.push(Track::new(TrackKind::Video, "V2"));
    let v1 = seq.tracks.iter().find(|t| t.name == "V1").unwrap().id;
    let v2 = seq.tracks.iter().find(|t| t.name == "V2").unwrap().id;
    let mut store = ProjectStore::new(project);
    // base red only [0,1s); the blue layer runs [0,3s) — at t=2s the base gaps
    store.insert_clip(v1, Clip::new_media(rid, 0, 1 * SEC, 0), InsertMode::Strict).unwrap();
    store.insert_clip(v2, Clip::new_media(bid, 0, 3 * SEC, 0), InsertMode::Strict).unwrap();

    // export frame at t=2s
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-gap-export.mp4");
    let _ = std::fs::remove_file(&out);
    export_sequence(&store.project, seq_id, dir, &out, &ExportSettings::default()).unwrap();

    // preview frame at t=2s
    let jpeg = ue_export::preview::render_preview_frame(&store.project, seq_id, dir, 2 * SEC, 960, &[])
        .unwrap()
        .unwrap();
    let prev = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-gap-prev.png");
    std::fs::write(dir.join("gap-prev.jpg"), &jpeg).unwrap();
    Command::new(ue_media::ffmpeg_bin())
        .args(["-y", "-v", "error", "-i"])
        .arg(dir.join("gap-prev.jpg"))
        .args(["-vf", "scale=960:540"])
        .arg(&prev)
        .status()
        .unwrap();

    // both: corner black (layer is NOT stretched), centre blue (layer visible)
    let (er, _eg, eb) = pixel_at(&out, 2.0, 80, 80);
    assert!(er < 30 && eb < 30, "export corner black, was ({er},{eb})");
    let (er2, _eg2, eb2) = pixel_at(&out, 2.0, 960, 540);
    assert!(eb2 > 150 && er2 < 90, "export centre blue, was ({er2},{eb2})");
    let (pr, _pg, pb) = pixel_at_w(&prev, 0.0, 40, 40, 960);
    assert!(pr < 30 && pb < 30, "preview corner black, was ({pr},{pb})");
    let (pr2, _pg2, pb2) = pixel_at_w(&prev, 0.0, 480, 270, 960);
    assert!(pb2 > 150 && pr2 < 90, "preview centre blue, was ({pr2},{pb2})");
}

/// Paused preview inside a transition runs the REAL xfade — same pattern and
/// progress as the export — for a fade and for a directional wipe.
#[test]
fn preview_transition_matches_export() {
    let Some(dir) = media_dir() else { return };
    for (name, color) in [("xf_red.mp4", "red"), ("xf_blue.mp4", "blue")] {
        let out = dir.join(name);
        if !out.exists() {
            let st = Command::new(ue_media::ffmpeg_bin())
                .args([
                    "-y", "-v", "error",
                    "-f", "lavfi", "-i",
                    &format!("color={color}:size=640x360:rate=30:duration=3"),
                    "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p",
                ])
                .arg(&out)
                .status()
                .unwrap();
            assert!(st.success());
        }
    }
    // A red [src 0..2) at 0; B blue [src 1..3) at 2 with a 1 s transition.
    // Handles: A has 1 s of tail, B has 1 s of lead → effective window
    // [1.5 s, 2.5 s), exactly like the export's xfade.
    for (kind, t_check) in [("core.crossfade", 2_000_000i64), ("core.wipeleft", 1_750_000i64)] {
        let mut project = Project::new("xfade");
        let seq_id = project.active_sequence;
        let red = ue_media::import_file(&dir.join("xf_red.mp4")).unwrap();
        let blue = ue_media::import_file(&dir.join("xf_blue.mp4")).unwrap();
        let (rid, bid) = (red.id, blue.id);
        project.assets.push(red);
        project.assets.push(blue);
        let v1 = project
            .sequence(seq_id)
            .unwrap()
            .tracks
            .iter()
            .find(|t| t.kind == TrackKind::Video)
            .unwrap()
            .id;
        let mut store = ProjectStore::new(project);
        store.insert_clip(v1, Clip::new_media(rid, 0, 2 * SEC, 0), InsertMode::Strict).unwrap();
        let mut b = Clip::new_media(bid, 1 * SEC, 3 * SEC, 2 * SEC);
        b.transition_in = Some(TransitionRef {
            effect_id: kind.into(),
            duration: 1 * SEC,
            params: Default::default(),
        });
        store.insert_clip(v1, b, InsertMode::Strict).unwrap();

        let slug = kind.replace('.', "-");
        let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("ue-{slug}.mp4"));
        let _ = std::fs::remove_file(&out);
        export_sequence(&store.project, seq_id, dir, &out, &ExportSettings::default()).unwrap();

        let jpeg = ue_export::preview::render_preview_frame(
            &store.project, seq_id, dir, t_check, 960, &[],
        )
        .unwrap()
        .unwrap();
        let prev = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("ue-{slug}-prev.png"));
        std::fs::write(dir.join("xfade-prev.jpg"), &jpeg).unwrap();
        Command::new(ue_media::ffmpeg_bin())
            .args(["-y", "-v", "error", "-i"])
            .arg(dir.join("xfade-prev.jpg"))
            .args(["-vf", "scale=960:540"])
            .arg(&prev)
            .status()
            .unwrap();

        let t = t_check as f64 / 1e6;
        for (x, y) in [(120u32, 270u32), (330, 270), (480, 200), (630, 270), (840, 270)] {
            let (er, eg, eb) = pixel_at(&out, t, x * 2, y * 2);
            let (pr, pg, pb) = pixel_at_w(&prev, 0.0, x, y, 960);
            let d = (er as i32 - pr as i32)
                .abs()
                .max((eg as i32 - pg as i32).abs())
                .max((eb as i32 - pb as i32).abs());
            eprintln!("{kind} ({x},{y}) export=({er},{eg},{eb}) preview=({pr},{pg},{pb}) d={d}");
            assert!(d <= 40, "{kind} at ({x},{y}) diverges: export=({er},{eg},{eb}) preview=({pr},{pg},{pb})");
        }
        // sanity per kind: fade at p=0.5 is a red/blue blend; wipeleft at
        // p=0.25 still shows opposite sides with different content
        let (mr, _mg, mb) = pixel_at_w(&prev, 0.0, 480, 270, 960);
        if kind == "core.crossfade" {
            assert!((60..=200).contains(&(mr as i32)) && (60..=200).contains(&(mb as i32)),
                "fade p=0.5 blends red+blue, was ({mr},{mb})");
        } else {
            let (lr, _lg, lb) = pixel_at_w(&prev, 0.0, 120, 270, 960);
            let (rr, _rg, rb) = pixel_at_w(&prev, 0.0, 840, 270, 960);
            assert!((lr > 150) != (rr > 150) || (lb > 150) != (rb > 150),
                "wipe p=0.25 shows different content on each side: left=({lr},{lb}) right=({rr},{rb})");
        }
    }
}

/// Regression: a transition that comes AFTER earlier cuts/gaps used to kill
/// the whole export ("xfade timebase … do not match") because concat moved
/// the accumulated stream to another timebase. Found live via MCP.
#[test]
fn transition_after_gaps_still_exports() {
    let Some(dir) = media_dir() else { return };
    for (name, color) in [("xf_red.mp4", "red"), ("xf_blue.mp4", "blue")] {
        let out = dir.join(name);
        if !out.exists() {
            let st = Command::new(ue_media::ffmpeg_bin())
                .args([
                    "-y", "-v", "error",
                    "-f", "lavfi", "-i",
                    &format!("color={color}:size=640x360:rate=30:duration=3"),
                    "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p",
                ])
                .arg(&out)
                .status()
                .unwrap();
            assert!(st.success());
        }
    }
    let mut project = Project::new("xfade-late");
    let seq_id = project.active_sequence;
    let red = ue_media::import_file(&dir.join("xf_red.mp4")).unwrap();
    let blue = ue_media::import_file(&dir.join("xf_blue.mp4")).unwrap();
    let counter = ue_media::import_file(&dir.join("counter.mp4")).unwrap();
    let (rid, bid, cid) = (red.id, blue.id, counter.id);
    project.assets.push(red);
    project.assets.push(blue);
    project.assets.push(counter);
    let v1 = project
        .sequence(seq_id)
        .unwrap()
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Video)
        .unwrap()
        .id;
    let mut store = ProjectStore::new(project);
    // earlier material + a GAP before the transition pair (this is what broke)
    store.insert_clip(v1, Clip::new_media(cid, 0, 2 * SEC, 0), InsertMode::Strict).unwrap();
    store.insert_clip(v1, Clip::new_media(rid, 0, 2 * SEC, 3 * SEC), InsertMode::Strict).unwrap();
    let mut b = Clip::new_media(bid, 1 * SEC, 3 * SEC, 5 * SEC);
    b.transition_in = Some(TransitionRef {
        effect_id: "core.crossfade".into(),
        duration: 1 * SEC,
        params: Default::default(),
    });
    store.insert_clip(v1, b, InsertMode::Strict).unwrap();

    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-xfade-late.mp4");
    let _ = std::fs::remove_file(&out);
    export_sequence(&store.project, seq_id, dir, &out, &ExportSettings::default()).unwrap();
    let meta = ffprobe_json(&out);
    let dur: f64 = meta["format"]["duration"].as_str().unwrap().parse().unwrap();
    assert!((6.8..=7.3).contains(&dur), "timeline stays 7 s (handles absorb the fade), was {dur}");
    // progress 0.5 of the fade at t=5.0 → red/blue blend
    let (r, _g, b2) = pixel_at(&out, 5.0, 960, 540);
    assert!((60..=200).contains(&(r as i32)) && (60..=200).contains(&(b2 as i32)),
        "mid-fade blend at 5 s, was ({r},{b2})");
}

/// The paused frame of a KARAOKE subtitle must show the same highlighted word
/// the export burns in (it used to degrade to a plain phrase line, so pause
/// and playback looked different).
#[test]
fn preview_karaoke_matches_export() {
    let Some(dir) = media_dir() else { return };
    let src = dir.join("kar_black.mp4");
    if !src.exists() {
        let st = Command::new(ue_media::ffmpeg_bin())
            .args([
                "-y", "-v", "error",
                "-f", "lavfi", "-i", "color=black:s=1920x1080:r=30:d=4",
                "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p",
            ])
            .arg(&src)
            .status()
            .unwrap();
        assert!(st.success());
    }
    let mut project = Project::new("karaoke");
    let seq_id = project.active_sequence;
    let asset = ue_media::import_file(&src).unwrap();
    let aid = asset.id;
    project.assets.push(asset);
    // two words, one second apart
    let words: Vec<Word> = ["UNO", "DOS"]
        .iter()
        .enumerate()
        .map(|(i, t)| Word {
            text: (*t).into(),
            start_us: i as i64 * SEC,
            end_us: i as i64 * SEC + 800_000,
            confidence: 1.0,
            rejected: false,
            display: None,
        })
        .collect();
    let doc = TranscriptDoc {
        id: Id::new(),
        asset_id: aid,
        language: "es".into(),
        model: "t".into(),
        segments: vec![ue_core::model::Segment {
            text: "UNO DOS".into(),
            start_us: 0,
            end_us: 2 * SEC,
            word_range: (0, 2),
            emotion: None,
            volume_rms: 0.0,
        }],
        words,
        global_avg_volume: 0.0,
    };
    let doc_id = doc.id;
    project.transcripts.push(doc);
    let v1 = project.sequence(seq_id).unwrap().tracks.iter()
        .find(|t| t.kind == TrackKind::Video).unwrap().id;
    let mut store = ProjectStore::new(project);
    store.insert_clip(v1, Clip::new_media(aid, 0, 4 * SEC, 0), InsertMode::Strict).unwrap();
    let style = TextStyle {
        size: 90.0,
        highlight_color: Some("#FFB224".into()),
        ..Default::default()
    };
    let sub = Clip {
        id: Id::new(),
        payload: ClipPayload::Subtitles { transcript_id: doc_id, style, mode: SubtitleMode::Karaoke, max_words: None },
        start: 0,
        duration: 4 * SEC,
        speed: 1.0,
        effects: vec![],
        transform: Default::default(),
        audio: Default::default(),
        transition_in: None,
        transition_out: None,
        label_color: None,
        name: None,
        group: None,
    };
    let seq = store.project.sequence_mut(seq_id).unwrap();
    seq.tracks.push(Track::new(TrackKind::Video, "V2"));
    let v2 = seq.tracks.last().unwrap().id;
    store.insert_clip(v2, sub, InsertMode::Strict).unwrap();

    // at t=1.5s the FIRST word is already spoken (highlighted) and the second
    // one has just started too → both amber. At t=0.5s only "UNO" is amber.
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-karaoke-preview.mp4");
    let _ = std::fs::remove_file(&out);
    export_sequence(&store.project, seq_id, dir, &out, &ExportSettings::default()).unwrap();

    let t = 500_000; // 0.5 s: "UNO" highlighted, "DOS" still dim
    let jpeg = ue_export::preview::render_preview_frame(&store.project, seq_id, dir, t, 960, &[])
        .unwrap()
        .unwrap();
    let prev = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-karaoke-prev.png");
    std::fs::write(dir.join("kar-prev.jpg"), &jpeg).unwrap();
    Command::new(ue_media::ffmpeg_bin())
        .args(["-y", "-v", "error", "-i"]).arg(dir.join("kar-prev.jpg"))
        .args(["-vf", "scale=960:540"]).arg(&prev).status().unwrap();

    // count amber-ish pixels (the highlight) in both, on the caption band
    let amber = |path: &Path, from_export: bool| -> usize {
        let out = Command::new(ue_media::ffmpeg_bin())
            .args(["-v", "error"])
            .args(if from_export { vec!["-ss", "0.5"] } else { vec![] })
            .args(["-i"]).arg(path)
            .args(["-frames:v", "1", "-vf", "scale=960:540", "-f", "rawvideo", "-pix_fmt", "rgb24", "pipe:1"])
            .output()
            .unwrap();
        out.stdout
            .chunks_exact(3)
            .filter(|p| p[0] > 180 && p[1] > 120 && p[1] < 200 && p[2] < 90)
            .count()
    };
    let e = amber(&out, true);
    let p = amber(&prev, false);
    eprintln!("amber pixels — export: {e}, preview: {p}");
    assert!(e > 200, "the export really highlights a word ({e} amber px)");
    assert!(p > 200, "the PAUSED preview highlights it too ({p} amber px)");
    let ratio = p as f64 / e as f64;
    assert!((0.7..=1.4).contains(&ratio), "same highlight: export={e} preview={p}");
}

/// core.drop_shadow must appear in the PAUSED preview, not only in playback.
#[test]
fn preview_drop_shadow_matches_export() {
    let Some(dir) = media_dir() else { return };
    for (name, filter) in [
        ("ds_white_bg.mp4", "color=white:s=1920x1080:r=30:d=2"),
        ("ds_red_pip.mp4", "color=red:s=400x400:r=30:d=2"),
    ] {
        let out = dir.join(name);
        if !out.exists() {
            let st = Command::new(ue_media::ffmpeg_bin())
                .args(["-y", "-v", "error", "-f", "lavfi", "-i", filter,
                       "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p"])
                .arg(&out).status().unwrap();
            assert!(st.success());
        }
    }
    let mut project = Project::new("shadow");
    let seq_id = project.active_sequence;
    let bg = ue_media::import_file(&dir.join("ds_white_bg.mp4")).unwrap();
    let pip = ue_media::import_file(&dir.join("ds_red_pip.mp4")).unwrap();
    let (bgid, pipid) = (bg.id, pip.id);
    project.assets.push(bg);
    project.assets.push(pip);
    let seq = project.sequence_mut(seq_id).unwrap();
    seq.tracks.push(Track::new(TrackKind::Video, "V2"));
    let v1 = seq.tracks.iter().find(|t| t.name == "V1").unwrap().id;
    let v2 = seq.tracks.iter().find(|t| t.name == "V2").unwrap().id;
    let mut store = ProjectStore::new(project);
    store.insert_clip(v1, Clip::new_media(bgid, 0, 2 * SEC, 0), InsertMode::Strict).unwrap();
    let mut top = Clip::new_media(pipid, 0, 2 * SEC, 0);
    top.effects.push(EffectInstance {
        effect_id: "core.drop_shadow".into(),
        enabled: true,
        params: Default::default(),
        color_params: Default::default(),
    });
    store.insert_clip(v2, top, InsertMode::Strict).unwrap();

    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-shadow.mp4");
    let _ = std::fs::remove_file(&out);
    export_sequence(&store.project, seq_id, dir, &out, &ExportSettings::default()).unwrap();
    let jpeg = ue_export::preview::render_preview_frame(&store.project, seq_id, dir, SEC, 960, &[])
        .unwrap()
        .unwrap();
    let prev = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-shadow-prev.png");
    std::fs::write(dir.join("shadow-prev.jpg"), &jpeg).unwrap();
    Command::new(ue_media::ffmpeg_bin())
        .args(["-y", "-v", "error", "-i"]).arg(dir.join("shadow-prev.jpg"))
        .args(["-vf", "scale=960:540"]).arg(&prev).status().unwrap();

    // count "grey" pixels (the soft shadow over the white bg) in each
    let greys = |path: &Path, from_export: bool| -> usize {
        let o = Command::new(ue_media::ffmpeg_bin())
            .args(["-v", "error"])
            .args(if from_export { vec!["-ss", "1"] } else { vec![] })
            .args(["-i"]).arg(path)
            .args(["-frames:v", "1", "-vf", "scale=960:540", "-f", "rawvideo", "-pix_fmt", "rgb24", "pipe:1"])
            .output().unwrap();
        o.stdout.chunks_exact(3)
            .filter(|p| {
                let (r, g, b) = (p[0] as i32, p[1] as i32, p[2] as i32);
                // grey: dark-ish, and all channels close together (not the red pip)
                r < 235 && r > 40 && (r - g).abs() < 24 && (g - b).abs() < 24
            })
            .count()
    };
    let e = greys(&out, true);
    let p = greys(&prev, false);
    eprintln!("shadow (grey) pixels — export: {e}, preview: {p}");
    assert!(e > 500, "the export really draws a shadow ({e} grey px)");
    assert!(p > 500, "the PAUSED preview draws it too ({p} grey px)");
}

/// Same, but the shadowed clip is an IMAGE (the user's repro: import an image,
/// give it an exaggerated drop shadow). Compares the paused frame against the
/// export PIXEL BY PIXEL instead of counting "grey": with opacity 1 the shadow
/// is pure black, and a grey-counting check silently calls that "no shadow".
#[test]
fn preview_drop_shadow_on_image_matches_export() {
    let Some(dir) = media_dir() else { return };
    let bgv = dir.join("dsi_white_bg.mp4");
    if !bgv.exists() {
        Command::new(ue_media::ffmpeg_bin())
            .args(["-y", "-v", "error", "-f", "lavfi", "-i", "color=white:s=1920x1080:r=30:d=2",
                   "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p"])
            .arg(&bgv).status().unwrap();
    }
    let png = dir.join("dsi_red.png");
    if !png.exists() {
        Command::new(ue_media::ffmpeg_bin())
            .args(["-y", "-v", "error", "-f", "lavfi", "-i", "color=red:s=400x400",
                   "-frames:v", "1"])
            .arg(&png).status().unwrap();
    }
    let mut project = Project::new("shadow-img");
    let seq_id = project.active_sequence;
    let bg = ue_media::import_file(&bgv).unwrap();
    let img = ue_media::import_file(&png).unwrap();
    let (bgid, imgid) = (bg.id, img.id);
    project.assets.push(bg);
    project.assets.push(img);
    let seq = project.sequence_mut(seq_id).unwrap();
    seq.tracks.push(Track::new(TrackKind::Video, "V2"));
    let v1 = seq.tracks.iter().find(|t| t.name == "V1").unwrap().id;
    let v2 = seq.tracks.iter().find(|t| t.name == "V2").unwrap().id;
    let mut store = ProjectStore::new(project);
    store.insert_clip(v1, Clip::new_media(bgid, 0, 2 * SEC, 0), InsertMode::Strict).unwrap();
    let mut top = Clip::new_media(imgid, 0, 2 * SEC, 0);
    let mut params: std::collections::BTreeMap<String, ue_core::keyframe::Param> = Default::default();
    params.insert("offset_x".into(), 60.0.into());
    params.insert("offset_y".into(), 60.0.into());
    params.insert("blur".into(), 30.0.into());
    params.insert("opacity".into(), 1.0.into());
    params.insert("margin".into(), 200.0.into());
    top.effects.push(EffectInstance {
        effect_id: "core.drop_shadow".into(),
        enabled: true,
        params,
        color_params: Default::default(),
    });
    store.insert_clip(v2, top, InsertMode::Strict).unwrap();

    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-shadow-img.mp4");
    let _ = std::fs::remove_file(&out);
    export_sequence(&store.project, seq_id, dir, &out, &ExportSettings::default()).unwrap();
    let jpeg = ue_export::preview::render_preview_frame(&store.project, seq_id, dir, SEC, 960, &[])
        .unwrap()
        .unwrap();
    let prev = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-shadow-img-prev.png");
    std::fs::write(dir.join("shadow-img-prev.jpg"), &jpeg).unwrap();
    Command::new(ue_media::ffmpeg_bin())
        .args(["-y", "-v", "error", "-i"]).arg(dir.join("shadow-img-prev.jpg"))
        .args(["-vf", "scale=960:540"]).arg(&prev).status().unwrap();

    // the shadow is DARK (not grey): count pixels that are neither the red
    // image nor the white background
    let dark = |path: &Path, from_export: bool| -> usize {
        let o = Command::new(ue_media::ffmpeg_bin())
            .args(["-v", "error"])
            .args(if from_export { vec!["-ss", "1"] } else { vec![] })
            .args(["-i"]).arg(path)
            .args(["-frames:v", "1", "-vf", "scale=960:540", "-f", "rawvideo", "-pix_fmt", "rgb24", "pipe:1"])
            .output().unwrap();
        o.stdout.chunks_exact(3)
            .filter(|p| {
                let (r, g, b) = (p[0] as i32, p[1] as i32, p[2] as i32);
                let is_red = r > 150 && g < 90;
                let is_white = r > 235 && g > 235 && b > 235;
                !is_red && !is_white
            })
            .count()
    };
    let e = dark(&out, true);
    let p = dark(&prev, false);
    eprintln!("IMAGE shadow px — export: {e}, preview: {p}");
    assert!(e > 5000, "the export draws the shadow ({e} px)");
    assert!(p > 5000, "the PAUSED preview draws it too ({p} px)");
    let ratio = p as f64 / e as f64;
    assert!((0.75..=1.3).contains(&ratio), "same shadow: export={e} preview={p}");
}

/// A subtitles clip with `max_words: Some(2)` chunks the captions two words at
/// a time, in the export AND in the paused preview (the user picks the number,
/// the frame-width heuristic steps aside).
#[test]
fn subtitles_respect_the_word_cap() {
    use ue_export::graph::transcript_phrases;
    let asset_id = Id::new();
    let labels = ["uno", "dos", "tres", "cuatro", "cinco", "seis"];
    let words: Vec<Word> = labels
        .iter()
        .enumerate()
        .map(|(i, t)| Word {
            text: (*t).into(),
            start_us: i as i64 * 300_000,
            end_us: i as i64 * 300_000 + 250_000,
            confidence: 1.0,
            rejected: false,
            display: None,
        })
        .collect();
    let doc = TranscriptDoc {
        id: Id::new(),
        asset_id,
        language: "es".into(),
        model: "t".into(),
        segments: vec![],
        words,
        global_avg_volume: 0.0,
    };
    // no cap: the whole run fits one caption line at 64 chars
    let auto = transcript_phrases(&doc, 64);
    assert_eq!(auto.len(), 1, "without a cap it packs the line: {auto:?}");

    // the cap is what the UI/MCP write into the clip; check the chunker honours it
    let two = ue_export::graph::caption_phrases_for_test(&doc, 64, Some(2));
    assert_eq!(two.len(), 3, "6 words / 2 per line = 3 captions: {two:?}");
    assert_eq!(two[0].0, "uno dos");
    assert_eq!(two[1].0, "tres cuatro");
    assert_eq!(two[2].0, "cinco seis");

    let one = ue_export::graph::caption_phrases_for_test(&doc, 64, Some(1));
    assert_eq!(one.len(), 6, "one word per caption");
    assert_eq!(one[3].0, "cuatro");
}

/// A TITLE with effects + a transform must actually get them: the clip becomes
/// its own RGBA layer and goes through the same effect/transform chain a media
/// clip does. Before this, every effect and every transform on text silently
/// did nothing (the text was just burned in with drawtext at the end).
#[test]
fn text_clip_gets_effects_and_transform() {
    let Some(dir) = media_dir() else { return };
    let bgv = dir.join("txfx_black.mp4");
    if !bgv.exists() {
        Command::new(ue_media::ffmpeg_bin())
            .args(["-y", "-v", "error", "-f", "lavfi", "-i", "color=black:s=1920x1080:r=30:d=2",
                   "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p"])
            .arg(&bgv).status().unwrap();
    }
    let mut project = Project::new("text-fx");
    let seq_id = project.active_sequence;
    let bg = ue_media::import_file(&bgv).unwrap();
    let bgid = bg.id;
    project.assets.push(bg);
    let seq = project.sequence_mut(seq_id).unwrap();
    seq.tracks.push(Track::new(TrackKind::Video, "V2"));
    let v1 = seq.tracks.iter().find(|t| t.name == "V1").unwrap().id;
    let v2 = seq.tracks.iter().find(|t| t.name == "V2").unwrap().id;
    let mut store = ProjectStore::new(project);
    store.insert_clip(v1, Clip::new_media(bgid, 0, 2 * SEC, 0), InsertMode::Strict).unwrap();

    // the SAME title twice: once plain, once moved far down + blurred
    let render = |styled: bool| -> Vec<u8> {
        let mut st = ProjectStore::new(store.project.clone());
        let mut text = Clip::new_text("HELLO", 0, 2 * SEC);
        if let ClipPayload::Text { style, .. } = &mut text.payload {
            style.size = 120.0;
        }
        if styled {
            text.transform.position = (0.0.into(), 300.0.into());
            let mut params: std::collections::BTreeMap<String, ue_core::keyframe::Param> =
                Default::default();
            params.insert("sigma".into(), 6.0.into());
            text.effects.push(EffectInstance {
                effect_id: "core.gaussian_blur".into(),
                enabled: true,
                params,
                color_params: Default::default(),
            });
        }
        st.insert_clip(v2, text, InsertMode::Strict).unwrap();
        let out = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(if styled { "ue-textfx-on.mp4" } else { "ue-textfx-off.mp4" });
        let _ = std::fs::remove_file(&out);
        export_sequence(&st.project, seq_id, dir, &out, &ExportSettings::default()).unwrap();
        let o = Command::new(ue_media::ffmpeg_bin())
            .args(["-v", "error", "-ss", "1", "-i"]).arg(&out)
            .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "pipe:1"])
            .output().unwrap();
        o.stdout
    };

    // brightest row of the frame = where the text sits
    let text_row = |buf: &[u8]| -> usize {
        let (w, h) = (1920usize, 1080usize);
        let mut best = (0usize, 0u64);
        for y in 0..h {
            let sum: u64 = (0..w).map(|x| buf[(y * w + x) * 3] as u64).sum();
            if sum > best.1 {
                best = (y, sum);
            }
        }
        best.0
    };
    let plain = render(false);
    let styled = render(true);
    let y_plain = text_row(&plain);
    let y_styled = text_row(&styled);
    eprintln!("text row — plain: {y_plain}, styled (moved +300px): {y_styled}");
    assert!(
        (y_plain as i64 - 540).abs() < 90,
        "the plain title sits at the middle, was {y_plain}"
    );
    assert!(
        y_styled > y_plain + 200,
        "the TRANSFORM moved the title down ({y_plain} → {y_styled})"
    );
    // and the blur really softened it: fewer near-white pixels than the sharp one
    let bright = |b: &[u8]| b.chunks_exact(3).filter(|p| p[0] > 230).count();
    let (bp, bs) = (bright(&plain), bright(&styled));
    eprintln!("near-white px — plain: {bp}, blurred: {bs}");
    assert!(bs < bp, "the EFFECT (blur) softened the title ({bp} → {bs})");
}

/// The PAUSED frame of a styled text clip matches the export (both now render
/// it as a layer through the effect/transform chain).
#[test]
fn preview_styled_text_matches_export() {
    let Some(dir) = media_dir() else { return };
    let bgv = dir.join("txfx_black.mp4");
    if !bgv.exists() {
        Command::new(ue_media::ffmpeg_bin())
            .args(["-y", "-v", "error", "-f", "lavfi", "-i", "color=black:s=1920x1080:r=30:d=2",
                   "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p"])
            .arg(&bgv).status().unwrap();
    }
    let mut project = Project::new("text-fx-preview");
    let seq_id = project.active_sequence;
    let bg = ue_media::import_file(&bgv).unwrap();
    let bgid = bg.id;
    project.assets.push(bg);
    let seq = project.sequence_mut(seq_id).unwrap();
    seq.tracks.push(Track::new(TrackKind::Video, "V2"));
    let v1 = seq.tracks.iter().find(|t| t.name == "V1").unwrap().id;
    let v2 = seq.tracks.iter().find(|t| t.name == "V2").unwrap().id;
    let mut store = ProjectStore::new(project);
    store.insert_clip(v1, Clip::new_media(bgid, 0, 2 * SEC, 0), InsertMode::Strict).unwrap();
    let mut text = Clip::new_text("HELLO", 0, 2 * SEC);
    if let ClipPayload::Text { style, .. } = &mut text.payload {
        style.size = 120.0;
    }
    text.transform.position = (0.0.into(), 300.0.into());
    text.transform.scale = (0.5.into(), 0.5.into());
    store.insert_clip(v2, text, InsertMode::Strict).unwrap();

    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-textfx-prev.mp4");
    let _ = std::fs::remove_file(&out);
    export_sequence(&store.project, seq_id, dir, &out, &ExportSettings::default()).unwrap();
    let jpeg = ue_export::preview::render_preview_frame(&store.project, seq_id, dir, SEC, 960, &[])
        .unwrap()
        .unwrap();
    let prev = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-textfx-prev.png");
    std::fs::write(dir.join("textfx-prev.jpg"), &jpeg).unwrap();
    Command::new(ue_media::ffmpeg_bin())
        .args(["-y", "-v", "error", "-i"]).arg(dir.join("textfx-prev.jpg"))
        .args(["-vf", "scale=960:540"]).arg(&prev).status().unwrap();

    // brightest row (the text) must land at the same place in both
    let row = |path: &Path, from_export: bool| -> usize {
        let o = Command::new(ue_media::ffmpeg_bin())
            .args(["-v", "error"])
            .args(if from_export { vec!["-ss", "1"] } else { vec![] })
            .args(["-i"]).arg(path)
            .args(["-frames:v", "1", "-vf", "scale=960:540", "-f", "rawvideo", "-pix_fmt", "rgb24", "pipe:1"])
            .output().unwrap();
        let (w, h) = (960usize, 540usize);
        let mut best = (0usize, 0u64);
        for y in 0..h {
            let sum: u64 = (0..w).map(|x| o.stdout[(y * w + x) * 3] as u64).sum();
            if sum > best.1 { best = (y, sum); }
        }
        best.0
    };
    let e = row(&out, true);
    let p = row(&prev, false);
    eprintln!("styled text row — export: {e}, paused preview: {p}");
    assert!(e > 300, "the export really moved the text down, was row {e}");
    assert!((e as i64 - p as i64).abs() <= 12, "pause == export: export={e} preview={p}");
}

/// Wrapping is measured with the REAL font: a caption too wide for the frame
/// becomes several lines, the block stays vertically centred on y_offset, and
/// the PAUSED frame wraps exactly where the export does.
#[test]
fn long_caption_wraps_and_pause_matches_export() {
    let Some(dir) = media_dir() else { return };
    let bgv = dir.join("wrap_black.mp4");
    if !bgv.exists() {
        Command::new(ue_media::ffmpeg_bin())
            .args(["-y", "-v", "error", "-f", "lavfi", "-i", "color=black:s=1080x1920:r=30:d=2",
                   "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p"])
            .arg(&bgv).status().unwrap();
    }
    let mut project = Project::new("wrap");
    let seq_id = project.active_sequence;
    project.sequence_mut(seq_id).unwrap().resolution = (1080, 1920);
    let bg = ue_media::import_file(&bgv).unwrap();
    let bgid = bg.id;
    project.assets.push(bg);
    let seq = project.sequence_mut(seq_id).unwrap();
    seq.tracks.push(Track::new(TrackKind::Video, "V2"));
    let v1 = seq.tracks.iter().find(|t| t.name == "V1").unwrap().id;
    let v2 = seq.tracks.iter().find(|t| t.name == "V2").unwrap().id;
    let mut store = ProjectStore::new(project);
    store.insert_clip(v1, Clip::new_media(bgid, 0, 2 * SEC, 0), InsertMode::Strict).unwrap();

    // a line far too long for a 1080-wide frame at 80 px
    let long = "esta frase es demasiado larga para caber en una sola linea del video";
    let mut text = Clip::new_text(long, 0, 2 * SEC);
    if let ClipPayload::Text { style, .. } = &mut text.payload {
        style.size = 80.0;
    }
    store.insert_clip(v2, text, InsertMode::Strict).unwrap();

    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-wrap.mp4");
    let _ = std::fs::remove_file(&out);
    export_sequence(&store.project, seq_id, dir, &out, &ExportSettings::default()).unwrap();
    let jpeg = ue_export::preview::render_preview_frame(&store.project, seq_id, dir, SEC, 1080, &[])
        .unwrap()
        .unwrap();
    let prev = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-wrap-prev.png");
    std::fs::write(dir.join("wrap-prev.jpg"), &jpeg).unwrap();
    Command::new(ue_media::ffmpeg_bin())
        .args(["-y", "-v", "error", "-i"]).arg(dir.join("wrap-prev.jpg"))
        .args(["-vf", "scale=1080:1920"]).arg(&prev).status().unwrap();

    // rows that contain text, and how many separate bands of them there are
    let bands = |path: &Path, from_export: bool| -> (Vec<usize>, usize, usize) {
        let o = Command::new(ue_media::ffmpeg_bin())
            .args(["-v", "error"])
            .args(if from_export { vec!["-ss", "1"] } else { vec![] })
            .args(["-i"]).arg(path)
            .args(["-frames:v", "1", "-vf", "scale=1080:1920", "-f", "rawvideo", "-pix_fmt", "rgb24", "pipe:1"])
            .output().unwrap();
        let (w, h) = (1080usize, 1920usize);
        let mut rows = vec![];
        let mut widest = 0usize;
        for y in 0..h {
            let lit = (0..w).filter(|x| o.stdout[(y * w + x) * 3] > 120).count();
            if lit > 0 {
                rows.push(y);
                widest = widest.max(lit);
            }
        }
        // count bands (runs of consecutive text rows)
        let mut n = 0;
        for (i, y) in rows.iter().enumerate() {
            if i == 0 || *y > rows[i - 1] + 3 {
                n += 1;
            }
        }
        (rows.clone(), n, widest)
    };
    let (erows, elines, ewide) = bands(&out, true);
    let (prows, plines, pwide) = bands(&prev, false);
    eprintln!("export: {elines} lines, rows {}..{}, widest {ewide}px",
        erows.first().unwrap(), erows.last().unwrap());
    eprintln!("paused: {plines} lines, rows {}..{}, widest {pwide}px",
        prows.first().unwrap(), prows.last().unwrap());

    assert!(elines >= 2, "the long caption WRAPPED ({elines} lines)");
    assert!(ewide < 1080, "no line spills past the frame ({ewide}px of 1080)");
    assert_eq!(elines, plines, "pause wraps into the same number of lines");

    // the block stays centred on y_offset (=0 → the middle of the frame)
    let mid_e = (erows.first().unwrap() + erows.last().unwrap()) / 2;
    let mid_p = (prows.first().unwrap() + prows.last().unwrap()) / 2;
    assert!((mid_e as i64 - 960).abs() < 60, "block centred, was {mid_e}");
    assert!((mid_e as i64 - mid_p as i64).abs() <= 6, "pause == export: {mid_e} vs {mid_p}");
}

/// Karaoke wraps too: a long phrase spreads over several lines, the highlight
/// still follows the spoken word, and pause matches the export.
#[test]
fn karaoke_wraps_across_lines() {
    let Some(dir) = media_dir() else { return };
    let src = dir.join("kwrap_black.mp4");
    if !src.exists() {
        Command::new(ue_media::ffmpeg_bin())
            .args(["-y", "-v", "error", "-f", "lavfi", "-i", "color=black:s=1080x1920:r=30:d=6",
                   "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p"])
            .arg(&src).status().unwrap();
    }
    let mut project = Project::new("kwrap");
    let seq_id = project.active_sequence;
    project.sequence_mut(seq_id).unwrap().resolution = (1080, 1920);
    let asset = ue_media::import_file(&src).unwrap();
    let aid = asset.id;
    project.assets.push(asset);
    // 8 long-ish words, close together so they stay one caption
    let labels = ["ESTA", "FRASE", "LARGUISIMA", "NECESITA", "VARIAS", "LINEAS", "PARA", "CABER"];
    let words: Vec<Word> = labels
        .iter()
        .enumerate()
        .map(|(i, t)| Word {
            text: (*t).into(),
            start_us: i as i64 * 200_000,
            end_us: i as i64 * 200_000 + 180_000,
            confidence: 1.0,
            rejected: false,
            display: None,
        })
        .collect();
    let doc = TranscriptDoc {
        id: Id::new(),
        asset_id: aid,
        language: "es".into(),
        model: "t".into(),
        segments: vec![],
        words,
        global_avg_volume: 0.0,
    };
    let doc_id = doc.id;
    project.transcripts.push(doc);
    let seq = project.sequence_mut(seq_id).unwrap();
    seq.tracks.push(Track::new(TrackKind::Video, "V2"));
    let v1 = seq.tracks.iter().find(|t| t.name == "V1").unwrap().id;
    let v2 = seq.tracks.iter().find(|t| t.name == "V2").unwrap().id;
    let mut store = ProjectStore::new(project);
    store.insert_clip(v1, Clip::new_media(aid, 0, 6 * SEC, 0), InsertMode::Strict).unwrap();
    let style = TextStyle {
        size: 80.0,
        highlight_color: Some("#FFB224".into()),
        ..Default::default()
    };
    let sub = Clip {
        id: Id::new(),
        payload: ClipPayload::Subtitles {
            transcript_id: doc_id,
            style,
            mode: SubtitleMode::Karaoke,
            max_words: None,
        },
        start: 0,
        duration: 6 * SEC,
        speed: 1.0,
        effects: vec![],
        transform: Default::default(),
        audio: Default::default(),
        transition_in: None,
        transition_out: None,
        label_color: None,
        name: None,
        group: None,
    };
    store.insert_clip(v2, sub, InsertMode::Strict).unwrap();

    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-kwrap.mp4");
    let _ = std::fs::remove_file(&out);
    export_sequence(&store.project, seq_id, dir, &out, &ExportSettings::default()).unwrap();
    let t = 1_000_000; // 1 s: the first 5 words are already spoken
    let jpeg = ue_export::preview::render_preview_frame(&store.project, seq_id, dir, t, 1080, &[])
        .unwrap()
        .unwrap();
    let prev = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-kwrap-prev.png");
    std::fs::write(dir.join("kwrap-prev.jpg"), &jpeg).unwrap();
    Command::new(ue_media::ffmpeg_bin())
        .args(["-y", "-v", "error", "-i"]).arg(dir.join("kwrap-prev.jpg"))
        .args(["-vf", "scale=1080:1920"]).arg(&prev).status().unwrap();

    let stats = |path: &Path, from_export: bool| -> (usize, usize, usize) {
        let o = Command::new(ue_media::ffmpeg_bin())
            .args(["-v", "error"])
            .args(if from_export { vec!["-ss", "1"] } else { vec![] })
            .args(["-i"]).arg(path)
            .args(["-frames:v", "1", "-vf", "scale=1080:1920", "-f", "rawvideo", "-pix_fmt", "rgb24", "pipe:1"])
            .output().unwrap();
        let (w, h) = (1080usize, 1920usize);
        let mut rows: Vec<usize> = vec![];
        let mut amber = 0usize;
        for y in 0..h {
            let mut lit = false;
            for x in 0..w {
                let i = (y * w + x) * 3;
                let (r, g, b) = (o.stdout[i], o.stdout[i + 1], o.stdout[i + 2]);
                if r > 100 || g > 100 {
                    lit = true;
                }
                if r > 180 && (120..200).contains(&g) && b < 90 {
                    amber += 1;
                }
            }
            if lit {
                rows.push(y);
            }
        }
        let mut lines = 0;
        for (i, y) in rows.iter().enumerate() {
            if i == 0 || *y > rows[i - 1] + 3 {
                lines += 1;
            }
        }
        (lines, amber, rows.len())
    };
    let (elines, eamber, _) = stats(&out, true);
    let (plines, pamber, _) = stats(&prev, false);
    eprintln!("karaoke — export: {elines} lines, {eamber} amber px | pause: {plines} lines, {pamber} amber px");
    assert!(elines >= 2, "the karaoke phrase WRAPPED ({elines} lines)");
    assert_eq!(elines, plines, "pause wraps the same");
    assert!(eamber > 200, "the highlight still lights the spoken words");
    let ratio = pamber as f64 / eamber as f64;
    assert!((0.7..=1.4).contains(&ratio), "same highlight: export={eamber} pause={pamber}");
}

/// A range deep inside a long recording must cost its own LENGTH, not its
/// offset. Before, the whole timeline was rendered from t=0 and trimmed at the
/// end of the filtergraph, so a 5 s clip at minute N took as long as rendering
/// N minutes. Now the sequence is cut to the range up front and each input is
/// seeked with `-ss`, so early and late ranges take the same time.
#[test]
fn late_range_costs_the_same_as_an_early_one() {
    let Some(dir) = media_dir() else { return };
    // 4 minutes of 720p: long enough that decoding it from 0 is unmissable
    let long = dir.join("long4min.mp4");
    if !long.exists() {
        let st = Command::new(ue_media::ffmpeg_bin())
            .args([
                "-y", "-v", "error",
                "-f", "lavfi", "-i", "testsrc2=size=1280x720:rate=30:duration=240",
                "-f", "lavfi", "-i", "sine=frequency=300:duration=240",
                "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p",
                "-g", "60", "-c:a", "aac", "-shortest",
            ])
            .arg(&long)
            .status()
            .unwrap();
        assert!(st.success());
    }
    let mut project = Project::new("late-range");
    let seq_id = project.active_sequence;
    project.sequence_mut(seq_id).unwrap().resolution = (1280, 720);
    let asset = ue_media::import_file(&long).unwrap();
    let aid = asset.id;
    let dur = asset.probe.duration_us;
    project.assets.push(asset);
    let v1 = project.sequence(seq_id).unwrap().tracks.iter()
        .find(|t| t.kind == TrackKind::Video).unwrap().id;
    let mut store = ProjectStore::new(project);
    // the whole recording as ONE clip — exactly the shape of the field report
    store.insert_clip(v1, Clip::new_media(aid, 0, dur, 0), InsertMode::Strict).unwrap();

    let export_range = |name: &str, from: i64, to: i64| -> (f64, f64) {
        let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
        let _ = std::fs::remove_file(&out);
        let settings = ExportSettings { range: Some((from, to)), ..Default::default() };
        let t0 = std::time::Instant::now();
        export_sequence(&store.project, seq_id, dir, &out, &settings).unwrap();
        let secs = t0.elapsed().as_secs_f64();
        let meta = ffprobe_json(&out);
        let d: f64 = meta["format"]["duration"].as_str().unwrap().parse().unwrap();
        (secs, d)
    };

    // the same 5-second clip, once at the start and once at 3.5 minutes in
    let (early_s, early_d) = export_range("ue-range-early.mp4", 2 * SEC, 7 * SEC);
    let (late_s, late_d) = export_range("ue-range-late.mp4", 210 * SEC, 215 * SEC);
    eprintln!("5 s range — early: {early_s:.1}s (out {early_d:.1}s) · late (3.5 min in): {late_s:.1}s (out {late_d:.1}s)");

    assert!((4.8..=5.3).contains(&early_d), "early piece is 5 s, was {early_d}");
    assert!((4.8..=5.3).contains(&late_d), "late piece is 5 s, was {late_d}");
    // the late one must not be dominated by its offset any more
    assert!(
        late_s < early_s * 3.0 + 2.0,
        "a late range still costs its offset: early {early_s:.1}s vs late {late_s:.1}s"
    );
}

/// A preview frame that never finishes must be KILLED, not waited on forever.
/// `Command::output()` used to block the calling thread for good and leave the
/// ffmpeg process alive; an agent retrying the tool then piled up processes
/// (~17 seen in the field) and pegged the machine.
#[test]
fn a_stalled_preview_frame_is_killed_not_waited_on() {
    if !ffmpeg_available() {
        return;
    }
    let before = ffmpeg_process_count();
    // a graph that produces frames forever and never satisfies `-frames:v 1`
    // would hang; here we simply prove the bound exists and reports itself
    let args: Vec<String> = vec![
        "-v".into(), "error".into(),
        "-f".into(), "lavfi".into(),
        // a source that never ends, and no frame limit → runs until killed
        "-i".into(), "testsrc2=size=64x64:rate=30".into(),
        "-f".into(), "null".into(), "-".into(),
    ];
    let t0 = std::time::Instant::now();
    let r = ue_export::preview::run_bounded_for_test(&args, std::time::Duration::from_secs(2));
    let secs = t0.elapsed().as_secs_f64();
    eprintln!("stalled ffmpeg: returned after {secs:.1}s → {r:?}");
    assert!(r.is_err(), "an endless render must fail, not hang");
    assert!(secs < 6.0, "it was killed at the deadline, took {secs:.1}s");

    // and no ffmpeg is left behind
    std::thread::sleep(std::time::Duration::from_millis(300));
    let after = ffmpeg_process_count();
    assert!(after <= before, "no ffmpeg leaked (before {before}, after {after})");
}

/// How many ffmpeg processes this machine currently has.
fn ffmpeg_process_count() -> usize {
    let out = Command::new("pgrep").args(["-f", "ffmpeg"]).output();
    out.map(|o| String::from_utf8_lossy(&o.stdout).lines().count()).unwrap_or(0)
}

/// EMOJI — STILL BROKEN, and this test says so out loud.
///
/// Moving to libass fixed the *font fallback* problem (`drawtext` loads exactly
/// one face and draws .notdef boxes for anything it lacks), and fontconfig here
/// does resolve "Apple Color Emoji". But libass/FreeType in this ffmpeg build
/// cannot rasterise it: Apple Color Emoji is an `sbix` BITMAP font, and colour
/// bitmap glyphs come out as boxes. Measured: 0 coloured pixels.
///
/// The check is for COLOUR, deliberately. An earlier version of this test only
/// counted "more ink than without the emoji" — which a .notdef box satisfies
/// perfectly, so it passed while the frame showed two empty rectangles. A test
/// that can be satisfied by the bug is worse than no test.
///
/// Real fixes: ship a CBDT/COLR colour font (Noto Color Emoji) and confirm the
/// bundled libass rasterises it, or rasterise text ourselves with a shaping
/// engine (cosmic-text, PLAN §6.6) instead of handing it to ffmpeg at all.
#[test]
fn a_title_with_an_emoji_renders_colour_glyphs() {
    let Some(dir) = media_dir() else { return };
    let bgv = dir.join("emoji_black.mp4");
    if !bgv.exists() {
        Command::new(ue_media::ffmpeg_bin())
            .args(["-y", "-v", "error", "-f", "lavfi", "-i", "color=black:s=1280x720:r=30:d=2",
                   "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p"])
            .arg(&bgv).status().unwrap();
    }
    let mut project = Project::new("emoji");
    let seq_id = project.active_sequence;
    project.sequence_mut(seq_id).unwrap().resolution = (1280, 720);
    let bg = ue_media::import_file(&bgv).unwrap();
    let bgid = bg.id;
    project.assets.push(bg);
    let seq = project.sequence_mut(seq_id).unwrap();
    seq.tracks.push(Track::new(TrackKind::Video, "V2"));
    let v1 = seq.tracks.iter().find(|t| t.name == "V1").unwrap().id;
    let v2 = seq.tracks.iter().find(|t| t.name == "V2").unwrap().id;
    let mut store = ProjectStore::new(project);
    store.insert_clip(v1, Clip::new_media(bgid, 0, 2 * SEC, 0), InsertMode::Strict).unwrap();
    let mut t = Clip::new_text("HOLA \u{1F3AC}\u{1F525}", 0, 2 * SEC);
    if let ClipPayload::Text { style, .. } = &mut t.payload {
        style.size = 90.0;
    }
    store.insert_clip(v2, t, InsertMode::Strict).unwrap();
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-emoji.mp4");
    let _ = std::fs::remove_file(&out);
    export_sequence(&store.project, seq_id, dir, &out, &ExportSettings::default()).unwrap();
    let buf = Command::new(ue_media::ffmpeg_bin())
        .args(["-v", "error", "-ss", "1", "-i"]).arg(&out)
        .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "pipe:1"])
        .output().unwrap().stdout;

    // a real emoji is COLOURED; a .notdef box is white, like the rest of the text
    let coloured = buf
        .chunks_exact(3)
        .filter(|p| {
            let (mx, mn) = (
                p[0].max(p[1]).max(p[2]) as i32,
                p[0].min(p[1]).min(p[2]) as i32,
            );
            mx > 90 && mx - mn > 60
        })
        .count();
    eprintln!("coloured pixels in the exported frame: {coloured}");
    assert!(coloured > 200, "the emoji rendered as colour glyphs, not boxes ({coloured} px)");
}

/// A LONG karaoke transcript used to blow the filtergraph past ffmpeg's parser
/// ("the subtitle filtergraph is too large (1263 KB)"). With libass the whole
/// transcript lives in a FILE, so the graph size no longer depends on it.
#[test]
fn a_long_karaoke_transcript_no_longer_explodes_the_filtergraph() {
    // 20 minutes of dense speech: ~6000 words. The old path emitted TWO
    // drawtext per word — well over a megabyte of filtergraph.
    let asset = fake_asset(MediaKind::Video, "long.mp4", 1200, true);
    let aid = asset.id;
    let mut project = Project::new("kbig");
    let seq_id = project.active_sequence;
    project.sequence_mut(seq_id).unwrap().resolution = (640, 360);
    project.assets.push(asset);
    let words: Vec<Word> = (0..6000)
        .map(|i| Word {
            text: format!("palabra{i}"),
            start_us: i as i64 * 200_000,
            end_us: i as i64 * 200_000 + 180_000,
            confidence: 1.0,
            rejected: false,
            display: None,
        })
        .collect();
    let doc = TranscriptDoc {
        id: Id::new(),
        asset_id: aid,
        language: "es".into(),
        model: "t".into(),
        segments: vec![],
        words,
        global_avg_volume: 0.0,
    };
    let doc_id = doc.id;
    project.transcripts.push(doc);
    let seq = project.sequence_mut(seq_id).unwrap();
    seq.tracks.push(Track::new(TrackKind::Video, "V2"));
    let v1 = seq.tracks.iter().find(|t| t.name == "V1").unwrap().id;
    let v2 = seq.tracks.iter().find(|t| t.name == "V2").unwrap().id;
    let mut store = ProjectStore::new(project);
    store.insert_clip(v1, Clip::new_media(aid, 0, 1200 * SEC, 0), InsertMode::Strict).unwrap();
    let sub = Clip {
        id: Id::new(),
        payload: ClipPayload::Subtitles {
            transcript_id: doc_id,
            style: TextStyle { size: 40.0, highlight_color: Some("#FFB224".into()), ..Default::default() },
            mode: SubtitleMode::Karaoke,
            max_words: None,
        },
        start: 0,
        duration: 1200 * SEC,
        speed: 1.0,
        effects: vec![],
        transform: Default::default(),
        audio: Default::default(),
        transition_in: None,
        transition_out: None,
        label_color: None,
        name: None,
        group: None,
    };
    store.insert_clip(v2, sub, InsertMode::Strict).unwrap();

    // only the PLAN: what matters is that the graph stays small no matter how
    // big the transcript is (rendering 20 minutes here would prove nothing)
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-kbig.mp4");
    let plan = ue_export::graph::build_ffmpeg_args(
        &store.project, seq_id, Path::new("."), &out, &ExportSettings::default(),
    )
    .expect("a 6000-word karaoke must not be rejected any more");
    let graph_len = plan
        .args
        .iter()
        .find(|a| a.contains("[vout]"))
        .map(|a| a.len())
        .unwrap_or(0);
    eprintln!("filtergraph with a 6000-word karaoke: {graph_len} bytes");
    assert!(graph_len < 20_000, "the graph no longer scales with the transcript ({graph_len} bytes)");
    let p = plan.subs_file.expect("an ASS script was written");
    let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
    let script = std::fs::read_to_string(&p).unwrap_or_default();
    eprintln!("…and the ASS script carrying it: {size} bytes, {} events", script.matches("Dialogue:").count());
    assert!(size > 100_000, "the whole transcript really is in the file ({size} bytes)");
    assert!(script.contains("\\k"), "karaoke uses ASS \\k timing tags");
    let _ = std::fs::remove_file(&p);
}

/// A transition must NEVER be a silent no-op: with a single clip (no previous
/// neighbour, no handles) it runs as an ENTRANCE from black on the base, and
/// on an upper layer it enters from TRANSPARENT over the tracks below —
/// identical in the export and the paused preview.
#[test]
fn entrance_transition_runs_with_a_single_clip_and_on_layers() {
    let Some(dir) = media_dir() else { return };
    for (name, color) in [("xf_red.mp4", "red"), ("xf_blue.mp4", "blue")] {
        let out = dir.join(name);
        if !out.exists() {
            let st = Command::new(ue_media::ffmpeg_bin())
                .args([
                    "-y", "-v", "error",
                    "-f", "lavfi", "-i",
                    &format!("color={color}:size=640x360:rate=30:duration=3"),
                    "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p",
                ])
                .arg(&out)
                .status()
                .unwrap();
            assert!(st.success());
        }
    }

    // ---- base track, SINGLE clip with a 1 s crossfade: enters from black ----
    let mut project = Project::new("entrance-base");
    let seq_id = project.active_sequence;
    let red = ue_media::import_file(&dir.join("xf_red.mp4")).unwrap();
    let rid = red.id;
    project.assets.push(red);
    let v1 = project
        .sequence(seq_id)
        .unwrap()
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Video)
        .unwrap()
        .id;
    let mut store = ProjectStore::new(project);
    let mut a = Clip::new_media(rid, 0, 2 * SEC, 0);
    a.transition_in = Some(TransitionRef {
        effect_id: "core.crossfade".into(),
        duration: 1 * SEC,
        params: Default::default(),
    });
    store.insert_clip(v1, a, InsertMode::Strict).unwrap();

    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-entrance-base.mp4");
    let _ = std::fs::remove_file(&out);
    export_sequence(&store.project, seq_id, dir, &out, &ExportSettings::default()).unwrap();
    let (r0, _, _) = pixel_at(&out, 0.05, 960, 540);
    let (rm, _, _) = pixel_at(&out, 0.5, 960, 540);
    let (rf, _, _) = pixel_at(&out, 1.5, 960, 540);
    eprintln!("base entrance: r(0.05)={r0} r(0.5)={rm} r(1.5)={rf}");
    assert!(r0 < 60, "at p≈0 the frame is still black, was r={r0}");
    assert!((60..=200).contains(&(rm as i32)), "at p=0.5 the fade is half red, was r={rm}");
    assert!(rf > 180, "after the entrance the clip is fully there, was r={rf}");

    // paused preview at the same instant matches the export
    let jpeg = ue_export::preview::render_preview_frame(&store.project, seq_id, dir, 500_000, 960, &[])
        .unwrap()
        .unwrap();
    std::fs::write(dir.join("entrance-prev.jpg"), &jpeg).unwrap();
    let prev = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-entrance-prev.png");
    Command::new(ue_media::ffmpeg_bin())
        .args(["-y", "-v", "error", "-i"])
        .arg(dir.join("entrance-prev.jpg"))
        .args(["-vf", "scale=960:540"])
        .arg(&prev)
        .status()
        .unwrap();
    let (pr, _, _) = pixel_at_w(&prev, 0.0, 480, 270, 960);
    eprintln!("base entrance paused: r(0.5)={pr} vs export {rm}");
    assert!((pr as i32 - rm as i32).abs() <= 40, "paused ({pr}) diverges from export ({rm})");

    // ---- upper layer with a 1 s crossfade: enters from TRANSPARENT ----
    let mut project = Project::new("entrance-layer");
    let seq_id = project.active_sequence;
    let red = ue_media::import_file(&dir.join("xf_red.mp4")).unwrap();
    let blue = ue_media::import_file(&dir.join("xf_blue.mp4")).unwrap();
    let (rid, bid) = (red.id, blue.id);
    project.assets.push(red);
    project.assets.push(blue);
    let seq = project.sequence_mut(seq_id).unwrap();
    seq.tracks.push(Track::new(TrackKind::Video, "V2"));
    let v1 = seq.tracks.iter().find(|t| t.kind == TrackKind::Video).unwrap().id;
    let v2 = seq.tracks.iter().filter(|t| t.kind == TrackKind::Video).nth(1).unwrap().id;
    let mut store = ProjectStore::new(project);
    store.insert_clip(v1, Clip::new_media(rid, 0, 2 * SEC, 0), InsertMode::Strict).unwrap();
    let mut b = Clip::new_media(bid, 0, 2 * SEC, 0);
    b.transition_in = Some(TransitionRef {
        effect_id: "core.crossfade".into(),
        duration: 1 * SEC,
        params: Default::default(),
    });
    store.insert_clip(v2, b, InsertMode::Strict).unwrap();

    let out2 = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-entrance-layer.mp4");
    let _ = std::fs::remove_file(&out2);
    export_sequence(&store.project, seq_id, dir, &out2, &ExportSettings::default()).unwrap();
    // centre: the blue PiP over the red base — half blended at p=0.5, blue after
    let (mr, _, mb) = pixel_at(&out2, 0.5, 960, 540);
    let (fr_, _, fb) = pixel_at(&out2, 1.5, 960, 540);
    eprintln!("layer entrance: p=0.5 rgb=({mr},{mb}) after=({fr_},{fb})");
    assert!((60..=200).contains(&(mr as i32)) && (60..=200).contains(&(mb as i32)),
        "p=0.5 blends red base + blue layer, was ({mr},{mb})");
    assert!(fb > 150 && fr_ < 90, "after the entrance the layer is opaque blue, was ({fr_},{fb})");

    // paused preview parity at p=0.5
    let jpeg = ue_export::preview::render_preview_frame(&store.project, seq_id, dir, 500_000, 960, &[])
        .unwrap()
        .unwrap();
    std::fs::write(dir.join("entrance-prev.jpg"), &jpeg).unwrap();
    Command::new(ue_media::ffmpeg_bin())
        .args(["-y", "-v", "error", "-i"])
        .arg(dir.join("entrance-prev.jpg"))
        .args(["-vf", "scale=960:540"])
        .arg(&prev)
        .status()
        .unwrap();
    let (qr, _, qb) = pixel_at_w(&prev, 0.0, 480, 270, 960);
    eprintln!("layer entrance paused: ({qr},{qb}) vs export ({mr},{mb})");
    assert!((qr as i32 - mr as i32).abs() <= 40 && (qb as i32 - mb as i32).abs() <= 40,
        "paused layer entrance diverges: preview=({qr},{qb}) export=({mr},{mb})");
    let _ = std::fs::remove_file(dir.join("entrance-prev.jpg"));
}

/// Exit transitions mirror the entrances: the clip's tail leaves to black on
/// the base track (single clip included) and to transparent on layers,
/// identical in the export and the paused preview.
#[test]
fn exit_transition_runs_on_base_and_layers() {
    let Some(dir) = media_dir() else { return };
    for (name, color) in [("xf_red.mp4", "red"), ("xf_blue.mp4", "blue")] {
        let out = dir.join(name);
        if !out.exists() {
            let st = Command::new(ue_media::ffmpeg_bin())
                .args([
                    "-y", "-v", "error",
                    "-f", "lavfi", "-i",
                    &format!("color={color}:size=640x360:rate=30:duration=3"),
                    "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p",
                ])
                .arg(&out)
                .status()
                .unwrap();
            assert!(st.success());
        }
    }

    // base track, single red clip with a 1 s crossfade OUT: leaves to black
    let mut project = Project::new("exit-base");
    let seq_id = project.active_sequence;
    let red = ue_media::import_file(&dir.join("xf_red.mp4")).unwrap();
    let blue = ue_media::import_file(&dir.join("xf_blue.mp4")).unwrap();
    let (rid, bid) = (red.id, blue.id);
    project.assets.push(red);
    project.assets.push(blue);
    let seq = project.sequence_mut(seq_id).unwrap();
    seq.tracks.push(Track::new(TrackKind::Video, "V2"));
    let v1 = seq.tracks.iter().find(|t| t.kind == TrackKind::Video).unwrap().id;
    let v2 = seq.tracks.iter().filter(|t| t.kind == TrackKind::Video).nth(1).unwrap().id;
    let mut store = ProjectStore::new(project);
    let mut a = Clip::new_media(rid, 0, 2 * SEC, 0);
    a.transition_out = Some(TransitionRef {
        effect_id: "core.crossfade".into(),
        duration: 1 * SEC,
        params: Default::default(),
    });
    store.insert_clip(v1, a, InsertMode::Strict).unwrap();
    // blue layer with a wipe-out over the tail
    let mut b = Clip::new_media(bid, 0, 2 * SEC, 0);
    b.transition_out = Some(TransitionRef {
        effect_id: "core.crossfade".into(),
        duration: 1 * SEC,
        params: Default::default(),
    });
    store.insert_clip(v2, b, InsertMode::Strict).unwrap();

    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-exit.mp4");
    let _ = std::fs::remove_file(&out);
    export_sequence(&store.project, seq_id, dir, &out, &ExportSettings::default()).unwrap();
    // centre pixel: blue layer over red base. At 0.5 s both are full → blue.
    // At 1.5 s both are half-left: blue@50% over red@50% over black.
    let (er, _, eb) = pixel_at(&out, 0.5, 960, 540);
    let (mr, _, mb) = pixel_at(&out, 1.5, 960, 540);
    let (fr_, _, fb) = pixel_at(&out, 1.95, 960, 540);
    eprintln!("exit: full=({er},{eb}) mid=({mr},{mb}) end=({fr_},{fb})");
    assert!(eb > 150 && er < 90, "before the exit the layer is opaque blue, was ({er},{eb})");
    assert!(mb < 150 && mb > 30, "half-way out the blue is fading, was ({mr},{mb})");
    assert!(fr_ < 70 && fb < 70, "at the very end everything left to black, was ({fr_},{fb})");

    // paused preview parity at 1.5 s
    let jpeg = ue_export::preview::render_preview_frame(&store.project, seq_id, dir, 1_500_000, 960, &[])
        .unwrap()
        .unwrap();
    std::fs::write(dir.join("exit-prev.jpg"), &jpeg).unwrap();
    let prev = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-exit-prev.png");
    Command::new(ue_media::ffmpeg_bin())
        .args(["-y", "-v", "error", "-i"])
        .arg(dir.join("exit-prev.jpg"))
        .args(["-vf", "scale=960:540"])
        .arg(&prev)
        .status()
        .unwrap();
    let (pr, _, pb) = pixel_at_w(&prev, 0.0, 480, 270, 960);
    eprintln!("exit paused: ({pr},{pb}) vs export ({mr},{mb})");
    assert!(
        (pr as i32 - mr as i32).abs() <= 40 && (pb as i32 - mb as i32).abs() <= 40,
        "paused exit diverges: preview=({pr},{pb}) export=({mr},{mb})"
    );
    let _ = std::fs::remove_file(dir.join("exit-prev.jpg"));
}

/// Manual repro: layer with position transform + chroma key + circleopen
/// entrance. Point UE_REPRO_AVATAR at a real avatar .mov to run it; it dumps
/// the paused frame for visual inspection and is skipped otherwise.
#[test]
fn repro_positioned_layer_circle_entrance() {
    let Ok(av_path) = std::env::var("UE_REPRO_AVATAR") else { return };
    let av = Path::new(&av_path);
    if !av.exists() { return; }
    let Some(dir) = media_dir() else { return };
    let mut project = Project::new("repro");
    let seq_id = project.active_sequence;
    let avatar = ue_media::import_file(av).unwrap();
    let aid = avatar.id;
    project.assets.push(avatar);
    let seq = project.sequence_mut(seq_id).unwrap();
    seq.tracks.push(Track::new(TrackKind::Video, "V2"));
    let v2 = seq.tracks.iter().filter(|t| t.kind == TrackKind::Video).nth(1).unwrap().id;
    let mut store = ProjectStore::new(project);
    // base clip on V1 so the avatar is a LAYER (like the real project)
    let base = ue_media::import_file(&dir.join("xf_red.mp4")).unwrap();
    let bid_base = base.id;
    store.project.assets.push(base);
    let v1 = store.project.sequence(seq_id).unwrap().tracks.iter().find(|t| t.kind == TrackKind::Video).unwrap().id;
    store.insert_clip(v1, Clip::new_media(bid_base, 0, 2_200_000, 0), InsertMode::Strict).unwrap();
    let mut c = Clip::new_media(aid, 0, 2_200_000, 0);
    c.transform.position = (762.0.into(), 298.0.into());
    c.effects.push(ue_core::model::EffectInstance {
        effect_id: "core.chroma_key".into(),
        enabled: true,
        params: [("similarity".to_string(), 0.2179.into()), ("blend".to_string(), 0.08.into()), ("despill".to_string(), 0.69.into())].into_iter().collect(),
        color_params: [("key_color".to_string(), "#00ff00".to_string())].into_iter().collect(),
    });
    c.transition_in = Some(TransitionRef {
        effect_id: "core.circleopen".into(),
        duration: 1_300_000,
        params: Default::default(),
    });
    store.insert_clip(v2, c, InsertMode::Strict).unwrap();

    let jpeg = ue_export::preview::render_preview_frame(&store.project, seq_id, dir, 650_000, 1280, &[])
        .unwrap()
        .unwrap();
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue_repro_circle.jpg");
    std::fs::write(&out, &jpeg).unwrap();
    eprintln!("wrote {} ({} bytes)", out.display(), jpeg.len());
}
