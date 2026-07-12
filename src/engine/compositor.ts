import type { Clip, Id, Project, Sequence, TextStyle, Track } from "./types";
import { paramValue } from "./types";

/**
 * Canvas compositor for the program monitor — the ONE renderer for paused and
 * playing. Every rule here mirrors the export's ffmpeg graph (ue-export
 * graph.rs / preview.rs + ue-render transform_vf), so what the canvas shows is
 * what the export burns in:
 *
 * - The BASE is the first unmuted video track that has media/generator clips
 *   (not "whatever layer happens to be first at t"): its clip fills the canvas
 *   (contain + pad); when it has a gap the frame behind the layers is black.
 * - Upper layers keep their native size (PiP), scaled by the clip transform
 *   and only then capped to the canvas: effective = min(s, cw/sw, ch/sh).
 * - Crop, rotation→flip order, opacity and position match transform_vf.
 * - core.color_correct maps to a CSS filter; core.chroma_key is keyed per
 *   pixel (same YCbCr distance as ffmpeg's chromakey); vertical_fill,
 *   gaussian_blur and drop_shadow as before.
 * - Transitions on the base track render as a crossfade using the handle
 *   material, with the exact window the export's xfade uses (edl.rs
 *   apply_transition_handles). Non-fade kinds approximate as a fade.
 * - Titles/subtitles use the clip's TextStyle with the export's 1080p-relative
 *   scaling, alignment, border and karaoke colors (drawtext_for).
 *
 * This module is pure (no store / IPC imports) so a headless harness can
 * drive it against real media and check pixels.
 */

export interface LayerPixels {
  source: CanvasImageSource;
  /** Logical source size in px (the original asset size, not the proxy's). */
  sw: number;
  sh: number;
}

export interface FrameSources {
  /** Decoded frame of a video asset at `timeSec`, one stream per clip. */
  video(assetId: Id, clipId: Id, timeSec: number): Promise<LayerPixels | null>;
  /** Loaded image element for an image asset (null while loading). */
  image(assetId: Id): LayerPixels | null;
}

/** One drawable layer active at the playhead, bottom track first. */
export interface Layer {
  kind: "media" | "image" | "generator";
  clip: Clip;
  assetId?: Id;
  /** Source time in µs (transitions may reach past the clip's src range). */
  srcUs?: number;
  /** The clip sits on the base track: it fills the canvas like the export. */
  isBase: boolean;
  /** Extra alpha (incoming clip of a crossfade). */
  alphaMul: number;
}

/** Export parity: transitions shorter than ~1 frame are dropped (edl.rs). */
const MIN_TRANSITION_US = 40_000;

/** First unmuted video track that actually carries media/generator clips —
 *  the track the export builds its EDL from. */
export function baseVideoTrack(seq: Sequence): Track | null {
  return (
    seq.tracks.find(
      (t) =>
        t.kind === "video" &&
        !t.muted &&
        t.clips.some((c) => c.payload.type === "media" || c.payload.type === "generator"),
    ) ?? null
  );
}

function activeClipAt(track: Track, tUs: number): Clip | undefined {
  return track.clips.find((c) => c.start <= tUs && tUs < c.start + c.duration);
}

/** Effective xfade half-window for `clip`'s transition_in, in µs of output
 *  time — the same handle math as edl.rs apply_transition_handles. */
function transitionHalf(project: Project, prev: Clip, clip: Clip): number {
  if (!clip.transition_in) return 0;
  if (prev.payload.type !== "media" || clip.payload.type !== "media") return 0;
  // adjacency: the transition shares the cut between the two clips
  if (Math.abs(prev.start + prev.duration - clip.start) > 1000) return 0;
  const prevAsset = project.assets.find(
    (a) => a.id === (prev.payload as { asset_id: Id }).asset_id,
  );
  if (!prevAsset) return 0;
  const availLeft = Math.max(
    0,
    (prevAsset.probe.duration_us - (prev.payload as { src_out: number }).src_out) / prev.speed,
  );
  const availRight = (clip.payload as { src_in: number }).src_in / clip.speed;
  const half = Math.min(clip.transition_in.duration / 2, availLeft, availRight);
  return half * 2 >= MIN_TRANSITION_US ? Math.round(half) : 0;
}

function pushMediaOrImage(
  out: Layer[],
  project: Project,
  clip: Clip,
  tUs: number,
  isBase: boolean,
  alphaMul: number,
) {
  if (clip.payload.type !== "media") return;
  const assetId = clip.payload.asset_id;
  const asset = project.assets.find((a) => a.id === assetId);
  const rel = tUs - clip.start; // may be negative / past the end during a crossfade
  out.push({
    kind: asset?.kind === "image" ? "image" : "media",
    clip,
    assetId,
    srcUs: clip.payload.src_in + rel * clip.speed,
    isBase,
    alphaMul,
  });
}

/** Layers active at the playhead, bottom track first. On the base track a
 *  crossfade window yields TWO layers (outgoing + incoming with alpha). */
export function videoLayers(project: Project, seq: Sequence, playheadUs: number): Layer[] {
  const base = baseVideoTrack(seq);
  const out: Layer[] = [];
  for (const track of seq.tracks) {
    if (track.kind !== "video" || track.muted) continue;
    const isBase = track === base;

    // base track: is the playhead inside a transition window?
    if (isBase) {
      let handled = false;
      for (let i = 1; i < track.clips.length && !handled; i++) {
        const clipB = track.clips[i];
        if (!clipB.transition_in) continue;
        const clipA = track.clips[i - 1];
        const half = transitionHalf(project, clipA, clipB);
        if (half <= 0) continue;
        const cut = clipB.start;
        if (playheadUs < cut - half || playheadUs >= cut + half) continue;
        const progress = (playheadUs - (cut - half)) / (2 * half);
        pushMediaOrImage(out, project, clipA, playheadUs, true, 1);
        pushMediaOrImage(out, project, clipB, playheadUs, true, Math.min(1, Math.max(0, progress)));
        handled = true;
      }
      if (handled) continue;
    }

    const clip = activeClipAt(track, playheadUs);
    if (!clip) continue;
    if (clip.payload.type === "media") {
      pushMediaOrImage(out, project, clip, playheadUs, isBase, 1);
    } else if (clip.payload.type === "generator") {
      out.push({ kind: "generator", clip, isBase, alphaMul: 1 });
    }
  }
  return out;
}

