//! Embedded MCP server (PLAN §7.A, v0).
//!
//! Direct implementation of the MCP protocol (JSON-RPC 2.0 over streamable
//! HTTP, application/json response) on 127.0.0.1:4599/mcp, no SDK:
//! initialize, tools/list and tools/call. Loopback only. The dispatcher
//! (`handle_rpc`) is a pure function over AppState → testable without HTTP.
//!
//! Connecting from Claude Code:
//!   claude mcp add --transport http ubereditor http://127.0.0.1:4599/mcp

use std::sync::atomic::Ordering;

use serde_json::{json, Value};
use ue_core::model::TransitionRef;
use ue_core::ops::InsertMode;

use crate::AppState;

pub const MCP_PORT: u16 = 4599;

// ---------------------------------------------------------------------------
// Tools
// ---------------------------------------------------------------------------

fn tool_defs() -> Value {
    json!([
        {
            "name": "get_project_summary",
            "description": "Summary of the open project: name, duration, tracks, clips, media and save status.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        },
        {
            "name": "get_timeline",
            "description": "Complete timeline of the active sequence: tracks with their clips (ids, times in µs, payloads, effects, transitions).",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        },
        {
            "name": "get_media_pool",
            "description": "Imported media: id, path, kind, duration and technical metadata.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        },
        {
            "name": "get_effects_catalog",
            "description": "Catalog of available effects (core + user packs) with their parameters.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        },
        {
            "name": "split_clip",
            "description": "Splits a clip at the given timeline time (µs). Returns the resulting ids.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clip_id": { "type": "string" },
                    "t_us": { "type": "integer", "description": "timeline time in microseconds" }
                },
                "required": ["clip_id", "t_us"]
            }
        },
        {
            "name": "delete_clips",
            "description": "Deletes clips by id. With ripple=true it closes the gaps.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "ids": { "type": "array", "items": { "type": "string" } },
                    "ripple": { "type": "boolean", "default": false }
                },
                "required": ["ids"]
            }
        },
        {
            "name": "add_clip",
            "description": "Adds a clip of a media from the pool to the timeline (at at_us or at the end of the compatible track).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "asset_id": { "type": "string" },
                    "at_us": { "type": "integer", "default": 0 }
                },
                "required": ["asset_id"]
            }
        },
        {
            "name": "set_clip_transition",
            "description": "Sets (or removes, with duration_us=0) a cross fade in on a clip.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clip_id": { "type": "string" },
                    "duration_us": { "type": "integer", "description": "0 = remove transition" }
                },
                "required": ["clip_id", "duration_us"]
            }
        },
        {
            "name": "get_transcript",
            "description": "Word-level transcript of an asset (words with timestamps in µs and phrases). Empty if not transcribed yet.",
            "inputSchema": {
                "type": "object",
                "properties": { "asset_id": { "type": "string" } },
                "required": ["asset_id"]
            }
        },
        {
            "name": "remove_silences",
            "description": "Detects a clip's silences and cuts them (mode=delete) or speeds them up 4x (mode=speedup); all tracks, 1 undo. Optional detection parameters.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clip_id": { "type": "string" },
                    "mode": { "type": "string", "enum": ["delete", "speedup", "split"] },
                    "threshold_db": { "type": "number", "description": "dBFS threshold (def -38)" },
                    "min_silence_ms": { "type": "integer", "description": "minimum silence in ms (def 400)" },
                    "pad_ms": { "type": "integer", "description": "margin around speech in ms (def 150)" }
                },
                "required": ["clip_id"]
            }
        },
        {
            "name": "export_video",
            "description": "Render the active sequence to a file. Optionally pass `ranges`: a list of [start_us, end_us] pieces of the timeline that are concatenated in order (render several chunks in one file). Blocking; returns the output path.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "absolute output path (.mp4/.m4a/.gif)" },
                    "ranges": {
                        "type": "array",
                        "items": {
                            "type": "array",
                            "items": { "type": "integer" },
                            "minItems": 2,
                            "maxItems": 2
                        },
                        "description": "[[start_us, end_us], …] pieces, concatenated in order"
                    },
                    "max_height": { "type": "integer" },
                    "crf": { "type": "integer" },
                    "loudnorm": { "type": "boolean" },
                    "format": { "type": "string", "enum": ["mp4", "m4a", "gif"] }
                },
                "required": ["path"]
            }
        },
        {
            "name": "playback",
            "description": "Drive the real player for debugging: action play (from_us), pause, or position.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["play", "pause", "position"] },
                    "from_us": { "type": "integer" }
                },
                "required": ["action"]
            }
        },
        {
            "name": "debug_render_frame",
            "description": "Render the paused-preview frame at t_us through the exact production path, write it to a temp JPEG and return {path, bytes}. For visual debugging.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "t_us": { "type": "integer" },
                    "max_width": { "type": "integer" }
                },
                "required": ["t_us"]
            }
        },
        {
            "name": "debug_playback_frame",
            "description": "Dump the CURRENT playback-stream frame buffer to a temp JPEG and return {path, bytes} (0 = empty buffer).",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        },
        {
            "name": "replace_words",
            "description": "Fix transcription errors: replaces every whole-word occurrence in a transcript (case-insensitive) with a corrected label. The audio timing is untouched; captions show the correction. Undoable.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "transcript_id": { "type": "string" },
                    "from": { "type": "string" },
                    "to": { "type": "string" }
                },
                "required": ["transcript_id", "from", "to"]
            }
        },
        {
            "name": "move_range",
            "description": "Moves the timeline range [from_us, to_us) to dest_us (reorders material on all tracks; 1 undo). Useful for reordering phrases.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "from_us": { "type": "integer" },
                    "to_us": { "type": "integer" },
                    "dest_us": { "type": "integer" }
                },
                "required": ["from_us", "to_us", "dest_us"]
            }
        },
        {
            "name": "generate_vertical",
            "description": "Generates a vertical 1080x1920 sequence (blurred background + centered video) from the active sequence and activates it. Undoable.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        },
        {
            "name": "undo",
            "description": "Undoes the last edit.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        },
        {
            "name": "redo",
            "description": "Redoes the last undone edit.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        }
    ])
}

