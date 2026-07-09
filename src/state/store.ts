import { create } from "zustand";

import { MockEngine, demoProject } from "../engine/mock";
import type { Id, Project, TimeUs } from "../engine/types";

const engine = new MockEngine(demoProject());

export interface UiState {
  project: Project;
  /** bump para forzar re-render de canvases */
  version: number;
  selection: Id[];
  playheadUs: TimeUs;
  playing: boolean;
  /** vista del timeline */
  viewStartUs: TimeUs;
  pxPerSec: number;
  dirty: boolean;
  lastActionLabel?: string;

  seek: (us: TimeUs) => void;
  select: (ids: Id[], additive?: boolean) => void;
  togglePlay: () => void;
  setView: (viewStartUs: TimeUs, pxPerSec: number) => void;
  splitAtPlayhead: () => void;
  deleteSelection: (ripple: boolean) => void;
  moveClip: (clipId: Id, toTrackId: Id, toStartUs: TimeUs) => void;
  setClipProp: (clipId: Id, patch: Record<string, unknown>) => void;
  toggleTrack: (trackId: Id, prop: "muted" | "solo" | "locked") => void;
  undo: () => void;
  redo: () => void;
}

function sync(set: (partial: Partial<UiState>) => void, label?: string) {
  set({
    project: engine.project,
    version: Date.now() + Math.random(),
    dirty: true,
    lastActionLabel: label,
  });
}

export const useStore = create<UiState>((set, get) => ({
  project: engine.project,
  version: 0,
  selection: [],
  playheadUs: 12_400_000,
  playing: false,
  viewStartUs: 0,
  pxPerSec: 26,
  dirty: false,
  lastActionLabel: undefined,

  seek: (us) => set({ playheadUs: Math.max(0, us) }),

  select: (ids, additive = false) =>
    set((s) => ({
      selection: additive ? [...new Set([...s.selection, ...ids])] : ids,
    })),

  togglePlay: () => set((s) => ({ playing: !s.playing })),

  setView: (viewStartUs, pxPerSec) =>
    set({ viewStartUs: Math.max(0, viewStartUs), pxPerSec: Math.min(600, Math.max(2, pxPerSec)) }),

  splitAtPlayhead: () => {
    const { playheadUs, selection } = get();
    const seq = engine.sequence;
    // clips seleccionados bajo el playhead; si no hay selección, todos los que lo tocan
    const candidates = seq.tracks
      .flatMap((t) => t.clips)
      .filter((c) => c.start < playheadUs && playheadUs < c.start + c.duration)
      .filter((c) => selection.length === 0 || selection.includes(c.id));
    if (!candidates.length) return;
    let newSelection: Id[] = [];
    for (const c of candidates) {
      try {
        const [l, r] = engine.splitClip(c.id, playheadUs);
        newSelection = [...newSelection, l, r];
      } catch {
        /* clip no divisible en ese punto */
      }
    }
    set({ selection: newSelection });
    sync(set, "Dividir clip");
  },

  deleteSelection: (ripple) => {
    const { selection } = get();
    if (!selection.length) return;
    engine.deleteClips(selection, ripple);
    set({ selection: [] });
    sync(set, ripple ? "Eliminar (ripple)" : "Eliminar");
  },

  moveClip: (clipId, toTrackId, toStartUs) => {
    try {
      engine.moveClip(clipId, toTrackId, toStartUs);
      sync(set, "Mover clip");
    } catch {
      /* colisión o pista incompatible: no-op */
    }
  },

  setClipProp: (clipId, patch) => {
    engine.setClipProp(clipId, patch as never);
    sync(set, "Editar propiedades");
  },

  toggleTrack: (trackId, prop) => {
    engine.toggleTrack(trackId, prop);
    sync(set, "Pista");
  },

  undo: () => {
    const label = engine.undo();
    if (label) sync(set, `Deshacer: ${label}`);
  },

  redo: () => {
    const label = engine.redo();
    if (label) sync(set, `Rehacer: ${label}`);
  },
}));

export function engineCanUndo(): boolean {
  return engine.canUndo();
}
export function engineCanRedo(): boolean {
  return engine.canRedo();
}
