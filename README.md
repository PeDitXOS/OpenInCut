# UberEditor

Editor de video de escritorio multiplataforma (Tauri 2 + Rust + React) con superpoderes de IA: edición basada en texto (Whisper word-level), eliminación de silencios, generación automática de verticales, avatar reactivo por emociones y servidor MCP embebido.

**El plan maestro completo está en [PLAN.md](PLAN.md)** — arquitectura, las 16 features al detalle, mapeo del Youtubers-toolkit, roadmap de 7 fases.

## Estado actual

| Fase | Estado |
|---|---|
| 0 — Fundaciones (`ue-core`) | ✅ modelo, acciones reversibles, historial transaccional, keyframes, proptest |
| 1 — MVP editable | ✅ importar (ffprobe+hash), timeline completo (split/trim por bordes/move/ripple), **reproducción con audio real** (cpal como reloj maestro + stream de video MJPEG 24fps), **export MP4** con progreso y cancelación, guardar/abrir `.uep`, títulos quemados (drawtext) |
| 2 — Efectos y shaders | 🔨 sistema de **packs modulares** (manifest → misma cadena en preview y export), chroma key/color/blur core, **packs de usuario en disco** con recarga, transform real (crop/escala/rotación/flips), **transiciones crossfade** con handles. Falta: keyframes en UI, gizmo, motor wgpu |
| 3 — IA (Whisper, silencios) | pendiente |
| 4 — Servidor MCP | ✅ v0: `http://127.0.0.1:4599/mcp` con 10 herramientas (leer estado + editar con undo) |
| 5–6 | pendientes (ver PLAN.md §14) |

## Desarrollo

Requisitos: Rust estable, Node ≥ 20, FFmpeg en PATH (fases futuras).

```bash
# Tests del núcleo (modelo, acciones, undo/redo, ops)
cargo test

# Frontend en navegador (motor mock, ideal para iterar UI)
npm install
npm run dev            # http://localhost:5175
npm run typecheck

# App de escritorio (backend real ue-core)
npx tauri dev

# Pruebas visuales globales: screenshots automatizadas + verificación
# (requiere `npm run dev` corriendo)
node scripts/screenshot.mjs   # → screenshots/<fecha>/*.png
```

## Pruebas visuales

`scripts/screenshot.mjs` recorre un guion de interacciones (seleccionar, dividir en el playhead, deshacer, zoom, reproducir) y guarda capturas numeradas en `screenshots/<fecha>/`. La prueba de reproducción falla si el timecode no avanza. Las capturas se revisan comparándolas contra el comportamiento esperado de cada paso.

## Estructura

```
crates/ue-core     # núcleo puro: modelo, acciones, historial, keyframes, validación
src-tauri          # backend Tauri: comandos IPC sobre ProjectStore
src                # frontend React: timeline canvas, preview, inspector (mock engine)
scripts            # screenshot.mjs (pruebas visuales), debug-preview.mjs
effects            # (Fase 2) packs de efectos: manifest.json + shader
screenshots        # salida de las pruebas visuales, por fecha
```
