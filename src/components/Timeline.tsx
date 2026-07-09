import { useCallback, useEffect, useRef, useState } from "react";

import type { Clip, Project, Track } from "../engine/types";
import { activeSequence, clipDisplayName } from "../engine/types";
import { hash32, mulberry32, usToDuration } from "../lib/time";
import { useStore } from "../state/store";

const RULER_H = 26;
const TRACK_GAP = 2;
const trackHeight = (t: Track) => (t.kind === "video" ? 52 : 42);

const COLORS = {
  bg: "#1a1816",
  laneVideo: "#1e1c19",
  laneAudio: "#1b1a17",
  line: "#2b2823",
  ink: "#e9e4db",
  inkDim: "#a49b8f",
  inkFaint: "#6e675e",
  accent: "#ffb224",
  clipVideo: "#43626e",
  clipVideoHi: "#6fa3b5",
  clipAudio: "#4a5c3d",
  clipAudioHi: "#8fb573",
  clipText: "#63496b",
  clipTextHi: "#b088bd",
};

interface View {
  viewStartUs: number;
  pxPerSec: number;
}

type DragMode = "move" | "trim-left" | "trim-right";

interface DragGhost {
  clipId: string;
  mode: DragMode;
  startUs: number;
  /** para trim: posición actual del borde arrastrado (µs) */
  edgeUs: number;
  trackIdx: number; // índice en displayTracks
  moved: boolean;
}

function displayTracks(project: Project): Track[] {
  return [...activeSequence(project).tracks].reverse();
}

function trackTops(tracks: Track[]): number[] {
  const tops: number[] = [];
  let y = RULER_H + TRACK_GAP;
  for (const t of tracks) {
    tops.push(y);
    y += trackHeight(t) + TRACK_GAP;
  }
  return tops;
}

// ---------------------------------------------------------------------------
// Dibujo
// ---------------------------------------------------------------------------

function pickTickStep(pxPerSec: number): number {
  const candidates = [0.5, 1, 2, 5, 10, 15, 30, 60, 120, 300];
  for (const c of candidates) if (c * pxPerSec >= 72) return c;
  return 600;
}

function drawRuler(
  ctx: CanvasRenderingContext2D,
  w: number,
  view: View,
  project: Project,
) {
  ctx.fillStyle = "#161412";
  ctx.fillRect(0, 0, w, RULER_H);
  ctx.strokeStyle = COLORS.line;
  ctx.beginPath();
  ctx.moveTo(0, RULER_H - 0.5);
  ctx.lineTo(w, RULER_H - 0.5);
  ctx.stroke();

  const step = pickTickStep(view.pxPerSec);
  const usToX = (us: number) => ((us - view.viewStartUs) / 1e6) * view.pxPerSec;
  const firstTick = Math.floor(view.viewStartUs / 1e6 / step) * step;
  const lastUs = view.viewStartUs + (w / view.pxPerSec) * 1e6;

  ctx.font = '500 9.5px "JetBrains Mono", monospace';
  ctx.textBaseline = "middle";
  for (let s = firstTick; s * 1e6 <= lastUs; s += step / 5) {
    const x = Math.round(usToX(s * 1e6)) + 0.5;
    const major = Math.abs(s % step) < 1e-9;
    ctx.strokeStyle = major ? COLORS.inkFaint : COLORS.line;
    ctx.beginPath();
    ctx.moveTo(x, major ? RULER_H - 12 : RULER_H - 6);
    ctx.lineTo(x, RULER_H - 1);
    ctx.stroke();
    if (major && s >= 0) {
      ctx.fillStyle = COLORS.inkDim;
      ctx.textAlign = "left";
      ctx.fillText(usToDuration(s * 1e6), x + 4, 9);
    }
  }

  // marcadores de secuencia
  for (const m of activeSequence(project).markers) {
    const x = usToX(m.t);
    if (x < -10 || x > w + 10) continue;
    ctx.fillStyle = m.color ?? COLORS.inkDim;
    ctx.beginPath();
    ctx.moveTo(x, RULER_H - 1);
    ctx.lineTo(x - 4, RULER_H - 8);
    ctx.lineTo(x + 4, RULER_H - 8);
    ctx.closePath();
    ctx.fill();
  }
}

