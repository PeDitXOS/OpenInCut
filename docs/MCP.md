# MCP server — agentic editing

UberEditor embeds an [MCP](https://modelcontextprotocol.io) server so an agent
(Claude Code, or anything that speaks MCP) can drive the editor: import
footage, transcribe it, cut it, animate it, subtitle it and render it — without
touching the GUI.

The design goal is **parity**: every feature a human can reach through the UI is
reachable through a tool, over the same code path. A change that adds a UI
feature and no tool is a bug (`tests/mcp_tests.rs::tools_cover_the_whole_editor_and_are_documented`
fails on purpose).

---

## Connecting

The server starts with the app on `http://127.0.0.1:4599/mcp`. It is
**loopback-only** and **requires a Bearer token**, regenerated on each startup
and shown in the **MCP** pill in the app header (click it to copy the command):

```bash
claude mcp add --transport http ubereditor http://127.0.0.1:4599/mcp \
  --header "Authorization: Bearer <token>"
```

Anything else gets `401` with a JSON-RPC error. Transport is JSON-RPC 2.0 over
HTTP POST (`initialize`, `ping`, `tools/list`, `tools/call`); notifications get
`202` and no body.

Quick check without an agent:

```bash
TOKEN=$(cat ~/Library/Application\ Support/net.pequesoft.ubereditor/mcp_token)   # macOS
curl -s http://127.0.0.1:4599/mcp \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call",
       "params":{"name":"get_project_summary","arguments":{}}}' | jq -r '.result.content[0].text' | jq
```

---

## The five rules

1. **Time is always integer microseconds (µs) on the timeline.** Never seconds,
   never frames. `1 s = 1_000_000`. A float is rejected at the boundary.
2. **One tool call = one undo entry.** `set_clip_properties` can change the
   transform, the audio, the effects and the speed at once; a single `undo`
   reverts all of it. The user can undo an agent's work from the UI.
3. **A failed call changes nothing.** Every mutation goes through
   `ProjectStore::dispatch`, which validates the project invariants and rolls
   back atomically. There is no half-applied state.
4. **Errors are tool errors, not protocol errors** — `isError: true` and a
   message meant to be acted on ("the audio is still being prepared…").
5. **Ids are ULID strings.** Get them from `get_timeline`, `get_media_pool` or
   `get_project_summary`. Never invent one.

---

## Typical session

```jsonc
get_project_summary {}                       // what is open? which sequence is active?
import_media { "paths": ["/abs/take1.mp4"] } // → asset_id
transcribe_asset { "asset_id": "…" }         // blocks; → transcript_id  (needed by 4 other tools)
add_clip { "asset_id": "…" }                 // → clip_id
remove_silences { "clip_id": "…", "mode": "delete" }
add_subtitles_clip { "clip_id": "…" }
export_video { "path": "/abs/out.mp4" }
save_project { "path": "/abs/project.uep" }
```

**Timing gotcha.** `import_media` returns immediately, but the audio *conform*
(the WAV the analysis tools read) is built in the background. Until it lands,
`transcribe_asset`, `remove_silences` and `generate_avatar_video` fail with
_"the audio is still being prepared (conform); try again in a few seconds"_.
Poll `get_media_pool` and wait for `audio_conform` to be non-null.

**Slow tools.** `transcribe_asset`, `generate_avatar_video` and `export_video`
block until they finish — minutes, and the first transcription also downloads
the Whisper model. The server is single-threaded, so nothing else runs
meanwhile.

---

## Tools

47 tools. `tools/list` carries the full schema for each, plus MCP annotations:

| annotation | meaning |
|---|---|
| `readOnlyHint` | reads state, changes nothing — always safe to call |
| `destructiveHint` | **cannot** be reverted with `undo` (writes a file, replaces the project, adds an asset) |
| neither | mutates the project; one `undo` reverts it |

### Reading state

| Tool | What you get |
|---|---|
| `get_project_summary` | name, save path, every sequence (id, resolution, fps, duration, tracks), asset/transcript counts, avatar setups, undo history |
| `get_timeline` | the full sequence: tracks → clips with payload, transform, audio, effects, transition |
| `get_media_pool` | assets: id, path, kind, duration, probe, and whether `audio_conform`/`proxy`/`transcript` are ready |
| `get_transcript` | words with µs timestamps, `display` overrides, segments with emotion and volume |
| `get_catalog` | effect ids + their params, generator ids, installed font families, saved text templates, avatar setups, subtitle modes, transition ids |

### Media

- **`import_media`** `{paths[]}` → asset ids. Idempotent by content hash. Does
  not place anything on the timeline.
- **`transcribe_asset`** `{asset_id, model?}` → `{transcript_id, words}`.
  Word-level Whisper. Blocking. Required by `add_subtitles_clip`,
  `replace_words`, `set_word_text` and `generate_avatar_video`.
- **`set_project_settings`** `{whisper_language?, whisper_model?}` — the
  defaults `transcribe_asset` uses. Set the language (`es`, `en`, `auto`)
  *before* transcribing.
- **`relink_asset`** `{asset_id, new_path}` — repairs media flagged `offline`
  after a project's footage moved.

### Timeline structure

`add_clip`, `add_text_clip`, `add_generator_clip`, `add_subtitles_clip`,
`split_clip`, `delete_clips`, `move_clip`, `trim_clip`, `unlink_clip`,
`cut_ranges`, `move_range`.

`cut_ranges` and `move_range` operate on **all tracks at once** and are the
right tools for text-based editing: read the word timestamps from
`get_transcript`, then cut or reorder those ranges.

### Clip properties

**`set_clip_properties`** is the workhorse. `transform` and `audio` are
**partial patches** (only the keys you send change); `effects` **replaces** the
chain. Everything numeric also accepts a keyframe curve.

```jsonc
set_clip_properties {
  "clip_id": "01J…",
  "transform": {
    "position_x": 120,                       // px from the canvas centre
    "opacity": { "keys": [                   // …or an animated curve
      { "t": 0,       "value": 0, "interp": { "kind": "linear" } },
      { "t": 1000000, "value": 1, "interp": { "kind": "linear" } }
    ]}
  },
  "audio": { "gain_db": -6, "denoise": true },
  "effects": [{ "effect_id": "core.blur", "params": { "sigma": 8 } }],
  "transition_in": { "duration_us": 500000 },  // null removes it
  "speed": 1.5                                  // pitch preserved
}
```

Curve keys are `{t, value, interp}` where `t` is **µs from the start of the
clip** (not the timeline) and `interp.kind` is `linear`, `hold` or `smooth`.
Keys are sorted and de-duplicated on write.

Transform patch keys: `position_x/y`, `scale_x/y`, `rotation` (degrees),
`opacity` (0..1), `crop_left/top/right/bottom` (0..1), `flip_h`, `flip_v`.

**`set_clip_content`** edits what a clip *shows*, depending on its payload: the
words and style of a Text clip, the style and `subtitles_mode`
(`phrase|word|karaoke`) of a Subtitles clip, or the parameters of a Generator
clip. `style` is a patch too.

### Tracks and sequences

`add_track`, `remove_track`, `set_track_prop` (exactly one of `name`, `muted`,
`solo`, `locked`, `volume_db` per call); `set_sequence_props` (resolution/fps),
`set_active_sequence`, `remove_sequence`, `generate_vertical`.

### AI

- **`remove_silences`** `{clip_id, mode: delete|speedup|split, threshold_db?, min_silence_ms?, pad_ms?}`
  — `delete` cuts and closes the gaps, `speedup` runs them at 4×, `split` only
  cuts at the edges. All tracks, one undo.
- **`replace_words`** `{transcript_id, from, to}` — fixes a recurring
  mis-transcription everywhere (`godo` → `Godot`). Audio untouched; captions
  show the correction.
- **`set_word_text`** `{transcript_id, index, text}` — one word by index.
- **`save_avatar_config`** `{config}` → `config_id`; `remove_avatar_config`,
  `import_avatar_config` (ours or a Youtubers-toolkit `config.json`; same name
  replaces instead of duplicating) and `export_avatar_config` (**never** writes
  the `api_key` out).
- **`generate_avatar_video`** `{config_id, driver_asset}` → a transparent avatar
  video, imported as an asset. **The driver is the voice**: only the asset's
  transcript and audio matter, never its video. Blocking, minutes.

### Project, render, history

- `new_project`, `open_project`, `save_project` — the first two **discard the
  open project and its history**.
- `reload_effect_packs` — re-read the user effect packs after writing a
  manifest to disk, so an agent can extend the editor and use the new effect
  in the same session.
- **`export_video`** `{path, ranges?, format?, max_height?, crf?, loudnorm?}` —
  blocking. `ranges: [[start_us, end_us], …]` renders several chunks of the
  timeline concatenated **into one file**, in the order given (the "pieces"
  feature). Omit it to render everything.
- `undo`, `redo`.

### Debugging what the user sees

The paused preview, the playback stream and the export are **three different
code paths**. When something looks wrong, check the one that is actually broken:

| Tool | Path |
|---|---|
| `debug_render_frame {t_us}` | the **paused** preview → temp JPEG, `{path, bytes}` |
| `debug_playback_frame {}` | whatever is in the **playback** stream buffer right now |
| `playback {action: play\|pause\|position}` | drives the real player |
| `export_video` | the export |

`bytes: 0` (or a suspiciously tiny JPEG) means the frame came out black. Read
the file to *see* what the editor sees.

---

## What is deliberately *not* a tool

The coverage test allows exactly these gaps, so the list stays honest:

| Command | Why not |
|---|---|
| `get_state`, `ui_log`, `mcp_status` | GUI plumbing; `get_project_summary` + `get_timeline` cover the state |
| `get_audio_peaks`, `ensure_thumbs`, `get_thumb_strip`, `playback_frame` | visual caches and binary streams for the timeline widget |
| `pick_avatar_media` | opens a native file dialog; an agent passes paths directly |
| `cancel_export` | `export_video` blocks the single-threaded server, so nothing could call it |
| `add_avatar_clip` | legacy toolkit path, superseded by `save_avatar_config` → `generate_avatar_video` → `add_clip` |
| `check_recovery`, `recover_project`, `discard_recovery` | the UI's crash-recovery prompt; they need the app's data dir |
| `set_clip_transform/audio/effects/speed/transition`, `set_clip_text`, `set_subtitles_props`, `set_clip_generator` | folded into `set_clip_properties` / `set_clip_content` (one call, one undo) |
| `rename_track`, `set_track_volume` | folded into `set_track_prop` |
| `get_effects_catalog`, `get_generators`, `list_fonts`, `list_avatar_configs`, `list_text_templates` | folded into `get_catalog` |
| `playback_play/pause/seek/position`, `render_frame` | folded into `playback` / `debug_render_frame` |

Everything else registered in `invoke_handler` has a tool.

---

## Extending

Tools live in `src-tauri/src/mcp.rs`:

- `tool_defs()` — the schema an agent reads. Every argument needs a
  `description`; `additionalProperties: false` makes typos fail loudly.
- `call_tool()` — the dispatch. Handlers reuse the UI's implementation
  (`crate::*_impl` in `lib.rs`) so the agent and the human hit the same code.
  Never re-implement an operation here.
- `handle_rpc()` is pure over `AppState`, so tests drive it without HTTP.

Adding a tool means: a `tool(...)` entry, a `call_tool` arm, and a name in the
coverage test. If the operation needs logic the UI already has, extract it into
a `pub(crate) fn …_impl(state: &AppState, …)` and let the `#[tauri::command]`
call it too.
