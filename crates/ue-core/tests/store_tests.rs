//! ProjectStore integration tests: split/trim/move/delete/cut_ranges,
//! atomicity, undo/redo, and the property "full undo ≡ initial state".

use ue_core::action::{Action, TrackProp};
use ue_core::keyframe::{Interp, Keyframe, KeyframeCurve, Param};
use ue_core::model::*;
use ue_core::ops::InsertMode;
use ue_core::time::US_PER_SEC;
use ue_core::validate::validate;
use ue_core::ProjectStore;

const SEC: i64 = US_PER_SEC;

/// Fixture project: 1 video asset of 60 s, 1 audio asset of 120 s,
/// a 30 fps sequence with V1/A1 (and the asset already in the pool).
/// Returns (store, seq_id, video_track, audio_track, video_asset, audio_asset).
fn fixture() -> (ProjectStore, Id, Id, Id, Id, Id) {
    let mut p = Project::new("Test");
    let seq_id = p.active_sequence;
    let video_asset = MediaAsset {
        id: Id::new(),
        kind: MediaKind::Video,
        path: "media/video.mp4".into(),
        content_hash: "xxh3:v".into(),
        probe: ProbeInfo {
            duration_us: 60 * SEC,
            fps: Some((30, 1)),
            width: 1920,
            height: 1080,
            rotation: 0,
            vcodec: Some("h264".into()),
            acodec: Some("aac".into()),
            audio_channels: 2,
            vfr: false,
        },
        proxy: None,
        audio_conform: None,
        peaks: None,
        thumbnails: None,
        transcript: None,
        offline: false,
    };
    let audio_asset = MediaAsset {
        id: Id::new(),
        kind: MediaKind::Audio,
        path: "media/music.mp3".into(),
        content_hash: "xxh3:a".into(),
        probe: ProbeInfo {
            duration_us: 120 * SEC,
            fps: None,
            width: 0,
            height: 0,
            rotation: 0,
            vcodec: None,
            acodec: Some("mp3".into()),
            audio_channels: 2,
            vfr: false,
        },
        proxy: None,
        audio_conform: None,
        peaks: None,
        thumbnails: None,
        transcript: None,
        offline: false,
    };
    let va = video_asset.id;
    let aa = audio_asset.id;
    p.assets.push(video_asset);
    p.assets.push(audio_asset);
    let seq = p.sequence(seq_id).unwrap();
    let audio_track = seq.tracks.iter().find(|t| t.kind == TrackKind::Audio).unwrap().id;
    let video_track = seq.tracks.iter().find(|t| t.kind == TrackKind::Video).unwrap().id;
    (ProjectStore::new(p), seq_id, video_track, audio_track, va, aa)
}

fn media_src(clip: &Clip) -> (i64, i64) {
    match &clip.payload {
        ClipPayload::Media { src_in, src_out, .. } => (*src_in, *src_out),
        _ => panic!("not media"),
    }
}

#[test]
fn insert_split_undo_roundtrip() {
    let (mut store, _seq, vtrack, _atrack, va, _aa) = fixture();
    let clip = Clip::new_media(va, 0, 10 * SEC, 0);
    store.insert_clip(vtrack, clip, InsertMode::Strict).unwrap();
    let snapshot = store.project.clone();

    let clip_id = store.project.track(vtrack).unwrap().clips[0].id;
    let (l, r) = store.split_clip(clip_id, 4 * SEC).unwrap();

    {
        let track = store.project.track(vtrack).unwrap();
        assert_eq!(track.clips.len(), 2);
        let (lc, rc) = (&track.clips[0], &track.clips[1]);
        assert_eq!((lc.id, rc.id), (l, r));
        assert_eq!((lc.start, lc.duration), (0, 4 * SEC));
        assert_eq!((rc.start, rc.duration), (4 * SEC, 6 * SEC));
        assert_eq!(media_src(lc), (0, 4 * SEC));
        assert_eq!(media_src(rc), (4 * SEC, 10 * SEC));
    }

    store.undo().unwrap();
    assert_eq!(store.project, snapshot, "undo must restore byte for byte");

    store.redo().unwrap();
    let track = store.project.track(vtrack).unwrap();
    assert_eq!(track.clips.len(), 2);
    assert_eq!(track.clips[0].id, l, "redo reuses the same ids");
}

