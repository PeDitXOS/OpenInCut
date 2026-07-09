import { useEffect, useRef, useState } from "react";

import type { Clip, Project } from "../engine/types";
import { activeSequence, activeSubtitleText, assetName } from "../engine/types";
import { frameToUs, hash32, usToTimecode } from "../lib/time";
import { engine, useStore } from "../state/store";

/** RMS → posición 0..1 en escala dB (-60..0). */
function meterFill(rms: number): number {
  if (rms <= 0) return 0;
  const db = 20 * Math.log10(rms);
  return Math.min(1, Math.max(0, (db + 60) / 60));
}

/** Medidores L/R compactos (escala dB, rojo por encima de -6 dB). */
function AudioMeters() {
  const meterL = useStore((s) => s.meterL);
  const meterR = useStore((s) => s.meterR);
  return (
    <div className="flex w-24 flex-col gap-0.5" title="Nivel RMS (dBFS)">
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
 * Monitor de programa. Dos modos:
 * - Escritorio (Tauri): frame REAL extraído por ffmpeg (ue-media) + overlays.
 * - Navegador (mock): representación esquemática del clip activo.
 * En ambos, los textos activos y las guías se dibujan encima; el motor wgpu
 * de la Fase 2 sustituirá la fuente del frame, no este componente.
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
    subtitles,
  };
}

function drawOverlays(
  ctx: CanvasRenderingContext2D,
  w: number,
  h: number,
  texts: Clip[],
  subtitles: { content: string; style: { size: number; color: string; y_offset: number } }[] = [],
) {
  // regla de tercios, sutil
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
    ctx.fillStyle = sub.style.color;
    ctx.fillText(sub.content, w / 2, y);
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
    ctx.fillText("Sin señal en este punto", w / 2, h / 2);
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

  // Frame real (solo escritorio).
  // Reproduciendo: stream continuo del FrameService a ~24 fps (playback_frame).
  useEffect(() => {
    if (engine.kind !== "tauri" || !playing) return;
    let alive = true;
    const id = window.setInterval(async () => {
      try {
        const bytes = await engine.playbackFrame();
        if (!alive) return;
        if (bytes) setRealFrame(await toBitmap(bytes));
      } catch {
        /* mantener el último frame */
      }
    }, 1000 / 24);
    return () => {
      alive = false;
      window.clearInterval(id);
    };
  }, [playing]);

  // En pausa/seek: un frame de alta calidad con debounce corto (render_frame).
  useEffect(() => {
    if (engine.kind !== "tauri" || playing) return;
    const req = ++frameReqRef.current;
    const handle = window.setTimeout(async () => {
      try {
        const bytes = await engine.renderFrame(playheadUs, 1280);
        if (frameReqRef.current !== req) return; // llegó tarde
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

  // Dibujo
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

    const { video, texts, subtitles } = activeClips(project, playheadUs);

    if (engine.kind === "tauri" && realFrame) {
      // frame real letterboxeado
      ctx.fillStyle = "#000";
      ctx.fillRect(0, 0, w, h);
      const scale = Math.min(w / realFrame.width, h / realFrame.height);
      const dw = realFrame.width * scale;
      const dh = realFrame.height * scale;
      ctx.drawImage(realFrame, (w - dw) / 2, (h - dh) / 2, dw, dh);
      badge(ctx, w, h, "FRAME REAL");
    } else if (engine.kind === "tauri" && !video) {
      ctx.fillStyle = "#0a0908";
      ctx.fillRect(0, 0, w, h);
      ctx.fillStyle = "rgba(164,155,143,0.35)";
      ctx.font = `500 ${Math.round(h * 0.05)}px "Space Grotesk", sans-serif`;
      ctx.textAlign = "center";
      ctx.fillText("Sin señal en este punto", w / 2, h / 2);
    } else if (engine.kind === "tauri") {
      // hay clip pero el frame aún no llegó
      ctx.fillStyle = "#000";
      ctx.fillRect(0, 0, w, h);
      badge(ctx, w, h, "CARGANDO…");
    } else {
      drawMockVideo(ctx, w, h, project, video);
      badge(ctx, w, h, "PREVIEW ½");
    }
    drawOverlays(ctx, w, h, texts, subtitles);
  }, [project, playheadUs, version, parentSize, realFrame]);

  const frameStep = (n: number) => seek(playheadUs + frameToUs(n, fps));

  return (
    <div className="flex h-full flex-col">
      <div className="flex min-h-0 flex-1 items-center justify-center p-3">
        <canvas ref={canvasRef} className="rounded-md shadow-[0_0_0_1px_var(--color-line)]" />
      </div>

      <div className="flex items-center gap-4 border-t border-line-soft px-4 py-2.5">
        {/* Firma: el timecode manda. Ámbar, mono, grande. */}
        <div
          className="font-[var(--font-mono)] text-[26px] font-medium tabular-nums tracking-tight text-accent"
          title="Posición actual"
        >
          {usToTimecode(playheadUs, fps)}
        </div>

        <div className="flex-1" />

        <div className="flex items-center gap-1">
          <TransportButton label="⏮" title="Ir al inicio (Inicio)" onClick={() => seek(0)} />
          <TransportButton label="◀︎" title="Frame anterior (←)" onClick={() => frameStep(-1)} />
          <TransportButton
            label={playing ? "❚❚" : "▶"}
            title="Reproducir/Pausa (Espacio)"
            onClick={togglePlay}
            primary
          />
          <TransportButton label="▶︎" title="Frame siguiente (→)" onClick={() => frameStep(1)} />
          <TransportButton
            label="⏭"
            title="Ir al final"
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
          {engine.kind === "tauri" && <AudioMeters />}
          <span className="rounded-md border border-line px-2 py-1">
            {engine.kind === "tauri" ? "Motor: escritorio" : "Motor: navegador"}
          </span>
          <span className="font-[var(--font-mono)]">0 drops</span>
        </div>
      </div>
    </div>
  );
}