function roundedRect(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  w: number,
  h: number,
  r: number,
) {
  const rr = Math.min(r, w / 2, h / 2);
  ctx.beginPath();
  ctx.moveTo(x + rr, y);
  ctx.arcTo(x + w, y, x + w, y + h, rr);
  ctx.arcTo(x + w, y + h, x, y + h, rr);
  ctx.arcTo(x, y + h, x, y, rr);
  ctx.arcTo(x, y, x + w, y, rr);
  ctx.closePath();
}

function drawClip(
  ctx: CanvasRenderingContext2D,
  clip: Clip,
  track: Track,
  label: string,
  x: number,
  y: number,
  w: number,
  h: number,
  selected: boolean,
  ghost: boolean,
) {
  const isText = ["text", "subtitles", "avatar"].includes(clip.payload.type);
  const isAudio = track.kind === "audio";
  const base = isText ? COLORS.clipText : isAudio ? COLORS.clipAudio : COLORS.clipVideo;
  const hi = isText ? COLORS.clipTextHi : isAudio ? COLORS.clipAudioHi : COLORS.clipVideoHi;

  ctx.save();
  if (ghost) ctx.globalAlpha = 0.55;

  roundedRect(ctx, x, y, w, h, 5);
  ctx.fillStyle = base;
  ctx.fill();
  ctx.save();
  ctx.clip();

  const seed = hash32(clip.id);
  if (!isAudio && !isText) {
    // filmstrip determinista
    const rng = mulberry32(seed);
    const cell = 22;
    for (let cx = 0; cx < w; cx += cell) {
      const lum = 0.16 + rng() * 0.22;
      ctx.fillStyle = `rgba(12, 20, 24, ${lum})`;
      ctx.fillRect(x + cx, y, cell - 1.5, h - 14);
    }
    ctx.fillStyle = "rgba(0,0,0,0.28)";
    ctx.fillRect(x, y + h - 14, w, 14);
  } else if (isAudio) {
    // waveform determinista
    const rng = mulberry32(seed);
    const mid = y + h / 2 + 3;
    const maxAmp = (h - 18) / 2;
    ctx.fillStyle = hi + "66";
    let prev = 0.4;
    for (let cx = 1.5; cx < w - 1.5; cx += 2.5) {
      const target = rng();
      prev = prev * 0.6 + target * 0.4;
      const amp = Math.max(1.2, prev * maxAmp * (0.55 + 0.45 * Math.sin(cx / 43)));
      ctx.fillRect(x + cx, mid - amp, 1.6, amp * 2);
    }
    // fades como rampas oscuras
    const fadeW = (us: number) => (us / 1e6) * ((w / clip.duration) * 1e6);
    if (clip.audio.fade_in_us > 0) {
      const fw = fadeW(clip.audio.fade_in_us);
      ctx.fillStyle = "rgba(0,0,0,0.4)";
      ctx.beginPath();
      ctx.moveTo(x, y);
      ctx.lineTo(x + fw, y);
      ctx.lineTo(x, y + h);
      ctx.closePath();
      ctx.fill();
    }
    if (clip.audio.fade_out_us > 0) {
      const fw = fadeW(clip.audio.fade_out_us);
      ctx.fillStyle = "rgba(0,0,0,0.4)";
      ctx.beginPath();
      ctx.moveTo(x + w, y);
      ctx.lineTo(x + w - fw, y);
      ctx.lineTo(x + w, y + h);
      ctx.closePath();
      ctx.fill();
    }
  } else {
    // clip de texto
    ctx.fillStyle = "rgba(0,0,0,0.25)";
    ctx.fillRect(x, y + h - 14, w, 14);
    ctx.fillStyle = hi;
    ctx.font = '700 13px "Space Grotesk", sans-serif';
    ctx.textAlign = "left";
    ctx.textBaseline = "middle";
    ctx.fillText("T", x + 7, y + (h - 14) / 2 + 1);
    if (clip.payload.type === "subtitles" && w > 60) {
      ctx.fillStyle = "rgba(233,228,219,0.85)";
      ctx.font = '500 10px "Inter", sans-serif';
      ctx.fillText("Subtítulos automáticos", x + 20, y + (h - 14) / 2 + 1, w - 28);
    }
    if (clip.payload.type === "text" && w > 60) {
      ctx.fillStyle = "rgba(233,228,219,0.85)";
      ctx.font = '500 10px "Inter", sans-serif';
      ctx.fillText(clip.payload.content.slice(0, 28), x + 20, y + (h - 14) / 2 + 1, w - 28);
    }
  }

  // etiqueta
  if (label && w > 46) {
    ctx.fillStyle = "rgba(233,228,219,0.9)";
    ctx.font = '500 9.5px "Inter", sans-serif';
    ctx.textAlign = "left";
    ctx.textBaseline = "middle";
    ctx.fillText(label, x + 6, y + h - 7, w - 12);
  }
  ctx.restore();

  // borde
  roundedRect(ctx, x + 0.5, y + 0.5, w - 1, h - 1, 5);
  ctx.lineWidth = selected ? 1.6 : 1;
  ctx.strokeStyle = selected ? COLORS.accent : "rgba(0,0,0,0.5)";
  ctx.stroke();

  // asas de trim en la selección
  if (selected && w > 26) {
    ctx.fillStyle = COLORS.accent;
    roundedRect(ctx, x + 2.5, y + h / 2 - 7, 3.5, 14, 2);
    ctx.fill();
    roundedRect(ctx, x + w - 6, y + h / 2 - 7, 3.5, 14, 2);
    ctx.fill();
  }

  ctx.restore();
}