#[test]
fn split_quantizes_to_frame() {
    let (mut store, _seq, vtrack, _at, va, _aa) = fixture();
    let clip = Clip::new_media(va, 0, 10 * SEC, 0);
    let clip_id = store.insert_clip(vtrack, clip, InsertMode::Strict).unwrap();
    // 1.017 s is not a frame boundary at 30 fps → quantizes to the nearest frame (31)
    store.split_clip(clip_id, 1_017_000).unwrap();
    let track = store.project.track(vtrack).unwrap();
    let boundary = track.clips[1].start;
    assert_eq!(boundary, ue_core::time::frame_to_time(31, (30, 1)), "cut at frame 31");
    assert_eq!(
        ue_core::time::quantize_to_frame(boundary, (30, 1)),
        boundary,
        "the boundary is idempotent under quantization"
    );
}

#[test]
fn split_keyframes_preserve_boundary_value() {
    let (mut store, _seq, vtrack, _at, va, _aa) = fixture();
    let mut clip = Clip::new_media(va, 0, 10 * SEC, 0);
    clip.transform.opacity = Param::Curve(KeyframeCurve::new(vec![
        Keyframe { t: 0, value: 0.0, interp: Interp::Linear },
        Keyframe { t: 10 * SEC, value: 1.0, interp: Interp::Linear },
    ]));
    let clip_id = store.insert_clip(vtrack, clip, InsertMode::Strict).unwrap();
    let (l, r) = store.split_clip(clip_id, 4 * SEC).unwrap();
    let track = store.project.track(vtrack).unwrap();
    let lc = track.clips.iter().find(|c| c.id == l).unwrap();
    let rc = track.clips.iter().find(|c| c.id == r).unwrap();
    // at the boundary both halves equal 0.4
    assert!((lc.transform.opacity.eval(4 * SEC) - 0.4).abs() < 1e-9);
    assert!((rc.transform.opacity.eval(0) - 0.4).abs() < 1e-9);
    // and the right one still reaches 1.0 at its end
    assert!((rc.transform.opacity.eval(6 * SEC) - 1.0).abs() < 1e-9);
}

#[test]
fn ripple_delete_closes_gap() {
    let (mut store, _seq, vtrack, _at, va, _aa) = fixture();
    let a = store.insert_clip(vtrack, Clip::new_media(va, 0, 2 * SEC, 0), InsertMode::Strict).unwrap();
    let _b = store.insert_clip(vtrack, Clip::new_media(va, 0, 3 * SEC, 2 * SEC), InsertMode::Strict).unwrap();
    let c = store.insert_clip(vtrack, Clip::new_media(va, 0, 1 * SEC, 5 * SEC), InsertMode::Strict).unwrap();

    // delete the middle clip with ripple
    let b_id = store.project.track(vtrack).unwrap().clips[1].id;
    store.delete_clips(&[b_id], true).unwrap();

    let track = store.project.track(vtrack).unwrap();
    assert_eq!(track.clips.len(), 2);
    assert_eq!(track.clips[0].id, a);
    assert_eq!(track.clips[1].id, c);
    assert_eq!(track.clips[1].start, 2 * SEC, "c shifts 3 s to the left");
    assert!(validate(&store.project).is_empty());
}

#[test]
fn delete_without_ripple_leaves_gap() {
    let (mut store, _seq, vtrack, _at, va, _aa) = fixture();
    store.insert_clip(vtrack, Clip::new_media(va, 0, 2 * SEC, 0), InsertMode::Strict).unwrap();
    let b = store.insert_clip(vtrack, Clip::new_media(va, 0, 3 * SEC, 2 * SEC), InsertMode::Strict).unwrap();
    let c = store.insert_clip(vtrack, Clip::new_media(va, 0, 1 * SEC, 5 * SEC), InsertMode::Strict).unwrap();
    store.delete_clips(&[b], false).unwrap();
    let track = store.project.track(vtrack).unwrap();
    assert_eq!(track.clips.len(), 2);
    let cc = track.clips.iter().find(|cl| cl.id == c).unwrap();
    assert_eq!(cc.start, 5 * SEC, "without ripple, c doesn't move");
}

