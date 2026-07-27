# OpenInCut

Cross-platform desktop video editor built for content creators, with AI superpowers: **text-based editing** (word-by-word Whisper), **one-click silence removal**, **automatic verticals**, **emotion-reactive avatar**, **karaoke subtitles**, **voiceover from text**, and an **embedded MCP server** so an AI agent can edit your project for you.

[English](README.md) | [فارسی](README-fa.md) | [العربية](README-ar.md) | [Русский](README-ru.md) | [中文](README-zh.md)

---

## ✨ Features

### Video Editor (FCP-Style Toolbar)
- **A/B/T/P/H/Z Tools**: Selection (A), Blade (B), Trim (T), Position (P), Hand (H), Zoom (Z)
- **Color Board**: Lift / Gamma / Gain color wheels with master offset
- **Transform Overlay**: Drag handles for position, scale, rotation on canvas
- **Speed Ramp**: Bezier-curve speed ramping with real-time preview
- **Context Menu**: Right-click clips → Audio / Speed submenus

### Audio System
- **Audio Roles**: Video / Dialogue / Music / SFX with role-based routing
- **Audio Dispatch**: Per-clip volume, mute, speed, solo/mute per role
- **Audio Detach**: Detach audio from video clip to separate track
- **Per-clip volume / mute / speed** from timeline context menu

### Color Correction
- **Color Board**: Lift / Gamma / Gain wheels + Master offset
- **Color Wheels**: Master / Shadows / Midtones / Highlights
- **HDR / HDR-ready** pipeline (16-bit float pipeline)

### HyperFrames (Text Templates + Render-to-Timeline)
- **Title / Lower Third / Subtitle** templates
- **Render-to-Timeline**: Bake text overlays directly onto timeline as video clips
- **Farsi / Arabic TTS Voices** (Kokoro-82M: 54 voices, 8 languages)
- **Voice Over / Dubbing**: Record or TTS-generate voiceover directly on timeline
- **RTL Text Direction** for Persian, Arabic, Hebrew
- **Karaoke Subtitles** (word-by-word highlight from Whisper word timestamps)
- **Emoji / Emoji-animation support** in titles

### AI & MCP Server
- **MCP Server** (53 tools) — binds `0.0.0.0:4599` for remote access
- **MCP Chat + AI**: Connect via 9Router / OpenAI / Ollama / Anthropic
- **Vision Support**: Attach frames + multimodal prompts to chat
- **Chat History Persistence** (SQLite)
- **Anthropic Direct** (native Anthropic API, no proxy)
- **Hermes Agent Connection**: Connect Hermes Agent as MCP client

### AI Editing (via MCP / Chat)
- Text-based editing: delete words → video cuts automatically
- "Remove silence 0:15–0:20"
- "Add Persian subtitles"
- "Fix perspective on this clip"
- "Add title: Welcome"
- "Remove silence from 0:15 to 0:20"
- "Add Persian subtitles"
- "Fix perspective on this clip"

### Color & Effects
- **Color Correction**: Brightness, Contrast, Saturation, Gamma
- **Color Board**: Lift / Gamma / Gain wheels + Master
- **Chroma Key**: Green / Blue screen removal
- **Gaussian Blur**: Background blur
- **Drop Shadow**: Shadow effects for overlays
- **Perspective Correction**: Horizon leveling (AI-assisted)
- **Vertical Fill**: Blurred background for vertical video

### Subtitles & TTS
- **Auto-subtitles** from transcription (word-by-word or karaoke mode)
- **Persian & Arabic** language support
- **RTL text direction** for RTL languages
- **Text-to-Speech**: Kokoro-82M (54 voices, 8 languages)
- **Farsi / Arabic TTS voices**
- **Voice Over / Dubbing**: Record or TTS-generate voiceover on timeline

### Timeline Editing
- Drag-and-drop timeline with magnetic snapping
- Multi-track compositing
- Keyframe animation on all properties
- 11 transition types (crossfade, wipe, slide, dissolve, etc.)
- Speed control (0.25× to 4×) with pitch preservation
- **Speed Ramp with Bezier curves**
- **Context Menu**: Right-click → Audio / Speed submenus

### Export
- MP4, WebM, GIF, **TS / MKV / M2TS** support
- Custom resolution and quality settings
- Batch export with piece ranges
- **Export Settings** dialog (persisted)

