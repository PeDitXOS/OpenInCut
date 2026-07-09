import { useEffect, useRef, useState } from "react";

import type { Clip, Project } from "../engine/types";
import { frameToUs, hash32, usToTimecode } from "../lib/time";
import { useStore } from "../state/store";

/**
 * Monitor de programa (mock): dibuja una representación del frame actual a
 * partir del estado real del timeline — el clip de video superior activo y los
 * textos activos — para que preview y timeline cuenten la misma historia.
 * El motor wgpu real sustituirá el contenido del canvas, no el componente.
 */
function drawFrame(
  ctx: CanvasRenderingContext2D,
  w: number,
  h: number,
  project: Project,
  playheadUs: number,
) {
  ctx.clearRect(0, 0, w, h);
  ctx.fillStyle = "#000";
  ctx.fillRect(0, 0, w, h);

  const seq = project.sequences[0];
  const topFirst = [...seq.tracks].reverse();

  const activeVideo: Clip | undefined = topFirst
    .filter((t) => t.kind === "video" && !t.muted)
    .flatMap((t) => t.clips)
    .find(
      (c) =>
        c.payload.type === "media" &&
        c.start <= playheadUs &&
        playheadUs < c.start + c.duration,
    );

  const activeTexts: Clip[] = topFirst
    .filter((t) => t.kind === "video" && !t.muted)
    .flatMap((t) => t.clips)
    .filter(
      (c) =>
        c.payload.type === "text" &&
        c.start <= playheadUs &&
        playheadUs < c.start + c.duration,
    );

  if (activeVideo && activeVideo.payload.type === "media") {
    const asset = project.assets.find(
      (a) => a.id === (activeVideo.payload as { asset_id: string }).asset_id,
    );
    const seed = hash32(asset?.path ?? "x");
    const hue = 175 + (seed % 60) - 30; // azules/verdosos apagados de "material"
    const g = ctx.createLinearGradient(0, 0, w, h);
    g.addColorStop(0, `hsl(${hue} 22% 22%)`);
    g.addColorStop(1, `hsl(${hue + 25} 26% 12%)`);
    ctx.fillStyle = g;
    ctx.fillRect(0, 0, w, h);

    // sugerencia de contenido: círculo de "sujeto" + horizonte
    ctx.fillStyle = "rgba(255,255,255,0.05)";
    ctx.beginPath();
    ctx.arc(w * 0.5, h * 0.44, h * 0.2, 0, Math.PI * 2);
    ctx.fill();
    ctx.fillRect(0, h * 0.72, w, 1.5);

    ctx.fillStyle = "rgba(233,228,219,0.6)";
    ctx.font = `500 ${Math.round(h * 0.045)}px "JetBrains Mono", monospace`;
    ctx.textAlign = "left";
    ctx.fillText(asset?.path.split("/").pop() ?? "", h * 0.05, h * 0.09);

    ctx.fillStyle = "rgba(0,0,0,0.45)";
    const badge = "PREVIEW ½";
    ctx.font = `500 ${Math.round(h * 0.035)}px "JetBrains Mono", monospace`;
    const bw = ctx.measureText(badge).width + h * 0.03;
    ctx.fillRect(w - bw - h * 0.04, h * 0.045, bw, h * 0.06);
    ctx.fillStyle = "rgba(233,228,219,0.75)";
    ctx.fillText(badge, w - bw - h * 0.04 + h * 0.015, h * 0.09);
  } else {
    ctx.fillStyle = "#0a0908";
    ctx.fillRect(0, 0, w, h);
    ctx.fillStyle = "rgba(164,155,143,0.35)";
    ctx.font = `500 ${Math.round(h * 0.05)}px "Space Grotesk", sans-serif`;
    ctx.textAlign = "center";
    ctx.fillText("Sin señal en este punto", w / 2, h / 2);
  }

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

  // textos activos (títulos/CTA)
  ctx.textAlign = "center";
  for (const t of activeTexts) {
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
  const project = useStore((s) => s.project);
  const playheadUs = useStore((s) => s.playheadUs);
  const playing = useStore((s) => s.playing);
  const togglePlay = useStore((s) => s.togglePlay);
  const seek = useStore((s) => s.seek);
  const version = useStore((s) => s.version);

  const fps = project.sequences[0].fps;

  const [parentSize, setParentSize] = useState({ w: 0, h: 0 });

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

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || parentSize.w === 0) return;
    // encajar 16:9 en el hueco disponible
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
    drawFrame(ctx, w, h, project, playheadUs);
  }, [project, playheadUs, version, parentSize]);

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
              const seq = project.sequences[0];
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
          <span className="rounded-md border border-line px-2 py-1">Calidad: Auto</span>
          <span className="font-[var(--font-mono)]">0 drops</span>
        </div>
      </div>
    </div>
  );
}
