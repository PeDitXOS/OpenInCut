//! Tests del dispatcher MCP (handle_rpc es puro sobre AppState: sin HTTP).

use serde_json::{json, Value};
use ue_core::model::*;
use ue_core::ops::InsertMode;
use ue_tauri_lib::mcp::handle_rpc;
use ue_tauri_lib::AppState;

const SEC: i64 = 1_000_000;

fn rpc(state: &AppState, method: &str, params: Value) -> Value {
    let req = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    handle_rpc(state, &req).expect("con id siempre hay respuesta")
}

/// Texto del primer content de un tool result, parseado como JSON.
fn tool_json(resp: &Value) -> Value {
    let text = resp
        .pointer("/result/content/0/text")
        .and_then(|v| v.as_str())
        .expect("tool result con texto");
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
    assert!(list.len() >= 10, "hay al menos 10 herramientas, fueron {}", list.len());
    assert!(list.iter().any(|t| t["name"] == "split_clip"));

    // notificación → sin respuesta
    let note = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
    assert!(handle_rpc(&state, &note).is_none());

    // método desconocido → error JSON-RPC
    let err = rpc(&state, "no/existe", json!({}));
    assert_eq!(err.pointer("/error/code").unwrap(), -32601);
}

#[test]
fn summary_and_timeline_reflect_project() {
    let (state, _clip) = state_with_clip();
    let resp = rpc(&state, "tools/call", json!({ "name": "get_project_summary", "arguments": {} }));
    let summary = tool_json(&resp);
    assert_eq!(summary["assets"], 1);
    assert_eq!(summary["sequence"]["duration_us"], 10 * SEC);

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
        assert_eq!(total, 2, "el split via MCP partió el clip");
    }

    let resp = rpc(&state, "tools/call", json!({ "name": "undo", "arguments": {} }));
    assert!(tool_json(&resp)["undone"].is_string());
    {
        let store = state.store.lock().unwrap();
        let seq = store.project.sequence(store.project.active_sequence).unwrap();
        let total: usize = seq.tracks.iter().map(|t| t.clips.len()).sum();
        assert_eq!(total, 1, "undo via MCP restauró el clip");
    }

    // errores de herramienta van como isError, no como error JSON-RPC
    let resp = rpc(
        &state,
        "tools/call",
        json!({ "name": "split_clip", "arguments": { "clip_id": "no-es-un-id", "t_us": 1 } }),
    );
    assert_eq!(resp.pointer("/result/isError").unwrap(), true);
}

/// remove_silences de punta a punta: video con tono-silencio-tono real,
/// conformado incluido. Verifica cortes y duración final.
#[test]
fn remove_silences_via_mcp_cuts_the_gap() {
    let ff_ok = std::process::Command::new(ue_media::ffmpeg_bin())
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !ff_ok {
        eprintln!("AVISO: sin ffmpeg; test saltado");
        return;
    }
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-mcp-media");
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("tono_silencio_tono.mp4");
    // 2s tono + 2s silencio + 2s tono, con video de color
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
        // conformar el audio a mano (en la app lo hace el job de import)
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
    assert_eq!(result["removed"], 1, "un silencio central: {result}");
    let removed_us = result["removed_us"].as_i64().unwrap();
    assert!((1_200_000..=2_200_000).contains(&removed_us), "≈2 s menos padding: {removed_us}");

    let store = state.store.lock().unwrap();
    let seq = store.project.sequence(store.project.active_sequence).unwrap();
    let dur = seq.duration_us();
    assert!((3_800_000..=4_900_000).contains(&dur), "duración final ≈ 6s - silencio: {dur}");
    let clips: usize = seq.tracks.iter().map(|t| t.clips.len()).sum();
    assert_eq!(clips, 2, "el clip quedó partido en dos alrededor del silencio");
}

/// El desenlace rompe el grupo entero y es deshacible.
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
    // desenlazar via las mismas acciones que usa el comando
    {
        let mut store = state.store.lock().unwrap();
        let members = ue_core::ops::linked_ids(&store.project, v_id);
        assert_eq!(members.len(), 2);
        let actions = members
            .into_iter()
            .map(|clip_id| ue_core::Action::SetClipGroup { clip_id, group: None })
            .collect();
        store.dispatch("Desenlazar clips", actions).unwrap();
        assert_eq!(ue_core::ops::linked_ids(&store.project, v_id).len(), 1, "grupo roto");
        store.undo().unwrap();
        assert_eq!(ue_core::ops::linked_ids(&store.project, v_id).len(), 2, "undo re-enlaza");
    }
}