// ---------------------------------------------------------------------------
// Effects the canvas can reproduce
// ---------------------------------------------------------------------------

/**
 * An effect that rewrites the SOURCE image, ported from the ffmpeg filter the
 * export runs. They are applied in the clip's effect order and BEFORE the
 * transform — exactly like `clip_vf` (effects, then transform_vf). Getting
 * that order wrong is what made position/scale/rotation silently no-op on any
 * clip carrying core.vertical_fill.
 */
export type SourceOp =
  | { kind: "chroma"; color: string; similarity: number; blend: number; despill: number }
  | { kind: "eq"; brightness: number; contrast: number; saturation: number; gamma: number }
  | { kind: "verticalFill"; width: number; height: number; blur: number }
  | { kind: "blur"; sigma: number };

export interface LayerFx {
  /** Source-rewriting effects, in the clip's order. */
  ops: SourceOp[];
  /** core.drop_shadow: offset + blurred tinted copy behind the layer. */
  shadow: { dx: number; dy: number; blur: number; opacity: number; color: string } | null;
}

export function layerEffects(clip: Clip, rel: number): LayerFx {
  const ops: SourceOp[] = [];
  let shadow: LayerFx["shadow"] = null;
  for (const fx of clip.effects) {
    if (!fx.enabled) continue;
    if (fx.effect_id === "core.vertical_fill") {
      ops.push({
        kind: "verticalFill",
        width: paramValue(fx.params["width"] ?? 1080, rel),
        height: paramValue(fx.params["height"] ?? 1920, rel),
        blur: paramValue(fx.params["blur"] ?? 20, rel),
      });
    } else if (fx.effect_id === "core.gaussian_blur") {
      ops.push({ kind: "blur", sigma: paramValue(fx.params["sigma"] ?? 0, rel) });
    } else if (fx.effect_id === "core.drop_shadow") {
      shadow = {
        dx: paramValue(fx.params["offset_x"] ?? 0, rel),
        dy: paramValue(fx.params["offset_y"] ?? 0, rel),
        blur: paramValue(fx.params["blur"] ?? 0, rel),
        opacity: paramValue(fx.params["opacity"] ?? 0.5, rel),
        color: fx.color_params["shadow_color"] ?? "#000000",
      };
    } else if (fx.effect_id === "core.color_correct") {
      ops.push({
        kind: "eq",
        brightness: paramValue(fx.params["brightness"] ?? 0, rel),
        contrast: paramValue(fx.params["contrast"] ?? 1, rel),
        saturation: paramValue(fx.params["saturation"] ?? 1, rel),
        gamma: paramValue(fx.params["gamma"] ?? 1, rel),
      });
    } else if (fx.effect_id === "core.chroma_key") {
      ops.push({
        kind: "chroma",
        color: fx.color_params["key_color"] ?? "#00ff00",
        similarity: paramValue(fx.params["similarity"] ?? 0.3, rel),
        blend: paramValue(fx.params["blend"] ?? 0.1, rel),
        despill: paramValue(fx.params["despill"] ?? 0.5, rel),
      });
    }
  }
  return { ops, shadow };
}

// ---------------------------------------------------------------------------
// Pixel ops: exact ports of ffmpeg's vf_eq and vf_chromakey, run in YCbCr
// limited range (the export converts to yuv420p before these filters)
// ---------------------------------------------------------------------------

/** Pixel processing is bounded to this width: cheap, and the paused preview
 *  is the export-exact backend frame anyway. */
const CHROMA_MAX_W = 960;

interface ChromaCacheEntry {
  srcRef: CanvasImageSource;
  paramsKey: string;
  out: LayerPixels;
}
const chromaCache = new Map<Id, ChromaCacheEntry>();

/** vf_eq's LUT: v = contrast*(v-0.5)+0.5+brightness, then gamma, on LUMA. */
function eqLut(brightness: number, contrast: number, gamma: number): Uint8ClampedArray {
  const lut = new Uint8ClampedArray(256);
  const g = 1 / Math.max(0.1, gamma);
  for (let i = 0; i < 256; i++) {
    let v = contrast * (i / 255 - 0.5) + 0.5 + brightness;
    v = v <= 0 ? 0 : Math.pow(v, g);
    lut[i] = Math.round(v * 255);
  }
  return lut;
}

/** RGB → YCbCr, BT.601 limited range (what yuv420p carries in the export). */
function rgbToYcc(r: number, g: number, b: number): [number, number, number] {
  return [
    16 + 0.256788 * r + 0.504129 * g + 0.097906 * b,
    128 - 0.148223 * r - 0.290993 * g + 0.439216 * b,
    128 + 0.439216 * r - 0.367788 * g - 0.071427 * b,
  ];
}

/** The ops that run as a per-pixel pass over the image data. */
type PerPixelOp = Extract<SourceOp, { kind: "eq" } | { kind: "chroma" }>;

/** Exposed so the parity harness can compare our eq/chroma maths against
 *  ffmpeg's on identical pixels, with no video decoder in between. */
export function applyEqForTest(d: Uint8ClampedArray, op: PerPixelOp) {
  applyOps(d, [op]);
}

