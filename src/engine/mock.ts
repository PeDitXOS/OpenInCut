/**
 * MockEngine: implementación en memoria de las operaciones del editor para
 * desarrollo en navegador y screenshots. Reproduce la semántica de ue-core
 * (split cuantizado a frame, ripple delete, historial transaccional) de forma
 * simplificada. El backend real (Tauri → ue-core) implementará esta misma
 * interfaz.
 */

import type { Clip, Id, Project, Sequence, TimeUs, Track } from "./types";
import { quantizeToFrame } from "../lib/time";

let idCounter = 0;
export function newId(prefix = "id"): Id {
  idCounter += 1;
  return `${prefix}_${idCounter.toString(36).padStart(4, "0")}`;
}

const S = 1_000_000;

export class MockEngine {
  project: Project;
  private undoStack: { label: string; snapshot: Project }[] = [];
  private redoStack: { label: string; snapshot: Project }[] = [];

  constructor(project: Project) {
    this.project = project;
  }

  get sequence(): Sequence {
    const seq = this.project.sequences.find(
      (s) => s.id === this.project.active_sequence,
    );
    if (!seq) throw new Error("secuencia activa no existe");
    return seq;
  }

  private track(id: Id): Track | undefined {
    return this.sequence.tracks.find((t) => t.id === id);
  }

  locateClip(id: Id): { track: Track; clip: Clip; index: number } | undefined {
    for (const track of this.sequence.tracks) {
      const index = track.clips.findIndex((c) => c.id === id);
      if (index >= 0) return { track, clip: track.clips[index], index };
    }
    return undefined;
  }

  private transaction<T>(label: string, fn: () => T): T {
    const snapshot = structuredClone(this.project);
    try {
      const result = fn();
      this.undoStack.push({ label, snapshot });
      this.redoStack = [];
      return result;
    } catch (e) {
      this.project = snapshot;
      throw e;
    }
  }

  canUndo() {
    return this.undoStack.length > 0;
  }
  canRedo() {
    return this.redoStack.length > 0;
  }
  undoLabel(): string | undefined {
    return this.undoStack[this.undoStack.length - 1]?.label;
  }

  undo(): string | undefined {
    const entry = this.undoStack.pop();
    if (!entry) return undefined;
    this.redoStack.push({ label: entry.label, snapshot: structuredClone(this.project) });
    this.project = entry.snapshot;
    return entry.label;
  }

  redo(): string | undefined {
    const entry = this.redoStack.pop();
    if (!entry) return undefined;
    this.undoStack.push({ label: entry.label, snapshot: structuredClone(this.project) });
    this.project = entry.snapshot;
    return entry.label;
  }

  /** Divide un clip en t (timeline, µs). Devuelve [izq, der]. */
  splitClip(clipId: Id, t: TimeUs): [Id, Id] {
    return this.transaction("Dividir clip", () => {
      const found = this.locateClip(clipId);
      if (!found) throw new Error("clip no encontrado");
      const { track, clip, index } = found;
      if (track.locked) throw new Error("pista bloqueada");
      const tq = quantizeToFrame(t, this.sequence.fps);
      if (tq <= clip.start || tq >= clip.start + clip.duration)
        throw new Error("punto de corte fuera del clip");
      const offset = tq - clip.start;

      const left: Clip = structuredClone(clip);
      const right: Clip = structuredClone(clip);
      left.id = newId("clip");
      right.id = newId("clip");
      left.duration = offset;
      left.fade_out_us = 0;
      right.start = tq;
      right.duration = clip.duration - offset;
      right.fade_in_us = 0;
      if (left.payload.type === "media" && right.payload.type === "media") {
        const srcOff = Math.round(offset * clip.speed);
        right.payload.src_in = left.payload.src_in + srcOff;
        left.payload.src_out = left.payload.src_in + srcOff;
      }
      track.clips.splice(index, 1, left, right);
      return [left.id, right.id];
    });
  }