fn text_result(v: Value) -> Value {
    json!({ "content": [{ "type": "text", "text": v.to_string() }] })
}

fn tool_error(msg: &str) -> Value {
    json!({ "content": [{ "type": "text", "text": msg }], "isError": true })
}

fn call_tool(state: &AppState, app: Option<&tauri::AppHandle>, name: &str, args: &Value) -> Value {
    let parse_id = |key: &str| -> Result<ue_core::model::Id, String> {
        args.get(key)
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("missing {key}"))?
            .parse()
            .map_err(|e| format!("invalid {key}: {e}"))
    };

    match name {
        "get_project_summary" => {
            let store = state.store.lock().unwrap();
            let p = &store.project;
            let seq = p.sequence(p.active_sequence);
            text_result(json!({
                "name": p.name,
                "dirty": store.dirty,
                "assets": p.assets.len(),
                "sequence": seq.map(|s| json!({
                    "name": s.name,
                    "resolution": s.resolution,
                    "fps": s.fps,
                    "duration_us": s.duration_us(),
                    "tracks": s.tracks.iter().map(|t| json!({
                        "id": t.id.to_string(),
                        "name": t.name,
                        "kind": t.kind,
                        "clips": t.clips.len(),
                    })).collect::<Vec<_>>(),
                })),
                "undo_history": store.undo_labels(),
            }))
        }
        "get_timeline" => {
            let store = state.store.lock().unwrap();
            let seq = store.project.sequence(store.project.active_sequence);
            text_result(serde_json::to_value(seq).unwrap_or(Value::Null))
        }
        "get_media_pool" => {
            let store = state.store.lock().unwrap();
            text_result(serde_json::to_value(&store.project.assets).unwrap_or(Value::Null))
        }
        "get_effects_catalog" => {
            text_result(ue_render::catalog_json(&state.registry.lock().unwrap()))
        }
        "split_clip" => {
            let clip_id = match parse_id("clip_id") {
                Ok(v) => v,
                Err(e) => return tool_error(&e),
            };
            let Some(t_us) = args.get("t_us").and_then(|v| v.as_i64()) else {
                return tool_error("missing t_us");
            };
            let mut store = state.store.lock().unwrap();
            match store.split_clip(clip_id, t_us) {
                Ok((l, r)) => text_result(json!({ "left": l.to_string(), "right": r.to_string() })),
                Err(e) => tool_error(&e.to_string()),
            }
        }
        "delete_clips" => {
            let Some(ids) = args.get("ids").and_then(|v| v.as_array()) else {
                return tool_error("missing ids");
            };
            let parsed: Result<Vec<ue_core::model::Id>, _> = ids
                .iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.parse::<ue_core::model::Id>())
                .collect();
            let ripple = args.get("ripple").and_then(|v| v.as_bool()).unwrap_or(false);
            match parsed {
                Ok(ids) => {
                    let mut store = state.store.lock().unwrap();
                    match store.delete_clips(&ids, ripple) {
                        Ok(()) => text_result(json!({ "deleted": ids.len() })),
                        Err(e) => tool_error(&e.to_string()),
                    }
                }
                Err(e) => tool_error(&format!("invalid id: {e}")),
            }
        }
        "add_clip" => {
            let asset_id = match parse_id("asset_id") {
                Ok(v) => v,
                Err(e) => return tool_error(&e),
            };
            let at_us = args.get("at_us").and_then(|v| v.as_i64()).unwrap_or(0);
            match add_clip_inner(state, asset_id, at_us) {
                Ok(clip_id) => text_result(json!({ "clip_id": clip_id })),
                Err(e) => tool_error(&e),
            }
        }
        "set_clip_transition" => {
            let clip_id = match parse_id("clip_id") {
                Ok(v) => v,
                Err(e) => return tool_error(&e),
            };
            let dur = args.get("duration_us").and_then(|v| v.as_i64()).unwrap_or(0);
            let transition = (dur > 0).then(|| TransitionRef {
                effect_id: "core.crossfade".into(),
                duration: dur,
                params: Default::default(),
            });
            let mut store = state.store.lock().unwrap();
            match store.dispatch(
                "[MCP] Edit transition",
                vec![ue_core::Action::SetClipTransition { clip_id, transition }],
            ) {
                Ok(()) => text_result(json!({ "ok": true })),
                Err(e) => tool_error(&e.to_string()),
            }
        }
        "get_transcript" => {
            let asset_id = match parse_id("asset_id") {
                Ok(v) => v,
                Err(e) => return tool_error(&e),
            };
            let store = state.store.lock().unwrap();
            match store.project.transcripts.iter().find(|t| t.asset_id == asset_id) {
                Some(doc) => text_result(serde_json::to_value(doc).unwrap_or(Value::Null)),
                None => tool_error("the asset has no transcript yet (use transcribe in the UI)"),
            }
        }
        "remove_silences" => {
            let clip_id = match parse_id("clip_id") {
                Ok(v) => v,
                Err(e) => return tool_error(&e),
            };
            let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("delete").to_string();
            let mut params = ue_ai::silence::SilenceParams::default();
            if let Some(db) = args.get("threshold_db").and_then(|v| v.as_f64()) {
                params.threshold_db = db.clamp(-80.0, -10.0);
            }
            if let Some(ms) = args.get("min_silence_ms").and_then(|v| v.as_i64()) {
                params.min_silence_us = ms.clamp(50, 5000) * 1000;
            }
            if let Some(ms) = args.get("pad_ms").and_then(|v| v.as_i64()) {
                params.pad_pre_us = ms.clamp(0, 1000) * 1000;
                params.pad_post_us = ms.clamp(0, 1000) * 1000;
            }
            match remove_silences_inner(state, clip_id, &mode, &params) {
                Ok((n, us)) => text_result(json!({ "removed": n, "removed_us": us })),
                Err(e) => tool_error(&e),
            }
        }
        "export_video" => {
            let Some(path) = args.get("path").and_then(|v| v.as_str()) else {
                return tool_error("missing path");
            };
            let ranges: Vec<(i64, i64)> = args
                .get("ranges")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|p| {
                            let p = p.as_array()?;
                            Some((p.first()?.as_i64()?, p.get(1)?.as_i64()?))
                        })
                        .filter(|(a, b)| b > a)
                        .collect()
                })
                .unwrap_or_default();
            let format = match args.get("format").and_then(|v| v.as_str()) {
                None | Some("mp4") => ue_export::ExportFormat::Mp4,
                Some("m4a") => ue_export::ExportFormat::M4a,
                Some("gif") => ue_export::ExportFormat::Gif,
                Some(o) => return tool_error(&format!("unknown format: {o}")),
            };
            let defaults = ue_export::ExportSettings::default();
            let settings = ue_export::ExportSettings {
                format,
                max_height: args.get("max_height").and_then(|v| v.as_u64()).map(|v| v as u32),
                crf: args
                    .get("crf")
                    .and_then(|v| v.as_u64())
                    .map(|v| (v as u8).clamp(10, 40))
                    .unwrap_or(defaults.crf),
                loudnorm: args.get("loudnorm").and_then(|v| v.as_bool()).unwrap_or(false),
                ranges,
                extra_packs: state.user_packs.lock().unwrap().clone(),
                ..defaults
            };
            let (project, seq_id, base_dir) = {
                let store = state.store.lock().unwrap();
                let base = state
                    .path
                    .lock()
                    .unwrap()
                    .as_ref()
                    .and_then(|p| p.parent().map(|d| d.to_path_buf()))
                    .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
                (store.project.clone(), store.project.active_sequence, base)
            };
            let cancel = state.export_cancel.clone();
            cancel.store(false, std::sync::atomic::Ordering::SeqCst);
            match ue_export::export_sequence_with_progress(
                &project,
                seq_id,
                &base_dir,
                std::path::Path::new(path),
                &settings,
                |_| {},
                &cancel,
            ) {
                Ok(()) => text_result(json!({ "path": path })),
                Err(e) => tool_error(&e.to_string()),
            }
        }
        "playback" => {
            let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("");
            match action {
                "play" => {
                    let from = args.get("from_us").and_then(|v| v.as_i64()).unwrap_or(0);
                    match crate::playback_play_impl(state, app, from) {
                        Ok(()) => text_result(json!({ "playing": true, "from_us": from })),
                        Err(e) => tool_error(&e),
                    }
                }
                "pause" => {
                    crate::stop_frame_service(state);
                    let guard = state.player.lock().unwrap();
                    match guard.as_ref() {
                        Some(p) => text_result(json!({ "paused_at_us": p.pause() })),
                        None => tool_error("no player"),
                    }
                }
                "position" => {
                    let guard = state.player.lock().unwrap();
                    match guard.as_ref() {
                        Some(p) => text_result(
                            json!({ "t_us": p.position_us(), "playing": p.is_playing() }),
                        ),
                        None => tool_error("no player"),
                    }
                }
                other => tool_error(&format!("unknown action: {other}")),
            }
        }
        "debug_render_frame" => {
            let t_us = args.get("t_us").and_then(|v| v.as_i64()).unwrap_or(0);
            let max_width =
                args.get("max_width").and_then(|v| v.as_i64()).unwrap_or(1280) as u32;
            match crate::render_frame_impl(state, t_us, max_width) {
                Ok(bytes) => {
                    let path = std::env::temp_dir().join("ue_debug_frame.jpg");
                    let n = bytes.len();
                    if let Err(e) = std::fs::write(&path, bytes) {
                        return tool_error(&e.to_string());
                    }
                    text_result(json!({ "path": path.display().to_string(), "bytes": n }))
                }
                Err(e) => tool_error(&e),
            }
        }
        "debug_playback_frame" => {
            let bytes = state
                .frames
                .lock()
                .unwrap()
                .as_ref()
                .map(|f| f.latest.lock().unwrap().clone())
                .unwrap_or_default();
            let path = std::env::temp_dir().join("ue_debug_stream.jpg");
            let n = bytes.len();
            let _ = std::fs::write(&path, &bytes);
            text_result(json!({ "path": path.display().to_string(), "bytes": n }))
        }
        "replace_words" => {
            let transcript_id = match parse_id("transcript_id") {
                Ok(v) => v,
                Err(e) => return tool_error(&e),
            };
            let (Some(from), Some(to)) = (
                args.get("from").and_then(|v| v.as_str()),
                args.get("to").and_then(|v| v.as_str()),
            ) else {
                return tool_error("missing from/to");
            };
            let needle = from.trim().to_lowercase();
            let mut store = state.store.lock().unwrap();
            let Some(doc) = store.project.transcripts.iter().find(|t| t.id == transcript_id)
            else {
                return tool_error("transcript not found");
            };
            let matches: Vec<usize> = doc
                .words
                .iter()
                .enumerate()
                .filter(|(_, w)| w.label().trim().to_lowercase() == needle)
                .map(|(i, _)| i)
                .collect();
            let display =
                if to.trim().is_empty() { None } else { Some(to.trim().to_string()) };
            let actions: Vec<ue_core::Action> = matches
                .iter()
                .map(|i| ue_core::Action::SetWordText {
                    transcript_id,
                    index: *i,
                    display: display.clone(),
                })
                .collect();
            let n = actions.len();
            if n > 0 {
                if let Err(e) = store.dispatch("Replace words", actions) {
                    return tool_error(&e.to_string());
                }
            }
            text_result(json!({ "replaced": n }))
        }
        "move_range" => {
            let get = |k: &str| args.get(k).and_then(|v| v.as_i64());
            let (Some(f), Some(t), Some(d)) = (get("from_us"), get("to_us"), get("dest_us"))
            else {
                return tool_error("missing from_us/to_us/dest_us");
            };
            let mut store = state.store.lock().unwrap();
            let seq_id = store.project.active_sequence;
            match store.move_range(seq_id, f, t, d) {
                Ok(()) => text_result(json!({ "ok": true })),
                Err(e) => tool_error(&e.to_string()),
            }
        }
        "generate_vertical" => match generate_vertical_inner(state) {
            Ok(id) => text_result(json!({ "sequence_id": id })),
            Err(e) => tool_error(&e),
        },
        "undo" => {
            let mut store = state.store.lock().unwrap();
            match store.undo() {
                Ok(label) => text_result(json!({ "undone": label })),
                Err(e) => tool_error(&e.to_string()),
            }
        }
        "redo" => {
            let mut store = state.store.lock().unwrap();
            match store.redo() {
                Ok(label) => text_result(json!({ "redone": label })),
                Err(e) => tool_error(&e.to_string()),
            }
        }
        _ => tool_error(&format!("unknown tool: {name}")),
    }
}