function applyOps(d: Uint8ClampedArray, ops: PerPixelOp[]) {
  for (const op of ops) {
    if (op.kind === "eq") {
      const lut = eqLut(op.brightness, op.contrast, op.gamma);
      const sat = op.saturation;
      for (let i = 0; i < d.length; i += 4) {
        const [y, cb, cr] = rgbToYcc(d[i], d[i + 1], d[i + 2]);
        const y2 = lut[Math.max(0, Math.min(255, Math.round(y)))];
        const cb2 = (cb - 128) * sat + 128;
        const cr2 = (cr - 128) * sat + 128;
        const c = (y2 - 16) * 1.164383;
        d[i] = c + 1.596027 * (cr2 - 128);
        d[i + 1] = c - 0.391762 * (cb2 - 128) - 0.812968 * (cr2 - 128);
        d[i + 2] = c + 2.017232 * (cb2 - 128);
      }
    } else {
      const [kr, kg, kb] = hexRgb(op.color);
      const [, kcb, kcr] = rgbToYcc(kr, kg, kb);
      const sim = Math.max(0.001, op.similarity);
      const blend = Math.max(0, op.blend);
      const despill = Math.min(1, Math.max(0, op.despill));
      // vf_chromakey: diff = hypot(du,dv) normalised by the max chroma
      // distance (255·√2); alpha = clip((diff - similarity)/blend)
      const NORM = 255 * Math.SQRT2;
      for (let i = 0; i < d.length; i += 4) {
        const r = d[i];
        const g = d[i + 1];
        const b = d[i + 2];
        const [, cb, cr] = rgbToYcc(r, g, b);
        const diff = Math.hypot(cb - kcb, cr - kcr) / NORM;
        let a: number;
        if (blend > 1e-4) a = Math.min(1, Math.max(0, (diff - sim) / blend));
        else a = diff > sim ? 1 : 0;
        if (a < 1 && despill > 0) {
          // suppress the key's dominant channel on semi-transparent edges
          const limit = (r + b) / 2;
          if (g > limit) d[i + 1] = g - (g - limit) * despill;
        }
        d[i + 3] = Math.round(d[i + 3] * a);
      }
    }
  }
}

function hexRgb(hex: string): [number, number, number] {
  const h = hex.replace("#", "");
  return [parseInt(h.slice(0, 2), 16) || 0, parseInt(h.slice(2, 4), 16) || 0, parseInt(h.slice(4, 6), 16) || 0];
}

/** Actual pixel size of a drawable source (`.width` is 0 on a <video> and the
 *  layout size on an <img>; the intrinsic properties are the truth). */
function sourceDims(src: CanvasImageSource): [number, number] {
  const v = src as Partial<HTMLVideoElement>;
  if (typeof v.videoWidth === "number" && v.videoWidth > 0) return [v.videoWidth, v.videoHeight ?? 0];
  const img = src as Partial<HTMLImageElement>;
  if (typeof img.naturalWidth === "number" && img.naturalWidth > 0) {
    return [img.naturalWidth, img.naturalHeight ?? 0];
  }
  const c = src as { width?: number; height?: number };
  return [c.width ?? 0, c.height ?? 0];
}

// ---------------------------------------------------------------------------
// Blur: NEVER rely on ctx.filter alone — WebKit (the Tauri webview on macOS)
// silently ignores it, which is why the vertical-fill background rendered
// sharp during playback while the paused (ffmpeg) frame showed it blurred.
// Feature-detect once, and fall back to a real separable box blur (3 passes
// ≈ gaussian, Wells' approximation) that works in every engine.
// ---------------------------------------------------------------------------

let filterOk: boolean | null = null;

/**
 * Does this engine actually BLUR when told to?
 *
 * The probe has to be functional, not a property check: WebKit (the Tauri
 * webview on macOS) accepts `ctx.filter = "blur(3px)"` and reads the value
 * back happily, yet draws the image completely sharp. Trusting the property
 * is exactly why the vertical-fill background rendered sharp in playback
 * while the paused (ffmpeg) frame showed it blurred. So: draw a hard
 * black|white edge through the filter and look for a mid grey at the seam.
 */
export function canvasFilterSupported(): boolean {
  if (filterOk === null) filterOk = probeFilter(false).blurred;
  return filterOk;
}

/**
 * Draws a hard black|white edge through `ctx.filter` and reports whether the
 * seam came out grey (a real blur) — on an ACCELERATED canvas by default.
 *
 * `readback` exists only to expose the trap: asking for `willReadFrequently`
 * makes WebKit hand out a software canvas, and a software canvas can silently
 * ignore filters. Probing on one of those reports "no filter support" even
 * when the accelerated canvas the preview actually draws on supports it fine.
 */
export function probeFilter(readback: boolean): { blurred: boolean; mid: number; reads: string } {
  try {
    const src = document.createElement("canvas");
    src.width = 64;
    src.height = 16;
    const sctx = src.getContext("2d");
    if (!sctx) return { blurred: false, mid: 0, reads: "no 2d context" };
    sctx.fillStyle = "#000";
    sctx.fillRect(0, 0, 32, 16);
    sctx.fillStyle = "#fff";
    sctx.fillRect(32, 0, 32, 16);

    const out = document.createElement("canvas");
    out.width = 64;
    out.height = 16;
    const octx = out.getContext("2d", readback ? { willReadFrequently: true } : undefined);
    if (!octx) return { blurred: false, mid: 0, reads: "no 2d context" };
    octx.filter = "blur(4px)";
    const reads = octx.filter; // what the engine claims it stored
    octx.drawImage(src, 0, 0);
    octx.filter = "none";
    // readback on an accelerated canvas is allowed (just slow) — fine once
    const d = octx.getImageData(24, 8, 16, 1).data;
    let mid = 0;
    for (let i = 0; i < 16; i++) {
      const v = d[i * 4];
      if (v > 40 && v < 215) mid++; // a real blur bleeds the edge into greys
    }
    return { blurred: mid >= 3, mid, reads };
  } catch (e) {
    return { blurred: false, mid: 0, reads: `threw: ${e}` };
  }
}