#[test]
fn overwrite_insert_carves_middle() {
    let (mut store, _seq, vtrack, _at, va, _aa) = fixture();
    // large clip [0, 10s)
    store.insert_clip(vtrack, Clip::new_media(va, 0, 10 * SEC, 0), InsertMode::Strict).unwrap();
    // overwrite in the middle [4s, 6s)
    let new_clip = Clip::new_media(va, 20 * SEC, 22 * SEC, 4 * SEC);
    let new_id = store.insert_clip(vtrack, new_clip, InsertMode::Overwrite).unwrap();

    let track = store.project.track(vtrack).unwrap();
    assert_eq!(track.clips.len(), 3, "left + new + right");
    let (l, m, r) = (&track.clips[0], &track.clips[1], &track.clips[2]);
    assert_eq!((l.start, l.end()), (0, 4 * SEC));
    assert_eq!(m.id, new_id);
    assert_eq!((m.start, m.end()), (4 * SEC, 6 * SEC));
    assert_eq!((r.start, r.end()), (6 * SEC, 10 * SEC));
    // the right clip's source material advanced: [6s, 10s) of the file
    assert_eq!(media_src(r), (6 * SEC, 10 * SEC));
    assert!(validate(&store.project).is_empty());

    // undo brings back the single clip
    store.undo().unwrap();
    assert_eq!(store.project.track(vtrack).unwrap().clips.len(), 1);
}

#[test]
fn overwrite_insert_trims_edges() {
    let (mut store, _seq, vtrack, _at, va, _aa) = fixture();
    store.insert_clip(vtrack, Clip::new_media(va, 0, 4 * SEC, 0), InsertMode::Strict).unwrap();
    store.insert_clip(vtrack, Clip::new_media(va, 0, 4 * SEC, 6 * SEC), InsertMode::Strict).unwrap();
    // overwrite [3s, 7s): trims the end of the first and the start of the second
    store
        .insert_clip(vtrack, Clip::new_media(va, 30 * SEC, 34 * SEC, 3 * SEC), InsertMode::Overwrite)
        .unwrap();
    let track = store.project.track(vtrack).unwrap();
    assert_eq!(track.clips.len(), 3);
    assert_eq!((track.clips[0].start, track.clips[0].end()), (0, 3 * SEC));
    assert_eq!((track.clips[1].start, track.clips[1].end()), (3 * SEC, 7 * SEC));
    assert_eq!((track.clips[2].start, track.clips[2].end()), (7 * SEC, 10 * SEC));
    // the third clip's src_in advanced 1 s
    assert_eq!(media_src(&track.clips[2]).0, 1 * SEC);
}

#[test]
fn trim_respects_source_material() {
    let (mut store, _seq, vtrack, _at, va, _aa) = fixture();
    // clip that uses [5s, 10s) of the file, placed at t=20s
    let clip_id = store
        .insert_clip(vtrack, Clip::new_media(va, 5 * SEC, 10 * SEC, 20 * SEC), InsertMode::Strict)
        .unwrap();
    // try to extend the left edge to t=0: only 5 s of handle → clamps to 15 s
    store.trim_clip(clip_id, true, 0).unwrap();
    let clip = store.project.clip(clip_id).unwrap();
    assert_eq!(clip.start, 15 * SEC, "the edge stops where the material runs out");
    assert_eq!(media_src(clip).0, 0, "src_in reached the start of the file");
    assert_eq!(clip.duration, 10 * SEC);

    // extend the right edge beyond the file (60 s asset)
    store.trim_clip(clip_id, false, 500 * SEC).unwrap();
    let clip = store.project.clip(clip_id).unwrap();
    assert_eq!(media_src(clip).1, 60 * SEC, "src_out clamped to the asset duration");
}

#[test]
fn cut_ranges_multitrack_ripple() {
    let (mut store, seq, vtrack, atrack, va, aa) = fixture();
    store.insert_clip(vtrack, Clip::new_media(va, 0, 10 * SEC, 0), InsertMode::Strict).unwrap();
    store.insert_clip(atrack, Clip::new_media(aa, 0, 10 * SEC, 0), InsertMode::Strict).unwrap();

    // cut [2s,3s) and [5s,6s) — overlap/merge included
    store.cut_ranges(seq, &[(2 * SEC, 3 * SEC), (5 * SEC, 6 * SEC)], true).unwrap();

    for track_id in [vtrack, atrack] {
        let track = store.project.track(track_id).unwrap();
        let total: i64 = track.clips.iter().map(|c| c.duration).sum();
        assert_eq!(total, 8 * SEC, "8 s of material remain on the track");
        // contiguous with no gaps (ripple)
        let mut expected_start = 0;
        for c in &track.clips {
            assert_eq!(c.start, expected_start);
            expected_start = c.end();
        }
    }
    assert!(validate(&store.project).is_empty());
    assert_eq!(store.undo_labels().last().copied(), Some("Cut 2 range(s)"));

    // and it's ONE undo entry
    store.undo().unwrap();
    let track = store.project.track(vtrack).unwrap();
    assert_eq!(track.clips.len(), 1);
    assert_eq!(track.clips[0].duration, 10 * SEC);
}