function drawTimeline(
  ctx: CanvasRenderingContext2D,
  w: number,
  h: number,
  project: Project,
  view: View,
  selection: string[],
  playheadUs: number,
  ghost: DragGhost | null,
) {
  ctx.clearRect(0, 0, w, h);
  ctx.fillStyle = COLORS.bg;
  ctx.fillRect(0, 0, w, h);

  const tracks = displayTracks(project);
  const tops = trackTops(tracks);
  const usToX = (us: number) => ((us - view.viewStartUs) / 1e6) * view.pxPerSec;

  // lanes
  tracks.forEach((t, i) => {
    const th = trackHeight(t);
    ctx.fillStyle = t.kind === "video" ? COLORS.laneVideo : COLORS.laneAudio;
    ctx.fillRect(0, tops[i], w, th);
    if (t.locked) {
      ctx.fillStyle = "rgba(0,0,0,0.25)";
      ctx.fillRect(0, tops[i], w, th);
    }
  });

  drawRuler(ctx, w, view, project);

  // clips
  tracks.forEach((t, i) => {
    const th = trackHeight(t);
    for (const clip of t.clips) {
      if (ghost && ghost.clipId === clip.id && ghost.moved) continue; // se dibuja como fantasma
      const x = usToX(clip.start);
      const cw = (clip.duration / 1e6) * view.pxPerSec;
      if (x + cw < 0 || x > w) continue;
      drawClip(
        ctx,
        clip,
        t,
        clipDisplayName(clip, project),
        x,
        tops[i] + 3,
        cw,
        th - 6,
        selection.includes(clip.id),
        false,
      );
    }
  });

  // fantasma de drag (mover o recortar)
  if (ghost && ghost.moved) {
    const found = activeSequence(project)
      .tracks.flatMap((t) => t.clips.map((c) => ({ c, t })))
      .find(({ c }) => c.id === ghost.clipId);
    if (found) {
      const t = tracks[ghost.trackIdx] ?? found.t;
      const th = trackHeight(t);
      let x: number;
      let cw: number;
      if (ghost.mode === "move") {
        x = usToX(ghost.startUs);
        cw = (found.c.duration / 1e6) * view.pxPerSec;
      } else if (ghost.mode === "trim-left") {
        const end = found.c.start + found.c.duration;
        const edge = Math.min(ghost.edgeUs, end - 33_333);
        x = usToX(edge);
        cw = ((end - edge) / 1e6) * view.pxPerSec;
      } else {
        const edge = Math.max(ghost.edgeUs, found.c.start + 33_333);
        x = usToX(found.c.start);
        cw = ((edge - found.c.start) / 1e6) * view.pxPerSec;
      }
      drawClip(
        ctx,
        found.c,
        t,
        clipDisplayName(found.c, project),
        x,
        tops[ghost.trackIdx] + 3,
        cw,
        th - 6,
        true,
        true,
      );
    }
  }

  // playhead (aguja ámbar, la firma)
  const px = usToX(playheadUs);
  if (px >= -8 && px <= w + 8) {
    ctx.strokeStyle = COLORS.accent;
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    ctx.moveTo(px, RULER_H - 4);
    ctx.lineTo(px, h);
    ctx.stroke();
    // capuchón
    ctx.fillStyle = COLORS.accent;
    ctx.beginPath();
    ctx.moveTo(px - 5.5, 4);
    ctx.lineTo(px + 5.5, 4);
    ctx.lineTo(px + 5.5, RULER_H - 10);
    ctx.lineTo(px, RULER_H - 3);
    ctx.lineTo(px - 5.5, RULER_H - 10);
    ctx.closePath();
    ctx.fill();
  }
}