// ---------------------------------------------------------------------------
// Canvas pool: allocating (and GC-ing) canvases + ImageData every frame is
// what dropped playback to ~1 fps. Scratches are keyed by purpose and reused.
// ---------------------------------------------------------------------------

const pool = new Map<string, CanvasRenderingContext2D>();

/**
 * A pooled scratch canvas.
 *
 * `readback` decides whether the canvas is CPU-backed. This matters enormously:
 * `willReadFrequently` turns OFF GPU acceleration for the WHOLE canvas, so a
 * full-size compositing scratch flagged that way does every drawImage in
 * software AND re-uploads a multi-megabyte texture each frame. Only the tiny
 * canvases we actually call getImageData on may ask for it.
 */
function scratch(
  key: string,
  w: number,
  h: number,
  readback = false,
): CanvasRenderingContext2D | null {
  let ctx = pool.get(key);
  if (!ctx) {
    const c = document.createElement("canvas");
    c.width = w;
    c.height = h;
    const got = c.getContext("2d", readback ? { willReadFrequently: true } : undefined);
    if (!got) return null;
    ctx = got;
    pool.set(key, ctx);
  }
  const c = ctx.canvas;
  if (c.width !== w || c.height !== h) {
    c.width = w;
    c.height = h;
  }
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  ctx.globalAlpha = 1;
  ctx.imageSmoothingQuality = "high";
  ctx.clearRect(0, 0, w, h);
  return ctx;
}

/**
 * Draws `src` into ctx at (x,y,dw,dh) blurred by `sigma` DESTINATION px.
 *
 * The blur is done by DOWNSCALING first (drawImage is GPU-filtered, so the
 * downscale already does most of the blurring for free) until the residual
 * sigma is a couple of pixels, running the box passes on that tiny image, and
 * scaling back up. Blurring at full size in JS is what made playback crawl:
 * a 480×270 pass is ~130k pixels × 3 passes × 2 directions EVERY frame.
 */
function drawBlurred(
  ctx: CanvasRenderingContext2D,
  key: string,
  src: CanvasImageSource,
  x: number,
  y: number,
  dw: number,
  dh: number,
  sigma: number,
) {
  if (sigma < 0.4 || dw < 2 || dh < 2) {
    ctx.drawImage(src, x, y, dw, dh);
    return;
  }
  if (canvasFilterSupported()) {
    const prev = ctx.filter;
    ctx.filter = `blur(${sigma.toFixed(2)}px)`;
    ctx.drawImage(src, x, y, dw, dh);
    ctx.filter = prev;
    return;
  }
  // ---- pure-GPU blur (dual filtering / "mip" blur) ----------------------
  //
  // NOT ONE JavaScript pixel loop. Reading pixels back with getImageData and
  // box-blurring them in JS cost 165 ms PER FRAME in the real webview (~6 fps,
  // measured in the app). Every step here is a `drawImage`, which the GPU
  // does with bilinear filtering: halving averages each 2×2 block (a true box
  // low-pass), and scaling back up in halving steps — instead of one giant
  // jump — is what removes the blotchy, smeared look of a single big upscale.
  //
  // Each halving roughly doubles the blur radius, so N ≈ log2(sigma) + 1.
  const steps = Math.min(
    MAX_BLUR_STEPS,
    Math.max(1, Math.round(Math.log2(Math.max(1, sigma))) + 1),
  );
  let cur: CanvasImageSource = src;
  const sizeAt = (i: number): [number, number] => [
    Math.max(2, Math.round(dw / 2 ** i)),
    Math.max(2, Math.round(dh / 2 ** i)),
  ];
  // down
  for (let i = 1; i <= steps; i++) {
    const [w2, h2] = sizeAt(i);
    const down = scratch(`${key}:d${i}`, w2, h2);
    if (!down) break;
    down.drawImage(cur, 0, 0, w2, h2);
    cur = down.canvas;
  }
  // back up, one halving at a time (each pass smooths the previous one)
  for (let i = steps - 1; i >= 1; i--) {
    const [w2, h2] = sizeAt(i);
    const up = scratch(`${key}:u${i}`, w2, h2);
    if (!up) break;
    up.drawImage(cur, 0, 0, w2, h2);
    cur = up.canvas;
  }
  ctx.drawImage(cur, x, y, dw, dh);
}

/** Beyond this the image is so small the blur turns into mush. */
const MAX_BLUR_STEPS = 6;

// ---------------------------------------------------------------------------
// Source pipeline: effects rewrite the SOURCE, then the transform runs on the
// result — the same order as clip_vf (render_chain, then transform_vf).
// ---------------------------------------------------------------------------

/**
 * core.vertical_fill: the source scaled to COVER width×height and blurred as
 * the background, with the source width-fitted and centred on top. The output
 * is a NEW image of width×height, which then goes through the clip transform
 * — that is precisely what the ffmpeg chain does (split/scale/crop/gblur/
 * overlay produces a width×height frame; transform_vf runs after it).
 *
 * `k` is canvas px per sequence px: the scratch is rendered at the size it
 * will actually occupy, so playback stays cheap.
 */
