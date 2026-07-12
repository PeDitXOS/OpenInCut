import { useEffect, useRef, useState } from "react";

import type { Clip, Id, Project } from "../engine/types";
import { activeSequence, activeSubtitleText, assetName } from "../engine/types";
import {
  canvasFilterSupported,
  compositeFrame,
  drawStyledText,
  probeFilter,
  textIsStyled,
  drawMediaLayer,
  drawOverlays,
  generatorPixels,
  layerEffects,
  videoLayers,
  type FrameSources,
} from "../engine/compositor";
import { frameToUs, hash32, usToTimecode } from "../lib/time";
import { videoCache } from "../engine/video-cache";
import { engine, useStore } from "../state/store";

/** RMS → position 0..1 on a dB scale (-60..0). */
function meterFill(rms: number): number {
  if (rms <= 0) return 0;
  const db = 20 * Math.log10(rms);
  return Math.min(1, Math.max(0, (db + 60) / 60));
}

/** JKL indicator when speed is not 1× (J reverse, L faster, K stop). */
function ShuttleBadge() {
  const rate = useStore((s) => s.shuttleRate);
  const playing = useStore((s) => s.playing);
  if (!playing || rate === 1) return null;
  return (
    <span className="rounded-md border border-(--color-accent) px-2 py-1 font-[var(--font-mono)] text-[11px] text-(--color-accent)">
      {rate < 0 ? "◀" : "▶"} {Math.abs(rate)}×
    </span>
  );
}

/** Compact L/R meters (dB scale, red above -6 dB). */
function AudioMeters() {
  const meterL = useStore((s) => s.meterL);
  const meterR = useStore((s) => s.meterR);
  return (
    <div className="flex w-24 flex-col gap-0.5" title="RMS level (dBFS)">
      {[meterL, meterR].map((m, i) => {
        const fill = meterFill(m);
        return (
          <div key={i} className="h-1.5 overflow-hidden rounded-sm bg-bg3">
            <div
              className="h-full rounded-sm"
              style={{
                width: `${fill * 100}%`,
                background:
                  fill > 0.9
                    ? "var(--color-danger, #e5484d)"
                    : "linear-gradient(90deg, #46a758, #ffb224)",
              }}
            />
          </div>
        );
      })}
    </div>
  );
}

/**
 * Program monitor.
 *
 * Desktop (Tauri): the frame is composited in the webview by the shared
 * compositor module (engine/compositor.ts), whose rules mirror the export's
 * ffmpeg graph — ONE renderer for paused and playing. Pixels come from
 * mediabunny (videoCache), with a backend ffmpeg fallback for codecs the
 * webview cannot decode. The Rust audio engine stays the master clock.
 * Browser (mock): a schematic representation of the active clip.
 */

/** Texts + resolved subtitles active at the playhead (drawn on top). */
function activeOverlays(project: Project, playheadUs: number) {
  const seq = activeSequence(project);
  const clips = seq.tracks
    .filter((t) => t.kind === "video" && !t.muted)
    .flatMap((t) => t.clips)
    .filter((c) => c.start <= playheadUs && playheadUs < c.start + c.duration);
  // A text clip with effects or a transform is a LAYER (same as the export),
  // so it is drawn separately through the effect/transform chain. Plain ones
  // are burned on top as before.
  const subtitleClips = clips.filter((c) => c.payload.type === "subtitles");
  const textClips = clips.filter((c) => c.payload.type === "text");
  const resolved = subtitleClips
    .map((c) => ({ clip: c, item: activeSubtitleText(project, c, playheadUs) }))
    .filter((x): x is { clip: Clip; item: NonNullable<typeof x.item> } => x.item !== null);
  const styled = [
    ...textClips.filter(textIsStyled).map((clip) => ({ clip, item: null })),
    ...resolved.filter((x) => textIsStyled(x.clip)),
  ];
  return {
    texts: textClips.filter((c) => !textIsStyled(c)),
    subtitles: resolved.filter((x) => !textIsStyled(x.clip)).map((x) => x.item),
    styled,
  };
}