// ---------------------------------------------------------------------------
// Cabeceras de pista (DOM)
// ---------------------------------------------------------------------------

function TrackHeader({ track }: { track: Track }) {
  const toggleTrack = useStore((s) => s.toggleTrack);
  const btn = (
    active: boolean,
    label: string,
    title: string,
    prop: "muted" | "solo" | "locked",
  ) => (
    <button
      className={`focus-ring h-5 w-5 rounded text-[9.5px] font-semibold leading-none ${
        active ? "bg-accent text-bg0" : "bg-bg3 text-ink-faint hover:text-ink"
      }`}
      title={title}
      onClick={() => void toggleTrack(track.id, prop)}
    >
      {label}
    </button>
  );

  return (
    <div
      className="flex items-center gap-1.5 border-b border-line-soft px-2"
      style={{ height: trackHeight(track) + TRACK_GAP }}
    >
      <span
        className={`w-7 shrink-0 font-[var(--font-display)] text-[12px] font-semibold ${
          track.kind === "video" ? "text-clip-video-hi" : "text-clip-audio-hi"
        }`}
      >
        {track.name}
      </span>
      {btn(track.muted, "M", "Silenciar pista", "muted")}
      {btn(track.solo, "S", "Solo", "solo")}
      {btn(track.locked, "🔒", "Bloquear pista", "locked")}
      {track.kind === "audio" && (
        <span className="ml-auto font-[var(--font-mono)] text-[9.5px] text-ink-faint">
          {track.volume_db > 0 ? "+" : ""}
          {track.volume_db} dB
        </span>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Componente principal
// ---------------------------------------------------------------------------

export function Timeline() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({ w: 800, h: 240 });
  const [ghost, setGhost] = useState<DragGhost | null>(null);
  const dragRef = useRef<{
    clipId: string;
    mode: DragMode;
    grabOffsetUs: number;
    startTrackIdx: number;
  } | null>(null);
  const scrubRef = useRef(false);

  const project = useStore((s) => s.project);
  const version = useStore((s) => s.version);
  const selection = useStore((s) => s.selection);
  const playheadUs = useStore((s) => s.playheadUs);
  const viewStartUs = useStore((s) => s.viewStartUs);
  const pxPerSec = useStore((s) => s.pxPerSec);
  const playing = useStore((s) => s.playing);
  const seek = useStore((s) => s.seek);
  const select = useStore((s) => s.select);
  const setView = useStore((s) => s.setView);
  const moveClip = useStore((s) => s.moveClip);
  const trimClip = useStore((s) => s.trimClip);
  const splitAtPlayhead = useStore((s) => s.splitAtPlayhead);
  const deleteSelection = useStore((s) => s.deleteSelection);
  const addTextClip = useStore((s) => s.addTextClip);
  const generateVertical = useStore((s) => s.generateVertical);
  const setActiveSequence = useStore((s) => s.setActiveSequence);

  // seguir el playhead durante la reproducción
  useEffect(() => {
    if (!playing) return;
    const rightEdge = viewStartUs + ((size.w - 80) / pxPerSec) * 1e6;
    if (playheadUs > rightEdge)
      setView(playheadUs - ((size.w * 0.15) / pxPerSec) * 1e6, pxPerSec);
  }, [playing, playheadUs, viewStartUs, pxPerSec, size.w, setView]);

  // observar tamaño
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const obs = new ResizeObserver((entries) => {
      const r = entries[0].contentRect;
      setSize({ w: Math.round(r.width), h: Math.round(r.height) });
    });
    obs.observe(el);
    return () => obs.disconnect();
  }, []);

  // dibujar
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const dpr = window.devicePixelRatio || 1;
    canvas.width = size.w * dpr;
    canvas.height = size.h * dpr;
    canvas.style.width = `${size.w}px`;
    canvas.style.height = `${size.h}px`;
    const ctx = canvas.getContext("2d")!;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    drawTimeline(
      ctx,
      size.w,
      size.h,
      project,
      { viewStartUs, pxPerSec },
      selection,
      playheadUs,
      ghost,
    );
  }, [project, version, selection, playheadUs, viewStartUs, pxPerSec, size, ghost]);

  const xToUs = useCallback(
    (x: number) => viewStartUs + (x / pxPerSec) * 1e6,
    [viewStartUs, pxPerSec],
  );

  const hitTest = useCallback(
    (x: number, y: number) => {
      const tracks = displayTracks(project);
      const tops = trackTops(tracks);
      const us = xToUs(x);
      for (let i = 0; i < tracks.length; i++) {
        if (y >= tops[i] && y < tops[i] + trackHeight(tracks[i])) {
          const clip = tracks[i].clips.find(
            (c) => c.start <= us && us < c.start + c.duration,
          );
          return { trackIdx: i, track: tracks[i], clip, us };
        }
      }
      return { trackIdx: -1, track: undefined, clip: undefined, us };
    },
    [project, xToUs],
  );

  const onMouseDown = (e: React.MouseEvent) => {
    const rect = canvasRef.current!.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const y = e.clientY - rect.top;

    if (y < RULER_H) {
      scrubRef.current = true;
      seek(Math.max(0, xToUs(x)));
      return;
    }
    const hit = hitTest(x, y);
    if (hit.clip) {
      select([hit.clip.id], e.shiftKey);
      // ¿agarró un borde? → trim
      const edgePx = 8;
      const clipX1 = ((hit.clip.start - viewStartUs) / 1e6) * pxPerSec;
      const clipX2 = clipX1 + (hit.clip.duration / 1e6) * pxPerSec;
      const mode: DragMode =
        x - clipX1 < edgePx ? "trim-left" : clipX2 - x < edgePx ? "trim-right" : "move";
      dragRef.current = {
        clipId: hit.clip.id,
        mode,
        grabOffsetUs: hit.us - hit.clip.start,
        startTrackIdx: hit.trackIdx,
      };
      setGhost({
        clipId: hit.clip.id,
        mode,
        startUs: hit.clip.start,
        edgeUs: mode === "trim-right" ? hit.clip.start + hit.clip.duration : hit.clip.start,
        trackIdx: hit.trackIdx,
        moved: false,
      });
    } else {
      select([]);
    }
  };

  useEffect(() => {
    const onMove = (e: MouseEvent) => {
      const canvas = canvasRef.current;
      if (!canvas) return;
      const rect = canvas.getBoundingClientRect();
      const x = e.clientX - rect.left;
      const y = e.clientY - rect.top;
      if (scrubRef.current) {
        seek(Math.max(0, xToUs(x)));
        return;
      }
      const drag = dragRef.current;
      if (drag) {
        if (drag.mode !== "move") {
          setGhost({
            clipId: drag.clipId,
            mode: drag.mode,
            startUs: 0,
            edgeUs: Math.max(0, xToUs(x)),
            trackIdx: drag.startTrackIdx,
            moved: true,
          });
          return;
        }
        const tracks = displayTracks(project);
        const tops = trackTops(tracks);
        let trackIdx = drag.startTrackIdx;
        for (let i = 0; i < tracks.length; i++) {
          if (y >= tops[i] && y < tops[i] + trackHeight(tracks[i])) trackIdx = i;
        }
        // solo pistas del mismo tipo
        if (tracks[trackIdx].kind !== tracks[drag.startTrackIdx].kind)
          trackIdx = drag.startTrackIdx;
        setGhost({
          clipId: drag.clipId,
          mode: "move",
          startUs: Math.max(0, xToUs(x) - drag.grabOffsetUs),
          edgeUs: 0,
          trackIdx,
          moved: true,
        });
      }
    };
    const onUp = () => {
      scrubRef.current = false;
      const drag = dragRef.current;
      dragRef.current = null;
      setGhost((g) => {
        if (drag && g && g.moved) {
          if (drag.mode === "move") {
            const tracks = displayTracks(project);
            void moveClip(drag.clipId, tracks[g.trackIdx].id, g.startUs);
          } else {
            void trimClip(drag.clipId, drag.mode === "trim-left", g.edgeUs);
          }
        }
        return null;
      });
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    return () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
  }, [project, xToUs, seek, moveClip]);

  const onWheel = (e: React.WheelEvent) => {
    const rect = canvasRef.current!.getBoundingClientRect();
    const x = e.clientX - rect.left;
    if (e.ctrlKey || e.metaKey) {
      const usAtCursor = xToUs(x);
      const next = Math.min(600, Math.max(2, pxPerSec * Math.exp(-e.deltaY * 0.0022)));
      setView(usAtCursor - (x / next) * 1e6, next);
    } else {
      const delta = e.deltaX !== 0 ? e.deltaX : e.deltaY;
      setView(viewStartUs + (delta / pxPerSec) * 1e6, pxPerSec);
    }
  };

  const zoomFit = () => {
    const seq = activeSequence(project);
    const end = Math.max(...seq.tracks.flatMap((t) => t.clips.map((c) => c.start + c.duration)), 1e6);
    setView(0, Math.max(2, ((size.w - 60) / end) * 1e6));
  };

  const tracks = displayTracks(project);

  return (
    <div className="flex h-full flex-col">
      <div className="flex h-9 shrink-0 items-center gap-1.5 border-b border-line-soft px-2">
        <h2 className="panel-eyebrow mr-2">Línea de tiempo</h2>
        <button
          className="focus-ring rounded-md px-2 py-1 text-[11.5px] text-ink-dim hover:bg-bg3 hover:text-ink"
          onClick={() => void splitAtPlayhead()}
          title="Dividir en el playhead (S)"
        >
          ✂ Dividir
        </button>
        <button
          className="focus-ring rounded-md px-2 py-1 text-[11.5px] text-ink-dim hover:bg-bg3 hover:text-ink"
          onClick={() => void deleteSelection(false)}
          title="Eliminar selección (Supr)"
        >
          Eliminar
        </button>
        <button
          className="focus-ring rounded-md px-2 py-1 text-[11.5px] text-ink-dim hover:bg-bg3 hover:text-ink"
          onClick={() => void deleteSelection(true)}
          title="Eliminar y cerrar hueco (⇧Supr)"
        >
          Eliminar y cerrar
        </button>
        <button
          className="focus-ring rounded-md px-2 py-1 text-[11.5px] text-ink-dim hover:bg-bg3 hover:text-ink"
          onClick={() => void addTextClip()}
          title="Añadir un título en el playhead"
        >
          + Título
        </button>
        <button
          className="focus-ring rounded-md px-2 py-1 text-[11.5px] text-ink-dim hover:bg-bg3 hover:text-ink"
          onClick={() => void generateVertical()}
          title="Genera una copia vertical 1080x1920 con fondo desenfocado (Shorts/Reels)"
        >
          📱 Vertical
        </button>
        {project.sequences.length > 1 && (
          <select
            className="focus-ring cursor-pointer rounded-md border border-line bg-bg2 px-1.5 py-0.5 text-[11px] text-ink"
            value={activeSequence(project).id}
            onChange={(e) => void setActiveSequence(e.target.value)}
            title="Secuencia activa"
          >
            {project.sequences.map((sq) => (
              <option key={sq.id} value={sq.id}>
                {sq.name}
              </option>
            ))}
          </select>
        )}
        <div className="flex-1" />
        <button
          className="focus-ring rounded-md px-2 py-1 text-[11.5px] text-ink-dim hover:bg-bg3 hover:text-ink"
          onClick={zoomFit}
          title="Ajustar todo (⇧Z)"
        >
          Ajustar
        </button>
        <input
          type="range"
          min={2}
          max={200}
          value={pxPerSec}
          onChange={(e) => setView(viewStartUs, Number(e.target.value))}
          className="h-1 w-28 cursor-pointer appearance-none rounded-full bg-bg3 accent-(--color-accent)"
          title="Zoom"
        />
      </div>

      <div className="flex min-h-0 flex-1">
        <div className="w-[148px] shrink-0 border-r border-line bg-bg1">
          <div style={{ height: RULER_H + TRACK_GAP }} className="border-b border-line-soft" />
          {tracks.map((t) => (
            <TrackHeader key={t.id} track={t} />
          ))}
        </div>
        <div ref={containerRef} className="relative min-w-0 flex-1 overflow-hidden">
          <canvas
            id="timeline-canvas"
            ref={canvasRef}
            className="block cursor-default"
            onMouseDown={onMouseDown}
            onWheel={onWheel}
            onMouseMove={(e) => {
              if (dragRef.current || scrubRef.current) return;
              const rect = canvasRef.current!.getBoundingClientRect();
              const x = e.clientX - rect.left;
              const y = e.clientY - rect.top;
              const hit = hitTest(x, y);
              let cursor = "default";
              if (hit.clip) {
                const clipX1 = ((hit.clip.start - viewStartUs) / 1e6) * pxPerSec;
                const clipX2 = clipX1 + (hit.clip.duration / 1e6) * pxPerSec;
                cursor =
                  x - clipX1 < 8 || clipX2 - x < 8 ? "ew-resize" : "grab";
              }
              canvasRef.current!.style.cursor = cursor;
            }}
          />
        </div>
      </div>
    </div>
  );
}