function renderVerticalFill(
  clipId: Id,
  i: number,
  pixels: LayerPixels,
  op: Extract<SourceOp, { kind: "verticalFill" }>,
  k: number,
): LayerPixels {
  const [srcW, srcH] = sourceDims(pixels.source);
  if (!srcW || !srcH) return pixels;
  const fw = Math.max(2, Math.round(op.width));
  const fh = Math.max(2, Math.round(op.height));
  // `k` is DEVICE px per sequence px: render the scratch at the size it will
  // really occupy on screen, so the sharp foreground stays sharp (rendering at
  // CSS px and letting the DPR transform upscale is what made it soft).
  const s = Math.min(1, Math.max(0.2, k));
  const cwPx = Math.max(2, Math.round(fw * s));
  const chPx = Math.max(2, Math.round(fh * s));
  const ctx = scratch(`${clipId}:vf${i}`, cwPx, chPx);
  if (!ctx) return pixels;

  // background: COVER (force_original_aspect_ratio=increase + crop), blurred
  const cover = Math.max(cwPx / srcW, chPx / srcH);
  const bw = srcW * cover;
  const bh = srcH * cover;
  drawBlurred(
    ctx,
    `${clipId}:vfb${i}`,
    pixels.source,
    (cwPx - bw) / 2,
    (chPx - bh) / 2,
    bw,
    bh,
    op.blur * s,
  );
  // foreground: scale={width}:-2 → fit the full width, centred. Sharp.
  const gh = cwPx * (srcH / srcW);
  ctx.drawImage(pixels.source, 0, (chPx - gh) / 2, cwPx, gh);

  return { source: ctx.canvas, sw: fw, sh: fh };
}

/** core.gaussian_blur: sigma is in the SOURCE's own pixels. */
function renderBlur(
  clipId: Id,
  i: number,
  pixels: LayerPixels,
  op: Extract<SourceOp, { kind: "blur" }>,
  k: number,
): LayerPixels {
  const [srcW, srcH] = sourceDims(pixels.source);
  if (!srcW || !srcH || op.sigma < 0.4) return pixels;
  // the source is drawn at (its logical size × k) device px
  const s = Math.min(1, Math.max(0.2, (k * pixels.sw) / srcW));
  const w = Math.max(2, Math.round(srcW * s));
  const h = Math.max(2, Math.round(srcH * s));
  const ctx = scratch(`${clipId}:gb${i}`, w, h);
  if (!ctx) return pixels;
  drawBlurred(ctx, `${clipId}:gbb${i}`, pixels.source, 0, 0, w, h, op.sigma * s);
  return { source: ctx.canvas, sw: pixels.sw, sh: pixels.sh };
}

/** eq / chroma_key: one per-pixel pass (batched while they are adjacent). */
function renderPerPixel(
  clipId: Id,
  i: number,
  pixels: LayerPixels,
  ops: PerPixelOp[],
): LayerPixels {
  const [srcW, srcH] = sourceDims(pixels.source);
  if (!srcW || !srcH) return pixels;
  const scale = Math.min(1, CHROMA_MAX_W / srcW);
  const w = Math.max(2, Math.round(srcW * scale));
  const h = Math.max(2, Math.round(srcH * scale));
  const ctx = scratch(`${clipId}:px${i}`, w, h, true); // getImageData below
  if (!ctx) return pixels;
  ctx.drawImage(pixels.source, 0, 0, w, h);
  const img = ctx.getImageData(0, 0, w, h);
  applyOps(img.data, ops);
  ctx.putImageData(img, 0, 0);
  return { source: ctx.canvas, sw: pixels.sw, sh: pixels.sh };
}

/**
 * Runs the clip's source-rewriting effects IN ORDER and returns the resulting
 * image (with its new logical size — vertical_fill changes it). Cached per
 * clip while the decoded frame and the params stay the same, so a paused
 * frame or a still image costs nothing.
 */
export function renderSource(
  clipId: Id,
  pixels: LayerPixels,
  ops: SourceOp[],
  k: number,
): LayerPixels {
  if (!ops.length) return pixels;
  const paramsKey = `${JSON.stringify(ops)}|${k.toFixed(3)}`;
  const cached = chromaCache.get(clipId);
  if (cached && cached.srcRef === pixels.source && cached.paramsKey === paramsKey) {
    return cached.out;
  }

  let cur = pixels;
  let batch: PerPixelOp[] = [];
  const flush = (i: number) => {
    if (batch.length) {
      cur = renderPerPixel(clipId, i, cur, batch);
      batch = [];
    }
  };
  ops.forEach((op, i) => {
    if (op.kind === "eq" || op.kind === "chroma") {
      batch.push(op);
    } else if (op.kind === "verticalFill") {
      flush(i);
      cur = renderVerticalFill(clipId, i, cur, op, k);
    } else {
      flush(i);
      cur = renderBlur(clipId, i, cur, op, k);
    }
  });
  flush(ops.length);

  chromaCache.set(clipId, { srcRef: pixels.source, paramsKey, out: cur });
  if (chromaCache.size > 16) {
    const first = chromaCache.keys().next().value;
    if (first !== undefined && first !== clipId) chromaCache.delete(first);
  }
  return cur;
}

// ---------------------------------------------------------------------------
// Generators (core.solid / core.gradient), rendered to an offscreen source
// ---------------------------------------------------------------------------

/** Renders a generator clip to a canvas of its own size, so it goes through
 *  the same fit/transform path as media (export: lavfi source + norm/fit). */
export function generatorPixels(clip: Clip): LayerPixels | null {
  if (clip.payload.type !== "generator") return null;
  const { generator_id, params, color_params } = clip.payload;
  const isGrad = generator_id === "core.gradient";
  const gw = Math.max(2, Math.round(paramValue(params["width"] ?? (isGrad ? 1920 : 640)) / 2) * 2);
  const gh = Math.max(2, Math.round(paramValue(params["height"] ?? (isGrad ? 1080 : 360)) / 2) * 2);
  const canvas = document.createElement("canvas");
  canvas.width = gw;
  canvas.height = gh;
  const ctx = canvas.getContext("2d");
  if (!ctx) return null;
  if (isGrad) {
    // diagonal gradient, same anchors as the lavfi source (0,0 → w,h)
    const g = ctx.createLinearGradient(0, 0, gw, gh);
    g.addColorStop(0, color_params["color_a"] ?? "#ffb224");
    g.addColorStop(1, color_params["color_b"] ?? "#16130f");
    ctx.fillStyle = g;
  } else {
    ctx.fillStyle = color_params["color"] ?? "#ff3355";
  }
  ctx.fillRect(0, 0, gw, gh);
  return { source: canvas, sw: gw, sh: gh };
}