#[test]
fn transaction_atomicity_on_failure() {
    let (mut store, _seq, vtrack, _at, va, _aa) = fixture();
    let a = Clip::new_media(va, 0, 2 * SEC, 0);
    let b_colliding = Clip::new_media(va, 0, 2 * SEC, SEC); // collides with a
    let snapshot = store.project.clone();

    let result = store.dispatch(
        "broken transaction",
        vec![
            Action::InsertClip { track_id: vtrack, clip: a },
            Action::InsertClip { track_id: vtrack, clip: b_colliding },
        ],
    );
    assert!(result.is_err());
    assert_eq!(store.project, snapshot, "full rollback: the project is left intact");
    assert!(!store.can_undo(), "a failed transaction does not enter the history");
}

#[test]
fn locked_track_rejects_ops() {
    let (mut store, _seq, vtrack, _at, va, _aa) = fixture();
    let clip_id = store
        .insert_clip(vtrack, Clip::new_media(va, 0, 2 * SEC, 0), InsertMode::Strict)
        .unwrap();
    store
        .dispatch(
            "Lock track",
            vec![Action::SetTrackProp { track_id: vtrack, prop: TrackProp::Locked(true) }],
        )
        .unwrap();
    assert!(store.split_clip(clip_id, SEC).is_err());
    assert!(store.delete_clips(&[clip_id], false).is_err());
    // but undoing the lock works
    store.undo().unwrap();
    assert!(store.split_clip(clip_id, SEC).is_ok());
}

#[test]
fn track_kind_rules() {
    let (mut store, _seq, vtrack, atrack, va, aa) = fixture();
    // a VIDEO asset can go on an audio track (audio-only use, linked pairs)
    let video_on_audio = Clip::new_media(va, 0, 2 * SEC, 0);
    assert!(store.insert_clip(atrack, video_on_audio, InsertMode::Strict).is_ok());
    // an AUDIO asset still can't go on a video track
    let audio_on_video = Clip::new_media(aa, 0, 2 * SEC, 0);
    assert!(
        store.insert_clip(vtrack, audio_on_video, InsertMode::Strict).is_err(),
        "an audio clip does not fit on a video track"
    );
    // a text clip doesn't fit on an audio track either
    let text = Clip::new_text("hi", 5 * SEC, 1 * SEC);
    assert!(store.insert_clip(atrack, text, InsertMode::Strict).is_err());
}

#[test]
fn project_save_load_after_edits() {
    let (mut store, _seq, vtrack, _at, va, _aa) = fixture();
    let clip_id = store
        .insert_clip(vtrack, Clip::new_media(va, 0, 10 * SEC, 0), InsertMode::Strict)
        .unwrap();
    store.split_clip(clip_id, 3 * SEC).unwrap();
    let json = store.project.to_json().unwrap();
    let loaded = Project::from_json(&json).unwrap();
    assert_eq!(store.project, loaded);
    assert!(validate(&loaded).is_empty());
}

// ---------------------------------------------------------------------------
// Property: random sequences of operations + full undo ≡ initial state
// ---------------------------------------------------------------------------

mod property_tests {
    use super::*;
    use proptest::collection::vec as prop_vec;
    use proptest::prelude::*;

    #[derive(Debug, Clone)]
    enum OpSpec {
        Insert { start_s: i64, dur_s: i64 },
        Split { clip_sel: usize, frac: f64 },
        Delete { clip_sel: usize, ripple: bool },
        Move { clip_sel: usize, start_s: i64, overwrite: bool },
        Trim { clip_sel: usize, left: bool, edge_s: i64 },
        CutRange { from_s: i64, len_s: i64 },
    }