fn generate_vertical_inner(state: &AppState) -> Result<String, String> {
    // same flow as the UI command
    crate::generate_vertical_impl(state)
}

fn remove_silences_inner(
    state: &AppState,
    clip_id: ue_core::model::Id,
    mode: &str,
    params: &ue_ai::silence::SilenceParams,
) -> Result<(usize, i64), String> {
    let mut store = state.store.lock().unwrap();
    let clip = store.project.clip(clip_id).ok_or("clip not found")?.clone();
    let ue_core::model::ClipPayload::Media { asset_id, src_in, src_out } = clip.payload else {
        return Err("the clip is not media".into());
    };
    let asset = store.project.asset(asset_id).ok_or("asset not found")?;
    let conform = asset.audio_conform.clone().ok_or("audio not conformed yet")?;
    let wav = ue_audio::wav::WavMap::open(std::path::Path::new(&conform))
        .map_err(|e| e.to_string())?;
    let ranges =
        ue_ai::silence::clip_silences_on_timeline(&wav, clip.start, src_in, src_out, params);
    if ranges.is_empty() {
        return Ok((0, 0));
    }
    let removed_us: i64 = ranges.iter().map(|(s, e)| e - s).sum();
    let seq_id = store.project.active_sequence;
    match mode {
        "speedup" => store.speedup_ranges(seq_id, &ranges, 4.0).map_err(|e| e.to_string())?,
        "split" => store.split_ranges(seq_id, &ranges).map_err(|e| e.to_string())?,
        _ => store.cut_ranges(seq_id, &ranges, true).map_err(|e| e.to_string())?,
    }
    Ok((ranges.len(), removed_us))
}

