# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.0.0] - 2025-07-27

### Added
- **OpenInCut** stable release - Professional video editor for OpenInCut ecosystem
- Complete video editing timeline with multi-track support
- Media import and management with thumbnail generation
- Real-time preview with hardware-accelerated playback
- Timeline editing: cut, trim, split, ripple delete, ripple trim
- Multi-track video and audio support
- Keyframe animation for transform, opacity, and effects
- Keyframe interpolation: linear, ease-in, ease-out, ease-in-out, hold
- Keyframe easing curves and velocity curves
- Keyframe color coding by property type
- Keyframe navigation and keyframe-only preview mode
- Audio waveforms and audio level meters
- Audio keyframing for volume and panning
- Real-time audio monitoring
- Video effects: brightness, contrast, saturation, hue, blur, sharpen
- Color correction and grading tools (lift, gamma, gain, offset)
- LUT support (.cube, .3dl formats)
- Text overlays with rich text formatting
- Image overlays with transform controls
- Transitions: cross dissolve, dip to color, push, slide, wipe
- Export presets: YouTube, TikTok, Instagram, Twitter/X, Vimeo, custom
- Hardware-accelerated export (NVENC, VAAPI, VideoToolbox, QSV)
- Multi-pass encoding support
- Custom export presets with CRF/bitrate control
- Project save/load with .opencut project format
- Auto-save and project recovery
- Project templates
- Keyboard shortcuts with customizable keybindings
- Undo/redo with history panel
- Ripple edit mode with ripple toggle
- Snapping: playhead, markers, clips, grid
- Markers with colors and notes
- Timecode display (drop-frame, non-drop-frame, frames)
- Proxy media generation and proxy workflow
- Proxy toggle with original media relink
- Media offline detection and relink
- Audio scrubbing and audio-only preview
- Loop playback and loop region
- Full-screen preview mode
- External monitor preview output
- Project settings: resolution, frame rate, audio sample rate, color space
- Color space support: sRGB, Rec.709, Rec.2020, P3-D65, ACES
- OCIO color management support
- GPU acceleration: Vulkan, Metal, CUDA, DirectX 12
- Multi-GPU support
- Background rendering and background export
- Render queue with priority and batch export
- Export progress with time estimation
- Failed export retry and resume
- Project packaging for collaboration
- Project export/import with media consolidation
- Localization: English, Arabic, Persian, Russian, Chinese (Simplified)
- RTL language support (Arabic, Persian)
- Dark/light theme with system preference detection
- High DPI support
- Touch and pen input support
- Accessibility: screen reader support, keyboard navigation, high contrast

### Changed
- Project renamed from "OpenCut" to "OpenInCut"
- Application identifier changed to `net.pequesoft.opencut`
- Major architecture refactor with modular crate architecture
- Migrated to Tauri v2 with Rust 2021 edition
- Updated to React 18, TypeScript 5.6, Vite 6, Tailwind CSS 4
- Tauri v2 migration with new plugin system
- Tauri plugins: dialog, fs, shell, opener, clipboard-manager
- Migrated from Tauri v1 plugins to v2 plugin architecture
- Updated all Rust crates to 2021 edition
- Migrated to Tauri v2 RPC system
- Upgraded Tauri plugins to v2 APIs
- Migrated from `tauri-plugin-dialog` v1 to v2
- Migrated from `tauri-plugin-fs` v1 to v2
- Migrated from `tauri-plugin-shell` v1 to v2
- Updated all workspace dependencies to latest compatible versions
- Rust edition 2021 across all crates
- Cargo workspace version unified to 1.0.0
- Tauri config updated to schema v2

### Fixed
- Fixed audio sync drift during long exports
- Fixed keyframe interpolation edge cases
- Fixed proxy media relink on project reload
- Fixed audio waveform rendering artifacts
- Fixed keyframe navigation edge cases
- Fixed export preset validation
- Fixed project recovery on crash
- Fixed proxy media cleanup on project close
- Fixed audio monitoring latency
- Fixed keyframe color coding consistency
- Fixed export preset validation edge cases
- Fixed project packaging media consolidation
- Fixed RTL text rendering in timeline
- Fixed high DPI timeline ruler rendering
- Fixed proxy media generation progress reporting
- Fixed export queue persistence across restarts
- Fixed marker serialization in project files
- Fixed timecode display at high frame rates
- Fixed color space conversion precision
- Fixed GPU memory leak in long rendering sessions

### Removed
- Removed Tauri v1 legacy plugin APIs
- Removed legacy project format v1 (v2 is current)
- Removed legacy export preset format v1
- Removed deprecated Tauri v1 RPC calls
- Removed deprecated Tauri v1 plugin APIs
- Removed legacy Tauri v1 configuration format
- Removed deprecated Tauri v1 window APIs
- Removed legacy Tauri v1 menu APIs
- Removed deprecated Tauri v1 tray APIs
- Removed legacy Tauri v1 CLI commands
- Removed Tauri v1 `tauri.conf.json` v1 schema
- Removed legacy project migration code (v1 to v2)
- Removed deprecated Tauri v1 event system
- Removed legacy Tauri v1 window state APIs

### Security
- Updated all dependencies to latest secure versions
- Tauri v2 with improved security model
- CSP (Content Security Policy) hardened
- Tauri capability-based permission system
- Removed dangerous Tauri v1 allowlist configurations
- Updated all Rust dependencies to latest secure versions
- Updated all npm dependencies to latest secure versions
- Enabled Tauri v2 capability-based permissions
- Removed broad filesystem access permissions
- Scoped filesystem access to project directories only
- Scoped shell command execution to approved commands only

## [0.2.1] - 2025-06-15

### Added
- Initial OpenCut beta release
- Basic timeline editing
- Basic media import
- Basic export functionality

### Fixed
- Various beta stability fixes

## [0.2.0] - 2025-05-01

### Added
- Initial OpenCut alpha release
- Basic video editing timeline
- Basic media import/export