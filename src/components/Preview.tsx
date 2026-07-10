import { useEffect, useRef, useState } from "react";

import type { Clip, Project } from "../engine/types";
import { activeSequence, activeSubtitleText, assetName } from "../engine/types";
import { paramValue } from "../engine/types";
import { frameToUs, hash32, usToTimecode } from "../lib/time";
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
 * Program monitor. Two modes:
 * - Desktop (Tauri): REAL frame extracted by ffmpeg (ue-media) + overlays.
 * - Browser (mock): schematic representation of the active clip.
 * In both, the active texts and guides are drawn on top; the Phase 2 wgpu
 * engine will replace the frame source, not this component.
 */

function activeClips(project: Project, playheadUs: number) {
  const seq = activeSequence(project);
  const topFirst = [...seq.tracks].reverse();
  const videoClips = topFirst
    .filter((t) => t.kind === "video" && !t.muted)
    .flatMap((t) => t.clips)
    .filter((c) => c.start <= playheadUs && playheadUs < c.start + c.duration);
  const subtitles = videoClips
    .filter((c) => c.payload.type === "subtitles")
    .map((c) => activeSubtitleText(project, c, playheadUs))
    .filter((s): s is NonNullable<typeof s> => s !== null);
  return {
    video: videoClips.find((c) => c.payload.type === "media"),
    texts: videoClips.filter((c) => c.payload.type === "text"),
    // bottom to top, the way the export composes
    generators: videoClips.filter((c) => c.payload.type === "generator").reverse(),
    subtitles,
  };
}

/** Draws the generator clips (rect/gradient) with their sampled transform. */
function drawGenerators(
  ctx: CanvasRenderingContext2D,
  w: number,
  h: number,
  seqRes: [number, number],
  generators: Clip[],
  playheadUs: number,
) {
  const sx = w / seqRes[0];
  for (const clip of generators) {
    if (clip.payload.type !== "generator") continue;
    const { generator_id, params, color_params } = clip.payload;
    const rel = Math.max(0, playheadUs - clip.start);
    const isGrad = generator_id === "core.gradient";
    const gw = paramValue(params["width"] ?? (isGrad ? 1920 : 640));
    const gh = paramValue(params["height"] ?? (isGrad ? 1080 : 360));
    const px = paramValue(clip.transform.position[0], rel);
    const py = paramValue(clip.transform.position[1], rel);
    const scale = paramValue(clip.transform.scale[0], rel);
    const opacity = paramValue(clip.transform.opacity, rel);
    const rw = gw * sx * scale;
    const rh = gh * sx * scale;
    const cx = w / 2 + px * sx;
    const cy = h / 2 + py * sx;
    ctx.save();
    ctx.globalAlpha = Math.max(0, Math.min(1, opacity));
    if (isGrad) {
      const g = ctx.createLinearGradient(cx - rw / 2, cy - rh / 2, cx + rw / 2, cy + rh / 2);
      g.addColorStop(0, color_params["color_a"] ?? "#ffb224");
      g.addColorStop(1, color_params["color_b"] ?? "#16130f");
      ctx.fillStyle = g;
    } else {
      ctx.fillStyle = color_params["color"] ?? "#ff3355";
    }
    ctx.fillRect(cx - rw / 2, cy - rh / 2, rw, rh);
    ctx.restore();
  }
}

function drawOverlays(
  ctx: CanvasRenderingContext2D,
  w: number,
  h: number,
  texts: Clip[],
  subtitles: {
    content: string;
    style: { size: number; color: string; y_offset: number; highlight_color?: string | null };
    spans?: { text: string; active: boolean }[];
  }[] = [],
) {
  // rule of thirds, subtle
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

  ctx.textAlign = "center";
  for (const sub of subtitles) {
    const size = (sub.style.size / 1080) * h;
    const y = h / 2 + (sub.style.y_offset / 1080) * h;
    ctx.font = `600 ${Math.round(size)}px "Inter", sans-serif`;
    ctx.shadowColor = "rgba(0,0,0,0.85)";
    ctx.shadowBlur = 8;
    if (sub.spans?.length) {
      // karaoke: centered phrase, words lit up at their time
      const space = ctx.measureText(" ").width;
      const widths = sub.spans.map((sp) => ctx.measureText(sp.text).width);
      const total = widths.reduce((a, b) => a + b, 0) + space * (sub.spans.length - 1);
      let x = w / 2 - total / 2;
      ctx.textAlign = "left";
      sub.spans.forEach((sp, i) => {
        ctx.fillStyle = sp.active
          ? (sub.style.highlight_color ?? "#FFB224")
          : "rgba(233,228,219,0.4)";
        ctx.fillText(sp.text, x, y);
        x += widths[i] + space;
      });
      ctx.textAlign = "center";
    } else {
      ctx.fillStyle = sub.style.color;
      ctx.fillText(sub.content, w / 2, y);
    }
    ctx.shadowBlur = 0;
  }
  for (const t of texts) {
    if (t.payload.type !== "text") continue;
    const content = t.payload.content;
    const isCta = content.length < 16;
    if (isCta) {
      ctx.font = `600 ${Math.round(h * 0.055)}px "Space Grotesk", sans-serif`;
      ctx.fillStyle = "#ffb224";
      ctx.fillText(content, w / 2, h * 0.86);
    } else {
      ctx.font = `700 ${Math.round(h * 0.075)}px "Space Grotesk", sans-serif`;
      ctx.shadowColor = "rgba(0,0,0,0.7)";
      ctx.shadowBlur = 12;
      ctx.fillStyle = "#e9e4db";
      ctx.fillText(content, w / 2, h * 0.62);
      ctx.shadowBlur = 0;
    }
  }
}

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