/**
 * Where the preview's time goes. Decoding and compositing are timed apart
 * because they fail differently: a slow composite is our maths, a slow decode
 * is the media pipeline — and guessing which one is why "it's at 1 fps" kept
 * getting misdiagnosed.
 */
const perf = { decodeMs: 0, drawMs: 0, frames: 0, layers: 0 };

/** Pixel sources for the compositor, backed by videoCache. */
const frameSources: FrameSources = {
  video: async (assetId: Id, clipId: Id, timeSec: number) => {
    const t0 = performance.now();
    const f = await videoCache.getFrameAt(assetId, clipId, timeSec);
    perf.decodeMs += performance.now() - t0;
    perf.layers += 1;
    return f;
  },
  image: (assetId: Id) => {
    const img = videoCache.getImage(assetId);
    return img ? { source: img, sw: img.naturalWidth, sh: img.naturalHeight } : null;
  },
};

function drawMockVideo(
  ctx: CanvasRenderingContext2D,
  w: number,
  h: number,
  project: Project,
  video: Clip | undefined,
) {
  if (video && video.payload.type === "media") {
    const asset = project.assets.find(
      (a) => a.id === (video.payload as { asset_id: string }).asset_id,
    );
    const seed = hash32(asset?.path ?? "x");
    const hue = 175 + (seed % 60) - 30;
    const g = ctx.createLinearGradient(0, 0, w, h);
    g.addColorStop(0, `hsl(${hue} 22% 22%)`);
    g.addColorStop(1, `hsl(${hue + 25} 26% 12%)`);
    ctx.fillStyle = g;
    ctx.fillRect(0, 0, w, h);
    ctx.fillStyle = "rgba(255,255,255,0.05)";
    ctx.beginPath();
    ctx.arc(w * 0.5, h * 0.44, h * 0.2, 0, Math.PI * 2);
    ctx.fill();
    ctx.fillRect(0, h * 0.72, w, 1.5);
    ctx.fillStyle = "rgba(233,228,219,0.6)";
    ctx.font = `500 ${Math.round(h * 0.045)}px "JetBrains Mono", monospace`;
    ctx.textAlign = "left";
    ctx.fillText(assetName(asset), h * 0.05, h * 0.09);
  } else {
    ctx.fillStyle = "#0a0908";
    ctx.fillRect(0, 0, w, h);
    ctx.fillStyle = "rgba(164,155,143,0.35)";
    ctx.font = `500 ${Math.round(h * 0.05)}px "Space Grotesk", sans-serif`;
    ctx.textAlign = "center";
    ctx.fillText("No signal at this point", w / 2, h / 2);
  }
}

function badge(ctx: CanvasRenderingContext2D, w: number, h: number, text: string) {
  ctx.fillStyle = "rgba(0,0,0,0.45)";
  ctx.font = `500 ${Math.round(h * 0.035)}px "JetBrains Mono", monospace`;
  ctx.textAlign = "left";
  const bw = ctx.measureText(text).width + h * 0.03;
  ctx.fillRect(w - bw - h * 0.04, h * 0.045, bw, h * 0.06);
  ctx.fillStyle = "rgba(233,228,219,0.75)";
  ctx.fillText(text, w - bw - h * 0.04 + h * 0.015, h * 0.09);
}

function TransportButton({
  label,
  title,
  onClick,
  primary,
}: {
  label: string;
  title: string;
  onClick: () => void;
  primary?: boolean;
}) {
  return (
    <button
      className={
        primary
          ? "focus-ring flex h-9 w-11 items-center justify-center rounded-lg bg-accent text-[15px] text-bg0 hover:bg-accent-deep"
          : "focus-ring flex h-8 w-9 items-center justify-center rounded-md text-[13px] text-ink-dim hover:bg-bg3 hover:text-ink"
      }
      title={title}
      onClick={onClick}
    >
      {label}
    </button>
  );
}

