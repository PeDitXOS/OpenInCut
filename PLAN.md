# UberEditor — Plan Maestro de Diseño e Implementación

> **Editor de video de escritorio, multiplataforma, con superpoderes de IA.**
> Documento de planificación. Versión 1.0 — 2026-07-09.
> Autor: Héctor Pulido + Claude. Estado: **borrador para revisión, no hay código todavía.**

---

## Índice

- [0. Resumen ejecutivo](#0-resumen-ejecutivo)
- [1. Visión, alcance y no-objetivos](#1-visión-alcance-y-no-objetivos)
- [2. Decisiones de stack tecnológico](#2-decisiones-de-stack-tecnológico)
- [3. Arquitectura general](#3-arquitectura-general)
- [4. Modelo de datos del proyecto](#4-modelo-de-datos-del-proyecto)
- [5. Motor de render y reproducción (el corazón)](#5-motor-de-render-y-reproducción)
- [6. Features básicas (diseño detallado, 1–11)](#6-features-básicas)
- [7. Features avanzadas (diseño detallado)](#7-features-avanzadas)
- [8. Reutilización del Youtubers-toolkit](#8-reutilización-del-youtubers-toolkit)
- [9. API IPC (frontend ↔ engine)](#9-api-ipc-frontend--engine)
- [10. Estructura del repositorio](#10-estructura-del-repositorio)
- [11. Presupuestos de rendimiento](#11-presupuestos-de-rendimiento)
- [12. Estrategia de testing y CI](#12-estrategia-de-testing-y-ci)
- [13. Empaquetado, distribución y licencias](#13-empaquetado-distribución-y-licencias)
- [14. Roadmap por fases](#14-roadmap-por-fases)
- [15. Riesgos y mitigaciones](#15-riesgos-y-mitigaciones)
- [16. Backlog futuro (post-v1)](#16-backlog-futuro)
- [Apéndice A. Ejemplo completo de archivo de proyecto](#apéndice-a-ejemplo-completo-de-archivo-de-proyecto)
- [Apéndice B. Ejemplo de efecto modular (manifest + WGSL)](#apéndice-b-ejemplo-de-efecto-modular)
- [Apéndice C. Comandos FFmpeg de referencia](#apéndice-c-comandos-ffmpeg-de-referencia)
- [Apéndice D. Formato de transcripción word-level](#apéndice-d-formato-de-transcripción-word-level)
- [Apéndice E. Catálogo inicial de herramientas MCP](#apéndice-e-catálogo-inicial-de-herramientas-mcp)

---

## 0. Resumen ejecutivo

**UberEditor** es un editor de video no-lineal (NLE) de escritorio para macOS, Windows y Linux, construido sobre **Tauri 2 + Rust + React/TypeScript**, con un motor de composición GPU propio (**wgpu/WGSL**) y **FFmpeg** como columna vertebral de decodificación/codificación.

Se diferencia de un NLE clásico en cuatro superpoderes:

1. **Edición basada en texto**: todo video importado se transcribe con Whisper palabra por palabra; borrar una palabra en el panel de transcripción corta el video (estilo Descript).
2. **Automatizaciones de creador**: eliminación/aceleración de silencios, generación automática de versiones verticales (Shorts/Reels), subtítulos automáticos.
3. **Avatar reactivo**: un avatar personalizable (clips por emoción) que "habla" al ritmo del audio, con emociones clasificadas por LLM y vibración proporcional al volumen.
4. **Servidor MCP embebido**: al arrancar, la app expone un servidor MCP local para que agentes (Claude Code, Claude Desktop, etc.) lean el estado completo del proyecto y, opcionalmente, ejecuten ediciones.

Gran parte de la lógica de IA ya existe y está probada en `/Users/hectorpulido/Videos Reel/Youtubers-toolkit` (Python: faster-whisper, recorte por silencios, generador de shorts, avatar con emociones). La sección 8 detalla el mapeo módulo a módulo y la estrategia de portado (nativo en Rust como objetivo, sidecar Python como puente si hace falta).

El roadmap propone **7 fases** (0–6). El MVP editable (importar → cortar → previsualizar → exportar) llega al final de la Fase 1; la paridad con el toolkit llega en la Fase 5.

---

## 1. Visión, alcance y no-objetivos

### 1.1 Visión

Un editor que un YouTuber/creador usa de principio a fin para su flujo real:

```
Grabar → Importar → Limpiar (silencios, muletillas) → Editar (texto + timeline)
      → Decorar (subtítulos, avatar, títulos, efectos) → Exportar (horizontal + vertical)
```

Y que además es **operable por agentes de IA** vía MCP: "Claude, elimina los silencios de este proyecto, genera la versión vertical y expórtala para Shorts".

### 1.2 Alcance v1 (lo que SÍ entra)

**Básicas (requisito):**

| # | Feature | Resumen |
|---|---------|---------|
| 1 | Línea de tiempo | Pistas múltiples de video y audio, zoom, snapping, drag & drop |
| 2 | Recorte y división | Cuchilla, split en playhead, trim de bordes, ripple delete |
| 3 | Importación multi-formato | Video, audio e imágenes vía FFmpeg; proxies y conformado |
| 4 | Vista previa en tiempo real | 720p30 estable como mínimo, escalado adaptativo |
| 5 | Transiciones y efectos modulares | Sistema de shaders (WGSL/GLSL) con manifest JSON, hot-reload |
| 6 | Texto y títulos | Clips de texto con estilos, plantillas y animación |
| 7 | Control de audio | Ganancia por clip, fades, volumen/mute/solo por pista, medidores |
| 8 | Ajustes de imagen | Brillo/contraste/saturación (+ más) como shaders; rotar, recortar encuadre, escala/posición |
| 9 | Exportación configurable | Presets + control fino de códec/bitrate/resolución/rango |
| 10 | Undo/redo y proyecto | Historial ilimitado-práctico, autosave, archivo de proyecto versionado |
| 11 | Keyframes básicos | Cualquier parámetro numérico animable; interpolación lineal/hold/ease |

**Avanzadas (requisito):**

| # | Feature | Resumen |
|---|---------|---------|
| A | Servidor MCP | Estado completo del proyecto expuesto a agentes; herramientas de edición opcionales |
| B | Whisper palabra-por-palabra | Transcripción word-level de todo video; edición borrando/moviendo texto |
| C | Silencios | Detectar, eliminar (ripple) o procesar (acelerar) silencios |
| D | Vertical automático | Plantilla 9:16 con fondo desenfocado + subtítulos + títulos, wizard de 1 clic |
| E | Avatar + subtítulos automáticos | Avatar por emociones que vibra con el volumen; subtítulos word-level automáticos |

### 1.3 No-objetivos v1 (explícitamente fuera)

- Edición multicámara, motion tracking, estabilización.
- Corrección de color profesional (scopes, LUTs, HDR) — solo ajustes básicos.
- Plugins de terceros VST/OFX (sí efectos modulares propios en shader).
- Colaboración multiusuario / proyectos en la nube.
- Doblaje automático con TTS (existe en el toolkit con Kokoro; va al backlog, sección 16).
- Edición 360°/VR, timelines anidados (compound clips) — backlog.

### 1.4 Principios de diseño

1. **Edición no destructiva siempre**: los archivos fuente jamás se modifican; todo es metadatos + render.
2. **El estado vive en Rust**: el frontend es una vista; una única fuente de verdad en el engine evita desincronizaciones y hace trivial el MCP y el undo/redo.
3. **Preview == Export**: el mismo grafo de render produce la vista previa y la exportación (a distinta resolución), incluyendo efectos aleatorios (ruido con semilla determinista). Lo que ves es lo que sale.
4. **Todo trabajo pesado es un job cancelable con progreso**: import, proxy, waveform, whisper, export.
5. **Los formatos de configuración del toolkit se respetan** donde sea razonable (config de avatar, estilos de subtítulos) para migración suave.
6. **Modularidad radical**: casi toda feature visual es *datos + shader* (efectos, transiciones, ajustes de imagen, chroma key, incluso el shake del avatar) descubiertos en tiempo de ejecución desde carpetas de "packs"; casi toda feature de edición es una `Action` registrada en un único registro que alimenta a la vez la UI, el undo/redo y el MCP. Añadir una feature nueva = añadir un archivo, no tocar el núcleo (sección 6.5.1).
7. **Chroma key de primera clase**: es un efecto core con supresión de spill, pensado tanto para material grabado con pantalla verde como para los avatares (sección 6.5.4).

---

## 2. Decisiones de stack tecnológico

### 2.1 Tabla de decisiones

| Área | Elección | Alternativas evaluadas | Justificación |
|------|----------|------------------------|---------------|
| Shell de app | **Tauri 2.x** | Electron, Qt, egui/iced nativo | Pedido explícito; binarios pequeños; el backend Rust ES el motor de video (en Electron habría que escribir un addon nativo igualmente). |
| Frontend | **React 18 + TypeScript + Vite** | Svelte 5, SolidJS | Ecosistema maduro para UIs complejas (DnD, virtualización); tipado compartido con el engine vía codegen. Svelte es viable si se prefiere; la arquitectura no depende del framework. |
| Estado UI | **Zustand** + eventos Tauri | Redux, Jotai | Store minimalista que refleja (mirror) el estado del engine; las mutaciones NO viven aquí (viven en Rust). |
| Estilos | **Tailwind CSS** + tokens propios | CSS modules | Velocidad para una UI densa tipo NLE con tema oscuro. |
| Motor gráfico | **wgpu (WGSL)** | OpenGL crudo, Skia, CPU (como MoviePy) | Multiplataforma real (Metal/Vulkan/DX12), shaders modernos, compute disponible. MoviePy (CPU) es justo el cuello de botella que sufre el toolkit. |
| Demux/decode | **FFmpeg sidecar (CLI)** en Fase 1 → opción `ffmpeg-next` (libav) en Fase 2+ | GStreamer, WebCodecs en el webview | Sidecar: cero problemas de linking, aislamiento de crashes, misma herramienta para probe/proxy/export. WebCodecs se descarta como base por soporte irregular en WebKitGTK (Linux). |
| Encode/export | **FFmpeg sidecar** (pipe rawvideo + wav) | libav embebido | Control total de códecs/containers, robustez probada. |
| Audio playback | **cpal** (callback de audio) + mixer propio | rodio, SDL | Necesitamos mezcla propia con keyframes y reloj maestro de audio; rodio es demasiado alto nivel. |
| Decode de audio | **Conformado a WAV PCM al importar** (vía ffmpeg) + lectura mmap | symphonia en tiempo real | Conformar de entrada (como hacen los NLE pro) simplifica seeks, waveforms y mezcla; el costo de disco es aceptable. |
| Texto/tipografía | **cosmic-text** (shaping + fallback) rasterizado a textura | glyphon, resvg | Soporte de emoji, RTL y fuentes del sistema; rasterizamos a atlas y componemos en GPU. |
| Transcripción | **whisper-rs** (bindings whisper.cpp, con Metal/CUDA) | Sidecar Python faster-whisper (como el toolkit) | Sin dependencia de Python en producción. faster-whisper queda como **plan B puente** (sección 8.3). |
| Clasif. de emociones | **API OpenAI-compatible configurable** (OpenAI, Ollama local, etc.) | Modelo local ONNX | Es exactamente lo que hace el toolkit (prompt probado); "OpenAI-compatible" permite privacidad total con Ollama. |
| Denoise | **nnnoiseless / RNNoise** (Rust/C, tiempo real) | DNS64+torch (toolkit) | DNS64 arrastra PyTorch (~2GB); RNNoise es liviano y suficiente para voz. DNS64 posible vía sidecar en backlog. |
| MCP | **rmcp** (SDK oficial de Rust) con transporte streamable-HTTP en localhost | Implementación manual, sidecar Node | SDK oficial, tokio-native, mismo proceso que el estado del proyecto. |
| Persistencia | **JSON (serde)** con `schema_version`, rutas relativas | SQLite, binario | Diffable en git, inspeccionable, trivial de exponer por MCP. SQLite solo para cachés/índices internos. |
| IDs | **ULID** | UUID v4 | Ordenables por tiempo (útil en logs/históricos), legibles. |
| Tiempo | **`i64` microsegundos** (`TimeUs`) + fps racional `(num, den)` | floats, frames | Los floats acumulan error (el toolkit sufre esto en SRT); microsegundos enteros son exactos y convertibles a cualquier fps. |

### 2.2 Versiones objetivo

- Rust stable ≥ 1.85, edition 2024.
- Tauri 2.x + plugins oficiales: `dialog`, `fs`, `shell` (sidecars), `store` (preferencias), `log`, `single-instance`, `window-state`, `updater`.
- FFmpeg 7.x sidecar (builds estáticos por plataforma, ver sección 13).
- Node 22 + pnpm para el frontend.
- whisper.cpp ≥ 1.7 vía whisper-rs (build con Metal en macOS; CPU AVX2 en Windows/Linux; CUDA opcional).

### 2.3 Plataformas soportadas

| OS | Mínimo | Webview | GPU backend (wgpu) |
|----|--------|---------|--------------------|
| macOS | 12+ (arm64 + x86_64) | WKWebView | Metal |
| Windows | 10 20H2+ (x86_64) | WebView2 (Chromium) | DX12 (fallback Vulkan) |
| Linux | Ubuntu 22.04+ (x86_64) | WebKitGTK | Vulkan (fallback GL) |

---

## 3. Arquitectura general

### 3.1 Diagrama de procesos y threads

```
┌──────────────────────────────── Proceso principal (Tauri / Rust) ────────────────────────────────┐
│                                                                                                   │
│  ┌───────────── WebView (React) ─────────────┐      ┌────────────── Engine (Rust) ─────────────┐  │
│  │  Timeline UI (canvas)                     │      │  ProjectStore  (estado único + historial) │  │
│  │  Preview (canvas WebGL / superficie)      │◄────►│  PlaybackController (reloj de audio)      │  │
│  │  Media Pool, Inspector, Transcript Panel  │ IPC  │  RenderGraph (wgpu, WGSL, cache texturas) │  │
│  │  Export dialog, Jobs panel                │      │  DecodePool (sesiones ffmpeg / frame LRU)  │  │
│  └───────────────────────────────────────────┘      │  AudioMixer (cpal, 48kHz, reloj maestro)   │  │
│         ▲  eventos state.patch / job.progress        │  JobRunner (import, proxy, whisper, export)│  │
│         │  frames vía canal binario / protocolo      │  McpServer (rmcp, HTTP 127.0.0.1:4599)     │  │
│         └────────────────────────────────────────────┴────────────────────────────────────────────┘  │
└───────────────────────────────────────────────────────────────────────────────────────────────────┘
        │                          │                              │
        ▼ sidecar                  ▼ sidecar                      ▼ (in-process, hilo tokio)
   ffmpeg / ffprobe        (opcional) toolkit-bridge         Cliente MCP externo
   (decode, proxy,          (Python congelado:               (Claude Code / Desktop
    waveform, export)        faster-whisper, kokoro)          se conecta por HTTP)
```

### 3.2 Reglas de flujo de datos

1. **Unidireccional**: la UI emite *intents* (`invoke("timeline.split_clip", …)`) → el engine valida, muta el `ProjectStore`, empuja al historial, y emite un evento `state.patch` (JSON Patch RFC 6902) → la UI aplica el patch a su mirror.
2. **El engine nunca confía en la UI**: todos los invariantes (no solapar clips en una pista, in < out, etc.) se validan en Rust. El MCP reutiliza las mismas acciones, gratis.
3. **Datos pesados fuera del IPC JSON**: frames de preview, waveforms y thumbnails viajan por canal binario (`tauri::ipc::Channel`) o por protocolo custom (`ueasset://`), nunca como JSON base64.
4. **Jobs**: toda operación > 100 ms es un `Job { id, kind, progress, cancel_token }`; la UI tiene un panel de jobs global; MCP puede consultarlos.

### 3.3 Módulos del engine (crates internos)

```
crates/
├── ue-core        # Modelo de datos, acciones, historial, validación, (de)serialización
├── ue-media       # ffprobe, import, proxies, conformado de audio, thumbnails, peaks
├── ue-render      # wgpu: grafo de composición, efectos, transiciones, texto, transform
├── ue-audio       # cpal, mixer, fades/keyframes de volumen, medidores, reloj maestro
├── ue-playback    # Orquestación: decode-ahead, cache, sync A/V, scrub
├── ue-export      # Cola de exportación, pipe a ffmpeg, presets
├── ue-ai          # whisper-rs, silencios, edición por texto, vertical, avatar, emociones
├── ue-mcp         # Servidor MCP (rmcp), mapeo de tools → acciones de ue-core
└── ue-tauri (src-tauri) # Comandos IPC, eventos, wiring, sidecars
```

Dependencias permitidas (flechas = "puede usar"): `ue-tauri → todos`; `ue-mcp → ue-core, ue-ai, ue-export`; `ue-playback → ue-media, ue-render, ue-audio, ue-core`; `ue-ai → ue-media, ue-core`. `ue-core` no depende de nadie (puro).

---

## 4. Modelo de datos del proyecto

### 4.1 Entidades (Rust, simplificado)

```rust
type TimeUs = i64;           // microsegundos; 1 s = 1_000_000
type Id = Ulid;

struct Project {
    schema_version: u32,             // migraciones explícitas
    id: Id,
    name: String,
    created_at: String,              // ISO-8601
    settings: ProjectSettings,       // carpeta de caché, idioma whisper por defecto, etc.
    assets: Vec<MediaAsset>,         // "media pool"
    sequences: Vec<Sequence>,        // v1: normalmente 1, pero el modelo admite varias
    active_sequence: Id,
}

struct MediaAsset {
    id: Id,
    kind: MediaKind,                 // Video | Audio | Image
    path: RelPath,                   // relativa al proyecto; relink si no existe
    content_hash: String,            // xxh3 de los primeros/últimos 4MB + tamaño (rápido)
    probe: ProbeInfo,                // duración, streams, códecs, fps, resolución, rotación
    proxy: Option<RelPath>,          // 720p h264 GOP corto (cache)
    audio_conform: Option<RelPath>,  // wav pcm_s16le 48k estéreo (cache)
    peaks: Option<RelPath>,          // picos de waveform binarios (cache)
    thumbnails: Option<RelPath>,     // sprite de miniaturas (cache)
    transcript: Option<Id>,          // → TranscriptDoc
}

struct Sequence {
    id: Id,
    name: String,
    resolution: (u32, u32),          // p.ej. (1920, 1080) o (1080, 1920)
    fps: (u32, u32),                 // racional, p.ej. (30000, 1001)
    sample_rate: u32,                // 48000
    tracks: Vec<Track>,              // orden = orden de composición (índice 0 = abajo)
    markers: Vec<Marker>,
}

struct Track {
    id: Id,
    kind: TrackKind,                 // Video | Audio
    name: String,
    muted: bool, solo: bool, locked: bool,
    volume_db: f32,                  // solo audio; keyframable
    clips: Vec<Clip>,                // SIEMPRE ordenados por start, sin solaparse
}

struct Clip {
    id: Id,
    payload: ClipPayload,
    start: TimeUs,                   // posición en el timeline
    duration: TimeUs,
    speed: f64,                      // 1.0 normal; >1 acelera (silencios procesados)
    effects: Vec<EffectInstance>,    // cadena ordenada de shaders
    transform: Transform2D,          // pos, escala, rotación, crop — keyframable
    audio: AudioProps,               // gain_db, pan, fade_in/out — keyframable
    transition_in: Option<TransitionRef>,   // compartida con el clip anterior
    label_color: Option<String>,
}

enum ClipPayload {
    Media { asset_id: Id, src_in: TimeUs, src_out: TimeUs },  // rango del archivo fuente
    Text  { content: RichText, style: TextStyle },            // títulos y subtítulos manuales
    Subtitles { transcript_id: Id, style: SubtitleStyle, mode: SubtitleMode }, // auto, word-level
    Avatar { config: AvatarConfig, driver_asset: Id },        // sección 7.E
    Solid { color: [f32; 4] },                                // fondos
}

struct EffectInstance {
    effect_id: String,               // "core.brightness_contrast", "user.vhs"
    enabled: bool,
    params: BTreeMap<String, ParamValue>,   // valor fijo o curva de keyframes
}

enum ParamValue { Const(f64), Color([f32;4]), Bool(bool), Curve(KeyframeCurve), Text(String) }

struct KeyframeCurve {
    keys: Vec<Keyframe>,             // ordenadas por t
}
struct Keyframe {
    t: TimeUs,                       // relativo al inicio del CLIP (sobrevive a moverlo)
    value: f64,
    interp: Interp,                  // Hold | Linear | Bezier { in_tangent, out_tangent }
}

struct TranscriptDoc {               // sección 7.B y Apéndice D
    id: Id,
    asset_id: Id,
    language: String,
    model: String,                   // "large-v3-turbo"
    words: Vec<Word>,                // { text, start, end, confidence } en tiempo del ASSET
    segments: Vec<Segment>,          // frases (para emociones de avatar y SRT clásico)
}
```

### 4.2 Invariantes (validados en `ue-core`, con tests)

1. Clips de una pista ordenados por `start` y sin solaparse (`clip[i].start + duration <= clip[i+1].start`).
2. `0 <= src_in < src_out <= asset.duration` para payloads Media.
3. Keyframes con `t` estrictamente creciente; toda curva tiene ≥ 1 key.
4. Toda `TransitionRef` referencia dos clips adyacentes en la misma pista y su duración ≤ material disponible (handles) a ambos lados.
5. Los IDs son únicos en todo el proyecto (mapa global de índices para lookup O(1)).

### 4.3 Archivo de proyecto

- Extensión: **`.uep`** (UberEditor Project). Contenido: JSON pretty-printed (diffable).
- Rutas de media **relativas** al archivo `.uep`; al abrir, verificación por `content_hash` → diálogo de **relink** con búsqueda por hash en carpetas que indique el usuario.
- Cachés (proxies, wavs, peaks, transcripts) van a `<app_data>/cache/<content_hash>/…`, NUNCA dentro del proyecto → un `.uep` es pequeño y portable; borrar caché nunca pierde trabajo.
- **Autosave**: cada 60 s (configurable) a `<proyecto>.uep.autosave`; al abrir tras crash se ofrece recuperar. Además, snapshot en cada export.
- Ver ejemplo completo en Apéndice A.

---

## 5. Motor de render y reproducción

Esta sección es la más crítica técnicamente: aquí se decide si la vista previa es fluida.

### 5.1 Grafo de evaluación de un frame

Para producir el frame en el tiempo `t` de la secuencia:

```
1. Para cada pista de VIDEO (de abajo hacia arriba):
   a. Buscar el clip activo en t (búsqueda binaria por start).
   b. Resolver el tiempo fuente: src_t = src_in + (t - clip.start) * speed.
   c. Obtener la textura fuente:
      - Media  → DecodePool.get_frame(asset, src_t)         (5.2)
      - Text   → TextRasterizer.texture(content, style, t)   (6.6)
      - Subtitles → SubtitleRenderer.texture(transcript, t)  (7.E)
      - Avatar → AvatarRenderer.texture(config, t)           (7.E)
      - Solid  → textura 1x1 escalada
   d. Aplicar cadena de efectos del clip (ping-pong entre 2 texturas offscreen,
      un draw call por efecto, uniforms evaluados con keyframes en t).       (6.5, 6.8)
   e. Si hay transición activa con el clip vecino: renderizar también el otro
      clip (pasos b–d) y ejecutar el shader de transición(A, B, progress).
   f. Aplicar Transform2D (crop → escala → rotación → posición) y componer
      sobre el framebuffer acumulado (blend premultiplied alpha).
2. El framebuffer final (formato RGBA8 sRGB en v1) es el frame de la secuencia.
```

Notas de implementación:

- **YUV→RGB en GPU**: ffmpeg entrega `yuv420p`; subimos los planos Y/U/V como 3 texturas R8 y convertimos en el primer shader. Ahorra ~40% de CPU y de ancho de banda de pipe frente a pedir `rgba`.
- **Determinismo**: efectos con aleatoriedad (shake del avatar, grain) usan ruido *hash* con semilla `(clip_id, frame_index)` — así preview y export son idénticos y los tests de golden-frame son estables. (Mejora directa sobre `np.random` del toolkit, que era irrepetible.)
- **Espacio de color v1**: sRGB 8-bit de punta a punta. HDR/lineal 16F queda anotado como evolución (el diseño de ping-pong lo permite cambiando el formato de textura).

### 5.2 DecodePool: obtención de frames fuente

- Una **sesión de decode** por asset activo: proceso `ffmpeg -ss <t> -i <proxy|original> -f rawvideo -pix_fmt yuv420p pipe:1` leyendo frames secuenciales por stdout.
- **Reproducción**: la sesión avanza linealmente (lectura secuencial = barata). Prefetch de N frames por delante del playhead en un ring buffer.
- **Seek**: matar sesión + relanzar con `-ss` (seek por keyframe + decode hasta el frame exacto). Con proxies de GOP corto (keyint 15) el peor caso es decodificar 14 frames ≈ decenas de ms.
- **Scrub** (arrastrar el playhead): política *latest-wins* — se cancela el seek anterior si llega otro; mientras tanto se muestra el frame en caché más cercano.
- **FrameCache LRU** global con presupuesto de RAM configurable (por defecto 2 GB): clave `(asset_id, quality, frame_idx)`. Los frames alrededor del playhead y de los bordes de clips (donde se corta a menudo) tienen prioridad.
- **Evolución Fase 2+**: reemplazar sesiones CLI por `ffmpeg-next` (libav in-process) para scrub frame-accurate más fino y hardware decode (VideoToolbox/D3D11VA/VAAPI). La interfaz `trait FrameSource` se define desde el día 1 para que el cambio sea interno.

### 5.3 Proxies y conformado (al importar)

Al importar un archivo se lanzan jobs en segundo plano (el clip es usable inmediatamente, con calidad degradada hasta que terminen):

| Job | Comando (ver Apéndice C) | Salida |
|-----|--------------------------|--------|
| Probe | `ffprobe -print_format json` | `ProbeInfo` (streams, duración, fps, rotación) |
| Proxy video | h264 720p, `-g 15`, CRF 20, audio copy | `<hash>/proxy.mp4` |
| Conformado audio | `pcm_s16le`, 48 kHz, estéreo | `<hash>/audio.wav` |
| Peaks | lectura del wav → min/max por ventana de 256 samples | `<hash>/peaks.bin` |
| Thumbnails | 1 frame cada N segundos, sprite 160px | `<hash>/thumbs.jpg` |
| Whisper (opt-in/auto) | sección 7.B | `<hash>/transcript.json` |

- Imágenes: se cargan con `image` crate directamente a textura (con downscale si > 8K). Un clip de imagen tiene duración libre (por defecto 5 s).
- El toggle **"calidad de preview"** (Auto / ½ / ¼ / Full) decide si el DecodePool lee el proxy o el original.

### 5.4 Audio: mixer y reloj maestro

- **cpal** abre un stream de salida a 48 kHz; el *callback* de audio pide `n` samples → el `AudioMixer` los produce leyendo los WAV conformados (mmap) de todos los clips audibles en la posición actual.
- Cadena por clip: `sample → gain(keyframes) → fades(in/out) → pan` → suma por pista (`volume_db`, mute/solo) → master → *soft clip limiter*.
- **El audio es el reloj maestro**: la posición de reproducción se deriva de los samples efectivamente consumidos por el dispositivo (`samples_played / 48000`). El video se sincroniza a ese reloj: si va tarde, saltar frames; si va adelantado, esperar. Es el esquema estándar que elimina el drift A/V.
- Velocidad de clip ≠ 1.0: resampleo con `rubato` (sinc) por segmento; para "silencios acelerados" (7.C) se usa time-stretch simple (v1: resample con cambio de pitch aceptable a 2–4x en silencio; v2: WSOLA para preservar pitch).
- Medidores: RMS + pico por pista y master, publicados a la UI a 15 Hz por evento.

### 5.5 Entrega del preview a la UI

**Fase 1 (simple y multiplataforma) — objetivo 1280×720@30:**

1. Render wgpu a textura offscreen a resolución de preview.
2. Copy a buffer de staging → CPU (`map_async`).
3. Envío del RGBA crudo (~3.7 MB/frame) por `tauri::ipc::Channel<Vec<u8>>` binario → el frontend lo sube a una textura WebGL y la pinta en un canvas. Si el canal no sostiene el caudal en alguna plataforma, fallback automático a JPEG (turbojpeg, calidad 85, ~200 KB/frame).
4. Backpressure: si la UI no confirma el frame anterior, se salta el envío (el render interno continúa; el audio nunca se bloquea).

**Fase 2 (optimización) — objetivo 4K@60 y latencia mínima:**

- Superficie nativa hija (child window/`CAMetalLayer`/HWND/wayland subsurface) posicionada bajo el hueco del preview en el layout web; wgpu presenta directamente (zero-copy). La UI web dibuja solo los controles alrededor. Riesgo/complejidad documentados en sección 15; por eso es Fase 2 y no base.

### 5.6 Pipeline de exportación

```
RenderGraph (resolución/fps de la secuencia, sin cache de preview, calidad Full)
   │  frames RGBA (o yuv420p convertido en GPU-→CPU)
   ▼
ffmpeg -f rawvideo -pix_fmt rgba -s WxH -r FPS -i pipe:0 \
       -i mixdown.wav \
       [flags del preset]  out.mp4
```

1. El `AudioMixer` renderiza primero el **mixdown completo** a `mixdown.wav` (más rápido que tiempo real; sirve de barra de progreso temprana).
2. El grafo renderiza frame a frame (sin reloj, a máxima velocidad) y escribe al stdin de ffmpeg. Presión regulada por el propio pipe.
3. Progreso = frames escritos / totales; cancelación = matar ffmpeg + borrar parcial.
4. Cola de exportación: múltiples jobs en serie (paralelo en backlog).
5. Al terminar: verificación con ffprobe (duración esperada ± 1 frame) y notificación del SO.

---

## 6. Features básicas

Formato de cada subsección: **Objetivo → UX → Diseño técnico → Casos borde → Criterios de aceptación (CA)**.

### 6.1 Línea de tiempo (feature 1)

**Objetivo.** Pistas ilimitadas de video y audio, manipulación directa fluida a 60 fps de UI incluso con cientos de clips.

**UX.**
- Layout clásico: regla de tiempo arriba, pistas de video (arriba) y audio (abajo), playhead vertical, cabeceras de pista a la izquierda (nombre, mute/solo/lock, volumen).
- Zoom: rueda+Ctrl (centrado en el cursor), atajos `+`/`-`, "zoom to fit" (`Shift+Z`). Rango: de 10 min/pantalla a 5 frames/pantalla.
- Scroll horizontal (rueda / arrastrar con espacio) y vertical (pistas).
- **Snapping** (toggle `S`): imán a playhead, bordes de clips, marcadores y a 0; tolerancia 8 px en espacio de pantalla.
- Drag & drop: desde el Media Pool a una pista (crea clip), entre pistas, y horizontal con preview fantasma + indicador de colisión.
- Selección: click, marco elástico, `Shift` para múltiple; `Ctrl/Cmd+A` todo.
- Clips muestran: nombre, thumbnails (video), waveform (audio), badges de efectos/velocidad, y color de etiqueta.
- Marcadores de secuencia (`M`) con nombre y color.

**Diseño técnico.**
- El timeline se dibuja en **un solo `<canvas>` 2D** (no DOM por clip): render inmediato tipo juego con lista de visibles calculada por búsqueda binaria sobre `start`. Con virtualización de pistas es O(visible), no O(total).
- Coordenadas: `px = (t_us - view_start_us) * pxPerUs`; todos los hit-tests en espacio de tiempo, no de píxeles (estable ante zoom).
- Thumbnails/waveforms del canvas provienen de los sprites/peaks del caché (5.3) vía protocolo `ueasset://` (el webview los trae como imágenes normales, cacheables).
- Interacciones emiten intents al engine (`timeline.move_clip`, etc.). Durante un drag, la UI muestra el fantasma localmente y solo al soltar emite la acción (una única entrada de undo).
- Reordenar/insertar con **modo overwrite** (por defecto) y **modo insert/ripple** (con `Alt`): el engine implementa ambos como acciones distintas.

**Casos borde.** Drop sobre clip existente (overwrite parte el clip de abajo); drag más allá del inicio (clamp a 0); pistas bloqueadas rechazan acciones con toast; zoom extremo con clips < 1 px (se dibujan como línea, siguen seleccionables por rango).

**CA.**
1. 500 clips en 8 pistas → pan/zoom a 60 fps en un portátil medio.
2. Todas las mutaciones pasan por acciones del engine (verificable: replay del historial reproduce el estado).
3. Snapping funciona a cualquier nivel de zoom con tolerancia en píxeles.

### 6.2 Recorte y división de clips (feature 2)

**Objetivo.** Cortar, partir y ajustar clips de forma no destructiva con precisión de frame.

**UX.**
- **Split en playhead** (`Ctrl/Cmd+K` o botón cuchilla): parte todos los clips seleccionados (o el que está bajo el playhead) en dos.
- **Herramienta cuchilla** (`C`): click sobre cualquier clip lo parte en ese punto.
- **Trim de bordes**: arrastrar el borde izquierdo/derecho de un clip ajusta `src_in`/`src_out` (cursor cambia; tooltip muestra +/- frames y nuevo timecode). Limitado por el material disponible del asset (los "handles").
- **Ripple delete** (`Shift+Del`): borra el clip y cierra el hueco desplazando lo posterior. `Del` normal deja el hueco.
- **Ripple trim** (`Alt` + arrastrar borde): trim que desplaza el resto de la pista.
- Slip (arrastrar con `Y` el contenido sin mover el clip): mueve `src_in/src_out` juntos. (Slide queda en backlog.)

**Diseño técnico.**
- `split(clip, t)` = clonar clip; el izquierdo recibe `src_out' = src_in + (t - start) * speed`, el derecho `src_in' = src_out'`, `start' = t`. Los **keyframes** (curvas con `t` relativo al clip) se reparten: los del lado derecho se re-basan restando el offset; se insertan keys interpoladas en el punto de corte para preservar el valor exacto.
- Los efectos y transform se **copian** a ambas mitades (comportamiento estándar NLE). La transición existente queda en el lado que toca a su vecino.
- Precisión: `t` se cuantiza al frame de la secuencia (`round(t * fps_num / (fps_den * 1e6))`) antes de operar, para que los cortes caigan siempre en frontera de frame.
- Todo son acciones puras sobre `ue-core` con inversa explícita (sección 6.10).

**Casos borde.** Split exactamente en el borde (no-op); trim que dejaría duración 0 (mínimo 1 frame); split de clip con transición activa en ese punto (se rechaza con mensaje); split de clip de texto/subtítulos/avatar (soportado: parten su línea de tiempo interna).

**CA.**
1. Split + undo restaura byte a byte el estado (test de serialización).
2. Trim nunca excede el material fuente; el tooltip refleja frames exactos.
3. Ripple delete sobre selección multi-pista mantiene la sincronía relativa de las demás pistas (opción "ripple all tracks" on/off).

### 6.3 Importación multi-formato (feature 3)

**Objetivo.** Arrastrar cualquier archivo razonable y que funcione: video, audio o imagen.

**Formatos v1** (los que soporte el build de FFmpeg; lista de extensiones aceptadas en UI):
- Video: mp4, mov, mkv, webm, avi, m4v, mts/m2ts, mpg, flv, wmv (códecs: h264, hevc, vp8/9, av1, prores, dnxhd, mpeg2/4…)
- Audio: wav, mp3, aac/m4a, flac, ogg/opus, aiff, wma
- Imagen: png, jpg/jpeg, webp, bmp, tiff, gif (v1: primer frame; gif animado → tratado como video), svg (rasterizado con resvg a la resolución de secuencia), heic (macOS)

**UX.**
- Vías de entrada: botón Importar, `Ctrl/Cmd+I`, drag & drop de archivos/carpetas al Media Pool o directamente al timeline (importa + inserta).
- Media Pool: grid o lista con miniatura, nombre, duración, resolución/fps, badges de estado de jobs (proxy ✓, audio ✓, whisper ⏳), búsqueda y carpetas virtuales (bins).
- Archivos con rotación en metadata (`rotate=90` típico de móvil) se muestran ya corregidos.
- Media offline: clip rojo + diálogo de relink (busca por nombre y por `content_hash`).

**Diseño técnico.**
- Import = `ffprobe` síncrono rápido (< 1 s) para validar y poblar `ProbeInfo` + programación de jobs de caché (5.3). El asset queda usable al instante (decode del original mientras no haya proxy).
- Detección de VFR (frame rate variable, típico de OBS/pantalla): si `avg_frame_rate ≠ r_frame_rate`, el proxy se genera con `-vsync cfr -r <fps_secuencia>` y se marca el asset (los originales VFR rompen la precisión de seek; el proxy CFR lo arregla — lección conocida de editar material de screen recording).
- `content_hash` = xxh3(primeros 4 MB + últimos 4 MB + tamaño) — suficiente para relink y claves de caché sin leer archivos de 50 GB enteros.
- Carpetas: import recursivo con filtro de extensiones.

**Casos borde.** Archivos sin pista de audio (waveform vacía, mixer los ignora); audio multicanal 5.1 (downmix a estéreo en el conformado, nota en inspector); imágenes con EXIF orientation; archivos corruptos (probe falla → toast con stderr resumido); rutas con caracteres no-ASCII y espacios (¡como `Videos Reel`!) — siempre pasar rutas como args separados al sidecar, jamás interpolar en shell.

**CA.**
1. Los 3 tipos importan por las 3 vías de entrada.
2. Un mp4 de móvil grabado en vertical (rotate=90) se ve correcto en preview y export.
3. Proyecto movido de carpeta con media al lado → abre sin relink (rutas relativas).

### 6.4 Vista previa en tiempo real (feature 4)

El grueso técnico está en la sección 5. Aquí, el contrato de UX:

- Transporte: espacio = play/pausa, `J/K/L` (reverse/pausa/forward con velocidades ×1/×2/×4 — reverse v1 = saltos hacia atrás de 1 frame, reverse fluido en backlog), `←/→` frame a frame, `Home/End`, `I/O` para marcas de rango.
- Indicador de calidad (Auto/½/¼/Full) y de frames perdidos (contador de dropped frames en la esquina, visible solo si > 0).
- **Degradación elegante**: si el render no llega a tiempo, primero baja resolución de preview (Auto), luego saltar frames de video; el audio no se interrumpe jamás.
- Al pausar: re-render inmediato a calidad Full de ese frame (el usuario ve nítido al detenerse).
- Zona segura / guías de tercios (toggle), fondo a cuadros para alpha.

**CA.**
1. Secuencia 1080p con 2 pistas de video + 1 de texto + música: reproducción 30 fps sin drops en hardware de referencia (M1 / Ryzen 5 + GPU integrada).
2. Latencia de scrub (soltar el playhead → frame correcto en pantalla) < 150 ms con proxy.
3. Desincronización A/V < 40 ms sostenida en clips de 30 min (medible con video de beep+flash).

### 6.5 Transiciones y efectos modulares — sistema de shaders (feature 5)

Esta subsección define **el sistema de extensibilidad central** de UberEditor (aplica también a 6.8, al chroma key y al avatar).

#### 6.5.1 Arquitectura modular ("packs")

Principio: **añadir un efecto, transición o preset jamás toca el código del núcleo.** Un *pack* es una carpeta:

```
effects/
├── core/                      # incluidos en la app (read-only, embebidos en el binario)
│   ├── brightness_contrast/
│   │   ├── manifest.json
│   │   └── shader.wgsl
│   ├── chroma_key/ …
│   ├── gaussian_blur/ …
│   └── transitions/crossfade/ …
└── user/                      # <app_data>/effects — carpeta del usuario, hot-reload
    └── vhs_retro/
        ├── manifest.json
        └── shader.wgsl        # o shader.frag (GLSL) — naga lo ingiere igualmente
```

- **Descubrimiento en runtime**: al arrancar (y con un file-watcher en la carpeta `user/`) se escanean manifests, se compilan shaders (con validación naga y errores legibles en un panel), y aparecen en la UI automáticamente. Editar el `.wgsl` con la app abierta re-compila y refresca el preview en caliente (**hot-reload**) — ciclo de iteración de segundos para crear efectos nuevos.
- **Contrato único**: todo efecto es `fn effect(tex_in, uv, params…) -> color`; toda transición es `fn transition(tex_a, tex_b, uv, progress, params…) -> color`. El runtime genera el binding de uniforms desde el manifest (nada de tocar Rust para exponer un parámetro).
- Los parámetros declarados en el manifest son **automáticamente keyframables** (6.11), aparecen en el Inspector con el widget correcto (slider/color/checkbox/ángulo/punto 2D) y son accesibles por MCP.
- Compatibilidad GLSL: naga acepta GLSL fragment shaders → se puede portar el catálogo open-source de **gl-transitions** (MIT, ~80 transiciones) casi tal cual.
- La misma filosofía de registro aplica fuera de los shaders: los `ClipPayload`, los `Job` y las `Action` se registran en tablas centrales (`ActionRegistry`), de modo que una feature nueva (p.ej. un payload "Screen Recording Zoom") se añade implementando 2 traits (`Renderable`, `Inspectable`) + 1 entrada de registro, y obtiene gratis undo/redo, persistencia, Inspector y exposición MCP.

#### 6.5.2 Manifest (contrato de datos)

Ejemplo completo en Apéndice B. Campos clave:

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

#### 6.5.3 Catálogo core v1

**Efectos** (todos keyframables): corrección de color (6.8), **chroma key** (abajo), gaussian blur (separable, 2 pasadas — reutilizado por el fondo del modo vertical), box blur, sharpen, viñeta, opacidad, escala de grises/sepia, invert, pixelate, ruido/grain (semilla determinista), glow simple, **shake** (portado del avatar, disponible para cualquier clip), speed ramp (via `speed` del clip), flip H/V.

**Transiciones**: crossfade, dip-to-black/white, wipe (dirección paramétrica), slide/push, zoom blur, circle reveal, + el port de gl-transitions como pack extra opcional.

- Modelo de transición: `TransitionRef { effect_id, duration, params }` entre dos clips adyacentes; el render necesita frames de A y B simultáneamente → requiere handles (material extra); la UI la dibuja como solape con forma de pajarita, arrastrable en duración.

#### 6.5.4 Chroma key (efecto de primera clase) 🔑

Requisito del proyecto: importa muchísimo, así que se especifica al detalle.

- **Algoritmo (shader)**: distancia al color clave en espacio **YCbCr** (robusto a variaciones de luma — solo se compara el plano de croma):
  ```
  d = distance(CbCr(pixel), CbCr(key_color))
  alpha = smoothstep(similarity, similarity + smoothness, d)
  ```
- **Supresión de spill** (el borde verdoso): tras el keying, se desatura el canal dominante del key en los píxeles semi-transparentes: `g' = min(g, mix(g, (r+b)/2, spill * (1 - alpha_edge)))`.
- **Parámetros**: `key_color` (con cuentagotas sobre el preview — la UI muestrea el frame renderizado), `similarity`, `smoothness`, `spill`, `edge_shrink` (erosión de 1px opcional en shader), `output_mode` (resultado / máscara en blanco y negro para depurar).
- **Integración**: es un efecto más de la cadena (composición con premultiplied alpha ya soportada por el pipeline 5.1) → sirve para pantalla verde de material grabado, para avatares mp4 con fondo verde (los `avatar_*.mp4` actuales del toolkit lo necesitan; el `.mov` con alpha no), y para cualquier overlay.
- **Preset "avatar del toolkit"**: verde puro `#00FF00`, similarity 0.30, smoothness 0.10, spill 0.6 — se valida contra los archivos reales de `avatar_config/`.
- **CA**: keyear `avatar_angry.mp4` sobre un video 1080p manteniendo 30 fps de preview; sin halo verde visible a similarity/spill por defecto; alpha correcto en export con y sin fondo debajo.

**Casos borde (sistema de efectos).** Shader de usuario que no compila (efecto deshabilitado + panel de error, nunca crash); parámetro renombrado en el manifest (los proyectos guardan por `key`: los desconocidos se preservan y se ignoran con warning); dos packs con el mismo `id` (gana `user/`, warning).

**CA (sistema).**
1. Crear un efecto nuevo copiando una carpeta y editando 2 archivos, sin recompilar la app, con hot-reload < 2 s.
2. Cadena de 5 efectos sobre un clip 1080p mantiene 30 fps de preview.
3. Una transición de gl-transitions portada funciona idéntica en preview y export.

### 6.6 Texto y títulos (feature 6)

**Objetivo.** Clips de texto de calidad (shaping correcto, emoji, tildes) con estilos y animación.

**UX.**
- Botón "Añadir texto" → clip `Text` en la pista superior; edición del contenido **directamente sobre el preview** (caja editable) o en el Inspector.
- Estilo: fuente (lista de fuentes del sistema + fuentes incluidas), tamaño, color, negrita/cursiva, alineación, interletrado, interlineado, **stroke** (color+ancho), **sombra** (offset, blur, color), **fondo/caja** (color, padding, radio de esquinas), opacidad.
- Posición: arrastrable en el preview con guías inteligentes (centro/tercios); anclas de 9 puntos.
- Animaciones preset de entrada/salida: fade, slide (4 direcciones), typewriter (por carácter), pop; duración configurable. Internamente son solo keyframes generados sobre transform/opacity → editables a mano después.
- **Plantillas**: guardar estilo+animación como plantilla con nombre (JSON en `<app_data>/templates/titles/`); pack inicial de ~8 (lower third, título centrado, esquina para shorts, etc.). Los `titles_clip_config`/`titles` del `config.json` del toolkit se importan como una plantilla "Toolkit clásico".

**Diseño técnico.**
- Layout/shaping con **cosmic-text** (maneja fallback de fuentes y emoji); rasterizado a textura RGBA con caché por `(contenido, estilo, ancho_max)`; el stroke y sombra se generan en el rasterizado (no en shader) para calidad; la textura entra al pipeline como cualquier fuente y recibe efectos/transform/keyframes estándar.
- Re-rasterizar solo cuando cambia contenido/estilo, no por frame. Typewriter: rasterizado por prefijos con caché (N texturas) o máscara por glifo — decisión en implementación, la interfaz lo oculta.
- Los subtítulos automáticos (7.E) reutilizan este mismo rasterizador con su propio payload.

**CA.**
1. Texto con emoji + tildes + CJK se renderiza correcto en las 3 plataformas.
2. Editar texto sobre el preview refleja cambios en < 50 ms.
3. Una plantilla creada en macOS abre igual en Windows (fuentes faltantes → sustitución con warning).

### 6.7 Control de audio (feature 7)

**Objetivo.** Control suficiente para publicar sin DAW externo.

**UX.**
- Por clip: ganancia (dB, -60..+12) con línea de volumen dibujada sobre el clip (arrastrable, con keyframes al hacer `Ctrl+click`), fade in/out con tiradores en las esquinas del clip (curva equal-power), pan.
- Por pista: fader de volumen, mute, solo, medidor RMS/pico vertical.
- Master: fader + medidor con indicador de clipping (retenedor de picos 2 s).
- Utilidades: "Normalizar clip" (analiza pico → ajusta ganancia a -1 dBFS), "Silenciar rango" (keyframes automáticos), **denoise de voz** (RNNoise on/off por clip — procesa el WAV conformado a un WAV alternativo en caché, job en background; sustituto ligero del `denoise.py`/DNS64 del toolkit).
- Export: opción **loudness normalization EBU R128** (dos pasadas de `loudnorm` de ffmpeg, target -14 LUFS para YouTube) en el diálogo de exportación.

**Diseño técnico.** Ya descrito en 5.4. Los fades son curvas de ganancia implícitas fusionadas con la curva de keyframes; el orden es `gain_kf → fades → pan`.

**Casos borde.** Solo en múltiples pistas (unión); clips solapados en pistas distintas suman (headroom del limiter); keyframes de volumen y `speed ≠ 1` (los keyframes viven en tiempo de clip → se estiran con él).

**CA.**
1. Fade in/out sin clicks ni pops (test: seno 1 kHz, inspección del WAV exportado).
2. Medidores consistentes con el archivo exportado (± 1 dB).
3. Mute/solo aplican en < 1 buffer de audio (sin cortes).

### 6.8 Ajustes básicos de imagen + encuadre (feature 8)

**Objetivo.** Brillo, contraste, saturación como mínimo; y rotar/recortar encuadre. Todo keyframable.

**Diseño.** Dos piezas separadas:

1. **Efecto "Corrección de color"** (`core.color_correct`, un solo pase WGSL, siempre disponible en el Inspector sin tener que añadirlo):
   - `brightness` (-1..1, aditivo en luma), `contrast` (0..2, pivote 0.5), `saturation` (0..2, mezcla con luma Rec.709), `exposure` (stops, multiplicativo), `temperature`/`tint` (desplazamiento de balance), `gamma` (0.2..3).
   - Orden fijo documentado en el shader: exposure → temperature/tint → contrast → brightness → saturation → gamma.
2. **Transform2D del clip** (no es shader; es la etapa de composición geométrica 5.1.f):
   - `position (x,y)` en píxeles de secuencia, `scale` (uniforme + no uniforme), `rotation` (grados, libre; atajos 90°/180°/-90°), `anchor point`, `crop` (left/top/right/bottom en % con feather opcional), `opacity`, flip H/V.
   - UI: gizmo sobre el preview (mover/escalar/rotar con handles) + campos numéricos en Inspector. Crop con 4 tiradores de borde.
   - "Fit / Fill / Stretch" de un clic para adecuar material de resolución distinta a la secuencia.

**Casos borde.** Rotación de material ya rotado por metadata (se componen); crop 100% (clip invisible pero válido); keyframes de crop + transición simultáneos.

**CA.**
1. B/C/S coinciden visualmente entre preview y export (test golden-frame con tolerancia ΔE).
2. Rotar 90° un clip vertical de móvil y recortar el encuadre a 16:9 es un flujo de < 5 clics.
3. Todos los parámetros aceptan keyframes y aparecen en el editor de curvas.

### 6.9 Exportación configurable (feature 9)

**Objetivo.** Presets de un clic + control total para usuarios avanzados. Pipeline técnico en 5.6.

**UX — diálogo de exportación.**
- Izquierda: **presets** (editables, guardables):

| Preset | Contenedor | Video | Audio | Notas |
|---|---|---|---|---|
| YouTube 1080p | mp4 | H.264 High, CRF 18, `-preset slow`, yuv420p | AAC 320k | por defecto |
| YouTube 4K | mp4 | H.265 CRF 20 (o H.264 CRF 17) | AAC 320k | aviso de tiempo |
| Shorts/Reels 1080×1920 | mp4 | H.264 CRF 18, ≤ 60 s aviso | AAC 256k | enlaza con 7.D |
| Web ligero | webm | VP9 CRF 32 | Opus 128k | |
| Máster edición | mov | ProRes 422 (`prores_ks`) | PCM 24-bit | intercambio |
| GIF | gif | paleta 2 pasadas, fps 15, ancho 720 | — | |
| Solo audio | mp3 / wav | — | 320k / PCM | podcast |

- Derecha: **overrides**: resolución (con escalado), fps, rango (secuencia completa / marcas I-O), códec, modo bitrate (CRF vs CBR/VBR objetivo), keyframe interval, loudness R128 on/off, nombre/carpeta de salida (plantillas `{proyecto}_{preset}_{fecha}`).
- Estimación de tamaño (heurística por bitrate) y botón "Añadir a cola".
- Panel de cola: progreso por job (fps de render, ETA), cancelar, abrir carpeta al terminar, notificación del SO.

**Casos borde.** Resolución impar con yuv420p (se fuerza par); overwrite de archivo existente (sufijo incremental); disco lleno (error de ffmpeg capturado con mensaje claro); export con media offline (bloqueado con lista de faltantes).

**CA.**
1. Export 1080p H.264 de una secuencia de 5 min con efectos ≥ 1× tiempo real en hardware de referencia.
2. El archivo pasa la validación ffprobe (duración ± 1 frame, fps y resolución exactos).
3. Cancelar deja el sistema limpio (sin procesos zombie ni parciales).

### 6.10 Deshacer/rehacer y guardado de proyecto (feature 10)

**Objetivo.** Undo/redo confiable e ilimitado en la práctica; nunca perder trabajo.

**Diseño técnico — Command pattern en el engine.**
- Toda mutación es una `Action` (enum serializable) con **inversa explícita**: `apply(&mut Project, Action) -> InverseAction`. Ejemplos: `SplitClip{clip, t} ↔ JoinClips{left, right}`, `MoveClip{id, from, to} ↔ MoveClip{id, to, from}`, `SetParam{path, old, new}`.
- `History { undo: Vec<Entry>, redo: Vec<Entry> }`, donde `Entry { actions: Vec<Action>, label, timestamp }` — una entrada puede agrupar N acciones (transacción): un drag, un "eliminar silencios" (¡cientos de cortes = 1 undo!), un wizard vertical completo.
- **Coalescing**: ediciones continuas del mismo parámetro (< 500 ms, mismo path) se funden en una entrada.
- Límite práctico: 1000 entradas (configurable); al excederse se descartan las más antiguas.
- La UI muestra historial navegable (panel con labels: "Dividir clip", "Eliminar 34 silencios") y `Ctrl/Cmd+Z / Shift+Z`.
- Lo NO-undoable (import de archivos, jobs de caché, exports) queda fuera del historial; borrar un asset del pool exige confirmación si hay clips que lo usan (y ES undoable: la acción guarda el asset serializado).

**Guardado.**
- `Ctrl/Cmd+S` → escribe `.uep` (4.3) de forma **atómica** (tmp + rename). Indicador de dirty (● en el título).
- Autosave a `.uep.autosave` cada 60 s si dirty; se elimina al guardar bien; al abrir, si existe y es más nuevo → diálogo de recuperación.
- "Guardar como" + "Guardar copia empaquetada" (backlog: copia media a una carpeta).
- Migraciones: `schema_version` + funciones `migrate_v1_v2(...)` puras y testeadas con fixtures de proyectos viejos.

**CA.**
1. Test de propiedad (proptest): secuencias aleatorias de 200 acciones + undo total ≡ proyecto inicial (comparación estructural).
2. Kill -9 durante autosave → el proyecto original nunca queda corrupto (escritura atómica).
3. "Eliminar silencios" (7.C) es exactamente 1 entrada de undo.

### 6.11 Animaciones por keyframe (feature 11)

**Objetivo.** Animar cualquier parámetro numérico/color declarado (transform, efectos, volumen, texto).

**UX.**
- En el Inspector, cada parámetro keyframable tiene un botón ⏱ (activar animación) y un diamante ◇ (añadir/quitar key en el playhead); flechas ◀▶ saltan entre keys.
- Bajo cada clip seleccionado, el timeline muestra una **lane de keyframes** (diamantes arrastrables; `Alt+drag` duplica; selección múltiple y desplazamiento en bloque).
- **Editor de curvas** (panel plegable): valor vs tiempo, tangentes bezier arrastrables, presets de easing (linear, ease-in/out/in-out, hold, bounce simple).
- Al activar animación de un parámetro con valor V, se crea key inicial `t=playhead, value=V` (comportamiento AE-like esperable).

**Diseño técnico.**
- `KeyframeCurve::eval(t)` — búsqueda binaria del segmento + interpolación según `interp` (Hold devuelve el valor izquierdo; Bezier: hermite cúbico con tangentes, solución iterativa para t-uniforme, precomputada por segmento y cacheada).
- Los tiempos son **relativos al clip** (sobreviven a mover/split, ver 6.2); `speed` del clip escala el mapeo timeline→clip antes de evaluar.
- Colores interpolan por componente en RGB lineal (v1); enums/bools solo Hold.
- El evaluador vive en `ue-core` (compartido por preview, export y tests).

**CA.**
1. Animación de posición (slide) + opacidad (fade) simultáneas, suaves a 30 fps.
2. Split de un clip animado conserva la trayectoria visual exacta (keys interpoladas insertadas en el corte).
3. `eval()` con 10 000 llamadas/frame (peor caso absurdo) < 0.5 ms (bench criterion).

---

## 7. Features avanzadas

### 7.A Servidor MCP embebido

**Objetivo.** Al arrancar la app, un servidor MCP local expone **todo el estado del proyecto** a agentes (Claude Code, Claude Desktop, cualquier cliente MCP), y opcionalmente permite editar.

**Diseño.**
- Crate `ue-mcp` sobre **rmcp** (SDK oficial Rust de MCP), transporte **streamable HTTP** en `http://127.0.0.1:4599/mcp` (puerto configurable; bind SOLO a loopback).
- Corre en el runtime tokio del proceso principal → acceso directo (lock de lectura) al `ProjectStore`. Las herramientas de escritura despachan **las mismas `Action` del ActionRegistry que usa la UI** (sección 6.5.1) → validación, undo y eventos gratis. Un agente que hace 50 ediciones genera entradas de undo etiquetadas `[MCP] …` que el usuario puede deshacer.
- **Seguridad**:
  - Token bearer aleatorio por sesión (visible en Ajustes → MCP, con botón copiar; opción de token fijo por proyecto).
  - Tres niveles configurables: `off` / `read-only` (default) / `read-write`. En read-write, opción "confirmar ediciones destructivas" (diálogo nativo cuando un tool borra > N clips o exporta).
  - Sin acceso al filesystem arbitrario: los tools hablan de IDs del proyecto, no de rutas.
- **Catálogo de tools** (detallado en Apéndice E): lectura (`get_project_summary`, `get_timeline`, `get_transcript`, `get_media_pool`, `get_jobs`, `get_selection_and_playhead`…), edición (`split_clip`, `remove_range`, `move_clip`, `set_clip_property`, `apply_effect`, `add_text_clip`…), IA de alto nivel (`remove_silences`, `delete_words`, `generate_vertical`, `generate_avatar_track`, `start_export`). Resources: `project://current` (JSON del proyecto), `transcript://{asset_id}` (JSON word-level y SRT).
- **Registro del cliente**: la pantalla de Ajustes muestra el snippet listo para copiar:
  ```bash
  claude mcp add --transport http ubereditor http://127.0.0.1:4599/mcp \
      --header "Authorization: Bearer <token>"
  ```
  y el JSON equivalente para Claude Desktop. Botón "probar conexión".
- Eventos: cambios de estado emiten `notifications/resources/updated` sobre `project://current` para clientes suscritos.

**Casos de uso objetivo** (guían el diseño de tools): "¿cuánto dura mi proyecto y qué media usa?", "elimina los silencios de la pista 1 con padding 200 ms", "borra todas las palabras 'este...' y 'o sea'", "genera la versión vertical y exporta para Shorts", "pon un título con el texto X en el minuto 2".

**CA.**
1. Claude Code conectado puede describir el proyecto completo (secuencias, clips, transcripts) sin ayuda.
2. `remove_silences` vía MCP ≡ mismo resultado que el botón de la UI, y es 1 undo.
3. Con nivel `read-only`, toda tool de escritura devuelve error MCP estándar y un mensaje útil.
4. Dos clientes MCP simultáneos no corrompen estado (todas las escrituras serializadas por el lock del store).

### 7.B Whisper palabra-por-palabra + edición basada en texto

**Objetivo.** Todo video/audio importado se transcribe con timestamps **por palabra**; un panel de transcripción permite editar el video borrando o reordenando texto.

#### 7.B.1 Transcripción (job de import)

- **whisper-rs** (whisper.cpp) con `token_timestamps + DTW` para timestamps de palabra; modelos ggml gestionados desde Ajustes → IA:

| Modelo | Tamaño aprox. | Uso recomendado |
|---|---|---|
| tiny / base (q5) | 40–80 MB | pruebas, máquinas lentas |
| small | ~500 MB | equilibrio CPU |
| large-v3-turbo (default) | ~1.6 GB | el que usa el toolkit ("turbo"); rápido y preciso |

  Descarga desde HuggingFace con progreso y verificación sha256, a `<app_data>/models/`. GPU: Metal en macOS (gran ganancia), CUDA opcional.
- Pipeline del job: audio conformado 16 kHz mono (derivado del WAV de 5.3) → VAD opcional (silero-vad ONNX) para trocear y acelerar → whisper por chunks con contexto → merge → normalización (colapsar espacios, unir tokens con apóstrofes) → `TranscriptDoc { words[], segments[] }` (Apéndice D).
- **Caché por `content_hash`** (mismo patrón que el `_segments.json` del toolkit): re-importar el mismo archivo o re-transcribir tras un crash es gratis.
- Configuración: idioma (auto/es/en/…), auto-transcribir al importar (on por defecto, desactivable), modelo, traducción a inglés opcional (capacidad nativa de whisper).
- **Plan B**: si los word-timestamps de whisper.cpp dieran problemas de calidad, el sidecar `toolkit-bridge` (Python congelado con faster-whisper, sección 8.3) implementa el mismo contrato JSON — el resto de la app no se entera.

#### 7.B.2 Panel de transcripción y edición por texto

**UX.**
- Panel lateral "Transcripción" con dos modos:
  - **Modo Asset**: transcript completo de un archivo del pool (para revisar).
  - **Modo Secuencia** (el potente): concatena las palabras de los clips en orden de timeline; refleja la edición actual. Cada palabra conoce su clip y su rango fuente.
- Palabra bajo el playhead resaltada; click en palabra = seek; doble click = selecciona la palabra en el timeline.
- **Borrar texto = cortar video**: seleccionar palabras y `Supr` → el engine corta los rangos correspondientes (con padding configurable, default 80 ms a cada lado, fusionando cortes a < 120 ms de distancia) y hace ripple. Una entrada de undo.
- **Modo tachado (no destructivo)**: `Ctrl+Supr` tacha palabras (se guardan como `rejected`); el preview las salta virtualmente; botón "Aplicar cortes" los materializa. Permite iterar sin comprometerse.
- **Reordenar**: seleccionar una frase y arrastrarla a otro punto del texto → mueve los clips correspondientes (split en fronteras + move + ripple). V1 limita el arrastre a fronteras de frase (los cortes en mitad de coarticulación suenan mal; se documenta).
- Búsqueda de texto con resaltado en timeline (encuentra muletillas: "eee", "o sea") + acción "tachar todas las coincidencias".
- Corrección de texto: editar una palabra corrige el transcript (para subtítulos), nunca el audio.

**Diseño técnico.**
- La operación central es `cut_ranges(sequence, Vec<(TimeUs, TimeUs)>, ripple: bool)` en `ue-core`: normaliza+fusiona rangos, split en fronteras, elimina, ripple; devuelve una transacción. La reutilizan: edición por texto, silencios (7.C) y MCP. **Escribirla una vez, testearla a fondo.**
- Mapeo palabra→timeline: `word.start` está en tiempo del asset; para cada clip Media se indexan las palabras con `src_in ≤ t < src_out`; posición en timeline = `clip.start + (word.start - src_in) / speed`. Índice invertido cacheado e invalidado por patch.

**CA.**
1. Borrar 10 palabras dispersas produce los cortes correctos (test con fixture de transcript sintético) y 1 undo.
2. El modo secuencia refleja splits/moves/deletes existentes correctamente.
3. Video de 20 min con large-v3-turbo transcribe en segundo plano sin bloquear la edición; en M1 ≤ ~2–3 min (Metal).

### 7.C Eliminación / procesado de silencios

**Objetivo.** Detectar silencios y: eliminarlos (ripple), acelerarlos, o solo marcarlos. Port directo mejorado de `trim.py` del toolkit.

**Algoritmo** (en `ue-ai::silence`, sobre el WAV conformado):

```
1. RMS por ventanas: window=50 ms, hop=10 ms (el toolkit usa ventana fija de 2 s /
   0.25 s vía "clip_interval"; el hop fino da fronteras precisas).
2. Umbral dual (histéresis): habla si RMS > T_on; silencio si RMS < T_off = T_on - 6 dB.
   T_on configurable: absoluto en dBFS (default -38 dBFS ≈ el 0.01 lineal del toolkit)
   o RELATIVO: percentil 15 del RMS del clip + 8 dB (robusto a niveles de grabación).
3. Fusionar: silencios < min_silence (default 400 ms) se ignoran (respiraciones);
   islas de habla < min_speech (default 150 ms) se absorben (clicks).
4. Padding: expandir habla pad_pre=150 ms / pad_post=200 ms (deja respirar los finales).
5. Salida: Vec<SpeechInterval> en tiempo del asset.
```

**UX.**
- Diálogo "Silencios…" sobre la selección (o pista/secuencia): sliders de parámetros + **preview en vivo**: regiones rojas (silencio) / verdes (habla) pintadas sobre los clips y el histograma de RMS con la línea de umbral — se recalcula al mover sliders (el análisis RMS se cachea; solo se re-umbraliza: instantáneo).
- Tres acciones:
  1. **Eliminar** → `cut_ranges(ripple=true)` (7.B.2).
  2. **Acelerar** → split de los rangos silenciosos y `speed = N×` (default 4×, con audio atenuado -12 dB opcional) — estilo jump-cut suave.
  3. **Marcar** → solo marcadores de secuencia (revisión manual).
- Estadística previa: "Se eliminarán 47 silencios (2:13 de 14:20 → 12:07)".

**CA.**
1. Sobre un fixture sintético (habla + silencios conocidos) el detector encuentra 100% de silencios > 400 ms sin falsos positivos a umbral default.
2. Preview de regiones se actualiza < 100 ms al mover sliders (solo re-umbralizado).
3. Resultado idéntico al aplicar vía UI, MCP o wizard vertical.

### 7.D Generación automática de videos verticales

**Objetivo.** De una secuencia/rango horizontal a un 9:16 listo para Shorts/Reels con 1 clic. Port del `generate_short_base` de `recipes.py` + `shorts.py`.

**Wizard "Generar vertical…"** (pasos, todos con defaults sensatos):

1. **Rango**: secuencia completa, marcas I-O, o selección de clips.
2. **Layout** (plantillas, sistema abierto a más):
   - `Fondo desenfocado` (port del toolkit): secuencia nueva 1080×1920; capa inferior = mismo material escalado a llenar (crop) + `core.gaussian_blur` (σ≈20, equivalente al `boxblur=10:1`) + oscurecido -20%; capa superior = material escalado a ancho 1080, centrado vertical.
   - `Zoom centrado`: crop 9:16 directo, con posición X keyframable manualmente después.
   - (Backlog: `Auto-reframe` con detección de caras — sección 16.)
3. **Silencios**: checkbox "eliminar silencios primero" (reusa 7.C).
4. **Subtítulos**: checkbox "subtítulos automáticos word-level" (estilo karaoke por defecto, reusa 7.E.2) — usa el transcript existente o lanza whisper.
5. **Títulos**: opcional, texto inicial/CTAs desde plantilla (port de `add_titles` + `titles` del config.json del toolkit: "Video completo en la descripción", "Suscríbete"…).
6. Resultado: **una secuencia nueva** `"<nombre> (Vertical)"` — el original queda intacto; todo es editable a mano después (son clips/efectos normales). El wizard es una transacción componiendo acciones existentes → 1 undo, reproducible por MCP (`generate_vertical`).

**CA.**
1. Wizard completo (con silencios + subtítulos) sobre un video de 3 min < 30 s de proceso (sin contar whisper si no está cacheado).
2. El resultado es 100% editable (mover subtítulos, cambiar blur, deshacer cortes).
3. Export directo con preset Shorts desde el paso final.

### 7.E Avatar personalizable + subtítulos automáticos

#### 7.E.1 Avatar que habla al ritmo del video

Port completo de `avatar_video_generation.py` del toolkit, convertido de "script que exporta un mp4" a **clip vivo del timeline**.

**Modelo.** `ClipPayload::Avatar { config: AvatarConfig, driver_asset: Id }`:

```rust
struct AvatarConfig {
    avatars: BTreeMap<String, RelPath>, // emoción → clip de video (loop). COMPATIBLE
                                        // con avatar_config/config.json del toolkit
                                        // (mismas claves: calm, angry, sad, amazed…)
    default_emotion: String,            // primera clave (como el toolkit)
    shake_factor: f32,                  // vibración ∝ volumen (idéntico concepto)
    chroma: Option<ChromaParams>,       // para avatares mp4 con fondo verde (6.5.4);
                                        // los .mov con alpha no lo necesitan
    scale: f32, anchor: Anchor9,        // posición en el frame (esquina inferior, etc.)
}
```

**Pipeline de análisis** (job "Analizar avatar", cacheado por `content_hash` del driver):
1. Transcript por **segmentos/frases** del asset conductor (reusa 7.B; no requiere word-level).
2. **Clasificación de emoción por segmento** vía endpoint OpenAI-compatible configurable (OpenAI, Ollama local, LM Studio…), con el **mismo prompt probado del toolkit**: *"You are an emotion classifier… reply with exactly one of: {labels}"*, matching laxo por substring y fallback a la emoción default ante error — comportamiento calcado de `classify_emotion()`. Paralelizado (N requests concurrentes), cacheado en el TranscriptDoc (`segments[].emotion`).
   - Fallback sin red/API: heurística por volumen+velocidad (RMS alto→"angry/amazed", pausado→"calm") para que la feature funcione offline, con calidad reducida documentada.
3. **Volumen RMS por segmento** (port de `get_subclip_volume_segment`) + media global → factor de vibración por segmento.

**Render en vivo** (implementa `Renderable`, como cualquier payload):
- En el tiempo `t`, el segmento activo determina la emoción → clip de avatar correspondiente, en **loop** (`src_t = (t - seg.start) % avatar_dur`, decodificado por el DecodePool normal); huecos entre segmentos → avatar default (idéntico a `build_avatar_subclips` del toolkit, incluida la cola final).
- **Shake**: efecto `core.shake` inyectado con `intensity = (seg.volume / global_avg) * shake_factor`, implementado como offset UV en shader con ruido determinista por frame (mejora sobre el `np.roll` aleatorio del toolkit: reproducible y sin coste CPU).
- Chroma key si está configurado; luego transform estándar (posición/escala/keyframes como cualquier clip).

**UX.**
- Editor de avatares (Ajustes → Avatares): crear avatar con nombre, mapear emoción→archivo (drag & drop, con preview en loop), añadir/renombrar emociones libres (el prompt se construye con las claves reales, como hace el toolkit), shake, chroma. **Botón "Importar config del toolkit"** que lee un `avatar_config/config.json` existente tal cual.
- Uso: arrastrar el avatar desde el pool de avatares a una pista sobre el video → se ancla al asset de audio/video conductor bajo él (o el que elija el usuario). Botón "Analizar" lanza el job; hasta entonces se muestra con la emoción default.
- El estado del análisis (emociones por segmento) es visible como colorcitos sobre el clip avatar y editable: click derecho en un segmento → forzar emoción (override manual persistido).

**CA.**
1. Con los archivos reales de `avatar_config/` del toolkit y un video de prueba, el resultado es visualmente equivalente al `output_video.mp4` del toolkit (validación manual lado a lado).
2. Cambiar de emoción un segmento manualmente re-renderiza al instante sin re-análisis.
3. Sin API key configurada, el avatar funciona con la heurística offline.
4. Preview 30 fps con avatar + chroma + video base 1080p.

#### 7.E.2 Subtítulos automáticos

- `ClipPayload::Subtitles { transcript_id, style, mode }` — un solo clip que cubre el rango y renderiza el texto que toque en cada `t` (no cientos de clips de texto; mucho más manejable).
- **Modos**: `Phrase` (agrupación por pausas > 1 s y máx. N palabras/caracteres — port de la lógica `process_transcript` de `translation.py` con `MAX_PAUSE=1.0`), `Word` (palabra a palabra, como `transcript_divided` del toolkit), `Karaoke` (frase visible + palabra actual resaltada con color/escala).
- **Estilo** (`SubtitleStyle`): tipografía completa de 6.6 + color de resaltado karaoke + posición (default: centrado, offset Y configurable — equivalente al `text_position_y_offset` del `config.json` del toolkit, que se importa como preset "Toolkit").
- Sincronizados con la edición: al ser render dinámico desde el transcript + mapeo clip→tiempo (7.B.2), **cortar/mover video reajusta los subtítulos solos**. Palabras `rejected` no se muestran.
- Export a archivo: `.srt` y `.vtt` (con la corrección de redondeo de `float_to_srt_time` hecha con enteros), por asset o por secuencia.
- Edición: corregir texto de una palabra en el panel 7.B.2 se refleja al instante.

**CA.**
1. Modo karaoke legible y sincronizado (± 1 frame de los timestamps de whisper).
2. Tras eliminar silencios, los subtítulos siguen cuadrando sin re-análisis.
3. SRT exportado válido (round-trip con un parser externo) y con acentos correctos (UTF-8).

---

## 8. Reutilización del Youtubers-toolkit

### 8.1 Inventario y mapeo módulo a módulo

| Toolkit (Python) | Qué hace hoy | Destino en UberEditor | Estrategia |
|---|---|---|---|
| `trim.py` (`trim_by_silence`) | RMS por chunks de `clip_interval`, umbral lineal 0.01, corta con moviepy | `ue-ai::silence` (7.C) | **Port a Rust** del algoritmo, mejorado (histéresis, min-duration, padding, umbral relativo). El parámetro `sound_threshold=0.01` ≈ -38 dBFS se conserva como default. |
| `transcript.py` (`generate_transcript`, `transcript_divided`) | faster-whisper "turbo", SRT por segmento o por palabra | `ue-ai::transcribe` (7.B) + export SRT (7.E.2) | **Port** a whisper-rs (mismo modelo turbo). El SRT word-level de `transcript_divided` ≡ modo `Word` de subtítulos. |
| `subtitles.py` + `config.json` (`subtitles_clip_config`, offsets) | Quema SRT con TextClip de moviepy | Payload Subtitles (7.E.2) | **Port de la config**: importador que convierte `config.json` en un `SubtitleStyle` preset ("Toolkit"). Fuente `Hey-Comic`, tamaños y offsets incluidos. |
| `shorts.py` (`blur_video`, `generate_video_base`, `add_titles`) | boxblur=10:1 + composición 1080×1920 + títulos de 3 s | Wizard vertical (7.D) + plantillas de título (6.6) | **Port conceptual**: el blur pasa a shader GPU, la composición a plantilla de layout; `titles` del config → plantilla de CTAs. |
| `set_orientation.py` | resize que INVIERTE w/h (estira) | Transform2D (6.8) | **Sustituido** (el original deforma la imagen; el layout vertical correcto es 7.D). |
| `denoise.py` (DNS64 + torch) | Denoise de voz offline | Denoise RNNoise por clip (6.7) | **Sustituido** por RNNoise (ligero). DNS64 vía sidecar queda en backlog si se echa de menos su calidad. |
| `avatar_video_generation.py` | Whisper → emociones GPT → volumen → loops con shake → mp4 | Payload Avatar (7.E.1) | **Port completo a Rust** conservando: prompt de clasificación, matching laxo + fallback, cache de segmentos, relleno de huecos/cola con default, shake ∝ volumen/media. |
| `avatar_config/config.json` + clips | Mapa emoción→archivo + shake_factor | `AvatarConfig` (7.E.1) | **Formato compatible**: botón de importación directa. Los mp4 con fondo verde usan chroma key core. |
| `translation.py` (`process_transcript`) | Agrupa palabras en frases por pausas > 1 s | Modo `Phrase` de subtítulos (7.E.2) | **Port del algoritmo** de agrupación tal cual. |
| `translation.py` (traducción Helsinki-NLP) + `audio_generator` (Kokoro TTS) | Doblaje: traducir + TTS + remontar audio | — | **Backlog** (sección 16, "Doblaje"). No es requisito v1. |
| `agents/` (killer_video_idea, title_gen, persona_testing) | Ideación de títulos/ideas con OpenAI | — | **No se embebe**: con el servidor MCP (7.A), un agente externo (Claude) hace esto mejor leyendo el transcript del proyecto. Se documenta como receta MCP. |
| `recipes.py` | Encadena comandos CLI | Wizards de la UI + tools MCP de alto nivel | Cada receta ≡ un wizard: `separate_video`→Silencios, `generate_short_base`→Vertical, `subtitle_video`→Subtítulos, `generate_avatar`→Avatar. |
| `utils.py` (`get_subclip_volume*`, `float_to_srt_time`, `apply_shake`, `get_audio`) | Helpers | `ue-media` / `ue-ai` / shader shake | **Port** trivial (RMS y SRT en enteros; shake en GPU). |
| `main.py` (pipeline de kwargs) | Orquestación CLI | ActionRegistry + transacciones | El patrón "pipeline de pasos componibles" sobrevive como composición de Actions. |

### 8.2 Qué NO se hereda (deudas conocidas del toolkit que se corrigen)

- MoviePy/CPU para todo → GPU (wgpu); es la razón #1 de lentitud del toolkit.
- Tiempos en float y SRT con redondeos → `TimeUs` enteros.
- `np.random` sin semilla en el shake → ruido determinista.
- `set_vertical` que estira la imagen → layout real 9:16.
- Subprocesos con `shell=True` e interpolación de rutas → sidecars con args (rutas con espacios seguras).
- Estado en archivos sueltos junto al video (`*_transcript.srt`, `*_segments.json`) → caché centralizada por hash + proyecto autocontenido.

### 8.3 Puente opcional: sidecar `toolkit-bridge`

Red de seguridad si algún port nativo se atasca (sobre todo whisper word-level o calidad de denoise):

- Empaquetar un subconjunto del toolkit (faster-whisper; opcional DNS64/Kokoro) con **PyInstaller** como binario sidecar por plataforma, expuesto por **JSON-RPC sobre stdio**: `transcribe(path, model, word_timestamps) → transcript.json`, `denoise(path) → wav`.
- Contrato de datos idéntico al nativo (Apéndice D) → intercambiable con un flag de configuración.
- Costo: ~300–500 MB de binario extra; por eso es **opt-in de desarrollo**, no el plan A de distribución.

---

## 9. API IPC (frontend ↔ engine)

Convención: comandos `dominio.acción` (Tauri `invoke`), respuestas `Result<T, UeError>` tipadas. Tipos TypeScript **generados** desde los structs Rust (ts-rs o specta) — una sola fuente de verdad.

### 9.1 Comandos (selección; la lista crece con el ActionRegistry)

| Dominio | Comandos |
|---|---|
| `project` | `new`, `open(path)`, `save`, `save_as(path)`, `close`, `undo`, `redo`, `get_state`, `get_history` |
| `media` | `import(paths[])`, `remove(asset_id)`, `relink(asset_id, path)`, `get_pool` |
| `timeline` | `add_clip`, `move_clip`, `split_clip`, `trim_clip`, `delete(ids, ripple)`, `add_track`, `set_track_props`, `add_marker`, `cut_ranges` |
| `clip` | `set_transform`, `set_audio_props`, `add_effect`, `remove_effect`, `set_param(path, value)`, `set_keyframe`, `delete_keyframe`, `add_transition` |
| `playback` | `play`, `pause`, `seek(t)`, `set_rate(r)`, `set_quality(q)`, `set_loop(in, out)` |
| `text` | `add_text_clip`, `set_content`, `set_style`, `save_template`, `list_templates` |
| `export` | `list_presets`, `start(preset, overrides)`, `cancel(job_id)`, `queue_status` |
| `ai` | `transcribe(asset_id, opts)`, `analyze_silences(scope, params)` (solo análisis), `apply_silences(analysis_id, action)`, `delete_words(word_ids, opts)`, `reject_words(word_ids)`, `apply_rejected`, `generate_vertical(opts)`, `avatar_analyze(clip_id)`, `avatar_set_emotion(clip_id, seg_idx, emotion)` |
| `effects` | `list`, `reload_user_packs`, `get_manifest(id)` |
| `mcp` | `status`, `set_mode(off/ro/rw)`, `regenerate_token` |
| `jobs` | `list`, `cancel(id)` |

### 9.2 Eventos (engine → UI)

| Evento | Payload | Frecuencia |
|---|---|---|
| `state.patch` | JSON Patch + nº de versión | por mutación |
| `playback.position` | `t_us`, estado, dropped | 30 Hz durante play |
| `preview.frame` | binario por Channel (5.5) | 30 Hz |
| `audio.meters` | RMS/pico por pista | 15 Hz |
| `job.progress` | id, kind, 0–1, mensaje | ≤ 4 Hz por job |
| `job.done` / `job.error` | id, resultado/error | por job |
| `mcp.activity` | tool llamada, cliente | por llamada (para el indicador de UI) |

---

## 10. Estructura del repositorio

```
ubereditor/
├── PLAN.md                      # este documento
├── package.json / pnpm-lock.yaml
├── src/                         # Frontend React + TS
│   ├── app/                     # shell, layout, ruteo de paneles, temas
│   ├── components/
│   │   ├── timeline/            # canvas del timeline, interacciones, lanes de keyframes
│   │   ├── preview/             # canvas WebGL, transporte, gizmo de transform, cuentagotas
│   │   ├── media-pool/
│   │   ├── inspector/           # widgets de params autogenerados desde manifests
│   │   ├── transcript/          # panel de edición por texto
│   │   ├── export/              # diálogo + cola
│   │   ├── wizards/             # vertical, silencios, avatar
│   │   └── settings/            # IA, MCP, avatares, atajos
│   ├── state/                   # stores zustand (mirror), aplicación de patches
│   ├── ipc/                     # wrappers tipados de invoke/eventos (código generado)
│   └── lib/
├── src-tauri/
│   ├── tauri.conf.json          # sidecars ffmpeg/ffprobe (externalBin), CSP, updater
│   ├── binaries/                # ffmpeg-<target-triple>, ffprobe-<target-triple>
│   ├── icons/
│   └── src/main.rs              # wiring: comandos, eventos, arranque de MCP
├── crates/                      # (ver 3.3) ue-core, ue-media, ue-render, ue-audio,
│   │                            #  ue-playback, ue-export, ue-ai, ue-mcp
│   └── ue-core/tests/           # proptest de acciones, fixtures .uep
├── effects/core/                # packs de efectos/transiciones embebidos
├── assets/                      # fuentes incluidas (OFL), plantillas de títulos, presets
├── docs/                        # ADRs (decisiones), guía de efectos de usuario, guía MCP
└── .github/workflows/           # CI (sección 12)
```

---

## 11. Presupuestos de rendimiento

Hardware de referencia: MacBook Air M1 8 GB / PC Ryzen 5 + GPU integrada Vega / Ubuntu equivalente.

| Métrica | Objetivo | Cómo se mide |
|---|---|---|
| UI del timeline | 60 fps con 500 clips | trazas de rAF en CI manual |
| Preview 1080p (2 pistas video + texto + música) | 30 fps sin drops sostenidos | contador de dropped frames |
| Latencia de scrub (con proxy) | < 150 ms p95 | test instrumentado |
| Seek de reproducción | < 300 ms p95 | ídem |
| Desincronización A/V | < 40 ms sostenida | video de test beep+flash |
| Import (probe + usable) | < 1 s | por archivo |
| Proxy 720p de 10 min 1080p | < 2 min en background | job timing |
| Export 1080p H.264 | ≥ 1× tiempo real | job timing |
| Whisper large-v3-turbo (M1, Metal) | ≥ 5× tiempo real | job timing |
| RAM en reposo con proyecto mediano | < 1.5 GB (sin contar FrameCache configurable) | Activity Monitor |
| Arranque en frío hasta interactivo | < 3 s | stopwatch CI manual |

Decisiones que protegen estos números: proxies GOP-corto, YUV en GPU, canvas único en timeline, cache LRU, patches en vez de estado completo por IPC, jobs fuera del hilo de render.

---

## 12. Estrategia de testing y CI

**Unit (Rust, `cargo test`):**
- `ue-core`: invariantes del modelo, cada Action y su inversa, **proptest** (secuencias aleatorias de acciones + undo total ≡ estado inicial), migraciones de schema con fixtures, evaluador de keyframes (valores exactos en bordes).
- `ue-ai`: detector de silencios sobre WAVs sintéticos generados en el test (tonos + silencios conocidos); agrupación por pausas; mapeo palabra→timeline con transcripts fixture; `cut_ranges` exhaustivo.
- `ue-media`: parsing de ffprobe con JSONs reales grabados como fixtures (incluye VFR, rotate=90, 5.1, sin audio).

**Golden frames (render):**
- Proyectos fixture pequeños → render de N frames concretos → hash perceptual (dHash) contra imágenes de referencia versionadas con tolerancia; corre en las 3 plataformas del CI (con adapter de software: `wgpu` + lavapipe/llvmpipe en Linux CI).
- Cubre: cada efecto core, cada transición, chroma key con imagen de test verde, texto con tildes/emoji, transform con rotación.

**Integración:**
- Export de un proyecto fixture de 10 s → validación con ffprobe (duración, fps, resolución) + extracción de 3 frames → golden.
- MCP: cliente de test que llama cada tool contra una app headless (el engine se puede instanciar sin webview — diseño que además habilita un futuro CLI).
- Whisper: audio corto fixture con texto conocido → asserts laxos sobre las palabras (por variabilidad del modelo, solo en el job nightly con modelo tiny).

**E2E (frontend):** Playwright + tauri-driver (WebDriver) para flujos: importar→cortar→exportar; undo/redo; wizard vertical. Solo smoke en CI (lentos), suite completa manual pre-release.

**CI (GitHub Actions):**
- Matrix macOS/Windows/Ubuntu: `cargo clippy -D warnings`, `cargo test`, `pnpm typecheck && pnpm test`, build Tauri sin firmar.
- Nightly: golden frames completos + bench criterion (regresiones > 15% fallan) + E2E smoke.
- Release: builds firmados (sección 13) + subida a GitHub Releases + manifiesto del updater.

---

## 13. Empaquetado, distribución y licencias

**Sidecars FFmpeg:** builds estáticos por plataforma en `src-tauri/binaries/` con sufijo de target-triple (`ffmpeg-aarch64-apple-darwin`, etc.), declarados en `externalBin`. Script `scripts/fetch-ffmpeg.ts` los descarga y verifica (sha256) en dev y CI.

**Licencias (importante):**
- FFmpeg con libx264/x265 → binario **GPL**: al distribuirlo como sidecar (proceso separado) cumplimos publicando la procedencia del build, su licencia y oferta de fuentes (página "Licencias de terceros" en la app). El código propio de UberEditor no se ve forzado a GPL por ejecutar un binario externo. Si algún día molesta: build LGPL sin x264 (usando openh264/hardware encoders) — decisión documentada como ADR.
- whisper.cpp (MIT), wgpu (MIT/Apache), cosmic-text (MIT/Apache), RNNoise (BSD), gl-transitions (MIT), rmcp (MIT/Apache) — sin fricción. Fuentes incluidas: solo OFL.
- Modelos whisper: se descargan del lado del usuario (no se redistribuyen en el instalador).

**Instaladores:** macOS: `.dmg` universal (arm64+x64), firmado + **notarizado** (cuenta Apple Developer necesaria — trámite a iniciar temprano). Windows: NSIS `.exe` firmado (certificado OV o firma con Azure Trusted Signing). Linux: AppImage + `.deb`.

**Updater:** tauri-plugin-updater con manifiesto en GitHub Releases y firma de update propia.

**Telemetría:** ninguna en v1 (decisión explícita). Crash reports opt-in vía sentry-rust en backlog.

---

## 14. Roadmap por fases

> Estimaciones para **1 desarrollador senior a tiempo completo** ayudado por agentes de IA. Los rangos asumen aprendizaje de wgpu/Tauri incluido. Cada fase termina con sus CA verificados y una tag git.

### Fase 0 — Fundaciones (1–2 semanas)
Scaffolding Tauri 2 + React + workspace de crates (3.3, 10); sidecars ffmpeg funcionando con rutas con espacios; CI matrix verde; `ue-core` con modelo de datos (4), acciones básicas, historial y proptest; ADRs iniciales.
**Hito:** `cargo test` verde en 3 OS; crear/guardar/abrir un `.uep` vacío desde la UI.

### Fase 1 — MVP editable (6–8 semanas)
Import + probe + jobs de caché (proxy, conformado, peaks, thumbs) (6.3); Media Pool; timeline canvas con drag&drop, split/trim/ripple (6.1, 6.2); DecodePool + RenderGraph mínimo (YUV→RGB, transform básico, composición) (5.1–5.3); AudioMixer + reloj maestro (5.4); preview por Channel (5.5); export H.264/AAC con 2 presets (5.6); save/undo/redo completos (6.10).
**Hito (demo):** importar 3 clips, cortarlos, reordenarlos, oír el audio en sync y exportar un mp4 correcto.

### Fase 2 — Editor completo (5–7 semanas)
Sistema de packs de efectos + hot-reload (6.5.1–6.5.2); catálogo core incluyendo **chroma key** (6.5.4); transiciones (6.5.3); corrección de color + Transform2D con gizmo (6.8); texto y títulos con plantillas (6.6); keyframes + lanes + editor de curvas (6.11); audio completo (fades, keyframes, medidores, RNNoise) (6.7); diálogo de export completo con cola y R128 (6.9).
**Hito:** un video "real" editado de punta a punta solo con UberEditor, con chroma key de pantalla verde incluido.

### Fase 3 — IA de texto y silencios (4–6 semanas)
whisper-rs + gestión de modelos + job de transcripción con caché (7.B.1); panel de transcripción, borrar/tachar palabras, búsqueda de muletillas (7.B.2); `cut_ranges` robusto; detector de silencios + diálogo con preview + 3 acciones (7.C); subtítulos automáticos (payload, modos phrase/word/karaoke, export SRT/VTT) (7.E.2).
**Hito:** flujo "importar → transcribir → borrar muletillas por texto → eliminar silencios → subtítulos karaoke".

### Fase 4 — MCP (1.5–2.5 semanas)
`ue-mcp` con rmcp: tools de lectura, resources, token, niveles off/ro/rw (7.A); tools de escritura mapeadas al ActionRegistry; docs + snippet de conexión; indicador de actividad MCP en UI.
**Hito (demo estrella):** Claude Code conectado responde "¿qué hay en mi proyecto?" y ejecuta "elimina los silencios y expórtame un preview".

### Fase 5 — Creador: vertical + avatar (4–6 semanas)
Wizard vertical con plantillas de layout, integración silencios+subtítulos+títulos (7.D); payload Avatar: editor de avatares, importador de config del toolkit, análisis (emociones LLM + fallback offline + volumen), render en vivo con shake determinista y chroma (7.E.1); tools MCP `generate_vertical` / `generate_avatar_track`.
**Hito:** paridad funcional con las recetas del toolkit (`separate_video`, `generate_short_base`, `generate_avatar`, `subtitle_video`) — validación lado a lado con los mismos archivos de entrada.

### Fase 6 — Pulido y release (3–4 semanas)
Rendimiento contra los presupuestos de la sección 11 (perf pass); firmas/notarización + updater (13); onboarding (proyecto demo incluido), atajos configurables, i18n es/en; docs de usuario (efectos custom, MCP, avatares); bug bash + suite E2E completa; v1.0.
**Hito:** instaladores firmados en las 3 plataformas, descargables.

**Total estimado: ~6–7.5 meses** a tiempo completo. Camino crítico: Fase 1 (motor). Las fases 3–5 son paralelizables entre sí si se suma gente (dependen de F1–F2, no entre ellas — salvo que 7.D y 7.E.2 usan 7.B/7.C).

---

## 15. Riesgos y mitigaciones

| # | Riesgo | Prob. | Impacto | Mitigación |
|---|---|---|---|---|
| 1 | El preview por IPC no sostiene 30 fps en alguna plataforma (5.5) | Media | Alto | Fallback JPEG automático; plan Fase-2 de superficie nativa diseñado desde el día 1 (el RenderGraph no sabe cómo se presenta); bajar resolución Auto. |
| 2 | Seeks lentos / VFR rompen precisión de frame | Media | Alto | Proxies CFR GOP-corto SIEMPRE para preview; detección de VFR al importar; `ffmpeg-next` como upgrade path. |
| 3 | Word-timestamps de whisper.cpp menos precisos que faster-whisper | Media | Medio | Plan B `toolkit-bridge` (8.3) con contrato idéntico; padding configurable en cortes por texto amortigua ±50 ms. |
| 4 | wgpu en GPUs viejas/drivers Linux rotos | Baja-media | Medio | Fallback GL de wgpu; render por software (llvmpipe) como último recurso con aviso; matriz de GPUs mínimas documentada. |
| 5 | Complejidad del motor supera el estimado (es lo normal) | Alta | Alto | Fase 1 es SOLO lo mínimo; recortes pre-acordados: reverse playback, slide tool, editor de curvas (pueden caer a Fase 6/backlog sin tocar requisitos). |
| 6 | Licencia GPL de ffmpeg incomoda a futuro | Baja | Medio | Sidecar aislado + ADR con plan LGPL (13). |
| 7 | Clasificación de emociones cara/lenta con API de pago | Media | Bajo | Cache agresivo por hash; soporte Ollama local; heurística offline; batch de segmentos por request. |
| 8 | Scope creep (¡es un editor de video!) | Alta | Alto | Este documento es el contrato: lo que no está en secciones 6–7 va a la 16 y espera a v1.1. |
| 9 | Notarización/firma bloquea el release | Media | Bajo | Iniciar trámites (Apple Developer, cert Windows) en Fase 0–1, no al final. |
| 10 | Cambios de API en Tauri 2 / rmcp (ecosistemas jóvenes) | Media | Bajo | Versiones pineadas; wrappers propios finos alrededor de ambos. |

---

## 16. Backlog futuro (post-v1)

Ordenado por valor estimado para el flujo del usuario:

1. **Auto-reframe con detección de caras** para el modo vertical (ONNX yolov8-face/mediapipe + suavizado EMA → keyframes de crop automáticos).
2. **Doblaje automático** (port de `translation.py` + Kokoro TTS del toolkit: transcribir → traducir → TTS → remontar con ajuste de velocidad).
3. **Recetas MCP de ideación** (documentar prompts para que Claude genere títulos/ideas desde `transcript://` — sustituye a `agents/` del toolkit).
4. Reverse playback fluido, slide tool, compound clips / timelines anidados.
5. Detección y eliminación de **muletillas por lista** con 1 clic (sobre 7.B: buscar "eee", "o sea", "¿vale?" → tachar todo).
6. Grabación directa (cámara/micro/pantalla) dentro de la app.
7. Denoise DNS64 vía sidecar (calidad máxima), de-esser, compresor de voz.
8. Export en paralelo, render distribuido de la cola.
9. HDR / espacio de color lineal 16F; LUTs .cube.
10. Marketplace/carpeta compartida de packs de efectos y plantillas.
11. Crash reporting opt-in; métricas de rendimiento locales.
12. CLI headless (`ubereditor render proyecto.uep --preset youtube`) — el engine ya lo permite (12).

---

## Apéndice A. Ejemplo completo de archivo de proyecto

```jsonc
{
  "schema_version": 1,
  "id": "01JZK3M9V2Q8XW5T7YBGN4RducK",
  "name": "Devlog 12",
  "created_at": "2026-07-09T10:00:00Z",
  "settings": { "whisper_language": "es", "autosave_secs": 60 },
  "assets": [
    {
      "id": "01JZK3MA...A1", "kind": "video", "path": "media/toma1.mp4",
      "content_hash": "xxh3:9f2c…", 
      "probe": { "duration_us": 754000000, "fps": [30000, 1001], "width": 1920,
                 "height": 1080, "rotation": 0, "vcodec": "h264", "acodec": "aac",
                 "audio_channels": 2, "vfr": false },
      "proxy": null, "audio_conform": null, "peaks": null, "thumbnails": null,
      "transcript": "01JZK3TR...T1"
    },
    { "id": "01JZK3MA...A2", "kind": "audio", "path": "media/musica.mp3", "…": "…" },
    { "id": "01JZK3MA...A3", "kind": "image", "path": "media/logo.png", "…": "…" }
  ],
  "transcripts": [ { "id": "01JZK3TR...T1", "asset_id": "01JZK3MA...A1", "…": "ver Apéndice D" } ],
  "sequences": [
    {
      "id": "01JZK3SQ...S1", "name": "Principal",
      "resolution": [1920, 1080], "fps": [30000, 1001], "sample_rate": 48000,
      "markers": [ { "t": 12000000, "name": "Intro fin", "color": "#e5484d" } ],
      "tracks": [
        {
          "id": "01JZ...TA1", "kind": "audio", "name": "Música",
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

## Apéndice B. Ejemplo de efecto modular

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

`effects/user/vhs_retro/shader.wgsl` (contrato: el runtime provee `tex`, `samp`, uniforms del manifest en `params`, y globals `time_s`, `seed`, `resolution`):

```wgsl
@fragment
fn effect(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    // aberración cromática horizontal
    let off = params.color_bleed * 0.004;
    let r = textureSample(tex, samp, uv + vec2(off, 0.0)).r;
    let g = textureSample(tex, samp, uv).g;
    let b = textureSample(tex, samp, uv - vec2(off, 0.0)).b;
    var col = vec3(r, g, b) * params.tint.rgb;
    // scanlines
    let scan = 0.5 + 0.5 * sin(uv.y * params.line_count * 3.14159);
    col *= mix(1.0, scan, params.intensity * 0.35);
    // ruido determinista (hash por pixel+frame, reproducible en export)
    let n = hash13(vec3(uv * resolution, f32(seed)));
    col += (n - 0.5) * params.intensity * 0.08;
    return vec4(col, textureSample(tex, samp, uv).a);
}
```

Al guardar estos dos archivos con la app abierta, el efecto aparece en el Inspector con sus 4 controles, todos keyframables. **Cero cambios en el núcleo.**

## Apéndice C. Comandos FFmpeg de referencia

```bash
# Probe (import)
ffprobe -v quiet -print_format json -show_format -show_streams INPUT

# Proxy 720p GOP-corto (seeks rápidos); CFR si el origen es VFR
ffmpeg -y -i INPUT -vf "scale=-2:720" -c:v libx264 -preset veryfast -crf 20 \
       -g 15 -pix_fmt yuv420p -vsync cfr -r FPS -an CACHE/proxy.mp4

# Conformado de audio (mixer y whisper parten de aquí)
ffmpeg -y -i INPUT -vn -ac 2 -ar 48000 -c:a pcm_s16le CACHE/audio.wav
ffmpeg -y -i CACHE/audio.wav -ac 1 -ar 16000 CACHE/audio16k.wav   # para whisper

# Thumbnails (sprite: 1 frame cada 2 s, 160px de alto)
ffmpeg -y -i INPUT -vf "fps=1/2,scale=-2:90,tile=100x1" -frames:v 1 CACHE/thumbs.jpg

# Sesión de decode del preview (stdout de frames crudos)
ffmpeg -v error -ss SEEK -i PROXY -f rawvideo -pix_fmt yuv420p pipe:1

# Export (frames por stdin + mixdown)
ffmpeg -y -f rawvideo -pix_fmt rgba -s 1920x1080 -r 30000/1001 -i pipe:0 \
       -i mixdown.wav -map 0:v -map 1:a \
       -c:v libx264 -preset slow -crf 18 -pix_fmt yuv420p \
       -c:a aac -b:a 320k -movflags +faststart OUT.mp4

# Loudness R128 (2 pasadas: análisis → aplicar con measured_*)
ffmpeg -i mixdown.wav -af loudnorm=I=-14:TP=-1.5:LRA=11:print_format=json -f null -

# GIF (paleta en 2 pasadas)
ffmpeg -y -i pipe:0 -vf "fps=15,scale=720:-1:flags=lanczos,palettegen" palette.png
ffmpeg -y -i pipe:0 -i palette.png -lavfi "fps=15,scale=720:-1 [x]; [x][1:v] paletteuse" OUT.gif
```

## Apéndice D. Formato de transcripción word-level

`CACHE/<content_hash>/transcript.json` — contrato compartido por whisper-rs nativo y el sidecar `toolkit-bridge`:

```json
{
  "version": 1,
  "asset_hash": "xxh3:9f2c…",
  "language": "es",
  "model": "large-v3-turbo",
  "generated_at": "2026-07-09T10:12:00Z",
  "words": [
    { "i": 0, "text": "Hola", "start_us": 480000, "end_us": 820000,
      "confidence": 0.97, "rejected": false },
    { "i": 1, "text": "a",    "start_us": 860000, "end_us": 920000,
      "confidence": 0.91, "rejected": false }
  ],
  "segments": [
    { "i": 0, "text": "Hola a todos, bienvenidos.", "start_us": 480000,
      "end_us": 2600000, "word_range": [0, 4],
      "emotion": "calm", "volume_rms": 812.4 }
  ],
  "global_avg_volume": 640.2
}
```

- `segments[].emotion` y `volume_rms` los rellena el análisis de avatar (7.E.1) — mismo rol que el `<audio>_segments.json` del toolkit, del que existe un importador.
- `rejected` implementa el modo tachado (7.B.2).
- Export SRT/VTT se deriva de aquí (por words o por segments según el modo).

## Apéndice E. Catálogo inicial de herramientas MCP

**Lectura (nivel `read-only`):**

| Tool | Args | Devuelve |
|---|---|---|
| `get_project_summary` | — | nombre, duración, nº assets/secuencias/clips, jobs activos, dirty |
| `get_media_pool` | — | lista de assets con probe, estado de cachés y transcript |
| `get_timeline` | `sequence_id?`, `include_params?` | pistas y clips (payloads, tiempos, efectos) |
| `get_clip` | `clip_id` | detalle completo: params, keyframes, transiciones |
| `get_transcript` | `asset_id`, `format: json\|srt\|text` | transcript word-level / SRT / texto plano |
| `search_transcript` | `query`, `scope` | palabras coincidentes con ids y tiempos |
| `get_selection_and_playhead` | — | qué mira/tiene seleccionado el usuario ahora |
| `get_jobs` | — | jobs con progreso |
| `get_effects_catalog` | — | manifests disponibles (para que el agente sepa qué puede aplicar) |
| `get_history` | `limit?` | últimas entradas de undo (auditoría) |

**Escritura (nivel `read-write`; toda tool = transacción con label `[MCP] …`):**

| Tool | Args (resumen) |
|---|---|
| `split_clip` | `clip_id`, `t_us` |
| `cut_ranges` | `sequence_id`, `ranges[]`, `ripple` |
| `move_clip` / `trim_clip` / `delete_clips` | ids + tiempos |
| `set_clip_property` | `clip_id`, `path` (p.ej. `transform.scale`), `value` o `curve` |
| `apply_effect` / `remove_effect` | `clip_id`, `effect_id`, `params?` |
| `add_text_clip` | `sequence_id`, `track_id`, `t`, `content`, `style?/template?` |
| `add_marker` | `t`, `name`, `color?` |
| `delete_words` / `reject_words` | `word_ids[]` u `objeto {asset_id, indices}` + `padding_ms?` |
| `remove_silences` | `scope`, `params?` (defaults de 7.C), `action: delete\|speedup\|mark` |
| `generate_vertical` | opciones del wizard 7.D |
| `generate_avatar_track` | `sequence_id`, `avatar_name`, `driver_asset` |
| `transcribe_asset` | `asset_id`, `model?`, `language?` |
| `start_export` | `preset`, `overrides?` → `job_id` |
| `undo` / `redo` | — (el agente puede deshacerse a sí mismo) |

**Resources:** `project://current` (JSON completo, se actualiza con notificaciones), `transcript://{asset_id}` (JSON), `transcript://{asset_id}.srt`.

---

*Fin del plan. Documento vivo: las desviaciones durante la implementación se registran como ADRs en `docs/` y se reflejan aquí.*