// ---------------------------------------------------------------------------
// Layer drawing (fit rules mirror ue-render transform_vf + the export norm)
// ---------------------------------------------------------------------------

export function drawMediaLayer(
  ctx: CanvasRenderingContext2D,
  w: number,
  h: number,
  seqRes: [number, number],
  pixels: LayerPixels,
  layer: Layer,
  rel: number,
  fx: LayerFx,
) {
  if (pixels.sw <= 0 || pixels.sh <= 0) return;
  const [cw, ch] = seqRes;
  const k = w / cw; // CSS px per sequence px (drawing happens in CSS px)
  const t = layer.clip.transform;
  const clipOpacity = Math.max(0, Math.min(1, paramValue(t.opacity, rel)));
  const opacity = clipOpacity * layer.alphaMul;
  if (opacity <= 0) return;

  // Effect scratches must be sized in DEVICE px (the ctx carries a DPR
  // transform), otherwise everything an effect renders gets upscaled by the
  // DPR afterwards and comes out soft — that is why the vertical-fill
  // foreground looked blurry during playback but crisp in the paused frame.
  const dpr = ctx.getTransform().a || 1;

  // EFFECTS FIRST, TRANSFORM AFTER (the clip_vf order). vertical_fill rewrites
  // the source into a width×height frame, so `sw`/`sh` below are the effect's
  // OUTPUT size — and the transform then applies to it, as in the export.
  const { source, sw, sh } = renderSource(layer.clip.id, pixels, fx.ops, k * dpr);
  if (sw <= 0 || sh <= 0) return;

  ctx.save();
  ctx.globalAlpha = opacity;

  // crop (transform_vf evaluates crop at t=0; fractions clamped like ffmpeg)
  const cropL = Math.min(0.49, Math.max(0, paramValue(t.crop[0], 0)));
  const cropT = Math.min(0.49, Math.max(0, paramValue(t.crop[1], 0)));
  const cropR = Math.min(0.49, Math.max(0, paramValue(t.crop[2], 0)));
  const cropB = Math.min(0.49, Math.max(0, paramValue(t.crop[3], 0)));
  const hasCrop = cropL + cropT + cropR + cropB > 1e-4;
  const [rawW, rawH] = sourceDims(source);
  const srcW = rawW || sw;
  const srcH = rawH || sh;
  const sx = srcW * cropL;
  const sy = srcH * cropT;
  const sWidth = Math.max(1, srcW * (1 - cropL - cropR));
  const sHeight = Math.max(1, srcH * (1 - cropT - cropB));
  const cropW = sw * (1 - cropL - cropR); // logical cropped size
  const cropH = sh * (1 - cropT - cropB);

  const tsx = Math.min(10, Math.max(0.01, paramValue(t.scale[0], rel)));
  const tsy = Math.min(10, Math.max(0.01, paramValue(t.scale[1], rel)));
  const tpx = paramValue(t.position[0], rel);
  const tpy = paramValue(t.position[1], rel);
  const rot = paramValue(t.rotation, rel);
  const hasGeom =
    Math.abs(tsx - 1) > 1e-4 ||
    Math.abs(tsy - 1) > 1e-4 ||
    Math.round(tpx) !== 0 ||
    Math.round(tpy) !== 0 ||
    Math.abs(rot) > 1e-4 ||
    clipOpacity < 0.999;

  // destination size in sequence px, then to canvas px with k
  let dw: number;
  let dh: number;
  if (layer.isBase) {
    if (hasGeom) {
      // export: fit the FULL frame to the canvas first, then crop + scale
      const f0 = Math.min(cw / sw, ch / sh);
      dw = cropW * f0 * tsx;
      dh = cropH * f0 * tsy;
    } else {
      // export: crop first, then the norm contains the cropped frame
      const f0 = Math.min(cw / cropW, ch / cropH);
      dw = cropW * f0;
      dh = cropH * f0;
    }
  } else {
    // export layer: crop native → scale → cap to the canvas (never upscale)
    const cap = Math.min(1, cw / (cropW * tsx), ch / (cropH * tsy));
    dw = cropW * tsx * cap;
    dh = cropH * tsy * cap;
  }

  const cx = w / 2 + tpx * k;
  const cy = h / 2 + tpy * k;
  if (fx.shadow) {
    // ffmpeg's shadow lives in the SOURCE's own pixels, so convert with the
    // source→screen scale (not the sequence scale, which is only the same when
    // the clip happens to be drawn at 1:1)
    const srcToScreen = sw > 0 ? (dw * k) / sw : k;
    const [sr, sg, sb] = hexRgb(fx.shadow.color);
    ctx.shadowColor = `rgba(${sr},${sg},${sb},${Math.max(0, Math.min(1, fx.shadow.opacity))})`;
    ctx.shadowBlur = fx.shadow.blur * srcToScreen;
    ctx.shadowOffsetX = fx.shadow.dx * srcToScreen;
    ctx.shadowOffsetY = fx.shadow.dy * srcToScreen;
  }
  if (rot || layer.clip.transform.flip_h || layer.clip.transform.flip_v) {
    ctx.translate(cx, cy);
    // export order: rotate first, then flip in world space (hflip after rotate)
    ctx.scale(layer.clip.transform.flip_h ? -1 : 1, layer.clip.transform.flip_v ? -1 : 1);
    if (rot) ctx.rotate((rot * Math.PI) / 180);
    ctx.translate(-cx, -cy);
  }
  if (hasCrop) {
    ctx.drawImage(source, sx, sy, sWidth, sHeight, cx - (dw * k) / 2, cy - (dh * k) / 2, dw * k, dh * k);
  } else {
    ctx.drawImage(source, cx - (dw * k) / 2, cy - (dh * k) / 2, dw * k, dh * k);
  }
  ctx.restore();
}

