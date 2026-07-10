# UberEditor — Master Design and Implementation Plan

> **Cross-platform desktop video editor with AI superpowers.**
> Planning document. Version 1.0 — 2026-07-09.
> Author: Héctor Pulido + Claude. Status: **draft for review, no code yet.**

---

## Index

- [0. Executive summary](#0-executive-summary)
- [1. Vision, scope and non-goals](#1-vision-scope-and-non-goals)
- [2. Technology stack decisions](#2-technology-stack-decisions)
- [3. General architecture](#3-general-architecture)
- [4. Project data model](#4-project-data-model)
- [5. Render and playback engine (the heart)](#5-render-and-playback-engine)
- [6. Basic features (detailed design, 1–11)](#6-basic-features)
- [7. Advanced features (detailed design)](#7-advanced-features)
- [8. Reuse of the Youtubers-toolkit](#8-reuse-of-the-youtubers-toolkit)
- [9. IPC API (frontend ↔ engine)](#9-ipc-api-frontend--engine)
- [10. Repository structure](#10-repository-structure)
- [11. Performance budgets](#11-performance-budgets)
- [12. Testing and CI strategy](#12-testing-and-ci-strategy)
- [13. Packaging, distribution and licenses](#13-packaging-distribution-and-licenses)
- [14. Phased roadmap](#14-phased-roadmap)
- [15. Risks and mitigations](#15-risks-and-mitigations)
- [16. Future backlog (post-v1)](#16-future-backlog-post-v1)
- [Appendix A. Complete project file example](#appendix-a-complete-project-file-example)
- [Appendix B. Modular effect example (manifest + WGSL)](#appendix-b-modular-effect-example)
- [Appendix C. Reference FFmpeg commands](#appendix-c-reference-ffmpeg-commands)
- [Appendix D. Word-level transcript format](#appendix-d-word-level-transcript-format)
- [Appendix E. Initial catalog of MCP tools](#appendix-e-initial-catalog-of-mcp-tools)

---

## 0. Executive summary

**UberEditor** is a non-linear desktop video editor (NLE) for macOS, Windows and Linux, built on **Tauri 2 + Rust + React/TypeScript**, with a homegrown GPU compositing engine (**wgpu/WGSL**) and **FFmpeg** as the decode/encode backbone.

It differs from a classic NLE in four superpowers:

1. **Text-based editing**: every imported video is transcribed with Whisper word by word; deleting a word in the transcript panel cuts the video (Descript-style).
2. **Creator automations**: silence removal/speed-up, automatic generation of vertical versions (Shorts/Reels), automatic subtitles.
3. **Reactive avatar**: a customizable avatar (clips per emotion) that "speaks" to the rhythm of the audio, with emotions classified by an LLM and vibration proportional to the volume.
4. **Embedded MCP server**: on startup, the app exposes a local MCP server so that agents (Claude Code, Claude Desktop, etc.) can read the full project state and, optionally, run edits.

Much of the AI logic already exists and is proven in `/Users/hectorpulido/Videos Reel/Youtubers-toolkit` (Python: faster-whisper, silence trimming, shorts generator, avatar with emotions). Section 8 details the module-by-module mapping and the porting strategy (native Rust as the goal, Python sidecar as a bridge if needed).

The roadmap proposes **7 phases** (0–6). The editable MVP (import → cut → preview → export) lands at the end of Phase 1; parity with the toolkit arrives in Phase 5.

---

## 1. Vision, scope and non-goals

### 1.1 Vision

An editor a YouTuber/creator uses end to end for their real workflow:

```
Record → Import → Clean up (silences, filler words) → Edit (text + timeline)
      → Decorate (subtitles, avatar, titles, effects) → Export (horizontal + vertical)
```

And which is also **operable by AI agents** via MCP: "Claude, remove the silences from this project, generate the vertical version and export it for Shorts".

### 1.2 v1 scope (what IS included)

**Basic (required):**

| # | Feature | Summary |
|---|---------|---------|
| 1 | Timeline | Multiple video and audio tracks, zoom, snapping, drag & drop |
| 2 | Trimming and splitting | Blade, split at playhead, edge trim, ripple delete |
| 3 | Multi-format import | Video, audio and images via FFmpeg; proxies and conforming |
| 4 | Real-time preview | Stable 720p30 at minimum, adaptive scaling |
| 5 | Modular transitions and effects | Shader system (WGSL/GLSL) with JSON manifest, hot-reload |
| 6 | Text and titles | Text clips with styles, templates and animation |
| 7 | Audio control | Per-clip gain, fades, volume/mute/solo per track, meters |
| 8 | Image adjustments | Brightness/contrast/saturation (+ more) as shaders; rotate, crop framing, scale/position |
| 9 | Configurable export | Presets + fine control over codec/bitrate/resolution/range |
| 10 | Undo/redo and project | Practically unlimited history, autosave, versioned project file |
| 11 | Basic keyframes | Any numeric parameter animatable; linear/hold/ease interpolation |

**Advanced (required):**

| # | Feature | Summary |
|---|---------|---------|
| A | MCP server | Full project state exposed to agents; optional editing tools |
| B | Word-by-word Whisper | Word-level transcript of every video; editing by deleting/moving text |
| C | Silences | Detect, remove (ripple) or process (speed up) silences |
| D | Automatic vertical | 9:16 template with blurred background + subtitles + titles, 1-click wizard |
| E | Avatar + automatic subtitles | Emotion-based avatar that vibrates with the volume; automatic word-level subtitles |

### 1.3 v1 non-goals (explicitly out)

- Multicam editing, motion tracking, stabilization.
- Professional color grading (scopes, LUTs, HDR) — basic adjustments only.
- Third-party VST/OFX plugins (yes to homegrown modular shader effects).
- Multi-user collaboration / cloud projects.
- Automatic dubbing with TTS (exists in the toolkit with Kokoro; goes to the backlog, section 16).
- 360°/VR editing, nested timelines (compound clips) — backlog.

### 1.4 Design principles

1. **Non-destructive editing always**: source files are never modified; everything is metadata + render.
2. **State lives in Rust**: the frontend is a view; a single source of truth in the engine avoids desyncs and makes MCP and undo/redo trivial.
3. **Preview == Export**: the same render graph produces the preview and the export (at different resolution), including random effects (noise with deterministic seed). What you see is what you get.
4. **All heavy work is a cancelable job with progress**: import, proxy, waveform, whisper, export.
5. **The toolkit's config formats are honored** wherever reasonable (avatar config, subtitle styles) for smooth migration.
6. **Radical modularity**: almost every visual feature is *data + shader* (effects, transitions, image adjustments, chroma key, even the avatar shake) discovered at runtime from "pack" folders; almost every editing feature is an `Action` registered in a single registry that simultaneously feeds the UI, undo/redo and MCP. Adding a new feature = adding a file, not touching the core (section 6.5.1).
7. **First-class chroma key**: it is a core effect with spill suppression, intended both for green-screen footage and for the avatars (section 6.5.4).

---

## 2. Technology stack decisions

### 2.1 Decision table

| Area | Choice | Alternatives evaluated | Rationale |
|------|----------|------------------------|---------------|
| App shell | **Tauri 2.x** | Electron, Qt, native egui/iced | Explicit request; small binaries; the Rust backend IS the video engine (in Electron you'd have to write a native addon anyway). |
| Frontend | **React 18 + TypeScript + Vite** | Svelte 5, SolidJS | Mature ecosystem for complex UIs (DnD, virtualization); typing shared with the engine via codegen. Svelte is viable if preferred; the architecture doesn't depend on the framework. |
| UI state | **Zustand** + Tauri events | Redux, Jotai | Minimalist store that mirrors the engine's state; mutations do NOT live here (they live in Rust). |
| Styling | **Tailwind CSS** + custom tokens | CSS modules | Speed for a dense NLE-style UI with a dark theme. |
| Graphics engine | **wgpu (WGSL)** | raw OpenGL, Skia, CPU (like MoviePy) | Real cross-platform (Metal/Vulkan/DX12), modern shaders, compute available. MoviePy (CPU) is exactly the bottleneck the toolkit suffers. |
| Demux/decode | **FFmpeg sidecar (CLI)** in Phase 1 → `ffmpeg-next` (libav) option in Phase 2+ | GStreamer, WebCodecs in the webview | Sidecar: zero linking problems, crash isolation, same tool for probe/proxy/export. WebCodecs is ruled out as a base due to spotty support in WebKitGTK (Linux). |
| Encode/export | **FFmpeg sidecar** (pipe rawvideo + wav) | embedded libav | Full control over codecs/containers, proven robustness. |
| Audio playback | **cpal** (audio callback) + custom mixer | rodio, SDL | We need our own mixing with keyframes and a master audio clock; rodio is too high-level. |
| Audio decode | **Conform to WAV PCM on import** (via ffmpeg) + mmap read | symphonia in real time | Conforming up front (as pro NLEs do) simplifies seeks, waveforms and mixing; the disk cost is acceptable. |
| Text/typography | **cosmic-text** (shaping + fallback) rasterized to a texture | glyphon, resvg | Support for emoji, RTL and system fonts; we rasterize to an atlas and compose on the GPU. |
| Transcription | **whisper-rs** (whisper.cpp bindings, with Metal/CUDA) | Python faster-whisper sidecar (like the toolkit) | No dependency on Python in production. faster-whisper remains as a **plan B bridge** (section 8.3). |
| Emotion classif. | **Configurable OpenAI-compatible API** (OpenAI, local Ollama, etc.) | Local ONNX model | It is exactly what the toolkit does (proven prompt); "OpenAI-compatible" allows full privacy with Ollama. |
| Denoise | **nnnoiseless / RNNoise** (Rust/C, real time) | DNS64+torch (toolkit) | DNS64 drags in PyTorch (~2GB); RNNoise is lightweight and sufficient for voice. DNS64 possible via sidecar in the backlog. |
| MCP | **rmcp** (official Rust SDK) with streamable-HTTP transport on localhost | Manual implementation, Node sidecar | Official SDK, tokio-native, same process as the project state. |
| Persistence | **JSON (serde)** with `schema_version`, relative paths | SQLite, binary | Diffable in git, inspectable, trivial to expose over MCP. SQLite only for internal caches/indexes. |
| IDs | **ULID** | UUID v4 | Time-sortable (useful in logs/histories), readable. |
| Time | **`i64` microseconds** (`TimeUs`) + rational fps `(num, den)` | floats, frames | Floats accumulate error (the toolkit suffers this in SRT); integer microseconds are exact and convertible to any fps. |

### 2.2 Target versions

- Rust stable ≥ 1.85, edition 2024.
- Tauri 2.x + official plugins: `dialog`, `fs`, `shell` (sidecars), `store` (preferences), `log`, `single-instance`, `window-state`, `updater`.
- FFmpeg 7.x sidecar (static builds per platform, see section 13).
- Node 22 + pnpm for the frontend.
- whisper.cpp ≥ 1.7 via whisper-rs (Metal build on macOS; CPU AVX2 on Windows/Linux; CUDA optional).

### 2.3 Supported platforms

| OS | Minimum | Webview | GPU backend (wgpu) |
|----|--------|---------|--------------------|
| macOS | 12+ (arm64 + x86_64) | WKWebView | Metal |
| Windows | 10 20H2+ (x86_64) | WebView2 (Chromium) | DX12 (Vulkan fallback) |
| Linux | Ubuntu 22.04+ (x86_64) | WebKitGTK | Vulkan (GL fallback) |

---

## 3. General architecture

### 3.1 Process and thread diagram

```
┌──────────────────────────────── Main process (Tauri / Rust) ────────────────────────────────┐
│                                                                                                   │
│  ┌───────────── WebView (React) ─────────────┐      ┌────────────── Engine (Rust) ─────────────┐  │
│  │  Timeline UI (canvas)                     │      │  ProjectStore  (single state + history)   │  │
│  │  Preview (WebGL canvas / surface)         │◄────►│  PlaybackController (audio clock)         │  │
│  │  Media Pool, Inspector, Transcript Panel  │ IPC  │  RenderGraph (wgpu, WGSL, texture cache)  │  │
│  │  Export dialog, Jobs panel                │      │  DecodePool (ffmpeg sessions / frame LRU)  │  │
│  └───────────────────────────────────────────┘      │  AudioMixer (cpal, 48kHz, master clock)    │  │
│         ▲  state.patch / job.progress events         │  JobRunner (import, proxy, whisper, export)│  │
│         │  frames via binary channel / protocol      │  McpServer (rmcp, HTTP 127.0.0.1:4599)     │  │
│         └────────────────────────────────────────────┴────────────────────────────────────────────┘  │
└───────────────────────────────────────────────────────────────────────────────────────────────────┘
        │                          │                              │
        ▼ sidecar                  ▼ sidecar                      ▼ (in-process, tokio thread)
   ffmpeg / ffprobe        (optional) toolkit-bridge          External MCP client
   (decode, proxy,          (frozen Python:                  (Claude Code / Desktop
    waveform, export)        faster-whisper, kokoro)          connects over HTTP)
```

### 3.2 Data-flow rules

1. **Unidirectional**: the UI emits *intents* (`invoke("timeline.split_clip", …)`) → the engine validates, mutates the `ProjectStore`, pushes to history, and emits a `state.patch` event (JSON Patch RFC 6902) → the UI applies the patch to its mirror.
2. **The engine never trusts the UI**: all invariants (no overlapping clips on a track, in < out, etc.) are validated in Rust. MCP reuses the same actions, for free.
3. **Heavy data outside the JSON IPC**: preview frames, waveforms and thumbnails travel over a binary channel (`tauri::ipc::Channel`) or a custom protocol (`ueasset://`), never as base64 JSON.
4. **Jobs**: every operation > 100 ms is a `Job { id, kind, progress, cancel_token }`; the UI has a global jobs panel; MCP can query them.

### 3.3 Engine modules (internal crates)

```
crates/
├── ue-core        # Data model, actions, history, validation, (de)serialization
├── ue-media       # ffprobe, import, proxies, audio conforming, thumbnails, peaks
├── ue-render      # wgpu: compositing graph, effects, transitions, text, transform
├── ue-audio       # cpal, mixer, volume fades/keyframes, meters, master clock
├── ue-playback    # Orchestration: decode-ahead, cache, A/V sync, scrub
├── ue-export      # Export queue, pipe to ffmpeg, presets
├── ue-ai          # whisper-rs, silences, text-based editing, vertical, avatar, emotions
├── ue-mcp         # MCP server (rmcp), mapping of tools → ue-core actions
└── ue-tauri (src-tauri) # IPC commands, events, wiring, sidecars
```

Allowed dependencies (arrows = "may use"): `ue-tauri → everything`; `ue-mcp → ue-core, ue-ai, ue-export`; `ue-playback → ue-media, ue-render, ue-audio, ue-core`; `ue-ai → ue-media, ue-core`. `ue-core` depends on nobody (pure).

---

## 4. Project data model

### 4.1 Entities (Rust, simplified)

```rust
type TimeUs = i64;           // microseconds; 1 s = 1_000_000
type Id = Ulid;

struct Project {
    schema_version: u32,             // explicit migrations
    id: Id,
    name: String,
    created_at: String,              // ISO-8601
    settings: ProjectSettings,       // cache folder, default whisper language, etc.
    assets: Vec<MediaAsset>,         // "media pool"
    sequences: Vec<Sequence>,        // v1: normally 1, but the model allows several
    active_sequence: Id,
}

struct MediaAsset {
    id: Id,
    kind: MediaKind,                 // Video | Audio | Image
    path: RelPath,                   // relative to the project; relink if it doesn't exist
    content_hash: String,            // xxh3 of the first/last 4MB + size (fast)
    probe: ProbeInfo,                // duration, streams, codecs, fps, resolution, rotation
    proxy: Option<RelPath>,          // 720p h264 short GOP (cache)
    audio_conform: Option<RelPath>,  // wav pcm_s16le 48k stereo (cache)
    peaks: Option<RelPath>,          // binary waveform peaks (cache)
    thumbnails: Option<RelPath>,     // thumbnail sprite (cache)
    transcript: Option<Id>,          // → TranscriptDoc
}

struct Sequence {
    id: Id,
    name: String,
    resolution: (u32, u32),          // e.g. (1920, 1080) or (1080, 1920)
    fps: (u32, u32),                 // rational, e.g. (30000, 1001)
    sample_rate: u32,                // 48000
    tracks: Vec<Track>,              // order = compositing order (index 0 = bottom)
    markers: Vec<Marker>,
}

struct Track {
    id: Id,
    kind: TrackKind,                 // Video | Audio
    name: String,
    muted: bool, solo: bool, locked: bool,
    volume_db: f32,                  // audio only; keyframable
    clips: Vec<Clip>,                // ALWAYS ordered by start, non-overlapping
}

struct Clip {
    id: Id,
    payload: ClipPayload,
    start: TimeUs,                   // position on the timeline
    duration: TimeUs,
    speed: f64,                      // 1.0 normal; >1 speeds up (processed silences)
    effects: Vec<EffectInstance>,    // ordered shader chain
    transform: Transform2D,          // pos, scale, rotation, crop — keyframable
    audio: AudioProps,               // gain_db, pan, fade_in/out — keyframable
    transition_in: Option<TransitionRef>,   // shared with the previous clip
    label_color: Option<String>,
}

enum ClipPayload {
    Media { asset_id: Id, src_in: TimeUs, src_out: TimeUs },  // range of the source file
    Text  { content: RichText, style: TextStyle },            // titles and manual subtitles
    Subtitles { transcript_id: Id, style: SubtitleStyle, mode: SubtitleMode }, // auto, word-level
    Avatar { config: AvatarConfig, driver_asset: Id },        // section 7.E
    Solid { color: [f32; 4] },                                // backgrounds
}

struct EffectInstance {
    effect_id: String,               // "core.brightness_contrast", "user.vhs"
    enabled: bool,
    params: BTreeMap<String, ParamValue>,   // fixed value or keyframe curve
}

enum ParamValue { Const(f64), Color([f32;4]), Bool(bool), Curve(KeyframeCurve), Text(String) }

struct KeyframeCurve {
    keys: Vec<Keyframe>,             // ordered by t
}
struct Keyframe {
    t: TimeUs,                       // relative to the start of the CLIP (survives moving it)
    value: f64,
    interp: Interp,                  // Hold | Linear | Bezier { in_tangent, out_tangent }
}

struct TranscriptDoc {               // section 7.B and Appendix D
    id: Id,
    asset_id: Id,
    language: String,
    model: String,                   // "large-v3-turbo"
    words: Vec<Word>,                // { text, start, end, confidence } in ASSET time
    segments: Vec<Segment>,          // phrases (for avatar emotions and classic SRT)
}
```

### 4.2 Invariants (validated in `ue-core`, with tests)

1. A track's clips are ordered by `start` and non-overlapping (`clip[i].start + duration <= clip[i+1].start`).
2. `0 <= src_in < src_out <= asset.duration` for Media payloads.
3. Keyframes with strictly increasing `t`; every curve has ≥ 1 key.
4. Every `TransitionRef` references two adjacent clips on the same track and its duration ≤ available material (handles) on both sides.
5. IDs are unique across the whole project (global index map for O(1) lookup).

### 4.3 Project file

- Extension: **`.uep`** (UberEditor Project). Content: pretty-printed JSON (diffable).
- Media paths **relative** to the `.uep` file; on open, verification via `content_hash` → **relink** dialog with hash-based search in folders the user specifies.
- Caches (proxies, wavs, peaks, transcripts) go to `<app_data>/cache/<content_hash>/…`, NEVER inside the project → a `.uep` is small and portable; deleting the cache never loses work.
- **Autosave**: every 60 s (configurable) to `<project>.uep.autosave`; on opening after a crash, recovery is offered. Additionally, a snapshot on every export.
- See the full example in Appendix A.

---

## 5. Render and playback engine

This section is the most technically critical: it decides whether the preview is smooth.

### 5.1 Evaluation graph of a frame

To produce the frame at time `t` of the sequence:

```
1. For each VIDEO track (from bottom to top):
   a. Find the active clip at t (binary search by start).
   b. Resolve the source time: src_t = src_in + (t - clip.start) * speed.
   c. Get the source texture:
      - Media  → DecodePool.get_frame(asset, src_t)         (5.2)
      - Text   → TextRasterizer.texture(content, style, t)   (6.6)
      - Subtitles → SubtitleRenderer.texture(transcript, t)  (7.E)
      - Avatar → AvatarRenderer.texture(config, t)           (7.E)
      - Solid  → scaled 1x1 texture
   d. Apply the clip's effect chain (ping-pong between 2 offscreen textures,
      one draw call per effect, uniforms evaluated with keyframes at t).       (6.5, 6.8)
   e. If there is an active transition with the neighboring clip: also render the
      other clip (steps b–d) and run the transition shader(A, B, progress).
   f. Apply Transform2D (crop → scale → rotation → position) and compose
      onto the accumulated framebuffer (premultiplied alpha blend).
2. The final framebuffer (RGBA8 sRGB format in v1) is the sequence frame.
```

Implementation notes:

- **YUV→RGB on the GPU**: ffmpeg delivers `yuv420p`; we upload the Y/U/V planes as 3 R8 textures and convert in the first shader. Saves ~40% of CPU and pipe bandwidth versus requesting `rgba`.
- **Determinism**: effects with randomness (avatar shake, grain) use *hash* noise with seed `(clip_id, frame_index)` — so preview and export are identical and the golden-frame tests are stable. (A direct improvement over the toolkit's `np.random`, which was unrepeatable.)
- **Color space v1**: sRGB 8-bit end to end. HDR/linear 16F is noted as an evolution (the ping-pong design allows it by changing the texture format).

### 5.2 DecodePool: obtaining source frames

- One **decode session** per active asset: a process `ffmpeg -ss <t> -i <proxy|original> -f rawvideo -pix_fmt yuv420p pipe:1` reading sequential frames over stdout.
- **Playback**: the session advances linearly (sequential reading = cheap). Prefetch of N frames ahead of the playhead in a ring buffer.
- **Seek**: kill the session + relaunch with `-ss` (seek by keyframe + decode up to the exact frame). With short-GOP proxies (keyint 15) the worst case is decoding 14 frames ≈ tens of ms.
- **Scrub** (dragging the playhead): *latest-wins* policy — the previous seek is canceled if another arrives; meanwhile the nearest cached frame is shown.
- Global **FrameCache LRU** with a configurable RAM budget (2 GB by default): key `(asset_id, quality, frame_idx)`. Frames around the playhead and around clip edges (where cuts happen often) get priority.
- **Phase 2+ evolution**: replace CLI sessions with `ffmpeg-next` (in-process libav) for finer frame-accurate scrub and hardware decode (VideoToolbox/D3D11VA/VAAPI). The `trait FrameSource` interface is defined from day 1 so the change is internal.

### 5.3 Proxies and conforming (on import)

On importing a file, background jobs are launched (the clip is usable immediately, at degraded quality until they finish):

| Job | Command (see Appendix C) | Output |
|-----|--------------------------|--------|
| Probe | `ffprobe -print_format json` | `ProbeInfo` (streams, duration, fps, rotation) |
| Video proxy | h264 720p, `-g 15`, CRF 20, audio copy | `<hash>/proxy.mp4` |
| Audio conforming | `pcm_s16le`, 48 kHz, stereo | `<hash>/audio.wav` |
| Peaks | read the wav → min/max per window of 256 samples | `<hash>/peaks.bin` |
| Thumbnails | 1 frame every N seconds, 160px sprite | `<hash>/thumbs.jpg` |
| Whisper (opt-in/auto) | section 7.B | `<hash>/transcript.json` |

- Images: loaded with the `image` crate directly to a texture (with downscale if > 8K). An image clip has a free duration (5 s by default).
- The **"preview quality"** toggle (Auto / ½ / ¼ / Full) decides whether the DecodePool reads the proxy or the original.

### 5.4 Audio: mixer and master clock

- **cpal** opens an output stream at 48 kHz; the audio *callback* requests `n` samples → the `AudioMixer` produces them by reading the conformed WAVs (mmap) of all audible clips at the current position.
- Per-clip chain: `sample → gain(keyframes) → fades(in/out) → pan` → sum per track (`volume_db`, mute/solo) → master → *soft clip limiter*.
- **Audio is the master clock**: the playback position is derived from the samples actually consumed by the device (`samples_played / 48000`). Video syncs to that clock: if it falls behind, skip frames; if it runs ahead, wait. It is the standard scheme that eliminates A/V drift.
- Clip speed ≠ 1.0: resampling with `rubato` (sinc) per segment; for "sped-up silences" (7.C) simple time-stretch is used (v1: resample with acceptable pitch change at 2–4x during silence; v2: WSOLA to preserve pitch).
- Meters: RMS + peak per track and master, published to the UI at 15 Hz by event.

### 5.5 Preview delivery to the UI

**Phase 1 (simple and cross-platform) — target 1280×720@30:**

1. wgpu render to an offscreen texture at preview resolution.
2. Copy to a staging buffer → CPU (`map_async`).
3. Send the raw RGBA (~3.7 MB/frame) over a binary `tauri::ipc::Channel<Vec<u8>>` → the frontend uploads it to a WebGL texture and paints it on a canvas. If the channel can't sustain the throughput on some platform, automatic fallback to JPEG (turbojpeg, quality 85, ~200 KB/frame).
4. Backpressure: if the UI doesn't confirm the previous frame, the send is skipped (the internal render continues; the audio is never blocked).

**Phase 2 (optimization) — target 4K@60 and minimal latency:**

- Native child surface (child window/`CAMetalLayer`/HWND/wayland subsurface) positioned under the preview slot in the web layout; wgpu presents directly (zero-copy). The web UI draws only the controls around it. Risk/complexity documented in section 15; that's why it is Phase 2 and not the base.

### 5.6 Export pipeline

```
RenderGraph (sequence resolution/fps, no preview cache, Full quality)
   │  RGBA frames (or yuv420p converted on GPU→CPU)
   ▼
ffmpeg -f rawvideo -pix_fmt rgba -s WxH -r FPS -i pipe:0 \
       -i mixdown.wav \
       [preset flags]  out.mp4
```

1. The `AudioMixer` first renders the **complete mixdown** to `mixdown.wav` (faster than real time; serves as an early progress bar).
2. The graph renders frame by frame (no clock, at maximum speed) and writes to ffmpeg's stdin. Pressure regulated by the pipe itself.
3. Progress = frames written / total; cancellation = kill ffmpeg + delete the partial.
4. Export queue: multiple jobs in series (parallel in the backlog).
5. On completion: verification with ffprobe (expected duration ± 1 frame) and OS notification.

---

## 6. Basic features

Format of each subsection: **Objective → UX → Technical design → Edge cases → Acceptance criteria (AC)**.

### 6.1 Timeline (feature 1)

**Objective.** Unlimited video and audio tracks, fluid direct manipulation at 60 fps UI even with hundreds of clips.

**UX.**
- Classic layout: time ruler at the top, video tracks (top) and audio (bottom), vertical playhead, track headers on the left (name, mute/solo/lock, volume).
- Zoom: wheel+Ctrl (centered on the cursor), shortcuts `+`/`-`, "zoom to fit" (`Shift+Z`). Range: from 10 min/screen to 5 frames/screen.
- Horizontal scroll (wheel / drag with space) and vertical (tracks).
- **Snapping** (toggle `S`): magnet to the playhead, clip edges, markers and to 0; 8 px tolerance in screen space.
- Drag & drop: from the Media Pool to a track (creates a clip), between tracks, and horizontally with a ghost preview + collision indicator.
- Selection: click, rubber-band marquee, `Shift` for multiple; `Ctrl/Cmd+A` all.
- Clips show: name, thumbnails (video), waveform (audio), effect/speed badges, and label color.
- Sequence markers (`M`) with name and color.

**Technical design.**
- The timeline is drawn on **a single 2D `<canvas>`** (not DOM per clip): immediate, game-like rendering with a visible list computed by binary search over `start`. With track virtualization it is O(visible), not O(total).
- Coordinates: `px = (t_us - view_start_us) * pxPerUs`; all hit-tests in time space, not pixels (stable under zoom).
- Canvas thumbnails/waveforms come from the sprites/peaks of the cache (5.3) via the `ueasset://` protocol (the webview fetches them as normal, cacheable images).
- Interactions emit intents to the engine (`timeline.move_clip`, etc.). During a drag, the UI shows the ghost locally and only emits the action on drop (a single undo entry).
- Reorder/insert with **overwrite mode** (default) and **insert/ripple mode** (with `Alt`): the engine implements both as distinct actions.

**Edge cases.** Drop over an existing clip (overwrite splits the clip below); drag past the start (clamp to 0); locked tracks reject actions with a toast; extreme zoom with clips < 1 px (drawn as a line, still selectable by range).

**AC.**
1. 500 clips on 8 tracks → pan/zoom at 60 fps on a mid-range laptop.
2. All mutations go through engine actions (verifiable: replaying the history reproduces the state).
3. Snapping works at any zoom level with pixel tolerance.

### 6.2 Clip trimming and splitting (feature 2)

**Objective.** Cut, split and adjust clips non-destructively with frame precision.

**UX.**
- **Split at playhead** (`Ctrl/Cmd+K` or blade button): splits all selected clips (or the one under the playhead) in two.
- **Blade tool** (`C`): clicking on any clip splits it at that point.
- **Edge trim**: dragging a clip's left/right edge adjusts `src_in`/`src_out` (cursor changes; tooltip shows +/- frames and new timecode). Limited by the asset's available material (the "handles").
- **Ripple delete** (`Shift+Del`): deletes the clip and closes the gap by shifting what follows. A plain `Del` leaves the gap.
- **Ripple trim** (`Alt` + drag edge): trim that shifts the rest of the track.
- Slip (drag the content with `Y` without moving the clip): moves `src_in/src_out` together. (Slide stays in the backlog.)

**Technical design.**
- `split(clip, t)` = clone the clip; the left one gets `src_out' = src_in + (t - start) * speed`, the right one `src_in' = src_out'`, `start' = t`. The **keyframes** (curves with `t` relative to the clip) are split: those on the right side are re-based by subtracting the offset; interpolated keys are inserted at the cut point to preserve the exact value.
- Effects and transform are **copied** to both halves (standard NLE behavior). The existing transition stays on the side that touches its neighbor.
- Precision: `t` is quantized to the sequence frame (`round(t * fps_num / (fps_den * 1e6))`) before operating, so cuts always land on a frame boundary.
- Everything is pure actions over `ue-core` with an explicit inverse (section 6.10).

**Edge cases.** Split exactly at the edge (no-op); trim that would leave duration 0 (minimum 1 frame); split of a clip with an active transition at that point (rejected with a message); split of a text/subtitles/avatar clip (supported: they split their internal timeline).

**AC.**
1. Split + undo restores the state byte for byte (serialization test).
2. Trim never exceeds the source material; the tooltip reflects exact frames.
3. Ripple delete over a multi-track selection keeps the relative sync of the other tracks ("ripple all tracks" option on/off).

### 6.3 Multi-format import (feature 3)

**Objective.** Drag any reasonable file and have it work: video, audio or image.

**v1 formats** (whatever the FFmpeg build supports; the list of accepted extensions in the UI):
- Video: mp4, mov, mkv, webm, avi, m4v, mts/m2ts, mpg, flv, wmv (codecs: h264, hevc, vp8/9, av1, prores, dnxhd, mpeg2/4…)
- Audio: wav, mp3, aac/m4a, flac, ogg/opus, aiff, wma
- Image: png, jpg/jpeg, webp, bmp, tiff, gif (v1: first frame; animated gif → treated as video), svg (rasterized with resvg to the sequence resolution), heic (macOS)

**UX.**
- Entry paths: Import button, `Ctrl/Cmd+I`, drag & drop of files/folders to the Media Pool or directly to the timeline (imports + inserts).
- Media Pool: grid or list with thumbnail, name, duration, resolution/fps, job status badges (proxy ✓, audio ✓, whisper ⏳), search and virtual folders (bins).
- Files with rotation in metadata (`rotate=90`, typical of phones) are shown already corrected.
- Offline media: red clip + relink dialog (searches by name and by `content_hash`).

**Technical design.**
- Import = quick synchronous `ffprobe` (< 1 s) to validate and populate `ProbeInfo` + scheduling of cache jobs (5.3). The asset is usable instantly (decode of the original while there is no proxy).
- VFR detection (variable frame rate, typical of OBS/screen): if `avg_frame_rate ≠ r_frame_rate`, the proxy is generated with `-vsync cfr -r <sequence_fps>` and the asset is flagged (VFR originals break seek precision; the CFR proxy fixes it — a known lesson from editing screen-recording material).
- `content_hash` = xxh3(first 4 MB + last 4 MB + size) — enough for relink and cache keys without reading whole 50 GB files.
- Folders: recursive import with an extension filter.

**Edge cases.** Files with no audio track (empty waveform, the mixer ignores them); 5.1 multichannel audio (downmix to stereo in the conforming, note in the inspector); images with EXIF orientation; corrupted files (probe fails → toast with summarized stderr); paths with non-ASCII characters and spaces (like `Videos Reel`!) — always pass paths as separate args to the sidecar, never interpolate into a shell.

**AC.**
1. All 3 types import via the 3 entry paths.
2. A phone mp4 shot vertically (rotate=90) looks correct in preview and export.
3. A project moved to a folder with its media alongside → opens without relink (relative paths).

### 6.4 Real-time preview (feature 4)

The technical bulk is in section 5. Here, the UX contract:

- Transport: space = play/pause, `J/K/L` (reverse/pause/forward with speeds ×1/×2/×4 — reverse in v1 = 1-frame jumps backward, smooth reverse in the backlog), `←/→` frame by frame, `Home/End`, `I/O` for range marks.
- Quality indicator (Auto/½/¼/Full) and dropped-frames indicator (dropped-frame counter in the corner, visible only if > 0).
- **Graceful degradation**: if the render doesn't make it in time, it first lowers the preview resolution (Auto), then skips video frames; the audio is never interrupted.
- On pause: immediate re-render of that frame at Full quality (the user sees it sharp when stopping).
- Safe zone / thirds guides (toggle), checkerboard background for alpha.

**AC.**
1. 1080p sequence with 2 video tracks + 1 text + music: 30 fps playback with no drops on reference hardware (M1 / Ryzen 5 + integrated GPU).
2. Scrub latency (release the playhead → correct frame on screen) < 150 ms with proxy.
3. A/V desync < 40 ms sustained on 30 min clips (measurable with a beep+flash video).

### 6.5 Modular transitions and effects — shader system (feature 5)

This subsection defines **UberEditor's central extensibility system** (it also applies to 6.8, to the chroma key and to the avatar).

#### 6.5.1 Modular architecture ("packs")

Principle: **adding an effect, transition or preset never touches the core code.** A *pack* is a folder:

```
effects/
├── core/                      # bundled with the app (read-only, embedded in the binary)
│   ├── brightness_contrast/
│   │   ├── manifest.json
│   │   └── shader.wgsl
│   ├── chroma_key/ …
│   ├── gaussian_blur/ …
│   └── transitions/crossfade/ …
└── user/                      # <app_data>/effects — the user's folder, hot-reload
    └── vhs_retro/
        ├── manifest.json
        └── shader.wgsl        # or shader.frag (GLSL) — naga ingests it just the same
```

- **Runtime discovery**: on startup (and with a file-watcher on the `user/` folder) manifests are scanned, shaders are compiled (with naga validation and readable errors in a panel), and they appear in the UI automatically. Editing the `.wgsl` with the app open recompiles and refreshes the preview live (**hot-reload**) — an iteration cycle of seconds to author new effects.
- **Single contract**: every effect is `fn effect(tex_in, uv, params…) -> color`; every transition is `fn transition(tex_a, tex_b, uv, progress, params…) -> color`. The runtime generates the uniform binding from the manifest (no touching Rust to expose a parameter).
- The parameters declared in the manifest are **automatically keyframable** (6.11), appear in the Inspector with the right widget (slider/color/checkbox/angle/2D point) and are accessible via MCP.
- GLSL compatibility: naga accepts GLSL fragment shaders → the open-source catalog of **gl-transitions** (MIT, ~80 transitions) can be ported almost as-is.
- The same registry philosophy applies outside shaders: the `ClipPayload`s, the `Job`s and the `Action`s are registered in central tables (`ActionRegistry`), so a new feature (e.g. a "Screen Recording Zoom" payload) is added by implementing 2 traits (`Renderable`, `Inspectable`) + 1 registry entry, and gets undo/redo, persistence, Inspector and MCP exposure for free.

#### 6.5.2 Manifest (data contract)

Full example in Appendix B. Key fields:

```json
{
  "id": "core.chroma_key",
  "kind": "effect",                    // "effect" | "transition"
  "name": { "es": "Chroma Key", "en": "Chroma Key" },
  "category": "keying",
  "shader": "shader.wgsl",
  "params": [
    { "key": "key_color",  "type": "color",  "default": [0.0, 1.0, 0.0, 1.0] },
    { "key": "similarity", "type": "float",  "default": 0.35, "min": 0, "max": 1 },
    { "key": "smoothness", "type": "float",  "default": 0.08, "min": 0, "max": 0.5 },
    { "key": "spill",      "type": "float",  "default": 0.5,  "min": 0, "max": 1 }
  ]
}
```

#### 6.5.3 Core catalog v1

**Effects** (all keyframable): color correction (6.8), **chroma key** (below), gaussian blur (separable, 2 passes — reused by the vertical-mode background), box blur, sharpen, vignette, opacity, grayscale/sepia, invert, pixelate, noise/grain (deterministic seed), simple glow, **shake** (ported from the avatar, available for any clip), speed ramp (via the clip's `speed`), flip H/V.

**Transitions**: crossfade, dip-to-black/white, wipe (parametric direction), slide/push, zoom blur, circle reveal, + the gl-transitions port as an optional extra pack.

- Transition model: `TransitionRef { effect_id, duration, params }` between two adjacent clips; the render needs frames of A and B simultaneously → requires handles (extra material); the UI draws it as a bowtie-shaped overlap, draggable in duration.

#### 6.5.4 Chroma key (first-class effect) 🔑

Project requirement: it matters a lot, so it is specified in detail.

- **Algorithm (shader)**: distance to the key color in **YCbCr** space (robust to luma variations — only the chroma plane is compared):
  ```
  d = distance(CbCr(pixel), CbCr(key_color))
  alpha = smoothstep(similarity, similarity + smoothness, d)
  ```
- **Spill suppression** (the greenish edge): after keying, the dominant channel of the key is desaturated in the semi-transparent pixels: `g' = min(g, mix(g, (r+b)/2, spill * (1 - alpha_edge)))`.
- **Parameters**: `key_color` (with an eyedropper over the preview — the UI samples the rendered frame), `similarity`, `smoothness`, `spill`, `edge_shrink` (optional 1px erosion in the shader), `output_mode` (result / black-and-white mask for debugging).
- **Integration**: it is just another effect in the chain (compositing with premultiplied alpha already supported by pipeline 5.1) → it works for green screen of recorded material, for avatar mp4s with a green background (the toolkit's current `avatar_*.mp4` need it; the `.mov` with alpha does not), and for any overlay.
- **"Toolkit avatar" preset**: pure green `#00FF00`, similarity 0.30, smoothness 0.10, spill 0.6 — validated against the real files in `avatar_config/`.
- **AC**: key `avatar_angry.mp4` over a 1080p video maintaining 30 fps preview; no visible green halo at default similarity/spill; correct alpha in export with and without a background underneath.

**Edge cases (effect system).** A user shader that doesn't compile (effect disabled + error panel, never a crash); a parameter renamed in the manifest (projects save by `key`: unknown ones are preserved and ignored with a warning); two packs with the same `id` (`user/` wins, warning).

**AC (system).**
1. Create a new effect by copying a folder and editing 2 files, without recompiling the app, with hot-reload < 2 s.
2. A chain of 5 effects over a 1080p clip maintains 30 fps preview.
3. A ported gl-transitions transition works identically in preview and export.

### 6.6 Text and titles (feature 6)

**Objective.** Quality text clips (correct shaping, emoji, accents) with styles and animation.

**UX.**
- "Add text" button → `Text` clip on the top track; editing the content **directly over the preview** (editable box) or in the Inspector.
- Style: font (list of system fonts + bundled fonts), size, color, bold/italic, alignment, letter spacing, line spacing, **stroke** (color+width), **shadow** (offset, blur, color), **background/box** (color, padding, corner radius), opacity.
- Position: draggable in the preview with smart guides (center/thirds); 9-point anchors.
- Preset entry/exit animations: fade, slide (4 directions), typewriter (per character), pop; configurable duration. Internally they are just keyframes generated over transform/opacity → editable by hand afterward.
- **Templates**: save style+animation as a named template (JSON in `<app_data>/templates/titles/`); initial pack of ~8 (lower third, centered title, corner for shorts, etc.). The toolkit's `titles_clip_config`/`titles` from `config.json` are imported as a "Classic toolkit" template.

**Technical design.**
- Layout/shaping with **cosmic-text** (handles font fallback and emoji); rasterized to an RGBA texture with a cache by `(content, style, max_width)`; the stroke and shadow are generated during rasterization (not in the shader) for quality; the texture enters the pipeline like any source and receives standard effects/transform/keyframes.
- Re-rasterize only when content/style changes, not per frame. Typewriter: prefix-based rasterization with cache (N textures) or per-glyph mask — decided at implementation time, the interface hides it.
- Automatic subtitles (7.E) reuse this same rasterizer with their own payload.

**AC.**
1. Text with emoji + accents + CJK renders correctly on all 3 platforms.
2. Editing text over the preview reflects changes in < 50 ms.
3. A template created on macOS opens the same on Windows (missing fonts → substitution with a warning).

### 6.7 Audio control (feature 7)

**Objective.** Enough control to publish without an external DAW.

**UX.**
- Per clip: gain (dB, -60..+12) with a volume line drawn over the clip (draggable, with keyframes on `Ctrl+click`), fade in/out with handles at the clip corners (equal-power curve), pan.
- Per track: volume fader, mute, solo, vertical RMS/peak meter.
- Master: fader + meter with a clipping indicator (2 s peak hold).
- Utilities: "Normalize clip" (analyzes peak → adjusts gain to -1 dBFS), "Mute range" (automatic keyframes), **voice denoise** (RNNoise on/off per clip — processes the conformed WAV into an alternate cached WAV, background job; lightweight substitute for the toolkit's `denoise.py`/DNS64).
- Export: **EBU R128 loudness normalization** option (two passes of ffmpeg's `loudnorm`, target -14 LUFS for YouTube) in the export dialog.

**Technical design.** Already described in 5.4. Fades are implicit gain curves merged with the keyframe curve; the order is `gain_kf → fades → pan`.

**Edge cases.** Solo across multiple tracks (union); overlapping clips on different tracks sum (limiter headroom); volume keyframes and `speed ≠ 1` (keyframes live in clip time → they stretch with it).

**AC.**
1. Fade in/out with no clicks or pops (test: 1 kHz sine, inspection of the exported WAV).
2. Meters consistent with the exported file (± 1 dB).
3. Mute/solo apply in < 1 audio buffer (no cuts).

### 6.8 Basic image adjustments + framing (feature 8)

**Objective.** Brightness, contrast, saturation at minimum; and rotate/crop framing. All keyframable.

**Design.** Two separate pieces:

1. **"Color correction" effect** (`core.color_correct`, a single WGSL pass, always available in the Inspector without having to add it):
   - `brightness` (-1..1, additive in luma), `contrast` (0..2, pivot 0.5), `saturation` (0..2, mix with Rec.709 luma), `exposure` (stops, multiplicative), `temperature`/`tint` (balance shift), `gamma` (0.2..3).
   - Fixed order documented in the shader: exposure → temperature/tint → contrast → brightness → saturation → gamma.
2. **The clip's Transform2D** (not a shader; it is the geometric compositing stage 5.1.f):
   - `position (x,y)` in sequence pixels, `scale` (uniform + non-uniform), `rotation` (degrees, free; 90°/180°/-90° shortcuts), `anchor point`, `crop` (left/top/right/bottom in % with optional feather), `opacity`, flip H/V.
   - UI: gizmo over the preview (move/scale/rotate with handles) + numeric fields in the Inspector. Crop with 4 edge handles.
   - One-click "Fit / Fill / Stretch" to adapt material of a different resolution to the sequence.

**Edge cases.** Rotation of material already rotated by metadata (they compose); crop 100% (clip invisible but valid); crop keyframes + a simultaneous transition.

**AC.**
1. B/C/S match visually between preview and export (golden-frame test with ΔE tolerance).
2. Rotating a vertical phone clip 90° and cropping the framing to 16:9 is a flow of < 5 clicks.
3. All parameters accept keyframes and appear in the curve editor.

### 6.9 Configurable export (feature 9)

**Objective.** One-click presets + full control for advanced users. Technical pipeline in 5.6.

**UX — export dialog.**
- Left: **presets** (editable, savable):

| Preset | Container | Video | Audio | Notes |
|---|---|---|---|---|
| YouTube 1080p | mp4 | H.264 High, CRF 18, `-preset slow`, yuv420p | AAC 320k | default |
| YouTube 4K | mp4 | H.265 CRF 20 (or H.264 CRF 17) | AAC 320k | time warning |
| Shorts/Reels 1080×1920 | mp4 | H.264 CRF 18, ≤ 60 s warning | AAC 256k | links with 7.D |
| Lightweight web | webm | VP9 CRF 32 | Opus 128k | |
| Editing master | mov | ProRes 422 (`prores_ks`) | PCM 24-bit | interchange |
| GIF | gif | 2-pass palette, fps 15, width 720 | — | |
| Audio only | mp3 / wav | — | 320k / PCM | podcast |

- Right: **overrides**: resolution (with scaling), fps, range (full sequence / I-O marks), codec, bitrate mode (CRF vs target CBR/VBR), keyframe interval, R128 loudness on/off, output name/folder (templates `{project}_{preset}_{date}`).
- Size estimate (bitrate heuristic) and "Add to queue" button.
- Queue panel: progress per job (render fps, ETA), cancel, open folder on completion, OS notification.

**Edge cases.** Odd resolution with yuv420p (forced to even); overwriting an existing file (incremental suffix); disk full (ffmpeg error captured with a clear message); export with offline media (blocked with a list of missing items).

**AC.**
1. 1080p H.264 export of a 5 min sequence with effects ≥ 1× real time on reference hardware.
2. The file passes ffprobe validation (duration ± 1 frame, exact fps and resolution).
3. Canceling leaves the system clean (no zombie processes or partials).

### 6.10 Undo/redo and project saving (feature 10)

**Objective.** Reliable and practically unlimited undo/redo; never lose work.

**Technical design — Command pattern in the engine.**
- Every mutation is an `Action` (serializable enum) with an **explicit inverse**: `apply(&mut Project, Action) -> InverseAction`. Examples: `SplitClip{clip, t} ↔ JoinClips{left, right}`, `MoveClip{id, from, to} ↔ MoveClip{id, to, from}`, `SetParam{path, old, new}`.
- `History { undo: Vec<Entry>, redo: Vec<Entry> }`, where `Entry { actions: Vec<Action>, label, timestamp }` — an entry can group N actions (transaction): a drag, a "remove silences" (hundreds of cuts = 1 undo!), a full vertical wizard.
- **Coalescing**: continuous edits of the same parameter (< 500 ms, same path) are merged into one entry.
- Practical limit: 1000 entries (configurable); when exceeded, the oldest are discarded.
- The UI shows a navigable history (panel with labels: "Split clip", "Remove 34 silences") and `Ctrl/Cmd+Z / Shift+Z`.
- The NON-undoable (file import, cache jobs, exports) stays out of the history; deleting an asset from the pool requires confirmation if there are clips using it (and IS undoable: the action stores the serialized asset).

**Saving.**
- `Ctrl/Cmd+S` → writes the `.uep` (4.3) **atomically** (tmp + rename). Dirty indicator (● in the title).
- Autosave to `.uep.autosave` every 60 s if dirty; removed on a successful save; on opening, if it exists and is newer → recovery dialog.
- "Save as" + "Save packaged copy" (backlog: copies media to a folder).
- Migrations: `schema_version` + pure `migrate_v1_v2(...)` functions tested with fixtures of old projects.

**AC.**
1. Property test (proptest): random sequences of 200 actions + full undo ≡ initial project (structural comparison).
2. Kill -9 during autosave → the original project is never left corrupted (atomic write).
3. "Remove silences" (7.C) is exactly 1 undo entry.

### 6.11 Keyframe animations (feature 11)

**Objective.** Animate any declared numeric/color parameter (transform, effects, volume, text).

**UX.**
- In the Inspector, each keyframable parameter has a ⏱ button (enable animation) and a diamond ◇ (add/remove a key at the playhead); ◀▶ arrows jump between keys.
- Under each selected clip, the timeline shows a **keyframe lane** (draggable diamonds; `Alt+drag` duplicates; multiple selection and block shifting).
- **Curve editor** (collapsible panel): value vs time, draggable bezier tangents, easing presets (linear, ease-in/out/in-out, hold, simple bounce).
- On enabling animation of a parameter with value V, an initial key `t=playhead, value=V` is created (expected AE-like behavior).

**Technical design.**
- `KeyframeCurve::eval(t)` — binary search for the segment + interpolation according to `interp` (Hold returns the left value; Bezier: cubic hermite with tangents, iterative solution for uniform-t, precomputed per segment and cached).
- Times are **relative to the clip** (they survive move/split, see 6.2); the clip's `speed` scales the timeline→clip mapping before evaluating.
- Colors interpolate per component in linear RGB (v1); enums/bools Hold only.
- The evaluator lives in `ue-core` (shared by preview, export and tests).

**AC.**
1. Simultaneous position (slide) + opacity (fade) animation, smooth at 30 fps.
2. Splitting an animated clip preserves the exact visual trajectory (interpolated keys inserted at the cut).
3. `eval()` with 10,000 calls/frame (absurd worst case) < 0.5 ms (criterion bench).

---

## 7. Advanced features

### 7.A Embedded MCP server

**Objective.** On app startup, a local MCP server exposes **the entire project state** to agents (Claude Code, Claude Desktop, any MCP client), and optionally allows editing.

**Design.**
- `ue-mcp` crate on **rmcp** (official Rust MCP SDK), **streamable HTTP** transport at `http://127.0.0.1:4599/mcp` (configurable port; bind ONLY to loopback).
- Runs in the main process's tokio runtime → direct access (read lock) to the `ProjectStore`. Write tools dispatch **the same `Action`s from the ActionRegistry that the UI uses** (section 6.5.1) → validation, undo and events for free. An agent that makes 50 edits generates undo entries labeled `[MCP] …` that the user can undo.
- **Security**:
  - Random bearer token per session (visible in Settings → MCP, with a copy button; option of a fixed token per project).
  - Three configurable levels: `off` / `read-only` (default) / `read-write`. In read-write, a "confirm destructive edits" option (native dialog when a tool deletes > N clips or exports).
  - No arbitrary filesystem access: the tools speak of project IDs, not paths.
- **Tool catalog** (detailed in Appendix E): reads (`get_project_summary`, `get_timeline`, `get_transcript`, `get_media_pool`, `get_jobs`, `get_selection_and_playhead`…), edits (`split_clip`, `remove_range`, `move_clip`, `set_clip_property`, `apply_effect`, `add_text_clip`…), high-level AI (`remove_silences`, `delete_words`, `generate_vertical`, `generate_avatar_track`, `start_export`). Resources: `project://current` (project JSON), `transcript://{asset_id}` (word-level JSON and SRT).
- **Client registration**: the Settings screen shows the ready-to-copy snippet:
  ```bash
  claude mcp add --transport http ubereditor http://127.0.0.1:4599/mcp \
      --header "Authorization: Bearer <token>"
  ```
  and the equivalent JSON for Claude Desktop. "Test connection" button.
- Events: state changes emit `notifications/resources/updated` on `project://current` for subscribed clients.

**Target use cases** (they guide the tool design): "how long is my project and what media does it use?", "remove the silences from track 1 with 200 ms padding", "delete all the words 'um...' and 'you know'", "generate the vertical version and export for Shorts", "put a title with the text X at minute 2".

**AC.**
1. Connected Claude Code can describe the whole project (sequences, clips, transcripts) unaided.
2. `remove_silences` via MCP ≡ same result as the UI button, and it is 1 undo.
3. At `read-only` level, every write tool returns a standard MCP error and a useful message.
4. Two simultaneous MCP clients don't corrupt state (all writes serialized by the store's lock).

### 7.B Word-by-word Whisper + text-based editing

**Objective.** Every imported video/audio is transcribed with **per-word** timestamps; a transcript panel allows editing the video by deleting or reordering text.

#### 7.B.1 Transcription (import job)

- **whisper-rs** (whisper.cpp) with `token_timestamps + DTW` for word timestamps; ggml models managed from Settings → AI:

| Model | Approx. size | Recommended use |
|---|---|---|
| tiny / base (q5) | 40–80 MB | testing, slow machines |
| small | ~500 MB | CPU balance |
| large-v3-turbo (default) | ~1.6 GB | the one the toolkit uses ("turbo"); fast and accurate |

  Download from HuggingFace with progress and sha256 verification, to `<app_data>/models/`. GPU: Metal on macOS (big gain), CUDA optional.
- Job pipeline: conformed audio 16 kHz mono (derived from the WAV of 5.3) → optional VAD (silero-vad ONNX) to chunk and accelerate → whisper by chunks with context → merge → normalization (collapse spaces, join tokens with apostrophes) → `TranscriptDoc { words[], segments[] }` (Appendix D).
- **Cache by `content_hash`** (same pattern as the toolkit's `_segments.json`): re-importing the same file or re-transcribing after a crash is free.
- Configuration: language (auto/es/en/…), auto-transcribe on import (on by default, can be disabled), model, optional translation to English (whisper's native capability).
- **Plan B**: if whisper.cpp's word-timestamps have quality problems, the `toolkit-bridge` sidecar (frozen Python with faster-whisper, section 8.3) implements the same JSON contract — the rest of the app doesn't notice.

#### 7.B.2 Transcript panel and text-based editing

**UX.**
- Side "Transcript" panel with two modes:
  - **Asset mode**: complete transcript of a file from the pool (for review).
  - **Sequence mode** (the powerful one): concatenates the words of the clips in timeline order; reflects the current edit. Each word knows its clip and its source range.
- Word under the playhead highlighted; clicking a word = seek; double-clicking = selects the word in the timeline.
- **Deleting text = cutting video**: select words and `Del` → the engine cuts the corresponding ranges (with configurable padding, default 80 ms on each side, merging cuts < 120 ms apart) and ripples. One undo entry.
- **Strikethrough mode (non-destructive)**: `Ctrl+Del` strikes words through (saved as `rejected`); the preview virtually skips them; an "Apply cuts" button materializes them. Lets you iterate without committing.
- **Reorder**: select a phrase and drag it to another point in the text → moves the corresponding clips (split at boundaries + move + ripple). V1 limits the drag to phrase boundaries (cuts in the middle of coarticulation sound bad; documented).
- Text search with highlighting in the timeline (finds filler words: "um", "you know") + a "strike through all matches" action.
- Text correction: editing a word corrects the transcript (for subtitles), never the audio.

**Technical design.**
- The central operation is `cut_ranges(sequence, Vec<(TimeUs, TimeUs)>, ripple: bool)` in `ue-core`: it normalizes+merges ranges, splits at boundaries, deletes, ripples; returns a transaction. It is reused by: text-based editing, silences (7.C) and MCP. **Write it once, test it thoroughly.**
- Word→timeline mapping: `word.start` is in asset time; for each Media clip the words with `src_in ≤ t < src_out` are indexed; timeline position = `clip.start + (word.start - src_in) / speed`. Inverted index cached and invalidated by patch.

**AC.**
1. Deleting 10 scattered words produces the correct cuts (test with a synthetic transcript fixture) and 1 undo.
2. Sequence mode reflects existing splits/moves/deletes correctly.
3. A 20 min video with large-v3-turbo transcribes in the background without blocking editing; on M1 ≤ ~2–3 min (Metal).

### 7.C Silence removal / processing

**Objective.** Detect silences and: remove them (ripple), speed them up, or just mark them. A direct, improved port of the toolkit's `trim.py`.

**Algorithm** (in `ue-ai::silence`, over the conformed WAV):

```
1. RMS per window: window=50 ms, hop=10 ms (the toolkit uses a fixed 2 s /
   0.25 s window via "clip_interval"; the fine hop gives precise boundaries).
2. Dual threshold (hysteresis): speech if RMS > T_on; silence if RMS < T_off = T_on - 6 dB.
   T_on configurable: absolute in dBFS (default -38 dBFS ≈ the toolkit's 0.01 linear)
   or RELATIVE: 15th percentile of the clip's RMS + 8 dB (robust to recording levels).
3. Merge: silences < min_silence (default 400 ms) are ignored (breaths);
   speech islands < min_speech (default 150 ms) are absorbed (clicks).
4. Padding: expand speech pad_pre=150 ms / pad_post=200 ms (lets endings breathe).
5. Output: Vec<SpeechInterval> in asset time.
```

**UX.**
- "Silences…" dialog over the selection (or track/sequence): parameter sliders + **live preview**: red regions (silence) / green regions (speech) painted over the clips and the RMS histogram with the threshold line — recomputed as the sliders move (the RMS analysis is cached; only re-thresholding happens: instant).
- Three actions:
  1. **Remove** → `cut_ranges(ripple=true)` (7.B.2).
  2. **Speed up** → split the silent ranges and `speed = N×` (default 4×, with optional -12 dB attenuated audio) — smooth jump-cut style.
  3. **Mark** → only sequence markers (manual review).
- Preview stat: "47 silences will be removed (2:13 of 14:20 → 12:07)".

**AC.**
1. Over a synthetic fixture (speech + known silences) the detector finds 100% of silences > 400 ms with no false positives at default threshold.
2. Region preview updates < 100 ms when moving sliders (only re-thresholding).
3. Result identical whether applied via UI, MCP or the vertical wizard.

### 7.D Automatic vertical video generation

**Objective.** From a horizontal sequence/range to a 9:16 ready for Shorts/Reels with 1 click. Port of `generate_short_base` from `recipes.py` + `shorts.py`.

**"Generate vertical…" wizard** (steps, all with sensible defaults):

1. **Range**: full sequence, I-O marks, or clip selection.
2. **Layout** (templates, a system open to more):
   - `Blurred background` (toolkit port): new 1080×1920 sequence; bottom layer = same material scaled to fill (crop) + `core.gaussian_blur` (σ≈20, equivalent to `boxblur=10:1`) + darkened -20%; top layer = material scaled to 1080 width, vertically centered.
   - `Centered zoom`: direct 9:16 crop, with an X position that can be keyframed manually afterward.
   - (Backlog: `Auto-reframe` with face detection — section 16.)
3. **Silences**: "remove silences first" checkbox (reuses 7.C).
4. **Subtitles**: "automatic word-level subtitles" checkbox (karaoke style by default, reuses 7.E.2) — uses the existing transcript or launches whisper.
5. **Titles**: optional, initial text/CTAs from a template (port of `add_titles` + `titles` from the toolkit's config.json: "Full video in the description", "Subscribe"…).
6. Result: **a new sequence** `"<name> (Vertical)"` — the original stays intact; everything is editable by hand afterward (they are normal clips/effects). The wizard is a transaction composing existing actions → 1 undo, reproducible via MCP (`generate_vertical`).

**AC.**
1. Full wizard (with silences + subtitles) over a 3 min video < 30 s of processing (not counting whisper if it isn't cached).
2. The result is 100% editable (move subtitles, change blur, undo cuts).
3. Direct export with the Shorts preset from the final step.

### 7.E Customizable avatar + automatic subtitles

#### 7.E.1 Avatar that talks to the rhythm of the video

Full port of the toolkit's `avatar_video_generation.py`, converted from "a script that exports an mp4" into a **live timeline clip**.

**Model.** `ClipPayload::Avatar { config: AvatarConfig, driver_asset: Id }`:

```rust
struct AvatarConfig {
    avatars: BTreeMap<String, RelPath>, // emotion → video clip (loop). COMPATIBLE
                                        // with the toolkit's avatar_config/config.json
                                        // (same keys: calm, angry, sad, amazed…)
    default_emotion: String,            // first key (like the toolkit)
    shake_factor: f32,                  // vibration ∝ volume (identical concept)
    chroma: Option<ChromaParams>,       // for avatar mp4s with a green background (6.5.4);
                                        // the .mov with alpha don't need it
    scale: f32, anchor: Anchor9,        // position in the frame (bottom corner, etc.)
}
```

**Analysis pipeline** (job "Analyze avatar", cached by the driver's `content_hash`):
1. Transcript by **segments/phrases** of the driver asset (reuses 7.B; doesn't require word-level).
2. **Per-segment emotion classification** via a configurable OpenAI-compatible endpoint (OpenAI, local Ollama, LM Studio…), with the **same proven prompt from the toolkit**: *"You are an emotion classifier… reply with exactly one of: {labels}"*, lax substring matching and fallback to the default emotion on error — behavior identical to `classify_emotion()`. Parallelized (N concurrent requests), cached in the TranscriptDoc (`segments[].emotion`).
   - Fallback with no network/API: heuristic by volume+speed (high RMS→"angry/amazed", slow→"calm") so the feature works offline, with documented reduced quality.
3. **RMS volume per segment** (port of `get_subclip_volume_segment`) + global mean → vibration factor per segment.

**Live render** (implements `Renderable`, like any payload):
- At time `t`, the active segment determines the emotion → the corresponding avatar clip, in **loop** (`src_t = (t - seg.start) % avatar_dur`, decoded by the normal DecodePool); gaps between segments → default avatar (identical to the toolkit's `build_avatar_subclips`, including the final tail).
- **Shake**: `core.shake` effect injected with `intensity = (seg.volume / global_avg) * shake_factor`, implemented as a UV offset in the shader with deterministic per-frame noise (an improvement over the toolkit's random `np.roll`: reproducible and with no CPU cost).
- Chroma key if configured; then standard transform (position/scale/keyframes like any clip).

**UX.**
- Avatar editor (Settings → Avatars): create an avatar with a name, map emotion→file (drag & drop, with looping preview), add/rename free emotions (the prompt is built from the real keys, as the toolkit does), shake, chroma. **"Import toolkit config" button** that reads an existing `avatar_config/config.json` as-is.
- Usage: drag the avatar from the avatar pool to a track over the video → it anchors to the driver audio/video asset below it (or whichever the user chooses). An "Analyze" button launches the job; until then it shows with the default emotion.
- The analysis state (emotions per segment) is visible as little colors over the avatar clip and editable: right-click a segment → force emotion (manual override persisted).

**AC.**
1. With the real files from the toolkit's `avatar_config/` and a test video, the result is visually equivalent to the toolkit's `output_video.mp4` (manual side-by-side validation).
2. Manually changing a segment's emotion re-renders instantly without re-analysis.
3. With no API key configured, the avatar works with the offline heuristic.
4. 30 fps preview with avatar + chroma + 1080p base video.

#### 7.E.2 Automatic subtitles

- `ClipPayload::Subtitles { transcript_id, style, mode }` — a single clip that covers the range and renders the appropriate text at each `t` (not hundreds of text clips; far more manageable).
- **Modes**: `Phrase` (grouping by pauses > 1 s and max. N words/characters — port of the `process_transcript` logic from `translation.py` with `MAX_PAUSE=1.0`), `Word` (word by word, like the toolkit's `transcript_divided`), `Karaoke` (visible phrase + current word highlighted with color/scale).
- **Style** (`SubtitleStyle`): full typography from 6.6 + karaoke highlight color + position (default: centered, configurable Y offset — equivalent to the `text_position_y_offset` from the toolkit's `config.json`, imported as a "Toolkit" preset).
- Synced with the edit: being a dynamic render from the transcript + clip→time mapping (7.B.2), **cutting/moving video re-adjusts the subtitles on its own**. `rejected` words are not shown.
- File export: `.srt` and `.vtt` (with the rounding correction of `float_to_srt_time` done with integers), per asset or per sequence.
- Editing: correcting a word's text in the 7.B.2 panel is reflected instantly.

**AC.**
1. Karaoke mode legible and synced (± 1 frame of whisper's timestamps).
2. After removing silences, subtitles still line up without re-analysis.
3. Exported SRT valid (round-trip with an external parser) and with correct accents (UTF-8).

---

## 8. Reuse of the Youtubers-toolkit

### 8.1 Inventory and module-by-module mapping

| Toolkit (Python) | What it does today | Destination in UberEditor | Strategy |
|---|---|---|---|
| `trim.py` (`trim_by_silence`) | RMS per `clip_interval` chunks, linear threshold 0.01, cuts with moviepy | `ue-ai::silence` (7.C) | **Port to Rust** of the algorithm, improved (hysteresis, min-duration, padding, relative threshold). The `sound_threshold=0.01` ≈ -38 dBFS parameter is kept as the default. |
| `transcript.py` (`generate_transcript`, `transcript_divided`) | faster-whisper "turbo", SRT per segment or per word | `ue-ai::transcribe` (7.B) + SRT export (7.E.2) | **Port** to whisper-rs (same turbo model). The word-level SRT of `transcript_divided` ≡ the `Word` subtitle mode. |
| `subtitles.py` + `config.json` (`subtitles_clip_config`, offsets) | Burns SRT with moviepy's TextClip | Subtitles payload (7.E.2) | **Config port**: importer that converts `config.json` into a `SubtitleStyle` preset ("Toolkit"). `Hey-Comic` font, sizes and offsets included. |
| `shorts.py` (`blur_video`, `generate_video_base`, `add_titles`) | boxblur=10:1 + 1080×1920 composition + 3 s titles | Vertical wizard (7.D) + title templates (6.6) | **Conceptual port**: the blur becomes a GPU shader, the composition a layout template; `titles` from config → CTA template. |
| `set_orientation.py` | resize that INVERTS w/h (stretches) | Transform2D (6.8) | **Replaced** (the original deforms the image; the correct vertical layout is 7.D). |
| `denoise.py` (DNS64 + torch) | Offline voice denoise | RNNoise denoise per clip (6.7) | **Replaced** by RNNoise (lightweight). DNS64 via sidecar stays in the backlog if its quality is missed. |
| `avatar_video_generation.py` | Whisper → GPT emotions → volume → loops with shake → mp4 | Avatar payload (7.E.1) | **Full port to Rust** preserving: classification prompt, lax matching + fallback, segment cache, gap/tail filling with default, shake ∝ volume/mean. |
| `avatar_config/config.json` + clips | emotion→file map + shake_factor | `AvatarConfig` (7.E.1) | **Compatible format**: direct import button. The green-background mp4s use the core chroma key. |
| `translation.py` (`process_transcript`) | Groups words into phrases by pauses > 1 s | `Phrase` subtitle mode (7.E.2) | **Port of the grouping algorithm** as-is. |
| `translation.py` (Helsinki-NLP translation) + `audio_generator` (Kokoro TTS) | Dubbing: translate + TTS + reassemble audio | — | **Backlog** (section 16, "Dubbing"). Not a v1 requirement. |
| `agents/` (killer_video_idea, title_gen, persona_testing) | Title/idea ideation with OpenAI | — | **Not embedded**: with the MCP server (7.A), an external agent (Claude) does this better by reading the project's transcript. Documented as an MCP recipe. |
| `recipes.py` | Chains CLI commands | UI wizards + high-level MCP tools | Each recipe ≡ a wizard: `separate_video`→Silences, `generate_short_base`→Vertical, `subtitle_video`→Subtitles, `generate_avatar`→Avatar. |
| `utils.py` (`get_subclip_volume*`, `float_to_srt_time`, `apply_shake`, `get_audio`) | Helpers | `ue-media` / `ue-ai` / shake shader | **Trivial port** (RMS and SRT in integers; shake on the GPU). |
| `main.py` (kwargs pipeline) | CLI orchestration | ActionRegistry + transactions | The "pipeline of composable steps" pattern survives as composition of Actions. |

### 8.2 What is NOT inherited (known toolkit debts that are fixed)

- MoviePy/CPU for everything → GPU (wgpu); it is the #1 reason for the toolkit's slowness.
- Times in float and SRT with rounding → integer `TimeUs`.
- `np.random` without a seed in the shake → deterministic noise.
- `set_vertical` that stretches the image → real 9:16 layout.
- Subprocesses with `shell=True` and path interpolation → sidecars with args (paths with spaces are safe).
- State in loose files next to the video (`*_transcript.srt`, `*_segments.json`) → centralized cache by hash + self-contained project.

### 8.3 Optional bridge: `toolkit-bridge` sidecar

Safety net if some native port gets stuck (especially whisper word-level or denoise quality):

- Package a subset of the toolkit (faster-whisper; optionally DNS64/Kokoro) with **PyInstaller** as a per-platform sidecar binary, exposed by **JSON-RPC over stdio**: `transcribe(path, model, word_timestamps) → transcript.json`, `denoise(path) → wav`.
- Data contract identical to the native one (Appendix D) → interchangeable with a config flag.
- Cost: ~300–500 MB of extra binary; that's why it is **development opt-in**, not the plan A for distribution.

---

## 9. IPC API (frontend ↔ engine)

Convention: `domain.action` commands (Tauri `invoke`), typed `Result<T, UeError>` responses. TypeScript types **generated** from the Rust structs (ts-rs or specta) — a single source of truth.

### 9.1 Commands (selection; the list grows with the ActionRegistry)

| Domain | Commands |
|---|---|
| `project` | `new`, `open(path)`, `save`, `save_as(path)`, `close`, `undo`, `redo`, `get_state`, `get_history` |
| `media` | `import(paths[])`, `remove(asset_id)`, `relink(asset_id, path)`, `get_pool` |
| `timeline` | `add_clip`, `move_clip`, `split_clip`, `trim_clip`, `delete(ids, ripple)`, `add_track`, `set_track_props`, `add_marker`, `cut_ranges` |
| `clip` | `set_transform`, `set_audio_props`, `add_effect`, `remove_effect`, `set_param(path, value)`, `set_keyframe`, `delete_keyframe`, `add_transition` |
| `playback` | `play`, `pause`, `seek(t)`, `set_rate(r)`, `set_quality(q)`, `set_loop(in, out)` |
| `text` | `add_text_clip`, `set_content`, `set_style`, `save_template`, `list_templates` |
| `export` | `list_presets`, `start(preset, overrides)`, `cancel(job_id)`, `queue_status` |
| `ai` | `transcribe(asset_id, opts)`, `analyze_silences(scope, params)` (analysis only), `apply_silences(analysis_id, action)`, `delete_words(word_ids, opts)`, `reject_words(word_ids)`, `apply_rejected`, `generate_vertical(opts)`, `avatar_analyze(clip_id)`, `avatar_set_emotion(clip_id, seg_idx, emotion)` |
| `effects` | `list`, `reload_user_packs`, `get_manifest(id)` |
| `mcp` | `status`, `set_mode(off/ro/rw)`, `regenerate_token` |
| `jobs` | `list`, `cancel(id)` |

### 9.2 Events (engine → UI)

| Event | Payload | Frequency |
|---|---|---|
| `state.patch` | JSON Patch + version number | per mutation |
| `playback.position` | `t_us`, state, dropped | 30 Hz during play |
| `preview.frame` | binary over Channel (5.5) | 30 Hz |
| `audio.meters` | RMS/peak per track | 15 Hz |
| `job.progress` | id, kind, 0–1, message | ≤ 4 Hz per job |
| `job.done` / `job.error` | id, result/error | per job |
| `mcp.activity` | tool called, client | per call (for the UI indicator) |

---

## 10. Repository structure

```
ubereditor/
├── PLAN.md                      # this document
├── package.json / pnpm-lock.yaml
├── src/                         # React + TS frontend
│   ├── app/                     # shell, layout, panel routing, themes
│   ├── components/
│   │   ├── timeline/            # timeline canvas, interactions, keyframe lanes
│   │   ├── preview/             # WebGL canvas, transport, transform gizmo, eyedropper
│   │   ├── media-pool/
│   │   ├── inspector/           # param widgets auto-generated from manifests
│   │   ├── transcript/          # text-based editing panel
│   │   ├── export/              # dialog + queue
│   │   ├── wizards/             # vertical, silences, avatar
│   │   └── settings/            # AI, MCP, avatars, shortcuts
│   ├── state/                   # zustand stores (mirror), applying patches
│   ├── ipc/                     # typed wrappers of invoke/events (generated code)
│   └── lib/
├── src-tauri/
│   ├── tauri.conf.json          # ffmpeg/ffprobe sidecars (externalBin), CSP, updater
│   ├── binaries/                # ffmpeg-<target-triple>, ffprobe-<target-triple>
│   ├── icons/
│   └── src/main.rs              # wiring: commands, events, MCP startup
├── crates/                      # (see 3.3) ue-core, ue-media, ue-render, ue-audio,
│   │                            #  ue-playback, ue-export, ue-ai, ue-mcp
│   └── ue-core/tests/           # action proptests, .uep fixtures
├── effects/core/                # embedded effect/transition packs
├── assets/                      # bundled fonts (OFL), title templates, presets
├── docs/                        # ADRs (decisions), user effects guide, MCP guide
└── .github/workflows/           # CI (section 12)
```

---

## 11. Performance budgets

Reference hardware: MacBook Air M1 8 GB / Ryzen 5 PC + integrated Vega GPU / equivalent Ubuntu.

| Metric | Target | How it's measured |
|---|---|---|
| Timeline UI | 60 fps with 500 clips | rAF traces in manual CI |
| 1080p preview (2 video tracks + text + music) | 30 fps with no sustained drops | dropped-frames counter |
| Scrub latency (with proxy) | < 150 ms p95 | instrumented test |
| Playback seek | < 300 ms p95 | ditto |
| A/V desync | < 40 ms sustained | beep+flash test video |
| Import (probe + usable) | < 1 s | per file |
| 720p proxy of 10 min 1080p | < 2 min in background | job timing |
| 1080p H.264 export | ≥ 1× real time | job timing |
| Whisper large-v3-turbo (M1, Metal) | ≥ 5× real time | job timing |
| RAM at rest with a medium project | < 1.5 GB (not counting the configurable FrameCache) | Activity Monitor |
| Cold start to interactive | < 3 s | manual CI stopwatch |

Decisions that protect these numbers: short-GOP proxies, YUV on the GPU, single canvas in the timeline, LRU cache, patches instead of full state over IPC, jobs off the render thread.

---

## 12. Testing and CI strategy

**Unit (Rust, `cargo test`):**
- `ue-core`: model invariants, each Action and its inverse, **proptest** (random sequences of actions + full undo ≡ initial state), schema migrations with fixtures, keyframe evaluator (exact values at boundaries).
- `ue-ai`: silence detector over synthetic WAVs generated in the test (tones + known silences); grouping by pauses; word→timeline mapping with fixture transcripts; exhaustive `cut_ranges`.
- `ue-media`: ffprobe parsing with real JSONs recorded as fixtures (includes VFR, rotate=90, 5.1, no audio).

**Golden frames (render):**
- Small fixture projects → render of N specific frames → perceptual hash (dHash) against versioned reference images with tolerance; runs on the 3 CI platforms (with a software adapter: `wgpu` + lavapipe/llvmpipe on Linux CI).
- Covers: each core effect, each transition, chroma key with a green test image, text with accents/emoji, transform with rotation.

**Integration:**
- Export of a 10 s fixture project → validation with ffprobe (duration, fps, resolution) + extraction of 3 frames → golden.
- MCP: a test client that calls each tool against a headless app (the engine can be instantiated without the webview — a design that additionally enables a future CLI).
- Whisper: a short fixture audio with known text → lax asserts over the words (due to model variability, only in the nightly job with the tiny model).

**E2E (frontend):** Playwright + tauri-driver (WebDriver) for flows: import→cut→export; undo/redo; vertical wizard. Only smoke in CI (slow), full suite manual pre-release.

**CI (GitHub Actions):**
- macOS/Windows/Ubuntu matrix: `cargo clippy -D warnings`, `cargo test`, `pnpm typecheck && pnpm test`, unsigned Tauri build.
- Nightly: full golden frames + criterion bench (regressions > 15% fail) + E2E smoke.
- Release: signed builds (section 13) + upload to GitHub Releases + updater manifest.

---

## 13. Packaging, distribution and licenses

**FFmpeg sidecars:** static per-platform builds in `src-tauri/binaries/` with a target-triple suffix (`ffmpeg-aarch64-apple-darwin`, etc.), declared in `externalBin`. A `scripts/fetch-ffmpeg.ts` script downloads and verifies them (sha256) in dev and CI.

**Licenses (important):**
- FFmpeg with libx264/x265 → **GPL** binary: distributing it as a sidecar (separate process) means we comply by publishing the build's provenance, its license and an offer of sources ("Third-party licenses" page in the app). UberEditor's own code is not forced to GPL by running an external binary. If it ever becomes a problem: LGPL build without x264 (using openh264/hardware encoders) — decision documented as an ADR.
- whisper.cpp (MIT), wgpu (MIT/Apache), cosmic-text (MIT/Apache), RNNoise (BSD), gl-transitions (MIT), rmcp (MIT/Apache) — no friction. Bundled fonts: OFL only.
- Whisper models: downloaded on the user's side (not redistributed in the installer).

**Installers:** macOS: universal `.dmg` (arm64+x64), signed + **notarized** (Apple Developer account needed — a process to start early). Windows: signed NSIS `.exe` (OV certificate or signing with Azure Trusted Signing). Linux: AppImage + `.deb`.

**Updater:** tauri-plugin-updater with a manifest in GitHub Releases and a custom update signature.

**Telemetry:** none in v1 (explicit decision). Opt-in crash reports via sentry-rust in the backlog.

---

## 14. Phased roadmap

> Estimates for **1 senior developer full-time** assisted by AI agents. The ranges assume learning wgpu/Tauri is included. Each phase ends with its AC verified and a git tag.

### Phase 0 — Foundations (1–2 weeks)
Tauri 2 + React scaffolding + crate workspace (3.3, 10); ffmpeg sidecars working with paths with spaces; green CI matrix; `ue-core` with the data model (4), basic actions, history and proptest; initial ADRs.
**Milestone:** green `cargo test` on 3 OSes; create/save/open an empty `.uep` from the UI.

### Phase 1 — Editable MVP (6–8 weeks)
Import + probe + cache jobs (proxy, conforming, peaks, thumbs) (6.3); Media Pool; timeline canvas with drag&drop, split/trim/ripple (6.1, 6.2); DecodePool + minimal RenderGraph (YUV→RGB, basic transform, compositing) (5.1–5.3); AudioMixer + master clock (5.4); preview over Channel (5.5); H.264/AAC export with 2 presets (5.6); full save/undo/redo (6.10).
**Milestone (demo):** import 3 clips, cut them, reorder them, hear the audio in sync and export a correct mp4.

### Phase 2 — Complete editor (5–7 weeks)
Effect pack system + hot-reload (6.5.1–6.5.2); core catalog including **chroma key** (6.5.4); transitions (6.5.3); color correction + Transform2D with gizmo (6.8); text and titles with templates (6.6); keyframes + lanes + curve editor (6.11); full audio (fades, keyframes, meters, RNNoise) (6.7); complete export dialog with queue and R128 (6.9).
**Milestone:** a "real" video edited end to end only with UberEditor, green-screen chroma key included.

### Phase 3 — Text and silence AI (4–6 weeks)
whisper-rs + model management + transcription job with cache (7.B.1); transcript panel, delete/strike words, filler-word search (7.B.2); robust `cut_ranges`; silence detector + dialog with preview + 3 actions (7.C); automatic subtitles (payload, phrase/word/karaoke modes, SRT/VTT export) (7.E.2).
**Milestone:** the flow "import → transcribe → delete filler words by text → remove silences → karaoke subtitles".

### Phase 4 — MCP (1.5–2.5 weeks)
`ue-mcp` with rmcp: read tools, resources, token, off/ro/rw levels (7.A); write tools mapped to the ActionRegistry; docs + connection snippet; MCP activity indicator in the UI.
**Milestone (star demo):** connected Claude Code answers "what's in my project?" and runs "remove the silences and export me a preview".

### Phase 5 — Creator: vertical + avatar (4–6 weeks)
Vertical wizard with layout templates, silences+subtitles+titles integration (7.D); Avatar payload: avatar editor, toolkit config importer, analysis (LLM emotions + offline fallback + volume), live render with deterministic shake and chroma (7.E.1); `generate_vertical` / `generate_avatar_track` MCP tools.
**Milestone:** functional parity with the toolkit's recipes (`separate_video`, `generate_short_base`, `generate_avatar`, `subtitle_video`) — side-by-side validation with the same input files.

### Phase 6 — Polish and release (3–4 weeks)
Performance against the budgets of section 11 (perf pass); signing/notarization + updater (13); onboarding (included demo project), configurable shortcuts, es/en i18n; user docs (custom effects, MCP, avatars); bug bash + full E2E suite; v1.0.
**Milestone:** signed installers on all 3 platforms, downloadable.

**Estimated total: ~6–7.5 months** full-time. Critical path: Phase 1 (the engine). Phases 3–5 are parallelizable among themselves if people are added (they depend on P1–P2, not on each other — except that 7.D and 7.E.2 use 7.B/7.C).

---

## 15. Risks and mitigations

| # | Risk | Prob. | Impact | Mitigation |
|---|---|---|---|---|
| 1 | The IPC preview doesn't sustain 30 fps on some platform (5.5) | Medium | High | Automatic JPEG fallback; Phase-2 native-surface plan designed from day 1 (the RenderGraph doesn't know how it's presented); lower Auto resolution. |
| 2 | Slow seeks / VFR break frame precision | Medium | High | ALWAYS short-GOP CFR proxies for preview; VFR detection on import; `ffmpeg-next` as an upgrade path. |
| 3 | whisper.cpp word-timestamps less precise than faster-whisper | Medium | Medium | Plan B `toolkit-bridge` (8.3) with an identical contract; configurable padding in text cuts cushions ±50 ms. |
| 4 | wgpu on old GPUs / broken Linux drivers | Low-medium | Medium | wgpu GL fallback; software render (llvmpipe) as a last resort with a warning; minimum GPU matrix documented. |
| 5 | Engine complexity exceeds the estimate (which is normal) | High | High | Phase 1 is ONLY the minimum; pre-agreed cuts: reverse playback, slide tool, curve editor (can drop to Phase 6/backlog without touching requirements). |
| 6 | ffmpeg's GPL license becomes uncomfortable in the future | Low | Medium | Isolated sidecar + ADR with an LGPL plan (13). |
| 7 | Emotion classification expensive/slow with a paid API | Medium | Low | Aggressive cache by hash; local Ollama support; offline heuristic; batch of segments per request. |
| 8 | Scope creep (it's a video editor!) | High | High | This document is the contract: whatever is not in sections 6–7 goes to 16 and waits for v1.1. |
| 9 | Notarization/signing blocks the release | Medium | Low | Start the paperwork (Apple Developer, Windows cert) in Phase 0–1, not at the end. |
| 10 | API changes in Tauri 2 / rmcp (young ecosystems) | Medium | Low | Pinned versions; thin custom wrappers around both. |

---

## 16. Future backlog (post-v1)

Ordered by estimated value for the user's workflow:

1. **Auto-reframe with face detection** for vertical mode (ONNX yolov8-face/mediapipe + EMA smoothing → automatic crop keyframes).
2. **Automatic dubbing** (port of the toolkit's `translation.py` + Kokoro TTS: transcribe → translate → TTS → reassemble with speed adjustment).
3. **MCP ideation recipes** (document prompts so Claude generates titles/ideas from `transcript://` — replaces the toolkit's `agents/`).
4. Smooth reverse playback, slide tool, compound clips / nested timelines.
5. Detection and removal of **filler words by list** with 1 click (over 7.B: search "um", "you know", "like" → strike all).
6. Direct recording (camera/mic/screen) inside the app.
7. DNS64 denoise via sidecar (maximum quality), de-esser, voice compressor.
8. Parallel export, distributed rendering of the queue.
9. HDR / linear 16F color space; .cube LUTs.
10. Marketplace/shared folder of effect packs and templates.
11. Opt-in crash reporting; local performance metrics.
12. Headless CLI (`ubereditor render project.uep --preset youtube`) — the engine already allows it (12).

---

## Appendix A. Complete project file example

```jsonc
{
  "schema_version": 1,
  "id": "01JZK3M9V2Q8XW5T7YBGN4RducK",
  "name": "Devlog 12",
  "created_at": "2026-07-09T10:00:00Z",
  "settings": { "whisper_language": "es", "autosave_secs": 60 },
  "assets": [
    {
      "id": "01JZK3MA...A1", "kind": "video", "path": "media/take1.mp4",
      "content_hash": "xxh3:9f2c…", 
      "probe": { "duration_us": 754000000, "fps": [30000, 1001], "width": 1920,
                 "height": 1080, "rotation": 0, "vcodec": "h264", "acodec": "aac",
                 "audio_channels": 2, "vfr": false },
      "proxy": null, "audio_conform": null, "peaks": null, "thumbnails": null,
      "transcript": "01JZK3TR...T1"
    },
    { "id": "01JZK3MA...A2", "kind": "audio", "path": "media/music.mp3", "…": "…" },
    { "id": "01JZK3MA...A3", "kind": "image", "path": "media/logo.png", "…": "…" }
  ],
  "transcripts": [ { "id": "01JZK3TR...T1", "asset_id": "01JZK3MA...A1", "…": "see Appendix D" } ],
  "sequences": [
    {
      "id": "01JZK3SQ...S1", "name": "Main",
      "resolution": [1920, 1080], "fps": [30000, 1001], "sample_rate": 48000,
      "markers": [ { "t": 12000000, "name": "Intro end", "color": "#e5484d" } ],
      "tracks": [
        {
          "id": "01JZ...TA1", "kind": "audio", "name": "Music",
          "muted": false, "solo": false, "locked": false, "volume_db": -12.0,
          "clips": [
            {
              "id": "01JZ...C10",
              "payload": { "type": "media", "asset_id": "01JZK3MA...A2",
                           "src_in": 0, "src_out": 90000000 },
              "start": 0, "duration": 90000000, "speed": 1.0,
              "effects": [], "transform": null,
              "audio": { "gain_db": { "type": "curve", "keys": [
                           { "t": 0, "value": -6.0, "interp": "linear" },
                           { "t": 5000000, "value": -18.0, "interp": "linear" } ] },
                         "pan": 0.0, "fade_in_us": 1500000, "fade_out_us": 3000000 }
            }
          ]
        },
        {
          "id": "01JZ...TV1", "kind": "video", "name": "V1",
          "clips": [
            {
              "id": "01JZ...C01",
              "payload": { "type": "media", "asset_id": "01JZK3MA...A1",
                           "src_in": 2500000, "src_out": 32500000 },
              "start": 0, "duration": 30000000, "speed": 1.0,
              "effects": [
                { "effect_id": "core.color_correct", "enabled": true,
                  "params": { "brightness": 0.05, "contrast": 1.1, "saturation": 1.15 } },
                { "effect_id": "core.chroma_key", "enabled": true,
                  "params": { "key_color": [0.0, 1.0, 0.0, 1.0], "similarity": 0.3,
                              "smoothness": 0.1, "spill": 0.6 } }
              ],
              "transform": { "position": [0, 0], "scale": [1.0, 1.0], "rotation": 0.0,
                             "crop": [0, 0, 0, 0], "opacity": 1.0 },
              "audio": { "gain_db": 0.0, "pan": 0.0 },
              "transition_in": { "effect_id": "core.crossfade", "duration": 500000 }
            }
          ]
        },
        {
          "id": "01JZ...TV2", "kind": "video", "name": "Subs + Avatar",
          "clips": [
            { "id": "01JZ...C20",
              "payload": { "type": "subtitles", "transcript_id": "01JZK3TR...T1",
                           "mode": "karaoke",
                           "style": { "font": "Hey-Comic", "size": 60, "color": "#ffffff",
                                      "highlight_color": "#ffd93d", "bg": "#000000cc",
                                      "y_offset": -500 } },
              "start": 0, "duration": 30000000 },
            { "id": "01JZ...C21",
              "payload": { "type": "avatar", "driver_asset": "01JZK3MA...A1",
                           "config": { "avatars": { "calm": "avatars/calm.mov",
                                                     "angry": "avatars/angry.mp4" },
                                       "default_emotion": "calm", "shake_factor": 1.0,
                                       "chroma": { "key_color": [0,1,0,1] },
                                       "scale": 0.35, "anchor": "bottom_right" } },
              "start": 0, "duration": 30000000 }
          ]
        }
      ]
    }
  ],
  "active_sequence": "01JZK3SQ...S1"
}
```

## Appendix B. Modular effect example

`effects/user/vhs_retro/manifest.json`:

```json
{
  "id": "user.vhs_retro",
  "kind": "effect",
  "version": "1.0.0",
  "name": { "es": "VHS Retro", "en": "Retro VHS" },
  "category": "stylize",
  "author": "hector",
  "shader": "shader.wgsl",
  "params": [
    { "key": "intensity",  "type": "float", "default": 0.5, "min": 0, "max": 1,
      "label": { "es": "Intensidad" } },
    { "key": "line_count", "type": "float", "default": 240, "min": 50, "max": 1080 },
    { "key": "color_bleed","type": "float", "default": 0.3, "min": 0, "max": 1 },
    { "key": "tint",       "type": "color", "default": [1.0, 0.95, 0.9, 1.0] }
  ]
}
```

`effects/user/vhs_retro/shader.wgsl` (contract: the runtime provides `tex`, `samp`, the manifest's uniforms in `params`, and globals `time_s`, `seed`, `resolution`):

```wgsl
@fragment
fn effect(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    // horizontal chromatic aberration
    let off = params.color_bleed * 0.004;
    let r = textureSample(tex, samp, uv + vec2(off, 0.0)).r;
    let g = textureSample(tex, samp, uv).g;
    let b = textureSample(tex, samp, uv - vec2(off, 0.0)).b;
    var col = vec3(r, g, b) * params.tint.rgb;
    // scanlines
    let scan = 0.5 + 0.5 * sin(uv.y * params.line_count * 3.14159);
    col *= mix(1.0, scan, params.intensity * 0.35);
    // deterministic noise (hash per pixel+frame, reproducible in export)
    let n = hash13(vec3(uv * resolution, f32(seed)));
    col += (n - 0.5) * params.intensity * 0.08;
    return vec4(col, textureSample(tex, samp, uv).a);
}
```

Saving these two files with the app open makes the effect appear in the Inspector with its 4 controls, all keyframable. **Zero changes to the core.**

## Appendix C. Reference FFmpeg commands

```bash
# Probe (import)
ffprobe -v quiet -print_format json -show_format -show_streams INPUT

# Short-GOP 720p proxy (fast seeks); CFR if the source is VFR
ffmpeg -y -i INPUT -vf "scale=-2:720" -c:v libx264 -preset veryfast -crf 20 \
       -g 15 -pix_fmt yuv420p -vsync cfr -r FPS -an CACHE/proxy.mp4

# Audio conforming (mixer and whisper start from here)
ffmpeg -y -i INPUT -vn -ac 2 -ar 48000 -c:a pcm_s16le CACHE/audio.wav
ffmpeg -y -i CACHE/audio.wav -ac 1 -ar 16000 CACHE/audio16k.wav   # for whisper

# Thumbnails (sprite: 1 frame every 2 s, 160px tall)
ffmpeg -y -i INPUT -vf "fps=1/2,scale=-2:90,tile=100x1" -frames:v 1 CACHE/thumbs.jpg

# Preview decode session (raw frames on stdout)
ffmpeg -v error -ss SEEK -i PROXY -f rawvideo -pix_fmt yuv420p pipe:1

# Export (frames on stdin + mixdown)
ffmpeg -y -f rawvideo -pix_fmt rgba -s 1920x1080 -r 30000/1001 -i pipe:0 \
       -i mixdown.wav -map 0:v -map 1:a \
       -c:v libx264 -preset slow -crf 18 -pix_fmt yuv420p \
       -c:a aac -b:a 320k -movflags +faststart OUT.mp4

# R128 loudness (2 passes: analysis → apply with measured_*)
ffmpeg -i mixdown.wav -af loudnorm=I=-14:TP=-1.5:LRA=11:print_format=json -f null -

# GIF (palette in 2 passes)
ffmpeg -y -i pipe:0 -vf "fps=15,scale=720:-1:flags=lanczos,palettegen" palette.png
ffmpeg -y -i pipe:0 -i palette.png -lavfi "fps=15,scale=720:-1 [x]; [x][1:v] paletteuse" OUT.gif
```

## Appendix D. Word-level transcript format

`CACHE/<content_hash>/transcript.json` — contract shared by native whisper-rs and the `toolkit-bridge` sidecar:

```json
{
  "version": 1,
  "asset_hash": "xxh3:9f2c…",
  "language": "en",
  "model": "large-v3-turbo",
  "generated_at": "2026-07-09T10:12:00Z",
  "words": [
    { "i": 0, "text": "Hello",    "start_us": 480000, "end_us": 820000,
      "confidence": 0.97, "rejected": false },
    { "i": 1, "text": "everyone", "start_us": 860000, "end_us": 920000,
      "confidence": 0.91, "rejected": false }
  ],
  "segments": [
    { "i": 0, "text": "Hello everyone, welcome.", "start_us": 480000,
      "end_us": 2600000, "word_range": [0, 4],
      "emotion": "calm", "volume_rms": 812.4 }
  ],
  "global_avg_volume": 640.2
}
```

- `segments[].emotion` and `volume_rms` are filled in by the avatar analysis (7.E.1) — same role as the toolkit's `<audio>_segments.json`, for which an importer exists.
- `rejected` implements the strikethrough mode (7.B.2).
- SRT/VTT export is derived from here (by words or by segments depending on the mode).

## Appendix E. Initial catalog of MCP tools

**Reads (`read-only` level):**

| Tool | Args | Returns |
|---|---|---|
| `get_project_summary` | — | name, duration, number of assets/sequences/clips, active jobs, dirty |
| `get_media_pool` | — | list of assets with probe, cache status and transcript |
| `get_timeline` | `sequence_id?`, `include_params?` | tracks and clips (payloads, times, effects) |
| `get_clip` | `clip_id` | full detail: params, keyframes, transitions |
| `get_transcript` | `asset_id`, `format: json\|srt\|text` | word-level transcript / SRT / plain text |
| `search_transcript` | `query`, `scope` | matching words with ids and times |
| `get_selection_and_playhead` | — | what the user is looking at/has selected now |
| `get_jobs` | — | jobs with progress |
| `get_effects_catalog` | — | available manifests (so the agent knows what it can apply) |
| `get_history` | `limit?` | latest undo entries (audit) |

**Writes (`read-write` level; every tool = a transaction with label `[MCP] …`):**

| Tool | Args (summary) |
|---|---|
| `split_clip` | `clip_id`, `t_us` |
| `cut_ranges` | `sequence_id`, `ranges[]`, `ripple` |
| `move_clip` / `trim_clip` / `delete_clips` | ids + times |
| `set_clip_property` | `clip_id`, `path` (e.g. `transform.scale`), `value` or `curve` |
| `apply_effect` / `remove_effect` | `clip_id`, `effect_id`, `params?` |
| `add_text_clip` | `sequence_id`, `track_id`, `t`, `content`, `style?/template?` |
| `add_marker` | `t`, `name`, `color?` |
| `delete_words` / `reject_words` | `word_ids[]` or `object {asset_id, indices}` + `padding_ms?` |
| `remove_silences` | `scope`, `params?` (defaults from 7.C), `action: delete\|speedup\|mark` |
| `generate_vertical` | wizard options from 7.D |
| `generate_avatar_track` | `sequence_id`, `avatar_name`, `driver_asset` |
| `transcribe_asset` | `asset_id`, `model?`, `language?` |
| `start_export` | `preset`, `overrides?` → `job_id` |
| `undo` / `redo` | — (the agent can undo itself) |

**Resources:** `project://current` (full JSON, updated with notifications), `transcript://{asset_id}` (JSON), `transcript://{asset_id}.srt`.

---

*End of the plan. Living document: deviations during implementation are recorded as ADRs in `docs/` and reflected here.*
