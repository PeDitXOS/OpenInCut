//! MCP dispatcher tests (handle_rpc is pure over AppState: no HTTP).

use serde_json::{json, Value};
use ue_core::model::*;
use ue_core::ops::InsertMode;
use ue_tauri_lib::mcp::handle_rpc;
use ue_tauri_lib::AppState;

const SEC: i64 = 1_000_000;

fn rpc(state: &AppState, method: &str, params: Value) -> Value {
    let req = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    handle_rpc(state, None, &req).expect("with an id there is always a response")
}

/// Text of the first content of a tool result, parsed as JSON.
fn tool_json(resp: &Value) -> Value {
    let text = resp
        .pointer("/result/content/0/text")
        .and_then(|v| v.as_str())
        .expect("tool result with text");
    serde_json::from_str(text).unwrap_or(Value::String(text.to_string()))
}

fn state_with_clip() -> (AppState, Id) {
    let state = AppState::new_default();
    let clip_id;
    {
        let mut store = state.store.lock().unwrap();
        let asset = MediaAsset {
            id: Id::new(),
            kind: MediaKind::Video,
            path: "media/x.mp4".into(),
            content_hash: "xxh3:x".into(),
            probe: ProbeInfo {
                duration_us: 30 * SEC,
                fps: Some((30, 1)),
                width: 1920,
                height: 1080,
                rotation: 0,
                vcodec: None,
                acodec: None,
                audio_channels: 0,
                vfr: false,
            },
            proxy: None,
            audio_conform: None,
            peaks: None,
            thumbnails: None,
            transcript: None,
            offline: false,
        };
        let aid = asset.id;
        store.project.assets.push(asset);
        let vtrack = store
            .project
            .sequence(store.project.active_sequence)
            .unwrap()
            .tracks
            .iter()
            .find(|t| t.kind == TrackKind::Video)
            .unwrap()
            .id;
        let clip = Clip::new_media(aid, 0, 10 * SEC, 0);
        clip_id = clip.id;
        store.insert_clip(vtrack, clip, InsertMode::Strict).unwrap();
    }
    (state, clip_id)
}

#[test]
fn initialize_and_tools_list() {
    let state = AppState::new_default();
    let init = rpc(&state, "initialize", json!({ "protocolVersion": "2025-06-18" }));
    assert_eq!(init.pointer("/result/protocolVersion").unwrap(), "2025-06-18");
    assert_eq!(init.pointer("/result/serverInfo/name").unwrap(), "ubereditor");

    let tools = rpc(&state, "tools/list", json!({}));
    let list = tools.pointer("/result/tools").unwrap().as_array().unwrap();
    assert!(list.len() >= 10, "at least 10 tools, got {}", list.len());
    assert!(list.iter().any(|t| t["name"] == "split_clip"));

    // notification → no response
    let note = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
    assert!(handle_rpc(&state, None, &note).is_none());

    // unknown method → JSON-RPC error
    let err = rpc(&state, "no/such/method", json!({}));
    assert_eq!(err.pointer("/error/code").unwrap(), -32601);
}