/// Same as the UI's add_clip command (a small, deliberate duplication).
fn add_clip_inner(state: &AppState, asset_id: ue_core::model::Id, at_us: i64) -> Result<String, String> {
    use ue_core::model::{Clip, MediaKind, TrackKind};
    let mut store = state.store.lock().unwrap();
    let asset = store
        .project
        .asset(asset_id)
        .ok_or_else(|| format!("asset {asset_id} does not exist"))?
        .clone();
    let duration = ue_media::default_clip_duration(&asset);
    if duration <= 0 {
        return Err("the file has no usable duration".into());
    }
    let want = if asset.kind == MediaKind::Audio { TrackKind::Audio } else { TrackKind::Video };
    let seq_id = store.project.active_sequence;
    let seq = store.project.sequence(seq_id).ok_or("no active sequence")?;
    let track = seq
        .tracks
        .iter()
        .find(|t| t.kind == want && !t.locked)
        .ok_or("no compatible track")?;
    let track_id = track.id;
    let at = at_us.max(0);
    let start = if track.collides(at, duration, None) {
        track.clips.iter().map(|c| c.end()).max().unwrap_or(0)
    } else {
        at
    };
    let clip = Clip::new_media(asset.id, 0, duration, start);
    let clip_id = clip.id;
    store.insert_clip(track_id, clip, InsertMode::Strict).map_err(|e| e.to_string())?;
    Ok(clip_id.to_string())
}