export function Preview() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const sizeRef = useRef({ w: 0, h: 0 });
  // Export-exact paused frame (rendered by the backend with the SAME ffmpeg
  // graph the export runs). `bitmap: null` = "nothing to show for this key,
  // keep the approximation" (avoids refetch loops).
  const exactRef = useRef<{ key: string; bitmap: ImageBitmap | null } | null>(null);
  const exactQueuedRef = useRef<string | null>(null);
  const exactTimerRef = useRef<number>(0);
  const exactLiveRef = useRef(false);
  const [exactLive, setExactLive] = useState(false);

  const project = useStore((s) => s.project);
  const playheadUs = useStore((s) => s.playheadUs);
  const playing = useStore((s) => s.playing);
  const togglePlay = useStore((s) => s.togglePlay);
  const seek = useStore((s) => s.seek);

  const fps = activeSequence(project).fps;

  useEffect(() => {
    const parent = canvasRef.current?.parentElement;
    if (!parent) return;
    const obs = new ResizeObserver((entries) => {
      const r = entries[0].contentRect;
      sizeRef.current = { w: r.width, h: r.height };
    });
    obs.observe(parent);
    return () => obs.disconnect();
  }, []);

  // Which blur path this webview gives us — reported for BOTH canvas kinds,
  // because asking for `willReadFrequently` silently downgrades to a software
  // canvas that ignores filters, and probing on one of those slandered the
  // whole engine as "no filter support".
  useEffect(() => {
    const gpu = probeFilter(false);
    const cpu = probeFilter(true);
    engine.uiLog(
      "info",
      `blur probe — accelerated canvas: ${gpu.blurred ? "BLURS" : "ignores filter"} ` +
        `(mid=${gpu.mid}, reads back "${gpu.reads}") · ` +
        `willReadFrequently canvas: ${cpu.blurred ? "BLURS" : "ignores filter"} (mid=${cpu.mid})`,
    );
    engine.uiLog(
      "info",
      `preview blur path: ${canvasFilterSupported() ? "ctx.filter (GPU, native)" : "mip downscale (GPU, no JS loops)"}`,
    );
  }, []);

  // ONE renderer (the OpenCut model): a single rAF loop composites the current
  // timeline state whether paused or playing. The render is async — it awaits
  // the frame decodes, then draws in one pass — so the previous frame stays
  // until the next is ready (no flicker). The Rust audio engine remains the
  // master clock for the playhead.
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    let running = true;
    let inFlight = false;
    let lastW = 0;
    let lastH = 0;
    let lastDpr = 0;

    const markExact = (v: boolean) => {
      if (exactLiveRef.current !== v) {
        exactLiveRef.current = v;
        setExactLive(v);
      }
    };

    // Debounced fetch of the export-exact frame: fires once the playhead /
    // project / size stop changing. The approximation stays on screen until
    // the exact bitmap arrives (no flicker, no black flash).
    const scheduleExact = (key: string, tUs: number, px: number) => {
      if (exactQueuedRef.current === key) return;
      exactQueuedRef.current = key;
      window.clearTimeout(exactTimerRef.current);
      exactTimerRef.current = window.setTimeout(async () => {
        try {
          const bytes = await engine.renderFrame(tUs, px);
          const bitmap = bytes
            ? await createImageBitmap(new Blob([bytes as unknown as BlobPart]))
            : null;
          if (!running) return;
          exactRef.current?.bitmap?.close();
          exactRef.current = { key, bitmap };
        } catch (e) {
          engine.uiLog("warn", `exact frame: ${e instanceof Error ? e.message : e}`);
          exactRef.current = { key, bitmap: null };
        }
      }, 160);
    };

    const render = async () => {
      const { w: pw, h: ph } = sizeRef.current;
      if (pw === 0) return;
      const s = useStore.getState();
      const seq = activeSequence(s.project);
      const aspect = seq.resolution[0] / seq.resolution[1] || 16 / 9;
      const maxW = pw - 24;
      const maxH = ph - 24;
      let w = maxW;
      let h = w / aspect;
      if (h > maxH) {
        h = maxH;
        w = h * aspect;
      }
      if (w < 10 || h < 10) return;
      const dpr = window.devicePixelRatio || 1;
      if (w !== lastW || h !== lastH || dpr !== lastDpr) {
        canvas.width = Math.round(w * dpr);
        canvas.height = Math.round(h * dpr);
        canvas.style.width = `${w}px`;
        canvas.style.height = `${h}px`;
        lastW = w;
        lastH = h;
        lastDpr = dpr;
      }
      const ctx = canvas.getContext("2d");
      if (!ctx) return;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

      const { texts, subtitles, styled } = activeOverlays(s.project, s.playheadUs);

      if (engine.kind === "tauri") {
        // Paused: show the export-exact backend frame once it's ready (same
        // ffmpeg graph as the export — 1:1 for ANY effect pack, transitions
        // and text). While it renders, keep the live approximation on screen.
        const key = `${s.playheadUs}|${s.version}|${Math.round(w * dpr)}|${seq.id}`;
        if (!s.playing) {
          const hit = exactRef.current;
          if (hit?.key === key && hit.bitmap) {
            ctx.drawImage(hit.bitmap, 0, 0, w, h);
            drawOverlays(ctx, w, h, [], [], true); // guides only: text is burned in
            markExact(true);
            return;
          }
          if (hit?.key !== key) scheduleExact(key, s.playheadUs, Math.round(w * dpr));
        }
        markExact(false);
        const any = await compositeFrame(ctx, w, h, s.project, seq, s.playheadUs, frameSources);
        if (!running) return;
        if (!any && !texts.length && !subtitles.length && !styled.length) {
          ctx.fillStyle = "#0a0908";
          ctx.fillRect(0, 0, w, h);
          ctx.fillStyle = "rgba(164,155,143,0.35)";
          ctx.font = `500 ${Math.round(h * 0.05)}px "Space Grotesk", sans-serif`;
          ctx.textAlign = "center";
          ctx.fillText("No signal at this point", w / 2, h / 2);
        }
      } else {
        ctx.clearRect(0, 0, w, h);
        const topVideo = [...seq.tracks]
          .reverse()
          .filter((t) => t.kind === "video" && !t.muted)
          .flatMap((t) => t.clips)
          .find(
            (c) =>
              c.payload.type === "media" &&
              c.start <= s.playheadUs &&
              s.playheadUs < c.start + c.duration,
          );
        drawMockVideo(ctx, w, h, s.project, topVideo);
        for (const layer of videoLayers(s.project, seq, s.playheadUs)) {
          if (layer.kind !== "generator") continue;
          const pixels = generatorPixels(layer.clip);
          if (!pixels) continue;
          const rel = Math.max(0, s.playheadUs - layer.clip.start);
          drawMediaLayer(ctx, w, h, seq.resolution, pixels, layer, rel, layerEffects(layer.clip, rel));
        }
        badge(ctx, w, h, "PREVIEW ½");
      }
      // styled text goes through effects + transform, plain text is burned on top
      for (const { clip, item } of styled) {
        drawStyledText(ctx, w, h, seq.resolution, clip, item, Math.max(0, s.playheadUs - clip.start));
      }
      drawOverlays(ctx, w, h, texts, subtitles);
    };

    const tick = () => {
      if (!running) return;
      raf = requestAnimationFrame(tick);
      if (inFlight) return;
      inFlight = true;
      const t0 = performance.now();
      const d0 = perf.decodeMs;
      // watchdog: a render that never settles freezes the preview forever
      // (the rAF loop is guarded by `inFlight`), and that looks exactly like
      // "10 seconds per frame". Say so out loud instead of guessing.
      const stuck = window.setTimeout(() => {
        engine.uiLog("warn", `preview: a frame has been rendering for 2 s (decode is the usual suspect)`);
      }, 2000);
      void render().finally(() => {
        window.clearTimeout(stuck);
        inFlight = false;
        const total = performance.now() - t0;
        const decode = perf.decodeMs - d0;
        perf.drawMs += total - decode;
        perf.frames += 1;
        if (perf.frames >= 10) {
          const f = perf.frames;
          const ms = (perf.decodeMs + perf.drawMs) / f;
          engine.uiLog(
            "info",
            `preview: ${ms.toFixed(1)} ms/frame (${(1000 / ms).toFixed(1)} fps) — ` +
              `decode ${(perf.decodeMs / f).toFixed(1)} · composite ${(perf.drawMs / f).toFixed(1)} · ` +
              `${(perf.layers / f).toFixed(1)} layers`,
          );
          perf.decodeMs = 0;
          perf.drawMs = 0;
          perf.frames = 0;
          perf.layers = 0;
        }
      });
    };
    let raf = requestAnimationFrame(tick);
    return () => {
      running = false;
      cancelAnimationFrame(raf);
      window.clearTimeout(exactTimerRef.current);
    };
  }, []);

  const frameStep = (n: number) => seek(playheadUs + frameToUs(n, fps));

  return (
    <div className="flex h-full flex-col">
      <div className="flex min-h-0 flex-1 items-center justify-center p-3">
        <canvas ref={canvasRef} className="rounded-md shadow-[0_0_0_1px_var(--color-line)]" />
      </div>

      <div className="flex items-center gap-4 border-t border-line-soft px-4 py-2.5">
        {/* Signature: the timecode rules. Amber, mono, large. */}
        <div
          className="font-[var(--font-mono)] text-[26px] font-medium tabular-nums tracking-tight text-accent"
          title="Current position"
        >
          {usToTimecode(playheadUs, fps)}
        </div>

        <div className="flex-1" />

        <div className="flex items-center gap-1">
          <TransportButton label="⏮" title="Go to start (Home)" onClick={() => seek(0)} />
          <TransportButton label="◀︎" title="Previous frame (←)" onClick={() => frameStep(-1)} />
          <TransportButton
            label={playing ? "❚❚" : "▶"}
            title="Play/Pause (Space)"
            onClick={togglePlay}
            primary
          />
          <TransportButton label="▶︎" title="Next frame (→)" onClick={() => frameStep(1)} />
          <TransportButton
            label="⏭"
            title="Go to end"
            onClick={() => {
              const seq = activeSequence(project);
              const end = Math.max(
                ...seq.tracks.flatMap((t) => t.clips.map((c) => c.start + c.duration)),
                0,
              );
              seek(end);
            }}
          />
        </div>

        <div className="flex-1" />

        <div className="flex items-center gap-2 text-[11px] text-ink-faint">
          <ShuttleBadge />
          {engine.kind === "tauri" && <AudioMeters />}
          {engine.kind === "tauri" && !playing && (
            <span
              className={
                exactLive
                  ? "rounded-md border border-(--color-accent) px-2 py-1 text-(--color-accent)"
                  : "rounded-md border border-line px-2 py-1"
              }
              title={
                exactLive
                  ? "This frame was rendered with the export's ffmpeg graph: what you see is exactly what exports"
                  : "Live approximation; the export-exact frame is rendering"
              }
            >
              {exactLive ? "1:1" : "≈"}
            </span>
          )}
          <span className="rounded-md border border-line px-2 py-1">
            {engine.kind === "tauri" ? "Engine: desktop" : "Engine: browser"}
          </span>
          <span className="font-[var(--font-mono)]">0 drops</span>
        </div>
      </div>
    </div>
  );
}