// ---------------------------------------------------------------------------
// Titles + subtitles (mirrors graph.rs drawtext_for: 1080p-relative scaling)
// ---------------------------------------------------------------------------

export interface SubtitleItem {
  content: string;
  style: TextStyle;
  spans?: { text: string; active: boolean }[];
}

// ---------------------------------------------------------------------------
// Line wrapping — the SAME algorithm graph.rs uses (real per-word measurement,
// greedy fill, never split a word), so playback wraps exactly where the export
// wraps. Measuring per word (not per string) is what keeps the two engines
// agreeing: kerning inside a word can differ, but the break decision cannot.
// ---------------------------------------------------------------------------

/** Usable fraction of the frame width for a caption (mirror of graph.rs). */
const CAPTION_WIDTH_FRACTION = 0.86;

function wrapWords(ctx: CanvasRenderingContext2D, words: string[], maxW: number): string[][] {
  if (!words.length) return [];
  const space = ctx.measureText(" ").width;
  const lines: string[][] = [];
  let line: string[] = [];
  let width = 0;
  for (const w of words) {
    const ww = ctx.measureText(w).width;
    const add = line.length ? space + ww : ww;
    if (line.length && width + add > maxW) {
      lines.push(line);
      line = [w];
      width = ww;
    } else {
      width += add;
      line.push(w);
    }
  }
  if (line.length) lines.push(line);
  return lines;
}

/** Vertical offset of line `i` of `n`: the block stays centred on y_offset. */
function lineYOffset(i: number, n: number, px: number, lineHeight: number): number {
  return (i - (n - 1) / 2) * px * Math.max(0.6, lineHeight || 1.2);
}

function textX(style: TextStyle, w: number, scale: number, textW: number): number {
  const xOff = style.x_offset * scale;
  const margin = 48 * scale;
  switch (style.align) {
    case "left":
      return margin + xOff;
    case "right":
      return w - textW - (margin - xOff);
    default:
      return (w - textW) / 2 + xOff;
  }
}

function setFont(ctx: CanvasRenderingContext2D, style: TextStyle, px: number) {
  const family =
    style.font && style.font !== "sans-serif" ? `"${style.font}", sans-serif` : "sans-serif";
  ctx.font = `${Math.round(px)}px ${family}`;
}

/** drawtext parity: borderw=2*scale black@0.6 outline, then the fill. */
function strokedText(
  ctx: CanvasRenderingContext2D,
  text: string,
  x: number,
  y: number,
  scale: number,
  fill: string,
) {
  ctx.lineJoin = "round";
  ctx.lineWidth = Math.max(1, 4 * scale);
  ctx.strokeStyle = "rgba(0,0,0,0.6)";
  ctx.strokeText(text, x, y);
  ctx.fillStyle = fill;
  ctx.fillText(text, x, y);
}

export function drawOverlays(
  ctx: CanvasRenderingContext2D,
  w: number,
  h: number,
  texts: Clip[],
  subtitles: SubtitleItem[] = [],
  guides = true,
) {
  if (guides) {
    // rule of thirds, subtle (monitor aid only; never exported)
    ctx.strokeStyle = "rgba(255,255,255,0.05)";
    ctx.lineWidth = 1;
    for (const f of [1 / 3, 2 / 3]) {
      ctx.beginPath();
      ctx.moveTo(w * f, 0);
      ctx.lineTo(w * f, h);
      ctx.stroke();
      ctx.beginPath();
      ctx.moveTo(0, h * f);
      ctx.lineTo(w, h * f);
      ctx.stroke();
    }
  }

  // export scale: sizes/offsets are 1080p-relative → on this canvas h/1080
  const scale = h / 1080;
  ctx.textAlign = "left";
  ctx.textBaseline = "middle";

  const maxW = w * CAPTION_WIDTH_FRACTION;

  for (const sub of subtitles) {
    const px = sub.style.size * scale;
    setFont(ctx, sub.style, px);
    const y0 = h / 2 + sub.style.y_offset * scale;
    const lh = sub.style.line_height;
    if (sub.spans?.length) {
      // karaoke: wrap the words, then place each one by (line, x)
      const space = ctx.measureText(" ").width;
      const lines = wrapWords(ctx, sub.spans.map((sp) => sp.text), maxW);
      const [r, g, b] = hexRgb(sub.style.color || "#ffffff");
      const dim = `rgba(${r},${g},${b},0.4)`;
      const hi = sub.style.highlight_color ?? "#FFB224";
      let idx = 0;
      lines.forEach((line, li) => {
        const widths = line.map((t) => ctx.measureText(t).width);
        const total = widths.reduce((a, b2) => a + b2, 0) + space * (line.length - 1);
        let x = (w - total) / 2 + sub.style.x_offset * scale;
        const y = y0 + lineYOffset(li, lines.length, px, lh);
        line.forEach((t, i) => {
          const sp = sub.spans![idx++];
          strokedText(ctx, t, x, y, scale, sp?.active ? hi : dim);
          x += widths[i] + space;
        });
      });
    } else {
      const lines = wrapWords(ctx, sub.content.split(/\s+/).filter(Boolean), maxW);
      lines.forEach((line, li) => {
        const text = line.join(" ");
        const textW = ctx.measureText(text).width;
        const y = y0 + lineYOffset(li, lines.length, px, lh);
        strokedText(ctx, text, textX(sub.style, w, scale, textW), y, scale, sub.style.color || "#ffffff");
      });
    }
  }

  for (const t of texts) {
    if (t.payload.type !== "text") continue;
    const { content, style } = t.payload;
    if (!content.trim()) continue;
    const px = style.size * scale;
    setFont(ctx, style, px);
    const y0 = h / 2 + style.y_offset * scale;
    const lines = wrapWords(ctx, content.split(/\s+/).filter(Boolean), maxW);
    lines.forEach((line, li) => {
      const text = line.join(" ");
      const textW = ctx.measureText(text).width;
      const y = y0 + lineYOffset(li, lines.length, px, style.line_height);
      strokedText(ctx, text, textX(style, w, scale, textW), y, scale, style.color || "#ffffff");
    });
  }
}