  deleteClips(ids: Id[], ripple: boolean): void {
    this.transaction(ripple ? "Eliminar (ripple)" : "Eliminar", () => {
      const removed: { trackId: Id; start: TimeUs; end: TimeUs }[] = [];
      for (const id of ids) {
        const found = this.locateClip(id);
        if (!found) continue;
        if (found.track.locked) throw new Error("pista bloqueada");
        removed.push({
          trackId: found.track.id,
          start: found.clip.start,
          end: found.clip.start + found.clip.duration,
        });
        found.track.clips.splice(found.index, 1);
      }
      if (ripple) {
        // v1 mock: ripple por pista de los rangos eliminados en esa pista
        for (const track of this.sequence.tracks) {
          const spans = removed.filter((r) => r.trackId === track.id);
          if (!spans.length) continue;
          for (const clip of track.clips) {
            const shift = spans
              .filter((s) => s.end <= clip.start)
              .reduce((acc, s) => acc + (s.end - s.start), 0);
            clip.start -= shift;
          }
        }
      }
    });
  }

  moveClip(clipId: Id, toTrackId: Id, toStart: TimeUs): void {
    this.transaction("Mover clip", () => {
      const found = this.locateClip(clipId);
      const target = this.track(toTrackId);
      if (!found || !target) throw new Error("clip o pista no encontrados");
      if (found.track.locked || target.locked) throw new Error("pista bloqueada");
      if (target.kind !== found.track.kind) throw new Error("tipo de pista incompatible");
      const startQ = Math.max(0, quantizeToFrame(toStart, this.sequence.fps));
      const dur = found.clip.duration;
      const collides = target.clips.some(
        (c) =>
          c.id !== clipId && c.start < startQ + dur && startQ < c.start + c.duration,
      );
      if (collides) throw new Error("colisión");
      found.track.clips.splice(found.index, 1);
      found.clip.start = startQ;
      target.clips.push(found.clip);
      target.clips.sort((a, b) => a.start - b.start);
    });
  }

  setClipProp(clipId: Id, patch: Partial<Clip>): void {
    this.transaction("Editar propiedades", () => {
      const found = this.locateClip(clipId);
      if (!found) throw new Error("clip no encontrado");
      Object.assign(found.clip, patch);
    });
  }

  toggleTrack(trackId: Id, prop: "muted" | "solo" | "locked"): void {
    this.transaction("Pista", () => {
      const track = this.track(trackId);
      if (!track) throw new Error("pista no encontrada");
      track[prop] = !track[prop];
    });
  }
}

