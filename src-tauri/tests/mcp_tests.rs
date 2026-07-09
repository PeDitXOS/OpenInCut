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