### Internationalization & UX
- **i18n**: 8 languages (English, فارسی, العربية, Русский, 中文, Español, Français, Deutsch)
- **RTL UI** (full RTL layout for Persian/Arabic/Hebrew)
- **Tooltips** on all toolbar buttons
- **Toast Notifications** (toast notifications for actions)
- **Splash Screen** on startup
- **Status Bar**: Real-time timecode, zoom, playback status
- **Settings Dialog**: All settings apply immediately (persisted)
- **Shortcuts Editor**: Fully editable keyboard shortcuts (persisted)
- **Logo + App Icon** (custom branding)

### MIDI & Hardware
- **MIDI Support**: LED / Backlight / Presets / Multi-device
- **MIDI Mapping** for transport / tools / jog wheel

### Remote & Integration
- **Remote MCP**: Binds `0.0.0.0:4599` for LAN/remote AI agents
- **MCP Chat + AI**: 9Router / OpenAI / Ollama / Anthropic
- **Vision Support**: Frame attach + multimodal prompts
- **Chat History Persistence** (SQLite)
- **Anthropic Direct** (native API, no proxy)
- **Hermes Agent Connection**

### Formats & Export
- **Input**: MP4, MOV, WebM, **TS / MKV / M2TS**, MP3, WAV, AAC, FLAC, images
- **Export**: MP4, WebM, GIF, TS, MKV, M2TS
- **Export Settings** dialog (persisted)

### Downloads (v0.2.1)
- **NSIS Installer** (~5 MB): [OpenInCut-Setup-0.2.1.exe](https://github.com/PeDitXOS/OpenInCut/releases/download/v0.2.1/OpenInCut-Setup-0.2.1.exe)
- **MSI Installer** (~7 MB): [OpenInCut-0.2.1.msi](https://github.com/PeDitXOS/OpenInCut/releases/download/v0.2.1/OpenInCut-0.2.1.msi)
- **GitHub Releases**: <https://github.com/PeDitXOS/OpenInCut/releases>

### Upstream
Based on **UberEditor** by **HectorPulido** — <https://github.com/HectorPulido/UberEditor>

---

## Quick Start

### Requirements
- FFmpeg 6+ on PATH
- Rust (stable) + Node 20+

### Install
```bash
git clone https://github.com/PeDitXOS/OpenInCut.git
cd OpenInCut
npm install
npx tauri dev
```

### Or download pre-built
Download the latest release from [GitHub Releases](https://github.com/PeDitXOS/OpenInCut/releases).

---

## MCP Server

OpenInCut runs an MCP server at `http://0.0.0.0:4599/mcp` on startup (binds `0.0.0.0` for LAN/remote). Any AI agent that speaks MCP can edit your project:

```bash
claude mcp add --transport http opencut http://127.0.0.1:4599/mcp \
  --header "Authorization: Bearer ***"
```

**53 tools available** — see [docs/MCP.md](docs/MCP.md).

Connect via:
- **Claude Desktop** (MCP)
- **9Router** (local gateway)
- **Ollama** (local LLMs)
- **OpenAI / Anthropic** (cloud)
- **Hermes Agent** (native MCP client)

---

## Language Support

| Feature | Supported Languages |
|---------|-------------------|
| Whisper Transcription | English, Persian, Arabic, Spanish, Portuguese, French, German |
| TTS Voice (Kokoro-82M) | English (US/GB), Spanish, French, Italian, Japanese, Hindi, Portuguese, Mandarin |
| Farsi / Arabic TTS | Yes (Kokoro voices) |
| Text Direction | LTR (English, etc.) + RTL (Persian, Arabic, Hebrew) |
| UI Language | 8 languages (EN, FA, AR, RU, ZH, ES, FR, DE) |

---

## Architecture

Nine crates (pure model, media, audio, render, text, export, AI, whisper, Tauri shell), one compositor shared by preview and export, microseconds everywhere.

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

---

## Credits

OpenInCut is based on **UberEditor** by **HectorPulido**. We are grateful for the original work on the MCP server architecture, timeline engine, and AI integration framework.

Upstream: <https://github.com/HectorPulido/UberEditor>

---

## Contributing

Contributions welcome! See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

---

## License

Apache-2.0 — see [LICENSE](LICENSE) for details. Copyright 2026 PeDitXOS and contributors.