async function toBitmap(bytes: Uint8Array): Promise<ImageBitmap> {
  return createImageBitmap(
    new Blob([bytes.slice().buffer as ArrayBuffer], { type: "image/jpeg" }),
  );
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
  const [parentSize, setParentSize] = useState({ w: 0, h: 0 });
  const [realFrame, setRealFrame] = useState<ImageBitmap | null>(null);
  const frameReqRef = useRef(0);

  const project = useStore((s) => s.project);
  const playheadUs = useStore((s) => s.playheadUs);
  const playing = useStore((s) => s.playing);
  const togglePlay = useStore((s) => s.togglePlay);
  const seek = useStore((s) => s.seek);
  const version = useStore((s) => s.version);

  const fps = activeSequence(project).fps;

  useEffect(() => {
    const parent = canvasRef.current?.parentElement;
    if (!parent) return;
    const obs = new ResizeObserver((entries) => {
      const r = entries[0].contentRect;
      setParentSize({ w: r.width, h: r.height });
    });
    obs.observe(parent);
    return () => obs.disconnect();
  }, []);

  // Real frame (desktop only).
  // Playing: continuous stream from the FrameService at ~24 fps (playback_frame).
  useEffect(() => {
    if (engine.kind !== "tauri" || !playing) return;
    let alive = true;
    const id = window.setInterval(async () => {
      try {
        const bytes = await engine.playbackFrame();
        if (!alive) return;
        if (bytes) setRealFrame(await toBitmap(bytes));
      } catch {
        /* keep the last frame */
      }
    }, 1000 / 24);
    return () => {
      alive = false;
      window.clearInterval(id);
    };
  }, [playing]);

  // On pause/seek: a high-quality frame with a short debounce (render_frame).
  useEffect(() => {
    if (engine.kind !== "tauri" || playing) return;
    const req = ++frameReqRef.current;
    const handle = window.setTimeout(async () => {
      try {
        const bytes = await engine.renderFrame(playheadUs, 1280);
        if (frameReqRef.current !== req) return; // arrived late
        if (!bytes) {
          setRealFrame(null);
          return;
        }
        const bmp = await toBitmap(bytes);
        if (frameReqRef.current === req) setRealFrame(bmp);
      } catch {
        if (frameReqRef.current === req) setRealFrame(null);
      }
    }, 90);
    return () => window.clearTimeout(handle);
  }, [playheadUs, version, playing]);

  // Drawing
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || parentSize.w === 0) return;
    const maxW = parentSize.w - 24;
    const maxH = parentSize.h - 24;
    let w = maxW;
    let h = (w * 9) / 16;
    if (h > maxH) {
      h = maxH;
      w = (h * 16) / 9;
    }
    if (w < 10 || h < 10) return;
    const dpr = window.devicePixelRatio || 1;
    canvas.width = Math.round(w * dpr);
    canvas.height = Math.round(h * dpr);
    canvas.style.width = `${w}px`;
    canvas.style.height = `${h}px`;
    const ctx = canvas.getContext("2d")!;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

    const { video, texts, generators, subtitles } = activeClips(project, playheadUs);

    if (engine.kind === "tauri" && realFrame) {
      // real frame, letterboxed
      ctx.fillStyle = "#000";
      ctx.fillRect(0, 0, w, h);
      const scale = Math.min(w / realFrame.width, h / realFrame.height);
      const dw = realFrame.width * scale;
      const dh = realFrame.height * scale;
      ctx.drawImage(realFrame, (w - dw) / 2, (h - dh) / 2, dw, dh);
      badge(ctx, w, h, "REAL FRAME");
    } else if (engine.kind === "tauri" && !video) {
      ctx.fillStyle = "#0a0908";
      ctx.fillRect(0, 0, w, h);
      ctx.fillStyle = "rgba(164,155,143,0.35)";
      ctx.font = `500 ${Math.round(h * 0.05)}px "Space Grotesk", sans-serif`;
      ctx.textAlign = "center";
      ctx.fillText("No signal at this point", w / 2, h / 2);
    } else if (engine.kind === "tauri") {
      // there's a clip but the frame hasn't arrived yet
      ctx.fillStyle = "#000";
      ctx.fillRect(0, 0, w, h);
      badge(ctx, w, h, "LOADING…");
    } else {
      drawMockVideo(ctx, w, h, project, video);
      badge(ctx, w, h, "PREVIEW ½");
    }
    drawGenerators(ctx, w, h, activeSequence(project).resolution, generators, playheadUs);
    drawOverlays(ctx, w, h, texts, subtitles);
  }, [project, playheadUs, version, parentSize, realFrame]);

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
          <span className="rounded-md border border-line px-2 py-1">
            {engine.kind === "tauri" ? "Engine: desktop" : "Engine: browser"}
          </span>
          <span className="font-[var(--font-mono)]">0 drops</span>
        </div>
      </div>
    </div>
  );
}