#[test]
fn summary_and_timeline_reflect_project() {
    let (state, _clip) = state_with_clip();
    let resp = rpc(&state, "tools/call", json!({ "name": "get_project_summary", "arguments": {} }));
    let summary = tool_json(&resp);
    assert_eq!(summary["assets"], 1);
    let seqs = summary["sequences"].as_array().unwrap();
    assert_eq!(seqs.len(), 1);
    assert_eq!(seqs[0]["duration_us"], 10 * SEC);
    assert_eq!(seqs[0]["active"], true);
    assert!(seqs[0]["sequence_id"].is_string(), "ids are addressable");

    let resp = rpc(&state, "tools/call", json!({ "name": "get_timeline", "arguments": {} }));
    let timeline = tool_json(&resp);
    let clips = timeline["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["clips"].as_array().unwrap().len())
        .sum::<usize>();
    assert_eq!(clips, 1);
}

/// The whole point of the server: an agent must be able to do everything a
/// human can. This is the contract; adding a UI feature without an MCP tool
/// breaks it on purpose.
#[test]
fn tools_cover_the_whole_editor_and_are_documented() {
    let state = AppState::new_default();
    let tools = rpc(&state, "tools/list", json!({}));
    let list = tools.pointer("/result/tools").unwrap().as_array().unwrap();
    let names: Vec<&str> = list.iter().map(|t| t["name"].as_str().unwrap()).collect();

    for expected in [
        // read
        "get_project_summary", "get_timeline", "get_media_pool", "get_transcript",
        "find_words", "get_catalog",
        // media
        "import_media", "transcribe_asset", "relink_asset", "set_project_settings",
        // timeline
        "add_clip", "add_text_clip", "add_generator_clip", "add_subtitles_clip",
        "split_clip", "delete_clips", "move_clip", "trim_clip", "unlink_clip",
        "cut_ranges", "move_range",
        // clip properties
        "set_clip_properties", "set_clip_content", "set_clip_name",
        // tracks / sequences
        "add_track", "remove_track", "set_track_prop",
        "set_sequence_props", "set_active_sequence", "remove_sequence", "generate_vertical",
        // ai
        "remove_silences", "replace_words", "set_word_text",
        "save_avatar_config", "remove_avatar_config",
        "import_avatar_config", "export_avatar_config", "generate_avatar_video",
        // project / render / history / jobs
        "new_project", "open_project", "save_project", "reload_effect_packs",
        "export_video", "debug_render_frame", "debug_playback_frame", "playback",
        "get_job_status", "list_jobs", "undo", "redo",
    ] {
        assert!(names.contains(&expected), "missing MCP tool: {expected}");
    }

    // Deliberately NOT exposed, so this list stays honest about the gap:
    //   pick_avatar_media / ui_log / mcp_status / get_state / playback_frame /
    //   get_audio_peaks / ensure_thumbs / get_thumb_strip  → GUI plumbing
    //   cancel_export                                      → export_video blocks the server
    //   check_recovery / recover_project / discard_recovery → need the AppHandle's
    //       data dir and are the UI's crash-recovery prompt
    // Everything else in `invoke_handler` has a tool (see docs/MCP.md).
    for gui_only in ["ui_log", "mcp_status", "pick_avatar_media", "get_state"] {
        assert!(!names.contains(&gui_only), "{gui_only} is GUI plumbing, not a tool");
    }

    // documentation is part of the contract: an agent only sees these strings
    for t in list {
        let name = t["name"].as_str().unwrap();
        let desc = t["description"].as_str().unwrap_or("");
        assert!(desc.len() > 40, "{name}: description too thin to act on");
        assert_eq!(
            t.pointer("/inputSchema/additionalProperties").unwrap(),
            false,
            "{name}: typos in arguments must fail loudly"
        );
        assert!(t.pointer("/annotations/readOnlyHint").is_some(), "{name}: no annotations");
        // every declared argument carries a description
        if let Some(props) = t.pointer("/inputSchema/properties").and_then(|p| p.as_object()) {
            for (arg, schema) in props {
                assert!(
                    schema.get("description").is_some() || schema.get("enum").is_some(),
                    "{name}.{arg}: undocumented argument"
                );
            }
        }
    }

    // read-only tools must never be flagged destructive, and vice versa
    let by_name = |n: &str| list.iter().find(|t| t["name"] == n).unwrap();
    assert_eq!(by_name("get_timeline")["annotations"]["readOnlyHint"], true);
    assert_eq!(by_name("split_clip")["annotations"]["readOnlyHint"], false);
    assert_eq!(by_name("split_clip")["annotations"]["destructiveHint"], false, "undoable");
    assert_eq!(by_name("open_project")["annotations"]["destructiveHint"], true);
    assert_eq!(by_name("export_video")["annotations"]["destructiveHint"], true);
}

/// `initialize` hands the agent the map: units, history semantics, the flow.
#[test]
fn initialize_instructions_state_the_invariants() {
    let state = AppState::new_default();
    let init = rpc(&state, "initialize", json!({}));
    let inst = init.pointer("/result/instructions").unwrap().as_str().unwrap();
    assert!(inst.contains("MICROSECONDS"), "units must be unmissable");
    assert!(inst.contains("undo"), "history semantics");
    assert!(inst.contains("transcribe_asset"), "flow references real tool names");
}

/// A patch touches only the keys it names, understands keyframe curves, and
/// several properties in one call collapse into ONE undo entry.
#[test]
fn set_clip_properties_patches_and_is_one_undo() {
    let (state, clip_id) = state_with_clip();

    let resp = rpc(
        &state,
        "tools/call",
        json!({ "name": "set_clip_properties", "arguments": {
            "clip_id": clip_id.to_string(),
            "transform": {
                "position_x": 120.0,
                "opacity": { "keys": [
                    { "t": 0, "value": 0.0, "interp": { "kind": "linear" } },
                    { "t": 1000000, "value": 1.0, "interp": { "kind": "linear" } }
                ]}
            },
            "audio": { "gain_db": -6.0, "muted": true },
            "speed": 2.0
        }}),
    );
    assert!(resp.pointer("/result/isError").is_none(), "{resp}");

    {
        let store = state.store.lock().unwrap();
        let clip = store.project.clip(clip_id).unwrap();
        assert_eq!(clip.transform.position.0.eval(0), 120.0, "position_x written");
        assert_eq!(clip.transform.position.1.eval(0), 0.0, "position_y untouched");
        assert_eq!(clip.transform.scale.0.eval(0), 1.0, "scale untouched by the patch");
        assert_eq!(clip.transform.opacity.eval(500_000), 0.5, "opacity is an animated curve");
        assert_eq!(clip.audio.gain_db.eval(0), -6.0);
        assert!(clip.audio.muted);
        assert!(!clip.audio.denoise, "denoise untouched");
        assert_eq!(clip.speed, 2.0);
        assert_eq!(clip.duration, 5 * SEC, "2x speed halves the clip");
    }

    // ONE undo reverts the whole call
    rpc(&state, "tools/call", json!({ "name": "undo", "arguments": {} }));
    let store = state.store.lock().unwrap();
    let clip = store.project.clip(clip_id).unwrap();
    assert_eq!(clip.transform.position.0.eval(0), 0.0, "one undo reverted everything");
    assert_eq!(clip.speed, 1.0);
    assert_eq!(clip.duration, 10 * SEC);
    assert!(!clip.audio.muted);
}

/// Typos must fail loudly instead of silently doing nothing.
#[test]
fn unknown_patch_fields_and_bad_ids_are_rejected() {
    let (state, clip_id) = state_with_clip();

    let bad_field = rpc(
        &state,
        "tools/call",
        json!({ "name": "set_clip_properties", "arguments": {
            "clip_id": clip_id.to_string(), "transform": { "positionX": 10 }
        }}),
    );
    assert_eq!(bad_field.pointer("/result/isError").unwrap(), true);
    let msg = bad_field.pointer("/result/content/0/text").unwrap().as_str().unwrap();
    assert!(msg.contains("positionX"), "the message names the offending field: {msg}");

    // nothing to change is an error, not a silent no-op
    let empty = rpc(
        &state,
        "tools/call",
        json!({ "name": "set_clip_properties", "arguments": { "clip_id": clip_id.to_string() } }),
    );
    assert_eq!(empty.pointer("/result/isError").unwrap(), true);

    // an effect that does not exist in the registry
    let bad_effect = rpc(
        &state,
        "tools/call",
        json!({ "name": "set_clip_properties", "arguments": {
            "clip_id": clip_id.to_string(),
            "effects": [{ "effect_id": "core.does_not_exist" }]
        }}),
    );
    assert_eq!(bad_effect.pointer("/result/isError").unwrap(), true);

    // an unknown tool name
    let unknown = rpc(&state, "tools/call", json!({ "name": "nope", "arguments": {} }));
    assert_eq!(unknown.pointer("/result/isError").unwrap(), true);
}

/// Text clips: content and a partial style patch, in one call.
#[test]
fn set_clip_content_edits_text_and_style() {
    let state = AppState::new_default();
    let resp = rpc(
        &state,
        "tools/call",
        json!({ "name": "add_text_clip", "arguments": { "content": "hola", "at_us": 0 } }),
    );
    let clip_id: Id = tool_json(&resp)["clip_id"].as_str().unwrap().parse().unwrap();

    let resp = rpc(
        &state,
        "tools/call",
        json!({ "name": "set_clip_content", "arguments": {
            "clip_id": clip_id.to_string(),
            "text": "adiós",
            "style": { "size": 72.0, "color": "#ff0000" }
        }}),
    );
    assert!(resp.pointer("/result/isError").is_none(), "{resp}");

    let store = state.store.lock().unwrap();
    match &store.project.clip(clip_id).unwrap().payload {
        ClipPayload::Text { content, style } => {
            assert_eq!(content, "adiós");
            assert_eq!(style.size, 72.0);
            assert_eq!(style.color, "#ff0000");
            assert_eq!(style.align, TextAlign::Center, "untouched keys keep their value");
        }
        other => panic!("expected a text clip, got {other:?}"),
    }
}

/// Tracks: create, rename, set volume, and refuse to delete the last one.
#[test]
fn track_tools_round_trip() {
    let state = AppState::new_default();
    let resp = rpc(&state, "tools/call", json!({ "name": "add_track", "arguments": { "kind": "video" } }));
    let track_id = tool_json(&resp)["track_id"].as_str().unwrap().to_string();

    let resp = rpc(
        &state,
        "tools/call",
        json!({ "name": "set_track_prop", "arguments": { "track_id": track_id, "name": "B-roll" } }),
    );
    assert!(resp.pointer("/result/isError").is_none(), "{resp}");

    // exactly one property per call
    let two = rpc(
        &state,
        "tools/call",
        json!({ "name": "set_track_prop", "arguments": {
            "track_id": track_id, "muted": true, "locked": true
        }}),
    );
    assert_eq!(two.pointer("/result/isError").unwrap(), true);

    let store = state.store.lock().unwrap();
    let seq = store.project.sequence(store.project.active_sequence).unwrap();
    assert!(seq.tracks.iter().any(|t| t.name == "B-roll"));
    let video_tracks: Vec<Id> =
        seq.tracks.iter().filter(|t| t.kind == TrackKind::Video).map(|t| t.id).collect();
    drop(store);

    // removing them all fails on the last one
    for (i, id) in video_tracks.iter().enumerate() {
        let resp = rpc(
            &state,
            "tools/call",
            json!({ "name": "remove_track", "arguments": { "track_id": id.to_string() } }),
        );
        let last = i + 1 == video_tracks.len();
        assert_eq!(
            resp.pointer("/result/isError").is_some(),
            last,
            "only the last video track is protected"
        );
    }
}

/// cut_ranges works across tracks and reports what it removed.
#[test]
fn cut_ranges_ripples_all_tracks() {
    let (state, clip_id) = state_with_clip();
    // a second clip, on the audio track, spanning the same time
    {
        let mut store = state.store.lock().unwrap();
        let aid = store.project.assets[0].id;
        let at = store
            .project
            .sequence(store.project.active_sequence)
            .unwrap()
            .tracks
            .iter()
            .find(|t| t.kind == TrackKind::Audio)
            .unwrap()
            .id;
        store.insert_clip(at, Clip::new_media(aid, 0, 10 * SEC, 0), InsertMode::Strict).unwrap();
    }

    let resp = rpc(
        &state,
        "tools/call",
        json!({ "name": "cut_ranges", "arguments": { "ranges": [[2 * SEC, 4 * SEC]] } }),
    );
    let out = tool_json(&resp);
    assert_eq!(out["cut"], 1);
    assert_eq!(out["removed_us"], 2 * SEC);

    let store = state.store.lock().unwrap();
    let seq = store.project.sequence(store.project.active_sequence).unwrap();
    assert_eq!(seq.duration_us(), 8 * SEC, "both tracks rippled");
    // the cut splices each track's clip, so ids are re-minted (ReplaceClips)
    assert!(store.project.clip(clip_id).is_none(), "the original clip was spliced");
    for track in &seq.tracks {
        // what was left and right of the cut, now butted together
        assert_eq!(track.clips.len(), 2, "each track kept both halves");
        assert_eq!(track.clips[0].end(), track.clips[1].start, "the gap was closed");
        assert_eq!(track.clips.iter().map(|c| c.duration).sum::<i64>(), 8 * SEC);
    }

    // an empty range list is an error, not a no-op
    drop(store);
    let empty = rpc(&state, "tools/call", json!({ "name": "cut_ranges", "arguments": { "ranges": [] } }));
    assert_eq!(empty.pointer("/result/isError").unwrap(), true);
}

/// The whole agentic loop against REAL ffmpeg: import a file, put it on the
/// timeline, animate it, cut a hole in it and render the result. If this
/// passes, an agent can edit a video with nothing but MCP calls.
#[test]
fn agentic_workflow_import_edit_export() {
    let ffmpeg = ue_media::ffmpeg_bin();
    if std::process::Command::new(&ffmpeg)
        .arg("-version")
        .output()
        .map(|o| !o.status.success())
        .unwrap_or(true)
    {
        eprintln!("NOTE: no ffmpeg; test skipped");
        return;
    }
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-mcp-agentic");
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("src.mp4");
    if !src.exists() {
        let st = std::process::Command::new(&ffmpeg)
            .args(["-y", "-v", "error", "-f", "lavfi", "-i", "testsrc=duration=6:size=320x180:rate=30"])
            .args(["-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p"])
            .arg(&src)
            .status()
            .unwrap();
        assert!(st.success());
    }

    let state = AppState::new_default();
    let call = |name: &str, arguments: Value| -> Value {
        let resp = rpc(&state, "tools/call", json!({ "name": name, "arguments": arguments }));
        assert!(
            resp.pointer("/result/isError").is_none(),
            "{name} failed: {}",
            resp.pointer("/result/content/0/text").unwrap_or(&Value::Null)
        );
        tool_json(&resp)
    };
    // launch an async tool (export/transcribe/avatar) and poll to completion.
    // In tests the job finishes inline, so a single poll returns the result.
    let run_job = |name: &str, arguments: Value| -> Value {
        let launched = call(name, arguments);
        let job_id = launched["job_id"].as_str().expect("async tool returns a job_id");
        let status = call("get_job_status", json!({ "job_id": job_id }));
        assert_eq!(status["status"], "done", "job failed: {status}");
        status["result"].clone()
    };

    // 1. import (no AppHandle → no background conform, which is fine here)
    let imported = call("import_media", json!({ "paths": [src.to_string_lossy()] }));
    assert_eq!(imported["imported"], 1);
    let asset_id = imported["assets"][0]["asset_id"].as_str().unwrap().to_string();
    assert_eq!(imported["assets"][0]["duration_us"], 6 * SEC);

    // importing the same content again is idempotent
    let again = call("import_media", json!({ "paths": [src.to_string_lossy()] }));
    assert_eq!(again["assets"][0]["asset_id"], asset_id, "same content → same asset");
    assert_eq!(state.store.lock().unwrap().project.assets.len(), 1, "no duplicate asset");

    // 2. put it on the timeline
    let clip_id = call("add_clip", json!({ "asset_id": asset_id, "at_us": 0 }))["clip_id"]
        .as_str()
        .unwrap()
        .to_string();

    // 3. animate the opacity and push it right
    call(
        "set_clip_properties",
        json!({ "clip_id": clip_id, "transform": {
            "position_x": 40.0,
            "opacity": { "keys": [
                { "t": 0, "value": 0.2, "interp": { "kind": "linear" } },
                { "t": 2000000, "value": 1.0, "interp": { "kind": "linear" } }
            ]}
        }}),
    );

    // 4. cut a hole out of the middle, rippling
    let cut = call("cut_ranges", json!({ "ranges": [[2 * SEC, 3 * SEC]] }));
    assert_eq!(cut["removed_us"], SEC);

    // 5. the timeline agrees
    let timeline = call("get_timeline", json!({}));
    let clips: Vec<&Value> = timeline["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|t| t["clips"].as_array().unwrap())
        .collect();
    assert_eq!(clips.len(), 2, "split around the hole");

    // 6. render it for real (async job → poll)
    let out = dir.join("out.mp4");
    let _ = std::fs::remove_file(&out);
    let exported = run_job("export_video", json!({ "path": out.to_string_lossy(), "crf": 30 }));
    assert_eq!(exported["pieces"], 0, "whole timeline");
    assert!(out.exists(), "the export wrote a file");
    let probed = ue_media::import_file(&out).expect("the export is a valid video");
    let dur = probed.probe.duration_us;
    assert!(
        (4_500_000..=5_500_000).contains(&dur),
        "6 s minus the 1 s hole ≈ 5 s, got {dur}"
    );

    // 7. multi-piece export: two chunks concatenated into one file
    let pieces = dir.join("pieces.mp4");
    let _ = std::fs::remove_file(&pieces);
    let exported = run_job(
        "export_video",
        json!({ "path": pieces.to_string_lossy(), "crf": 30,
                "ranges": [[0, SEC], [3 * SEC, 4 * SEC]] }),
    );
    assert_eq!(exported["pieces"], 2);
    let dur = ue_media::import_file(&pieces).unwrap().probe.duration_us;
    assert!((1_600_000..=2_400_000).contains(&dur), "two 1 s pieces ≈ 2 s, got {dur}");

    // 8. and the whole session is undoable, one call at a time
    let duration = || {
        let store = state.store.lock().unwrap();
        store.project.sequence(store.project.active_sequence).unwrap().duration_us()
    };
    assert_eq!(duration(), 5 * SEC);
    call("undo", json!({})); // the cut
    assert_eq!(duration(), 6 * SEC, "undo restored the cut");
}

/// Avatar setups are fully manageable from MCP: create, export (without the
/// api_key), re-import (replacing by name), delete.
#[test]
fn avatar_config_crud_round_trip() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-mcp-avatar-crud");
    std::fs::create_dir_all(&dir).unwrap();
    let face = dir.join("calm.png");
    std::fs::write(&face, b"not really a png, but it exists").unwrap();

    let state = AppState::new_default();
    let call = |name: &str, arguments: Value| -> Value {
        let resp = rpc(&state, "tools/call", json!({ "name": name, "arguments": arguments }));
        assert!(
            resp.pointer("/result/isError").is_none(),
            "{name} failed: {}",
            resp.pointer("/result/content/0/text").unwrap_or(&Value::Null)
        );
        tool_json(&resp)
    };

    let created = call(
        "save_avatar_config",
        json!({ "config": {
            "name": "Presenter",
            "expressions": [{ "name": "calm", "path": face.to_string_lossy(), "description": "neutral" }],
            "api_key": "sk-secret-do-not-leak",
            "model": "gpt-4o-mini"
        }}),
    );
    let config_id = created["config_id"].as_str().unwrap().to_string();

    // it shows up in the catalog an agent reads
    let cat = call("get_catalog", json!({}));
    assert_eq!(cat["avatar_setups"][0]["name"], "Presenter");

    // exporting must never write the api_key out
    let out = dir.join("avatar.json");
    call("export_avatar_config", json!({ "config_id": config_id, "path": out.to_string_lossy() }));
    let written = std::fs::read_to_string(&out).unwrap();
    assert!(!written.contains("sk-secret-do-not-leak"), "the api key leaked into the export");
    assert!(written.contains("Presenter"));

    // re-importing the same name replaces it instead of duplicating
    let reimported = call("import_avatar_config", json!({ "path": out.to_string_lossy() }));
    assert_eq!(reimported["config_id"], config_id, "same name → same setup");
    assert_eq!(state.store.lock().unwrap().project.avatars.len(), 1, "no duplicate");

    // and it can be deleted (undoably)
    call("remove_avatar_config", json!({ "config_id": config_id }));
    assert!(state.store.lock().unwrap().project.avatars.is_empty());
    call("undo", json!({}));
    assert_eq!(state.store.lock().unwrap().project.avatars.len(), 1, "undo restores the setup");
}

/// Relinking repairs an offline asset: new path, re-probe, back online.
#[test]
fn relink_asset_brings_media_back_online() {
    let (state, _clip) = state_with_clip();
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-mcp-relink");
    std::fs::create_dir_all(&dir).unwrap();
    let real = dir.join("moved.mp4");
    std::fs::write(&real, b"pretend this is a video").unwrap();

    let asset_id = {
        let mut store = state.store.lock().unwrap();
        store.project.assets[0].offline = true;
        store.project.assets[0].id
    };

    // a path that does not exist fails, and changes nothing
    let bad = rpc(
        &state,
        "tools/call",
        json!({ "name": "relink_asset", "arguments": {
            "asset_id": asset_id.to_string(), "new_path": "/nope/missing.mp4"
        }}),
    );
    assert_eq!(bad.pointer("/result/isError").unwrap(), true);
    assert!(state.store.lock().unwrap().project.assets[0].offline, "still offline");

    // import_file probes with ffprobe; without ffmpeg there is nothing to relink to
    let ff_ok = std::process::Command::new(ue_media::ffmpeg_bin())
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !ff_ok {
        eprintln!("NOTE: no ffmpeg; the success half is skipped");
        return;
    }
    // a real, probeable file
    let src = dir.join("real.mp4");
    if !src.exists() {
        let st = std::process::Command::new(ue_media::ffmpeg_bin())
            .args(["-y", "-v", "error", "-f", "lavfi", "-i", "testsrc=duration=1:size=64x64:rate=10"])
            .args(["-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p"])
            .arg(&src)
            .status()
            .unwrap();
        assert!(st.success());
    }
    let ok = rpc(
        &state,
        "tools/call",
        json!({ "name": "relink_asset", "arguments": {
            "asset_id": asset_id.to_string(), "new_path": src.to_string_lossy()
        }}),
    );
    assert!(ok.pointer("/result/isError").is_none(), "{ok}");
    let store = state.store.lock().unwrap();
    assert!(!store.project.assets[0].offline, "back online");
    assert_eq!(store.project.assets[0].path, src.to_string_lossy());
}

/// The server is loopback-only AND token-gated: no header, a wrong token, or
/// an empty session token must never authorize.
#[test]
fn only_the_exact_bearer_token_authorizes() {
    use ue_tauri_lib::mcp::is_authorized;
    assert!(is_authorized(Some("Bearer s3cret"), "s3cret"));
    assert!(is_authorized(Some("  Bearer s3cret  "), "s3cret"), "surrounding space tolerated");

    assert!(!is_authorized(None, "s3cret"), "no header");
    assert!(!is_authorized(Some(""), "s3cret"));
    assert!(!is_authorized(Some("Bearer"), "s3cret"));
    assert!(!is_authorized(Some("Bearer wrong"), "s3cret"));
    assert!(!is_authorized(Some("bearer s3cret"), "s3cret"), "scheme is case-sensitive");
    assert!(!is_authorized(Some("Bearer s3cret extra"), "s3cret"));
    assert!(!is_authorized(Some("Basic s3cret"), "s3cret"));
    // a server that failed to mint a token must not accept "Bearer "
    assert!(!is_authorized(Some("Bearer "), ""));
    assert!(!is_authorized(Some("Bearer"), ""));
}

/// The catalog is what an agent reads before naming an effect or a generator.
#[test]
fn catalog_lists_effects_generators_and_modes() {
    let state = AppState::new_default();
    let cat = tool_json(&rpc(&state, "tools/call", json!({ "name": "get_catalog", "arguments": {} })));
    let effects = cat["effects"].as_array().or_else(|| cat["effects"]["effects"].as_array());
    assert!(effects.is_some_and(|e| !e.is_empty()), "effects catalog: {}", cat["effects"]);
    assert!(!cat["generators"].is_null());
    assert_eq!(cat["subtitle_modes"][0], "phrase");
    assert!(cat["transitions"].as_array().unwrap().contains(&json!("core.crossfade")));
}

#[test]
fn split_edit_and_undo_via_mcp() {
    let (state, clip_id) = state_with_clip();
    let resp = rpc(
        &state,
        "tools/call",
        json!({ "name": "split_clip", "arguments": { "clip_id": clip_id.to_string(), "t_us": 4 * SEC } }),
    );
    let result = tool_json(&resp);
    assert!(result["left"].is_string() && result["right"].is_string());
    {
        let store = state.store.lock().unwrap();
        let seq = store.project.sequence(store.project.active_sequence).unwrap();
        let total: usize = seq.tracks.iter().map(|t| t.clips.len()).sum();
        assert_eq!(total, 2, "the split via MCP split the clip");
    }

    let resp = rpc(&state, "tools/call", json!({ "name": "undo", "arguments": {} }));
    assert!(tool_json(&resp)["undone"].is_string());
    {
        let store = state.store.lock().unwrap();
        let seq = store.project.sequence(store.project.active_sequence).unwrap();
        let total: usize = seq.tracks.iter().map(|t| t.clips.len()).sum();
        assert_eq!(total, 1, "undo via MCP restored the clip");
    }

    // tool errors come as isError, not as a JSON-RPC error
    let resp = rpc(
        &state,
        "tools/call",
        json!({ "name": "split_clip", "arguments": { "clip_id": "not-an-id", "t_us": 1 } }),
    );
    assert_eq!(resp.pointer("/result/isError").unwrap(), true);
}

/// remove_silences end to end: video with real tone-silence-tone,
/// conform included. Verifies cuts and final duration.
#[test]
fn remove_silences_via_mcp_cuts_the_gap() {
    let ff_ok = std::process::Command::new(ue_media::ffmpeg_bin())
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !ff_ok {
        eprintln!("NOTE: no ffmpeg; test skipped");
        return;
    }
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-mcp-media");
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("tone_silence_tone.mp4");
    // 2s tone + 2s silence + 2s tone, with a color video
    let st = std::process::Command::new(ue_media::ffmpeg_bin())
        .args([
            "-y", "-v", "error",
            "-f", "lavfi", "-i", "color=c=gray:s=320x180:d=6:r=30",
            "-f", "lavfi", "-i",
            "aevalsrc='if(lt(mod(t,6),2)*0.4+gte(mod(t,6),4)*0.4, 0.4*sin(880*2*PI*t), 0)':d=6",
            "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p",
            "-c:a", "aac", "-shortest",
        ])
        .arg(&src)
        .status()
        .unwrap();
    assert!(st.success());

    let state = AppState::new_default();
    let clip_id;
    {
        let mut store = state.store.lock().unwrap();
        let mut asset = ue_media::import_file(&src).unwrap();
        // conform the audio by hand (in the app the import job does it)
        let conform = dir.join("conform.wav");
        ue_media::conform_audio(&src, &conform).unwrap();
        asset.audio_conform = Some(conform.to_string_lossy().into_owned());
        let aid = asset.id;
        store.project.assets.push(asset);
        let vtrack = store
            .project
            .sequence(store.project.active_sequence)
            .unwrap()
            .tracks
            .iter()
            .find(|t| t.kind == TrackKind::Video)
            .unwrap()
            .id;
        let clip = Clip::new_media(aid, 0, 6 * SEC, 0);
        clip_id = clip.id;
        store.insert_clip(vtrack, clip, InsertMode::Strict).unwrap();
    }

    let resp = rpc(
        &state,
        "tools/call",
        json!({ "name": "remove_silences", "arguments": { "clip_id": clip_id.to_string() } }),
    );
    let result = tool_json(&resp);
    assert_eq!(result["removed"], 1, "one central silence: {result}");
    let removed_us = result["removed_us"].as_i64().unwrap();
    assert!((1_200_000..=2_200_000).contains(&removed_us), "≈2 s minus padding: {removed_us}");

    let store = state.store.lock().unwrap();
    let seq = store.project.sequence(store.project.active_sequence).unwrap();
    let dur = seq.duration_us();
    assert!((3_800_000..=4_900_000).contains(&dur), "final duration ≈ 6s - silence: {dur}");
    let clips: usize = seq.tracks.iter().map(|t| t.clips.len()).sum();
    assert_eq!(clips, 2, "the clip ended up split in two around the silence");
}

/// Unlinking breaks the whole group and is undoable.
#[test]
fn unlink_breaks_group_with_undo() {
    let state = AppState::new_default();
    let (v_id, group);
    {
        let mut store = state.store.lock().unwrap();
        let asset = MediaAsset {
            id: Id::new(),
            kind: MediaKind::Video,
            path: "x.mp4".into(),
            content_hash: "h".into(),
            probe: ProbeInfo {
                duration_us: 10 * SEC,
                fps: Some((30, 1)),
                width: 640,
                height: 360,
                rotation: 0,
                vcodec: None,
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
        let aid = asset.id;
        store.project.assets.push(asset);
        let seq = store.project.sequence(store.project.active_sequence).unwrap();
        let vt = seq.tracks.iter().find(|t| t.kind == TrackKind::Video).unwrap().id;
        let at = seq.tracks.iter().find(|t| t.kind == TrackKind::Audio).unwrap().id;
        group = Id::new();
        let mut vc = Clip::new_media(aid, 0, 5 * SEC, 0);
        vc.group = Some(group);
        v_id = vc.id;
        let mut ac = Clip::new_media(aid, 0, 5 * SEC, 0);
        ac.group = Some(group);
        store.insert_clip(vt, vc, InsertMode::Strict).unwrap();
        store.insert_clip(at, ac, InsertMode::Strict).unwrap();
    }
    // unlink via the same actions the command uses
    {
        let mut store = state.store.lock().unwrap();
        let members = ue_core::ops::linked_ids(&store.project, v_id);
        assert_eq!(members.len(), 2);
        let actions = members
            .into_iter()
            .map(|clip_id| ue_core::Action::SetClipGroup { clip_id, group: None })
            .collect();
        store.dispatch("Unlink clips", actions).unwrap();
        assert_eq!(ue_core::ops::linked_ids(&store.project, v_id).len(), 1, "group broken");
        store.undo().unwrap();
        assert_eq!(ue_core::ops::linked_ids(&store.project, v_id).len(), 2, "undo re-links");
    }
}

/// Portability: saving relativizes the paths and clears caches; opening resolves
/// against the project folder and marks offline what doesn't exist.
#[test]
fn portable_project_roundtrip() {
    use ue_tauri_lib::{make_portable, resolve_project_paths};
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-portable");
    std::fs::create_dir_all(dir.join("media")).unwrap();
    let real = dir.join("media/exists.mp4");
    std::fs::write(&real, b"fake").unwrap();

    let mut project = ue_core::model::Project::new("portable");
    let mk = |path: String| MediaAsset {
        id: Id::new(),
        kind: MediaKind::Video,
        path,
        content_hash: "h".into(),
        probe: ProbeInfo {
            duration_us: SEC,
            fps: Some((30, 1)),
            width: 1,
            height: 1,
            rotation: 0,
            vcodec: None,
            acodec: None,
            audio_channels: 0,
            vfr: false,
        },
        proxy: Some("/tmp/cache/proxy.mp4".into()),
        audio_conform: Some("/tmp/cache/a.wav".into()),
        peaks: None,
        thumbnails: None,
        transcript: None,
        offline: false,
    };
    project.assets.push(mk(real.to_string_lossy().into_owned()));
    project.assets.push(mk("/nonexistent/outside.mp4".into()));

    let portable = make_portable(&project, Some(&dir));
    assert_eq!(portable.assets[0].path, "media/exists.mp4", "under the project → relative");
    assert_eq!(portable.assets[1].path, "/nonexistent/outside.mp4", "outside → absolute");
    assert!(portable.assets[0].audio_conform.is_none(), "caches don't travel");

    let mut reopened = portable.clone();
    resolve_project_paths(&mut reopened, Some(&dir));
    assert_eq!(reopened.assets[0].path, real.to_string_lossy(), "resolved to absolute");
    assert!(!reopened.assets[0].offline, "exists → online");
    assert!(reopened.assets[1].offline, "doesn't exist → offline");
}

/// Regression for the black-playback field bug: the vf string embeds unique
/// graph labels so it differs on EVERY build — the playback session key must
/// therefore derive from the DATA, staying stable across ticks.
#[test]
fn playback_session_key_is_stable_across_ticks() {
    use ue_core::model::Transform2D;
    let mut t = Transform2D::default();
    t.position.0 = 110.0.into();

    // document the trap: same inputs, different vf string (unique labels)
    let reg = ue_render::core_registry();
    let vf1 = ue_render::clip_vf(&reg, &[], &t, Some((1920, 1080)));
    let vf2 = ue_render::clip_vf(&reg, &[], &t, Some((1920, 1080)));
    assert_ne!(vf1, vf2, "vf strings are label-unique by design");

    let resolved = |pos_x: f64, rel: i64| ue_media::frame::ResolvedFrame {
        asset_path: "/cache/proxy.mp4".into(),
        src_t_us: 18_000_000 + rel,
        clip_rel_us: rel,
        speed: 1.0,
        effects: vec![],
        transform: {
            let mut t = Transform2D::default();
            t.position.0 = pos_x.into();
            t
        },
    };
    // two consecutive ticks of the same playback → SAME key (session reused)
    let k1 = ue_tauri_lib::playback_session_key(&resolved(110.0, 0), Some((1920, 1080)));
    let k2 =
        ue_tauri_lib::playback_session_key(&resolved(110.0, 40_000), Some((1920, 1080)));
    assert_eq!(k1, k2, "the key must not change while playing");
    // a real composition change → different key (session reopens)
    let k3 = ue_tauri_lib::playback_session_key(&resolved(200.0, 80_000), Some((1920, 1080)));
    assert_ne!(k1, k3, "changing the transform must invalidate the session");
}

/// End-to-end playback path with an active transform: ONE MjpegSession with
/// the position/canvas vf must stream frames continuously (the field bug
/// was a reopen-per-tick storm that never let the stream produce anything).
#[test]
fn playback_stream_with_transform_yields_continuous_frames() {
    use std::process::Command;
    let ffmpeg = ue_media::ffmpeg_bin();
    if Command::new(&ffmpeg).arg("-version").output().map(|o| !o.status.success()).unwrap_or(true)
    {
        eprintln!("NOTE: no ffmpeg; test skipped");
        return;
    }
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-stream-transform");
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("clip.mp4");
    if !src.exists() {
        let st = Command::new(&ffmpeg)
            .args(["-y", "-v", "error", "-f", "lavfi", "-i",
                   "testsrc=duration=4:size=960x540:rate=30"])
            .args(["-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p"])
            .arg(&src)
            .status()
            .unwrap();
        assert!(st.success());
    }
    // the exact chain the FrameService opens: fit-to-canvas + position offset
    let mut t = ue_core::model::Transform2D::default();
    t.position.0 = 110.0.into();
    let vf = ue_render::clip_vf(&ue_render::core_registry(), &[], &t, Some((1920, 1080)))
        .expect("transform chain");
    let mut session =
        ue_media::stream::MjpegSession::open(&src, 500_000, 960, 24, Some(&vf)).unwrap();
    let mut frames = 0;
    for _ in 0..24 {
        match session.next_frame() {
            Ok(Some(jpeg)) => {
                assert!(jpeg.len() > 1000, "real JPEG frames");
                frames += 1;
            }
            other => panic!("stream died at frame {frames}: {other:?}"),
        }
    }
    assert_eq!(frames, 24, "one second of continuous frames from ONE session");
}

/// FULL playback-path simulation with production pieces: 30 ticks at 24 fps
/// over a real video with an active Position X transform must open EXACTLY
/// ONE session and stream frames from it. This is the field bug: the old
/// vf-string comparison reopened a session on every tick (his logs showed
/// one open per ~90 ms) and playback stayed black.
#[test]
fn simulated_playback_opens_exactly_one_session() {
    use std::path::PathBuf;
    use std::process::Command;
    let ffmpeg = ue_media::ffmpeg_bin();
    if Command::new(&ffmpeg).arg("-version").output().map(|o| !o.status.success()).unwrap_or(true)
    {
        eprintln!("NOTE: no ffmpeg; test skipped");
        return;
    }
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-playback-sim");
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("clip.mp4");
    if !src.exists() {
        let st = Command::new(&ffmpeg)
            .args(["-y", "-v", "error", "-f", "lavfi", "-i",
                   "testsrc=duration=6:size=960x540:rate=30"])
            .args(["-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p"])
            .arg(&src)
            .status()
            .unwrap();
        assert!(st.success());
    }

    // the user's scenario: clip at t=0, Position X = 110, 1080p canvas
    let mut transform = ue_core::model::Transform2D::default();
    transform.position.0 = 110.0.into();
    let canvas = Some((1920u32, 1080u32));
    let reg = ue_render::core_registry();

    let mut session: Option<ue_media::stream::MjpegSession> = None;
    let mut session_key: Option<String> = None;
    let mut opens = 0usize;
    let mut frames = 0usize;

    // 30 playback ticks, ~41.7 ms apart (the FrameService cadence)
    for tick in 0..30i64 {
        let t = 500_000 + tick * 41_667;
        let resolved = ue_media::frame::ResolvedFrame {
            asset_path: src.to_string_lossy().into_owned(),
            src_t_us: t,
            clip_rel_us: t,
            speed: 1.0,
            effects: vec![],
            transform: transform.clone(),
        };
        let path = PathBuf::from(&resolved.asset_path);
        let key = ue_tauri_lib::playback_session_key(&resolved, canvas);
        let reusable = ue_tauri_lib::should_reuse_session(
            session.as_ref().map(|s| (s.asset_path.as_path(), s.next_src_us())),
            session_key.as_deref() == Some(key.as_str()),
            &path,
            resolved.src_t_us,
        );
        if !reusable {
            let rel0 = resolved.clip_rel_us as f64 / 1e6;
            let tvar = format!("(t+{rel0:.6})");
            let vf = ue_render::clip_vf_at(&reg, &resolved.effects, &resolved.transform, canvas, &tvar);
            session = Some(
                ue_media::stream::MjpegSession::open(&path, resolved.src_t_us, 960, 24, vf.as_deref())
                    .unwrap(),
            );
            session_key = Some(key);
            opens += 1;
        }
        // consume like the loop: everything up to the current position
        if let Some(s) = session.as_mut() {
            while s.next_src_us() <= resolved.src_t_us {
                match s.next_frame() {
                    Ok(Some(_)) => frames += 1,
                    other => panic!("stream died at tick {tick}: {other:?}"),
                }
            }
        }
    }
    assert_eq!(opens, 1, "the reopen-per-tick storm must be gone (opens={opens})");
    assert!(frames >= 20, "frames flowed continuously (frames={frames})");
}

/// Importing the same avatar setup twice must REPLACE it (same name), not
/// pile up duplicates in the picker (field report: two "Imported avatar").
#[test]
fn importing_the_same_avatar_twice_replaces_it() {
    use ue_core::model::{AvatarConfig, AvatarExpression};
    let mut store = ue_core::ProjectStore::new(ue_core::model::Project::new("p"));

    let make = |name: &str| {
        let mut c = AvatarConfig::new(name);
        c.expressions.push(AvatarExpression {
            name: "calm".into(),
            path: "/x/calm.png".into(),
            description: String::new(),
        });
        c
    };

    // first import
    let first = make("Imported avatar");
    let first_id = first.id;
    store
        .dispatch("Import avatar", vec![ue_core::Action::UpsertAvatarConfig { config: first }])
        .unwrap();
    assert_eq!(store.project.avatars.len(), 1);

    // second import of the same NAME: the command reuses the existing id
    let mut second = make("Imported avatar");
    if let Some(existing) = store.project.avatars.iter().find(|c| c.name == second.name) {
        second.id = existing.id;
    }
    second.shake_factor = 2.5;
    store
        .dispatch("Import avatar", vec![ue_core::Action::UpsertAvatarConfig { config: second }])
        .unwrap();

    assert_eq!(store.project.avatars.len(), 1, "no duplicate entries");
    assert_eq!(store.project.avatars[0].id, first_id, "same id kept");
    assert_eq!(store.project.avatars[0].shake_factor, 2.5, "updated in place");

    // a DIFFERENT name is a different setup
    store
        .dispatch(
            "Import avatar",
            vec![ue_core::Action::UpsertAvatarConfig { config: make("Other") }],
        )
        .unwrap();
    assert_eq!(store.project.avatars.len(), 2);
}

/// The advertised tool count in the docs must match reality.
#[test]
fn tool_count_matches_the_docs() {
    let state = AppState::new_default();
    let tools = rpc(&state, "tools/list", json!({}));
    let n = tools.pointer("/result/tools").unwrap().as_array().unwrap().len();
    assert_eq!(n, 52, "README/docs/MCP.md advertise the tool count; update them");
}

/// GOLDEN RULE: the paused preview must show subtitles, exactly like the
/// export burns them in. Field bug: an agent rendered the frame, saw no text,
/// and concluded the subtitles were broken. Builds a black clip + a subtitles
/// clip, renders the PREVIEW frame through the production path, and checks the
/// caption pixels land in the same band the export writes them.
#[test]
fn preview_frame_shows_subtitles_like_export() {
    use std::process::Command;
    let ffmpeg = ue_media::ffmpeg_bin();
    if Command::new(&ffmpeg).arg("-version").output().map(|o| !o.status.success()).unwrap_or(true) {
        eprintln!("NOTE: no ffmpeg; test skipped");
        return;
    }
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-preview-subs");
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("black.mp4");
    if !src.exists() {
        let st = Command::new(&ffmpeg)
            .args(["-y", "-v", "error", "-f", "lavfi", "-i", "color=c=black:s=640x360:d=3:r=30"])
            .args(["-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p"])
            .arg(&src)
            .status()
            .unwrap();
        assert!(st.success());
    }

    let state = AppState::new_default();
    let (v1, v2, aid);
    {
        let mut store = state.store.lock().unwrap();
        let seq_id = store.project.active_sequence;
        // 1080p canvas so the y_offset band matches the export test
        store.project.sequence_mut(seq_id).unwrap().resolution = (1920, 1080);
        let asset = ue_media::import_file(&src).unwrap();
        aid = asset.id;
        store.project.assets.push(asset);
        // synthetic transcript: "hola mundo" [0.2..2.6s)
        let doc = TranscriptDoc {
            id: Id::new(),
            asset_id: aid,
            language: "es".into(),
            model: "test".into(),
            words: vec![
                Word { text: "hola".into(), start_us: 200_000, end_us: 1_200_000, confidence: 1.0, rejected: false, display: None },
                Word { text: "mundo".into(), start_us: 1_300_000, end_us: 2_600_000, confidence: 1.0, rejected: false, display: None },
            ],
            segments: vec![Segment {
                text: "hola mundo".into(),
                start_us: 200_000,
                end_us: 2_600_000,
                word_range: (0, 2),
                emotion: None,
                volume_rms: 0.0,
            }],
            global_avg_volume: 0.0,
        };
        let doc_id = doc.id;
        store.project.transcripts.push(doc);
        let seq = store.project.sequence_mut(seq_id).unwrap();
        seq.tracks.push(Track::new(TrackKind::Video, "V2"));
        v2 = seq.tracks.last().unwrap().id;
        v1 = seq.tracks.iter().find(|t| t.kind == TrackKind::Video && t.name == "V1").unwrap().id;
        store.insert_clip(v1, Clip::new_media(aid, 0, 3 * SEC, 0), InsertMode::Strict).unwrap();
        let style = TextStyle { size: 90.0, y_offset: 380.0, ..Default::default() };
        let subs = Clip {
            id: Id::new(),
            payload: ClipPayload::Subtitles { transcript_id: doc_id, style, mode: SubtitleMode::Phrase, max_words: None },
            start: 0,
            duration: 3 * SEC,
            speed: 1.0,
            effects: vec![],
            transform: Default::default(),
            audio: Default::default(),
            transition_in: None,
            label_color: None,
            name: None,
            group: None,
        };
        store.insert_clip(v2, subs, InsertMode::Strict).unwrap();
    }
    let _ = (v1, v2);

    // render the preview frame at t=1.0s (inside the caption) at canvas width
    let jpeg = ue_tauri_lib::render_frame_impl(&state, 1_000_000, 1920).expect("preview frame");
    assert!(jpeg.len() > 1000, "a real frame came back");
    let frame = dir.join("preview_1s.jpg");
    std::fs::write(&frame, &jpeg).unwrap();

    // bright pixels anywhere in the subtitle band (y≈870..990 at 1080p): crop
    // the whole band in one pass and count light pixels
    let bright = |png: &std::path::Path| -> usize {
        let out = Command::new(&ffmpeg)
            .args(["-v", "error", "-i"])
            .arg(png)
            .args(["-vf", "crop=1400:130:300:865", "-f", "rawvideo", "-pix_fmt", "gray", "-"])
            .output()
            .unwrap();
        out.stdout.iter().filter(|&&p| p > 160).count()
    };
    assert!(bright(&frame) >= 50, "subtitle text visible in the preview band");

    // and BEFORE the first word starts (t=0.1s < 0.2s) there is no caption
    // (captions linger ~600 ms AFTER they end, so the gap is only at the head)
    let jpeg2 = ue_tauri_lib::render_frame_impl(&state, 100_000, 1920).expect("frame");
    let frame2 = dir.join("preview_before.jpg");
    std::fs::write(&frame2, &jpeg2).unwrap();
    assert_eq!(bright(&frame2), 0, "no caption before the first word");
}

/// get_transcript respects granularity + a time window, and find_words locates
/// a phrase with its timestamp and neighbour context (the ergonomics an agent
/// needs instead of the 100k-char word dump).
#[test]
fn transcript_granularity_and_word_search() {
    let state = AppState::new_default();
    let (asset_id, transcript_id);
    {
        let mut store = state.store.lock().unwrap();
        // a tiny asset with a transcript: "programar es trabajo de alguien mas"
        let asset = MediaAsset {
            id: Id::new(), kind: MediaKind::Audio, path: "a.wav".into(), content_hash: "h".into(),
            probe: ProbeInfo { duration_us: 6 * SEC, fps: None, width: 0, height: 0, rotation: 0,
                vcodec: None, acodec: Some("pcm".into()), audio_channels: 1, vfr: false },
            proxy: None, audio_conform: None, peaks: None, thumbnails: None, transcript: None, offline: false,
        };
        asset_id = asset.id;
        let words: Vec<Word> = ["programar", "es", "trabajo", "de", "alguien", "mas"]
            .iter()
            .enumerate()
            .map(|(i, w)| Word {
                text: (*w).into(),
                start_us: i as i64 * SEC,
                end_us: i as i64 * SEC + 800_000,
                confidence: 1.0, rejected: false, display: None,
            })
            .collect();
        let doc = TranscriptDoc {
            id: Id::new(), asset_id, language: "es".into(), model: "t".into(),
            words,
            segments: vec![Segment {
                text: "programar es trabajo de alguien mas".into(),
                start_us: 0, end_us: 6 * SEC, word_range: (0, 6), emotion: None, volume_rms: 0.0,
            }],
            global_avg_volume: 0.0,
        };
        transcript_id = doc.id;
        store.project.assets.push(asset);
        store.project.transcripts.push(doc);
    }
    let call = |name: &str, arguments: Value| tool_json(&rpc(&state, "tools/call", json!({ "name": name, "arguments": arguments })));

    // text granularity: just the words joined
    let text = call("get_transcript", json!({ "asset_id": asset_id.to_string(), "granularity": "text" }));
    assert_eq!(text["text"], "programar es trabajo de alguien mas");

    // phrases: carry timestamps
    let ph = call("get_transcript", json!({ "asset_id": asset_id.to_string(), "granularity": "phrases" }));
    assert!(ph["phrases"].as_array().unwrap()[0]["start_us"].is_i64());

    // words windowed to [2s, 4s) → only "trabajo" (2.0..2.8) and "de" (3.0..3.8)
    let win = call("get_transcript", json!({
        "asset_id": asset_id.to_string(), "granularity": "words",
        "start_us": 2 * SEC, "end_us": 4 * SEC,
    }));
    let ws: Vec<String> = win["words"].as_array().unwrap().iter()
        .map(|w| w["text"].as_str().unwrap().to_string()).collect();
    assert_eq!(ws, vec!["trabajo", "de"], "the window clipped the words");

    // find_words by transcript_id, with context
    let hits = call("find_words", json!({ "transcript_id": transcript_id.to_string(), "query": "trabajo", "context": 1 }));
    assert_eq!(hits["matches"], 1);
    let hit = &hits["hits"][0];
    assert_eq!(hit["start_us"], 2 * SEC);
    assert_eq!(hit["context"], "es trabajo de", "one neighbour on each side");

    // multi-word phrase match
    let phrase = call("find_words", json!({ "asset_id": asset_id.to_string(), "query": "alguien mas" }));
    assert_eq!(phrase["matches"], 1);
    assert_eq!(phrase["hits"][0]["start_us"], 4 * SEC);

    // a miss is zero hits, not an error
    let none = call("find_words", json!({ "asset_id": asset_id.to_string(), "query": "godot" }));
    assert_eq!(none["matches"], 0);
}

/// Clips read as friendly labels, and set_clip_name overrides the label
/// without touching the id.
#[test]
fn clip_labels_and_naming() {
    let (state, clip_id) = state_with_clip();
    let timeline = || tool_json(&rpc(&state, "tools/call", json!({ "name": "get_timeline", "arguments": {} })));
    let name = |t: &Value| -> String {
        t["tracks"].as_array().unwrap().iter()
            .flat_map(|tr| tr["clips"].as_array().unwrap())
            .find(|c| c["id"] == clip_id.to_string())
            .unwrap()["label"].as_str().unwrap().to_string()
    };

    // derived from the media file, not the ULID
    assert_eq!(name(&timeline()), "x.mp4");

    // custom name wins
    let resp = rpc(&state, "tools/call", json!({ "name": "set_clip_name", "arguments": {
        "clip_id": clip_id.to_string(), "name": "intro hook"
    }}));
    assert!(resp.pointer("/result/isError").is_none(), "{resp}");
    assert_eq!(name(&timeline()), "intro hook");
    // the id is unchanged
    assert!(state.store.lock().unwrap().project.clip(clip_id).is_some());

    // clearing falls back to the derived label (and is undoable)
    rpc(&state, "tools/call", json!({ "name": "set_clip_name", "arguments": {
        "clip_id": clip_id.to_string(), "name": ""
    }}));
    assert_eq!(name(&timeline()), "x.mp4");
    rpc(&state, "tools/call", json!({ "name": "undo", "arguments": {} }));
    assert_eq!(name(&timeline()), "intro hook", "undo restores the name");
}

/// export_video returns a job_id immediately and finishes via get_job_status.
/// A client-side timeout on the launching call is therefore never a failure —
/// the point of this whole mechanism (field bug: agents re-ran on timeout).
#[test]
fn export_runs_as_a_pollable_job() {
    use std::process::Command;
    let ffmpeg = ue_media::ffmpeg_bin();
    if Command::new(&ffmpeg).arg("-version").output().map(|o| !o.status.success()).unwrap_or(true) {
        eprintln!("NOTE: no ffmpeg; test skipped");
        return;
    }
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-job-export");
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("clip.mp4");
    if !src.exists() {
        Command::new(&ffmpeg)
            .args(["-y", "-v", "error", "-f", "lavfi", "-i", "testsrc=duration=2:size=160x120:rate=15"])
            .args(["-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p"])
            .arg(&src).status().unwrap();
    }
    let state = AppState::new_default();
    let call = |name: &str, a: Value| tool_json(&rpc(&state, "tools/call", json!({ "name": name, "arguments": a })));
    let asset = call("import_media", json!({ "paths": [src.to_string_lossy()] }));
    let asset_id = asset["assets"][0]["asset_id"].as_str().unwrap().to_string();
    call("add_clip", json!({ "asset_id": asset_id }));

    // launch: comes back immediately with a job_id and a poll hint
    let out = dir.join("out.mp4");
    let launched = call("export_video", json!({ "path": out.to_string_lossy(), "crf": 32 }));
    let job_id = launched["job_id"].as_str().expect("job_id").to_string();
    assert_eq!(launched["poll_with"], "get_job_status");

    // poll: done, and the real result carries the path
    let status = call("get_job_status", json!({ "job_id": job_id }));
    assert_eq!(status["status"], "done", "{status}");
    assert_eq!(status["kind"], "export");
    assert_eq!(status["result"]["path"], out.to_string_lossy().as_ref());
    assert!(out.exists(), "the export wrote a file");

    // list_jobs sees it; an unknown id errors (not a silent empty)
    let jobs = call("list_jobs", json!({}));
    assert!(jobs["jobs"].as_array().unwrap().iter().any(|j| j["job_id"] == job_id));
    let missing = rpc(&state, "tools/call", json!({ "name": "get_job_status", "arguments": { "job_id": "nope" } }));
    assert_eq!(missing.pointer("/result/isError").unwrap(), true);
}
/// THE GOLDEN RULE, enforced: the paused preview must equal the export frame
/// for frame, pixel by pixel. Builds a real multi-layer composition (a red
/// base with a green PiP overlay and a subtitle), renders it BOTH ways at the
/// same instant, and requires the sampled pixels to match within JPEG
/// tolerance.
#[test]
fn preview_matches_export_pixel_for_pixel() {
    use std::process::Command;
    let ffmpeg = ue_media::ffmpeg_bin();
    if Command::new(&ffmpeg).arg("-version").output().map(|o| !o.status.success()).unwrap_or(true) {
        eprintln!("NOTE: no ffmpeg; skipped");
        return;
    }
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-parity");
    std::fs::create_dir_all(&dir).unwrap();
    let red = dir.join("red.mp4");
    let green = dir.join("green.mp4");
    for (p, c) in [(&red, "red"), (&green, "green")] {
        if !p.exists() {
            Command::new(&ffmpeg)
                .args(["-y","-v","error","-f","lavfi","-i",&format!("color=c={c}:s=640x360:d=3:r=30")])
                .args(["-c:v","libx264","-preset","ultrafast","-pix_fmt","yuv420p"])
                .arg(p).status().unwrap();
        }
    }

    let state = AppState::new_default();
    let doc_id;
    {
        let mut store = state.store.lock().unwrap();
        let seq_id = store.project.active_sequence;
        store.project.sequence_mut(seq_id).unwrap().resolution = (1280, 720);
        let ra = ue_media::import_file(&red).unwrap();   let raid = ra.id; store.project.assets.push(ra);
        let ga = ue_media::import_file(&green).unwrap();  let gaid = ga.id; store.project.assets.push(ga);
        // subtitle over the whole thing
        let doc = TranscriptDoc {
            id: Id::new(), asset_id: raid, language: "es".into(), model: "t".into(),
            words: vec![Word { text: "hola".into(), start_us: 200_000, end_us: 2_600_000, confidence: 1.0, rejected: false, display: None }],
            segments: vec![Segment { text: "hola".into(), start_us: 200_000, end_us: 2_600_000, word_range: (0,1), emotion: None, volume_rms: 0.0 }],
            global_avg_volume: 0.0,
        };
        doc_id = doc.id;
        store.project.transcripts.push(doc);
        let seq = store.project.sequence_mut(seq_id).unwrap();
        seq.tracks.push(Track::new(TrackKind::Video, "V2"));
        seq.tracks.push(Track::new(TrackKind::Video, "V3"));
        let v1 = seq.tracks.iter().find(|t| t.name=="V1").unwrap().id;
        let v2 = seq.tracks.iter().find(|t| t.name=="V2").unwrap().id;
        let v3 = seq.tracks.iter().find(|t| t.name=="V3").unwrap().id;
        store.insert_clip(v1, Clip::new_media(raid, 0, 3*SEC, 0), InsertMode::Strict).unwrap();
        let mut g = Clip::new_media(gaid, 0, 3*SEC, 0);
        g.transform.scale = (0.4.into(), 0.4.into());
        store.insert_clip(v2, g, InsertMode::Strict).unwrap();
        // subtitles clip on top
        let mut style = TextStyle { size: 80.0, y_offset: 300.0, ..Default::default() };
        style.color = "#ffffff".into();
        store.insert_clip(v3, Clip {
            id: Id::new(),
            payload: ClipPayload::Subtitles { transcript_id: doc_id, style, mode: SubtitleMode::Phrase, max_words: None },
            start: 0, duration: 3*SEC, speed: 1.0, effects: vec![], transform: Default::default(),
            audio: Default::default(), transition_in: None, label_color: None, name: None, group: None,
        }, InsertMode::Strict).unwrap();
    }

    // export → frame at t=1s
    let out = dir.join("export.mp4");
    {
        let store = state.store.lock().unwrap();
        ue_export::export_sequence(&store.project, store.project.active_sequence, &dir, &out, &ue_export::ExportSettings::default()).unwrap();
    }
    let exp = dir.join("exp.png");
    Command::new(&ffmpeg).args(["-y","-v","error","-ss","1","-i"]).arg(&out).args(["-frames:v","1"]).arg(&exp).status().unwrap();

    // preview → frame at t=1s, same width as the export (1280)
    let jpeg = ue_tauri_lib::render_frame_impl(&state, 1_000_000, 1280).unwrap();
    let prev = dir.join("prev.png");
    std::fs::write(dir.join("prev.jpg"), &jpeg).unwrap();
    // decode the preview jpeg to png at the SAME size for a fair compare
    Command::new(&ffmpeg).args(["-y","-v","error","-i"]).arg(dir.join("prev.jpg"))
        .args(["-vf","scale=1280:720"]).arg(&prev).status().unwrap();

    let rgb = |f: &std::path::Path, x: u32, y: u32| -> (i32,i32,i32) {
        let o = Command::new(&ffmpeg).args(["-v","error","-i"]).arg(f)
            .args(["-vf",&format!("crop=1:1:{x}:{y}"),"-f","rawvideo","-pix_fmt","rgb24","-"]).output().unwrap();
        let p=o.stdout; (*p.first().unwrap_or(&0) as i32, *p.get(1).unwrap_or(&0) as i32, *p.get(2).unwrap_or(&0) as i32)
    };
    // sample points: base corners (RED), the green PiP centre, and the subtitle band
    let points = [(40,40),(1240,40),(40,680),(1240,680),(640,360),(640,620)];
    let mut worst = 0i32;
    for (x,y) in points {
        let e = rgb(&exp,x,y); let p = rgb(&prev,x,y);
        let d = (e.0-p.0).abs().max((e.1-p.1).abs()).max((e.2-p.2).abs());
        println!("({x},{y}) export={e:?} preview={p:?} Δ={d}");
        worst = worst.max(d);
    }
    // JPEG + scaler rounding: allow a small tolerance, but the composition
    // (which pixels are red/green/white) must match — a divergence would be ~255
    assert!(worst <= 40, "preview must match the export frame (worst channel Δ={worst})");
}

