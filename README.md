# OpenInCut

Cross-platform desktop video editor built for content creators, with AI superpowers: **text-based editing** (word-by-word Whisper), **silences gone with one click**, **automatic verticals**, **emotion-reactive avatar**, **karaoke subtitles**, **voiceover from text**, and an **embedded MCP server** so an AI agent can edit your project for you.

[English](README.md) | [فارسی](README-fa.md) | [العربية](README-ar.md) | [Русский](README-ru.md) | [中文](README-zh.md)

---

## ✨ Features

### Text-Based Editing
Transcribe your video with Whisper (word-level timestamps), then edit by deleting words in the transcript — the video cuts automatically.

### AI-Powered Editing
Built-in MCP server with 53 tools. Connect Claude, ChatGPT, or any LLM to edit your project through natural language:
- "Add a title that says Welcome"
- "Remove the silence from 0:15 to 0:20"
- "Fix the perspective on this clip"
- "Add Persian subtitles"

### Color Correction and Effects
- **Color Correction**: brightness, contrast, saturation, gamma
- **Chroma Key**: green/blue screen removal
- **Gaussian Blur**: background blur
- **Drop Shadow**: shadow effects for overlays
- **Perspective Correction**: horizon leveling (AI-assisted)
- **Vertical Fill**: blurred background for vertical video

### Subtitles and TTS
- Auto-subtitles from transcription (word-by-word or karaoke mode)
- Persian and Arabic language support
- RTL text direction for right-to-left languages
- Text-to-Speech with Kokoro-82M (54 voices, 8 languages)

### Timeline Editing
- Drag-and-drop timeline with magnetic snapping
- Multi-track compositing
- Keyframe animation on all properties
- 11 transition types (crossfade, wipe, slide, dissolve, etc.)
- Speed control (0.25x to 4x) with pitch preservation

### Export
- MP4, WebM, GIF formats
- Custom resolution and quality settings
- Batch export with piece ranges

---

## Quick Start

### Requirements
- FFmpeg 6 or later on PATH
- Rust (stable) + Node 20 or later

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

OpenInCut runs an MCP server at `http://127.0.0.1:4599/mcp` on startup. Any AI agent that speaks MCP can edit your project:

```bash
claude mcp add --transport http opencut http://127.0.0.1:4599/mcp \
  --header "Authorization: Bearer YOUR_TOKEN"
```

53 tools available — see [docs/MCP.md](docs/MCP.md).

---

## Language Support

| Feature | Supported Languages |
|---------|-------------------|
| Whisper Transcription | English, Persian, Arabic, Spanish, Portuguese, French, German |
| TTS Voice | English (US/GB), Spanish, French, Italian, Japanese, Hindi, Portuguese, Mandarin |
| Text Direction | LTR (English, etc.) and RTL (Persian, Arabic, Hebrew) |
| UI | English (localization-ready) |

---

## Architecture

Nine crates (pure model, media, audio, render, text, export, AI, whisper, Tauri shell), one compositor shared by preview and export, microseconds everywhere.

docs/ARCHITECTURE.md

---

## Credits

OpenInCut is based on UberEditor by **HectorPulido**. We are grateful for the original work on the MCP server architecture, timeline engine, and AI integration framework.

**License**: Apache-2.0 (same as original). Copyright 2026 PeDitXOS and contributors.

---

## Contributing

Contributions welcome! See CONTRIBUTING.md for guidelines.

## License

Apache-2.0 — see LICENSE for details.
