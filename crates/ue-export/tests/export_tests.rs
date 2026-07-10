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
            Segment::Source { asset_id: a, src_in: 1 * SEC, src_out: 3 * SEC, speed: 1.0, vf: None, transition_in: None },
            Segment::Source { asset_id: b, src_in: 4 * SEC, src_out: 6 * SEC, speed: 1.0, vf: None, transition_in: None },
            Segment::Black { duration: 1 * SEC },
            Segment::Source { asset_id: a, src_in: 8 * SEC, src_out: 9 * SEC, speed: 1.0, vf: None, transition_in: None },
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
            Segment::Source { asset_id: a, src_in: 0, src_out: 2 * SEC, speed: 1.0, vf: None, transition_in: None },
            Segment::Source { asset_id: b, src_in: 0, src_out: 2 * SEC, speed: 1.0, vf: None, transition_in: None },
            Segment::Source { asset_id: a, src_in: 4 * SEC, src_out: 6 * SEC, speed: 1.0, vf: None, transition_in: None },
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
        vec![Segment::Source { asset_id: a, src_in: 1 * SEC, src_out: 5 * SEC, speed: 1.0, vf: None, transition_in: None }]
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
                    params: Default::default(),
                }),
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
                    params: Default::default(),
                }),
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
                    params: Default::default(),
                }),
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
        },
        start: 0,
        duration: 3 * SEC,
        speed: 1.0,
        effects: vec![],
        transform: Default::default(),
        audio: Default::default(),
        transition_in: None,
        label_color: None,
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

// ---------------------------------------------------------------------------
// Reactive avatar (movie+overlay per segment)
// ---------------------------------------------------------------------------

/// Avatar with two emotions (calm=blue, angry=red) over the video: the bottom-right
/// corner shows blue during the calm segment and red during angry;
/// the rest of the frame is still the base video.
#[test]
fn avatar_overlay_switches_emotion_per_segment() {
    let Some(dir) = media_dir() else { return };
    for (name, color) in [("avatar_calm.mp4", "blue"), ("avatar_angry.mp4", "red")] {
        let st = Command::new(ue_media::ffmpeg_bin())
            .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
            .arg(format!("color=c={color}:s=160x120:d=2:r=24"))
            .args(["-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p"])
            .arg(dir.join(name))
            .status()
            .unwrap();
        assert!(st.success());
    }

    let mut project = Project::new("avatar-test");
    let seq_id = project.active_sequence;
    let asset = ue_media::import_file(&dir.join("counter.mp4")).unwrap();
    let aid = asset.id;
    project.assets.push(asset);
    // transcript with emotions already classified
    project.transcripts.push(TranscriptDoc {
        id: Id::new(),
        asset_id: aid,
        language: "es".into(),
        model: "t".into(),
        words: vec![],
        segments: vec![
            ue_core::model::Segment {
                text: "calm".into(),
                start_us: 200_000,
                end_us: 1_800_000,
                word_range: (0, 0),
                emotion: Some("calm".into()),
                volume_rms: 1.0,
            },
            ue_core::model::Segment {
                text: "angry".into(),
                start_us: 2_200_000,
                end_us: 3_800_000,
                word_range: (0, 0),
                emotion: Some("angry".into()),
                volume_rms: 1.5,
            },
        ],
        global_avg_volume: 1.2,
    });
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
    store.insert_clip(v1, Clip::new_media(aid, 0, 4 * SEC, 0), InsertMode::Strict).unwrap();
    let mut avatars = std::collections::BTreeMap::new();
    avatars.insert("calm".to_string(), dir.join("avatar_calm.mp4").to_string_lossy().into_owned());
    avatars.insert("angry".to_string(), dir.join("avatar_angry.mp4").to_string_lossy().into_owned());
    let avatar = Clip {
        id: Id::new(),
        payload: ClipPayload::Avatar { driver_asset: aid, avatars, shake_factor: 1.0, scale: 0.3 },
        start: 0,
        duration: 4 * SEC,
        speed: 1.0,
        effects: vec![],
        transform: Default::default(),
        audio: Default::default(),
        transition_in: None,
        label_color: None,
        group: None,
    };
    store.insert_clip(v2, avatar, InsertMode::Strict).unwrap();

    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-avatar-out.mp4");
    export_sequence(&store.project, seq_id, dir, &out, &ExportSettings::default()).unwrap();

    // avatar: aw=576, x∈[1320,1896], height=432 → y∈[624,1056]; sample the center
    let sample = |t: f64| pixel_at(&out, t, 1600, 850);
    let (r, g, b) = sample(1.0); // calm → blue
    assert!(b > 120 && r < 90, "calm=blue at t=1: rgb({r},{g},{b})");
    let (r, g, b) = sample(3.0); // angry → red
    assert!(r > 120 && b < 90, "angry=red at t=3: rgb({r},{g},{b})");
    // outside the avatar the base video is still visible (testsrc: not black)
    let (r, g, b) = pixel_at(&out, 1.0, 400, 300);
    assert!(r as u32 + g as u32 + b as u32 > 100, "the base video is still underneath");
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
        },
        start: 0,
        duration: 3 * SEC,
        speed: 1.0,
        effects: vec![],
        transform: Default::default(),
        audio: Default::default(),
        transition_in: None,
        label_color: None,
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
        },
        start: 0,
        duration: 3 * SEC,
        speed: 1.0,
        effects: vec![],
        transform: Default::default(),
        audio: Default::default(),
        transition_in: None,
        label_color: None,
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
        },
        start: 0,
        duration: 4 * SEC,
        speed: 1.0,
        effects: vec![],
        transform: Default::default(),
        audio: Default::default(),
        transition_in: None,
        label_color: None,
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
        },
        start: 0,
        duration: 2 * SEC,
        speed: 1.0,
        effects: vec![],
        transform: Default::default(),
        audio: Default::default(),
        transition_in: None,
        label_color: None,
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
        AvatarExpression { name: "calm".into(), path: "c.png".into(), description: String::new() },
        AvatarExpression { name: "angry".into(), path: "a.png".into(), description: String::new() },
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
        AvatarExpression {
            name: "calm".into(),
            path: calm.to_string_lossy().into_owned(),
            description: "neutral".into(),
        },
        AvatarExpression {
            name: "angry".into(),
            path: angry.to_string_lossy().into_owned(),
            description: "furious, upset".into(),
        },
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