    fn op_strategy() -> impl Strategy<Value = OpSpec> {
        prop_oneof![
            (0i64..30, 1i64..8).prop_map(|(s, d)| OpSpec::Insert { start_s: s, dur_s: d }),
            (0usize..8, 0.05f64..0.95).prop_map(|(c, f)| OpSpec::Split { clip_sel: c, frac: f }),
            (0usize..8, any::<bool>()).prop_map(|(c, r)| OpSpec::Delete { clip_sel: c, ripple: r }),
            (0usize..8, 0i64..30, any::<bool>())
                .prop_map(|(c, s, o)| OpSpec::Move { clip_sel: c, start_s: s, overwrite: o }),
            (0usize..8, any::<bool>(), 0i64..30)
                .prop_map(|(c, l, e)| OpSpec::Trim { clip_sel: c, left: l, edge_s: e }),
            (0i64..25, 1i64..5).prop_map(|(f, l)| OpSpec::CutRange { from_s: f, len_s: l }),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn random_ops_then_undo_all_restores_initial(ops in prop_vec(op_strategy(), 1..40)) {
            let (mut store, seq, vtrack, _at, va, _aa) = fixture();
            // initial state with a base clip
            store.insert_clip(vtrack, Clip::new_media(va, 0, 10 * SEC, 0), InsertMode::Strict).unwrap();
            let initial = store.project.clone();
            let initial_undo_depth = store.undo_labels().len();

            for op in &ops {
                let clip_ids: Vec<Id> = store
                    .project
                    .track(vtrack)
                    .unwrap()
                    .clips
                    .iter()
                    .map(|c| c.id)
                    .collect();
                let pick = |sel: usize| clip_ids.get(sel % clip_ids.len().max(1)).copied();
                let _ = match *op {
                    OpSpec::Insert { start_s, dur_s } => {
                        let src_len = dur_s.min(50) * SEC;
                        store
                            .insert_clip(
                                vtrack,
                                Clip::new_media(va, 0, src_len, start_s * SEC),
                                InsertMode::Overwrite,
                            )
                            .map(|_| ())
                    }
                    OpSpec::Split { clip_sel, frac } => match pick(clip_sel) {
                        Some(id) => {
                            let c = store.project.clip(id).unwrap();
                            let t = c.start + (c.duration as f64 * frac) as i64;
                            store.split_clip(id, t).map(|_| ())
                        }
                        None => Ok(()),
                    },
                    OpSpec::Delete { clip_sel, ripple } => match pick(clip_sel) {
                        Some(id) => store.delete_clips(&[id], ripple),
                        None => Ok(()),
                    },
                    OpSpec::Move { clip_sel, start_s, overwrite } => match pick(clip_sel) {
                        Some(id) => {
                            let mode = if overwrite { InsertMode::Overwrite } else { InsertMode::Strict };
                            store.move_clip(id, vtrack, start_s * SEC, mode)
                        }
                        None => Ok(()),
                    },
                    OpSpec::Trim { clip_sel, left, edge_s } => match pick(clip_sel) {
                        Some(id) => store.trim_clip(id, left, edge_s * SEC),
                        None => Ok(()),
                    },
                    OpSpec::CutRange { from_s, len_s } => {
                        store.cut_ranges(seq, &[(from_s * SEC, (from_s + len_s) * SEC)], true)
                    }
                };
                // whether it succeeds or fails, the invariants ALWAYS hold
                prop_assert_eq!(validate(&store.project), Vec::<String>::new());
            }

            // undo everything applied in the loop
            while store.undo_labels().len() > initial_undo_depth {
                store.undo().unwrap();
            }
            prop_assert_eq!(&store.project, &initial);
        }
    }
}

#[test]
fn move_range_reorders_and_preserves_material() {
    let (mut store, seq, vtrack, atrack, va, aa) = fixture();
    // V1: a 9 s clip [0..9); A1: parallel audio [0..9)
    store.insert_clip(vtrack, Clip::new_media(va, 0, 9 * SEC, 0), InsertMode::Strict).unwrap();
    store.insert_clip(atrack, Clip::new_media(aa, 0, 9 * SEC, 0), InsertMode::Strict).unwrap();
    let snapshot = store.project.clone();

    // move the middle third [3..6) to the beginning (dest=0)
    store.move_range(seq, 3 * SEC, 6 * SEC, 0).unwrap();

    for track_id in [vtrack, atrack] {
        let track = store.project.track(track_id).unwrap();
        // total material preserved and contiguous
        let total: i64 = track.clips.iter().map(|c| c.duration).sum();
        assert_eq!(total, 9 * SEC);
        let mut expected = 0;
        for c in &track.clips {
            assert_eq!(c.start, expected, "no gaps after moving");
            expected = c.end();
        }
        // the first clip is now the source material [3..6)
        let first = &track.clips[0];
        match &first.payload {
            ClipPayload::Media { src_in, src_out, .. } => {
                assert_eq!((*src_in, *src_out), (3 * SEC, 6 * SEC), "the middle third goes first");
            }
            _ => panic!("unexpected payload"),
        }
    }
    assert!(validate(&store.project).is_empty());

    // a single undo entry reverts it all
    store.undo().unwrap();
    assert_eq!(store.project, snapshot);
}

#[test]
fn move_range_forward_and_edge_cases() {
    let (mut store, seq, vtrack, _at, va, _aa) = fixture();
    store.insert_clip(vtrack, Clip::new_media(va, 0, 9 * SEC, 0), InsertMode::Strict).unwrap();

    // move [0..3) to the end (dest=9): leaves [3..9)+[0..3)
    store.move_range(seq, 0, 3 * SEC, 9 * SEC).unwrap();
    let track = store.project.track(vtrack).unwrap();
    let last = track.clips.last().unwrap();
    match &last.payload {
        ClipPayload::Media { src_in, .. } => assert_eq!(*src_in, 0, "the start ended up at the end"),
        _ => panic!(),
    }
    assert_eq!(track.clips.last().unwrap().end(), 9 * SEC, "total duration intact");

    // destination inside the range → error and no changes
    let before = store.project.clone();
    assert!(store.move_range(seq, 0, 4 * SEC, 2 * SEC).is_err());
    assert_eq!(store.project, before);
}

#[test]
fn linked_pair_propagates_all_operations() {
    let (mut store, _seq, vtrack, atrack, va, _aa) = fixture();
    // linked pair: video on V1 + its audio on A1 (video asset on an audio track)
    let group = Id::new();
    let mut vclip = Clip::new_media(va, 0, 10 * SEC, 0);
    vclip.group = Some(group);
    vclip.audio.muted = true;
    let mut aclip = Clip::new_media(va, 0, 10 * SEC, 0);
    aclip.group = Some(group);
    let v_id = store.insert_clip(vtrack, vclip, InsertMode::Strict).unwrap();
    let _a_id = store.insert_clip(atrack, aclip, InsertMode::Strict).unwrap();

    // SPLIT: splits both; the right halves share a NEW group
    let (vl, vr) = store.split_clip(v_id, 4 * SEC).unwrap();
    let a_clips: Vec<Clip> = store.project.track(atrack).unwrap().clips.clone();
    assert_eq!(a_clips.len(), 2, "the linked audio was split too");
    let v_right = store.project.clip(vr).unwrap().clone();
    let a_right = a_clips.iter().find(|c| c.start == 4 * SEC).unwrap();
    assert_eq!(v_right.group, a_right.group, "right halves re-linked");
    assert_ne!(v_right.group, Some(group), "with a new group");
    let v_left = store.project.clip(vl).unwrap().clone();
    assert_eq!(v_left.group, Some(group), "the left halves keep the group");

    // MOVE: moving the right video +5s drags its audio along
    store.move_clip(vr, vtrack, 9 * SEC, InsertMode::Strict).unwrap();
    let a_right_now = store
        .project
        .track(atrack)
        .unwrap()
        .clips
        .iter()
        .find(|c| c.group == v_right.group)
        .unwrap()
        .clone();
    assert_eq!(a_right_now.start, 9 * SEC, "the audio followed the video");

    // TRIM: trimming the right edge of the video trims the audio
    store.trim_clip(vr, false, 12 * SEC).unwrap();
    let v_now = store.project.clip(vr).unwrap();
    let a_now = store
        .project
        .track(atrack)
        .unwrap()
        .clips
        .iter()
        .find(|c| c.group == v_right.group)
        .unwrap();
    assert_eq!(v_now.end(), a_now.end(), "edges aligned after the trim");

    // SPEED: 2x on both
    store.set_clip_speed(vr, 2.0).unwrap();
    let a_now = store
        .project
        .track(atrack)
        .unwrap()
        .clips
        .iter()
        .find(|c| c.group == v_right.group)
        .unwrap();
    assert_eq!(a_now.speed, 2.0, "speed propagated to the audio");
    assert_eq!(store.project.clip(vr).unwrap().duration, a_now.duration);

    // DELETE with ripple: deletes the pair and closes gaps on both tracks
    store.delete_clips(&[vl], true).unwrap();
    let v_clips = &store.project.track(vtrack).unwrap().clips;
    let a_clips = &store.project.track(atrack).unwrap().clips;
    assert_eq!(v_clips.len(), 1);
    assert_eq!(a_clips.len(), 1);
    assert_eq!(v_clips[0].start, a_clips[0].start, "tracks aligned after ripple");
    assert!(validate(&store.project).is_empty());
}

/// Split-at-silences mode: boundaries only, nothing removed, duration intact.
#[test]
fn split_ranges_segments_without_removing() {
    let (mut store, _seq, vtrack, atrack, va, _aa) = fixture();
    store
        .insert_clip(vtrack, Clip::new_media(va, 0, 10 * SEC, 0), InsertMode::Strict)
        .unwrap();
    store
        .insert_clip(atrack, Clip::new_media(va, 0, 10 * SEC, 0), InsertMode::Strict)
        .unwrap();
    let seq_id = store.project.active_sequence;
    // two "silences": [2,3) and [6,7)
    store
        .split_ranges(seq_id, &[(2 * SEC, 3 * SEC), (6 * SEC, 7 * SEC)])
        .unwrap();
    let seq = store.project.sequence(seq_id).unwrap();
    for tid in [vtrack, atrack] {
        let track = seq.tracks.iter().find(|t| t.id == tid).unwrap();
        assert_eq!(track.clips.len(), 5, "4 cuts → 5 segments");
        let total: i64 = track.clips.iter().map(|c| c.duration).sum();
        assert_eq!(total, 10 * SEC, "nothing was removed");
        let starts: Vec<i64> = track.clips.iter().map(|c| c.start).collect();
        assert_eq!(starts, vec![0, 2 * SEC, 3 * SEC, 6 * SEC, 7 * SEC]);
    }
    // one undo restores the original single clips
    store.undo().unwrap();
    let seq = store.project.sequence(seq_id).unwrap();
    assert!(seq.tracks.iter().filter(|t| t.id == vtrack || t.id == atrack).all(|t| t.clips.len() == 1));
}

/// Slowing a clip down next to another clamps its duration to the gap
/// instead of failing with a collision error.
#[test]
fn set_clip_speed_clamps_to_next_clip() {
    let (mut store, _seq, vtrack, _at, va, _aa) = fixture();
    let a = store
        .insert_clip(vtrack, Clip::new_media(va, 0, 4 * SEC, 0), InsertMode::Strict)
        .unwrap();
    store
        .insert_clip(vtrack, Clip::new_media(va, 4 * SEC, 8 * SEC, 4 * SEC), InsertMode::Strict)
        .unwrap();
    // 0.5x would need 8 s but only 4 s fit before the next clip
    store.set_clip_speed(a, 0.5).unwrap();
    let clip = store.project.clip(a).unwrap();
    assert!((clip.speed - 0.5).abs() < 1e-9);
    assert_eq!(clip.duration, 4 * SEC, "clamped to the gap");
}

/// A slider drag emits many same-label dispatches; they must coalesce into
/// ONE history entry whose undo restores the pre-gesture state (field
/// report: 'Edit transform' x10 per drag).
#[test]
fn coalesced_dispatches_undo_as_one_gesture() {
    let (mut store, _seq, vtrack, _at, va, _aa) = fixture();
    let c = store
        .insert_clip(vtrack, Clip::new_media(va, 0, 4 * SEC, 0), InsertMode::Strict)
        .unwrap();
    // simulate a drag: 0 → 10 → 20 → … → 100 px
    for x in (10..=100).step_by(10) {
        let mut t = store.project.clip(c).unwrap().transform.clone();
        t.position.0 = (x as f64).into();
        store
            .dispatch_coalesced(
                "Edit transform",
                vec![ue_core::Action::SetClipTransform { clip_id: c, transform: t }],
            )
            .unwrap();
    }
    assert_eq!(
        store.project.clip(c).unwrap().transform.position.0.eval(0),
        100.0,
        "latest value applied"
    );
    // ONE undo returns to the pre-drag state (0), not to 90
    store.undo().unwrap();
    assert_eq!(
        store.project.clip(c).unwrap().transform.position.0.eval(0),
        0.0,
        "the whole gesture undoes as one step"
    );
    // and redo replays the final value
    store.redo().unwrap();
    assert_eq!(store.project.clip(c).unwrap().transform.position.0.eval(0), 100.0);
}