// ---------------------------------------------------------------------------
// JSON-RPC
// ---------------------------------------------------------------------------

/// Processes a JSON-RPC message. `None` = notification with no response.
pub fn handle_rpc(state: &AppState, app: Option<&tauri::AppHandle>, req: &Value) -> Option<Value> {
    let method = req.get("method")?.as_str()?;
    let id = req.get("id").cloned();
    // notifications (no id) carry no response
    if id.is_none() || id == Some(Value::Null) {
        return None;
    }
    let id = id.unwrap();

    let result = match method {
        "initialize" => {
            let requested = req
                .pointer("/params/protocolVersion")
                .and_then(|v| v.as_str())
                .unwrap_or("2025-06-18");
            json!({
                "protocolVersion": requested,
                "capabilities": { "tools": {} },
                "serverInfo": {
                    "name": "ubereditor",
                    "title": "UberEditor",
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "instructions": "UberEditor video editor. Read the state with get_project_summary/get_timeline; edit with split_clip/delete_clips/add_clip. Every edit is undoable (undo)."
            })
        }
        "ping" => json!({}),
        "tools/list" => json!({ "tools": tool_defs() }),
        "tools/call" => {
            let name = req.pointer("/params/name").and_then(|v| v.as_str()).unwrap_or("");
            let empty = json!({});
            let args = req.pointer("/params/arguments").unwrap_or(&empty);
            ue_core::dlog("mcp", &format!("tool {name} {args}"));
            call_tool(state, app, name, args)
        }
        _ => {
            return Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": format!("unsupported method: {method}") }
            }));
        }
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