/** Proyecto demo: devlog con voz, música, B-roll, texto y clips ya editados. */
export function demoProject(): Project {
  const assets = [
    {
      id: newId("asset"),
      kind: "video" as const,
      path: "media/intro_camara.mp4",
      probe: {
        duration_us: 28 * S,
        fps: [30, 1] as [number, number],
        width: 1920,
        height: 1080,
        audio_channels: 2,
        vcodec: "h264",
        acodec: "aac",
      },
    },
    {
      id: newId("asset"),
      kind: "video" as const,
      path: "media/gameplay_fisicas.mp4",
      probe: {
        duration_us: 84 * S,
        fps: [60, 1] as [number, number],
        width: 2560,
        height: 1440,
        audio_channels: 2,
        vcodec: "hevc",
        acodec: "aac",
      },
      caching: 0.64,
    },
    {
      id: newId("asset"),
      kind: "video" as const,
      path: "media/pantalla_codigo.mp4",
      probe: {
        duration_us: 152 * S,
        fps: [30, 1] as [number, number],
        width: 1920,
        height: 1080,
        audio_channels: 0,
        vcodec: "h264",
      },
    },
    {
      id: newId("asset"),
      kind: "audio" as const,
      path: "media/voz_off.wav",
      probe: { duration_us: 58 * S, width: 0, height: 0, audio_channels: 1, acodec: "pcm_s16le" },
    },
    {
      id: newId("asset"),
      kind: "audio" as const,
      path: "media/musica_lofi.mp3",
      probe: { duration_us: 130 * S, width: 0, height: 0, audio_channels: 2, acodec: "mp3" },
    },
    {
      id: newId("asset"),
      kind: "image" as const,
      path: "media/logo_canal.png",
      probe: { duration_us: 0, width: 1024, height: 1024, audio_channels: 0 },
    },
  ];
  const [cam, gameplay, screen, voz, musica] = assets;

  const mediaClip = (
    asset: { id: Id },
    srcIn: number,
    srcOut: number,
    start: number,
    label?: string,
    extra?: Partial<Clip>,
  ): Clip => ({
    id: newId("clip"),
    payload: { type: "media", asset_id: asset.id, src_in: srcIn * S, src_out: srcOut * S },
    start: start * S,
    duration: (srcOut - srcIn) * S,
    speed: 1,
    gain_db: 0,
    fade_in_us: 0,
    fade_out_us: 0,
    opacity: 1,
    label,
    ...extra,
  });

  const tracks: Track[] = [
    {
      id: newId("track"),
      kind: "audio",
      name: "A2",
      muted: false,
      solo: false,
      locked: false,
      volume_db: -12,
      clips: [
        mediaClip(musica, 0, 46, 0, "musica_lofi.mp3", {
          gain_db: -14,
          fade_in_us: 1.5 * S,
          fade_out_us: 3 * S,
        }),
      ],
    },
    {
      id: newId("track"),
      kind: "audio",
      name: "A1",
      muted: false,
      solo: false,
      locked: false,
      volume_db: 0,
      clips: [
        mediaClip(voz, 0, 7.5, 0.8, "voz_off.wav"),
        mediaClip(voz, 8.1, 16.4, 8.3, "voz_off.wav"),
        mediaClip(voz, 17.0, 29.2, 16.7, "voz_off.wav"),
        mediaClip(voz, 30.1, 41.0, 29.0, "voz_off.wav"),
      ],
    },
    {
      id: newId("track"),
      kind: "video",
      name: "V1",
      muted: false,
      solo: false,
      locked: false,
      volume_db: 0,
      clips: [
        mediaClip(cam, 2, 10.5, 0, "intro_camara.mp4"),
        mediaClip(screen, 12, 26, 8.5, "pantalla_codigo.mp4"),
        mediaClip(gameplay, 5, 19.5, 22.5, "gameplay_fisicas.mp4"),
        mediaClip(cam, 14, 22, 37, "intro_camara.mp4"),
      ],
    },
    {
      id: newId("track"),
      kind: "video",
      name: "V2",
      muted: false,
      solo: false,
      locked: false,
      volume_db: 0,
      clips: [
        {
          id: newId("clip"),
          payload: { type: "text", content: "CÓMO HICE UN MOTOR DE FÍSICAS" },
          start: 1.2 * S,
          duration: 4.4 * S,
          speed: 1,
          gain_db: 0,
          fade_in_us: 0,
          fade_out_us: 0,
          opacity: 1,
          label: "Título",
        },
        {
          id: newId("clip"),
          payload: { type: "text", content: "suscríbete →" },
          start: 30 * S,
          duration: 3.5 * S,
          speed: 1,
          gain_db: 0,
          fade_in_us: 0,
          fade_out_us: 0,
          opacity: 1,
          label: "CTA",
        },
      ],
    },
  ];

  const seq: Sequence = {
    id: newId("seq"),
    name: "Principal",
    resolution: [1920, 1080],
    fps: [30, 1],
    tracks,
    markers: [
      { id: newId("mk"), t: 8.5 * S, name: "Demo código", color: "#6fa3b5" },
      { id: newId("mk"), t: 22.5 * S, name: "Gameplay", color: "#8fb573" },
    ],
  };

  return {
    name: "Devlog 12 — Motor de físicas",
    assets,
    sequences: [seq],
    active_sequence: seq.id,
  };
}