// ---------------------------------------------------------------------------
// Frame composition
// ---------------------------------------------------------------------------

interface ResolvedLayer {
  layer: Layer;
  pixels: LayerPixels | null;
}

/** Rotation-aware logical size of a media asset (probe dims), so proxies and
 *  scaled fallback frames never change the layout. */
function assetDisplaySize(project: Project, assetId: Id): [number, number] | null {
  const probe = project.assets.find((a) => a.id === assetId)?.probe;
  if (!probe || probe.width <= 0 || probe.height <= 0) return null;
  const rot = ((probe.rotation % 360) + 360) % 360;
  return rot === 90 || rot === 270 ? [probe.height, probe.width] : [probe.width, probe.height];
}

/**
 * Composites the frame at `playheadUs`: gathers every layer's pixels first
 * (async decodes), then draws bottom-to-top in one synchronous pass so the
 * previous frame stays on screen until the new one is fully ready.
 * Returns whether any video layer was present.
 */
export async function compositeFrame(
  ctx: CanvasRenderingContext2D,
  w: number,
  h: number,
  project: Project,
  seq: Sequence,
  playheadUs: number,
  sources: FrameSources,
): Promise<boolean> {
  const seqRes = seq.resolution;
  const layers = videoLayers(project, seq, playheadUs);

  const resolved: ResolvedLayer[] = await Promise.all(
    layers.map(async (layer): Promise<ResolvedLayer> => {
      if (layer.kind === "generator") return { layer, pixels: generatorPixels(layer.clip) };
      const assetId = layer.assetId!;
      const display = assetDisplaySize(project, assetId);
      if (layer.kind === "image") {
        const img = sources.image(assetId);
        if (!img) return { layer, pixels: null };
        return {
          layer,
          pixels: display ? { source: img.source, sw: display[0], sh: display[1] } : img,
        };
      }
      const frame = await sources.video(assetId, layer.clip.id, (layer.srcUs ?? 0) / 1e6);
      if (!frame) return { layer, pixels: null };
      return {
        layer,
        pixels: display ? { source: frame.source, sw: display[0], sh: display[1] } : frame,
      };
    }),
  );

  ctx.fillStyle = "#000";
  ctx.fillRect(0, 0, w, h);
  for (const { layer, pixels } of resolved) {
    if (!pixels) continue;
    const rel = playheadUs - layer.clip.start;
    // xfade parity: the incoming base frame is a full padded canvas, so its
    // black bars fade in over the outgoing clip too, not just the image
    if (layer.isBase && layer.alphaMul < 1) {
      ctx.save();
      ctx.globalAlpha = layer.alphaMul;
      ctx.fillStyle = "#000";
      ctx.fillRect(0, 0, w, h);
      ctx.restore();
    }
    drawMediaLayer(ctx, w, h, seqRes, pixels, layer, Math.max(0, rel), layerEffects(layer.clip, Math.max(0, rel)));
  }

  return layers.length > 0;
}

// ---------------------------------------------------------------------------
// Styled text: a title/subtitles clip carrying effects or a transform is NOT a
// burn-in — it is a layer, and it goes through the very same source pipeline
// and transform a media layer does (mirror of graph.rs `text_is_styled_pub`).
// ---------------------------------------------------------------------------

/** Does this text clip carry effects or a non-default transform? */
export function textIsStyled(clip: Clip): boolean {
  if (clip.effects.some((e) => e.enabled)) return true;
  const t = clip.transform;
  return (
    paramValue(t.position[0]) !== 0 ||
    paramValue(t.position[1]) !== 0 ||
    paramValue(t.scale[0]) !== 1 ||
    paramValue(t.scale[1]) !== 1 ||
    paramValue(t.rotation) !== 0 ||
    paramValue(t.opacity) !== 1 ||
    t.flip_h ||
    t.flip_v ||
    t.crop.some((c) => paramValue(c) !== 0)
  );
}

/**
 * Draws ONE styled text/subtitles clip: rasterise it to a frame-sized RGBA
 * scratch (so the export's 1080p-relative sizing still holds), then push that
 * through the clip's effects and transform like any other layer.
 */
export function drawStyledText(
  ctx: CanvasRenderingContext2D,
  w: number,
  h: number,
  seqRes: [number, number],
  clip: Clip,
  subtitle: SubtitleItem | null,
  rel: number,
) {
  const [cw, ch] = seqRes;
  const dpr = ctx.getTransform().a || 1;
  const k = Math.min(1, Math.max(0.2, (w / cw) * dpr));
  const tw = Math.max(2, Math.round(cw * k));
  const th = Math.max(2, Math.round(ch * k));
  const tctx = scratch(`${clip.id}:text`, tw, th);
  if (!tctx) return;
  drawOverlays(
    tctx,
    tw,
    th,
    clip.payload.type === "text" ? [clip] : [],
    subtitle ? [subtitle] : [],
    false, // no guides on a layer
  );
  drawMediaLayer(
    ctx,
    w,
    h,
    seqRes,
    { source: tctx.canvas, sw: cw, sh: ch },
    { kind: "media", clip, isBase: false, alphaMul: 1 },
    rel,
    layerEffects(clip, rel),
  );
}