/// Starts the server on a thread. Returns the port if it could listen.
pub fn start(app: tauri::AppHandle) -> Option<u16> {
    let server = tiny_http::Server::http(("127.0.0.1", MCP_PORT)).ok()?;
    std::thread::Builder::new()
        .name("ue-mcp".into())
        .spawn(move || {
            use tauri::Manager;
            for mut request in server.incoming_requests() {
                let state = app.state::<AppState>();
                if state.mcp_shutdown.load(Ordering::SeqCst) {
                    break;
                }
                // authentication: Authorization: Bearer <token>
                let expected = state.mcp_token.lock().unwrap().clone();
                let authorized = request.headers().iter().any(|h| {
                    h.field.as_str().as_str().eq_ignore_ascii_case("authorization")
                        && h.value.as_str().trim() == format!("Bearer {expected}")
                });
                if !authorized {
                    let _ = request.respond(
                        tiny_http::Response::from_string(
                            r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32001,"message":"invalid token: use Authorization: Bearer <token> (shown in the app's MCP pill)"}}"#,
                        )
                        .with_status_code(401),
                    );
                    continue;
                }
                let mut body = String::new();
                let _ = request.as_reader().read_to_string(&mut body);
                let response = match serde_json::from_str::<Value>(&body) {
                    Ok(msg) => handle_rpc(&state, Some(&app), &msg),
                    Err(_) => Some(json!({
                        "jsonrpc": "2.0", "id": Value::Null,
                        "error": { "code": -32700, "message": "invalid JSON" }
                    })),
                };
                let (status, text) = match response {
                    Some(v) => (200, v.to_string()),
                    None => (202, String::new()),
                };
                let header =
                    tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
                        .unwrap();
                let _ = request.respond(
                    tiny_http::Response::from_string(text)
                        .with_status_code(status)
                        .with_header(header),
                );
            }
        })
        .ok()?;
    Some(MCP_PORT)
}
