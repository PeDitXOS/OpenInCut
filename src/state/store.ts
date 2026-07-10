import { create } from "zustand";

import type { EngineClient, ExportUiSettings } from "../engine/client";
import { MockEngine, demoProject } from "../engine/mock";
import { TauriEngine, isTauri } from "../engine/tauri";
import type {
  AudioProps,
  EffectDef,
  EffectInstance,
  GeneratorDef,
  Id,
  Param,
  Project,
  StateSnapshot,
  TimeUs,
  TextStyle,
  Transform2D,
  TransitionRef,
} from "../engine/types";
import { activeSequence } from "../engine/types";

const mockEngine = isTauri() ? null : new MockEngine(demoProject());
export const engine: EngineClient = mockEngine ?? new TauriEngine();

export interface UiState {
  ready: boolean;
  project: Project;
  version: number;
  dirty: boolean;
  canUndo: boolean;
  canRedo: boolean;
  selection: Id[];
  playheadUs: TimeUs;
  playing: boolean;
  /** true = the position is dictated by the backend audio clock */
  engineClock: boolean;
  viewStartUs: TimeUs;
  pxPerSec: number;
  lastActionLabel?: string;

  init: () => Promise<void>;
  seek: (us: TimeUs) => void;
  select: (ids: Id[], additive?: boolean) => void;
  togglePlay: () => void;
  /** Current JKL shuttle rate (1 = normal, negative = reverse). */
  shuttleRate: number;
  shuttle: (direction: -1 | 0 | 1) => void;
  setView: (viewStartUs: TimeUs, pxPerSec: number) => void;
  splitAtPlayhead: () => Promise<void>;
  deleteSelection: (ripple: boolean) => Promise<void>;
  moveClip: (clipId: Id, toTrackId: Id, toStartUs: TimeUs) => Promise<void>;
  trimClip: (clipId: Id, left: boolean, newEdgeUs: TimeUs) => Promise<void>;
  setClipAudio: (clipId: Id, audio: AudioProps) => Promise<void>;
  setClipTransform: (clipId: Id, transform: Transform2D) => Promise<void>;
  importMedia: () => Promise<void>;
  addClipFromAsset: (assetId: Id) => Promise<void>;
  exporting: boolean;
  /** I-O working range in µs (for range export). */
  rangeInUs: number | null;
  rangeOutUs: number | null;
  setRangeIn: (us: number | null) => void;
  setRangeOut: (us: number | null) => void;
  /** Saved export pieces: rendered concatenated, in order. */
  exportRanges: [number, number][];
  addExportRange: () => void;
  removeExportRange: (index: number) => void;
  clearExportRanges: () => void;
  showExportDialog: boolean;
  setShowExportDialog: (v: boolean) => void;
  exportProgress: number | null;
  exportVideo: (settings?: ExportUiSettings) => Promise<void>;
  cancelExport: () => Promise<void>;
  saveProject: () => Promise<void>;
  openProject: () => Promise<void>;
  newProject: () => Promise<void>;
  relinkAsset: (assetId: Id) => Promise<void>;
  mcpPort: number | null;
  mcpToken: string | null;
  /** RMS 0..1 per channel from the last buffer (tauri engine only). */
  meterL: number;
  meterR: number;
  /** Increments when new waveforms/thumbnails arrive (redraw). */
  visualsBump: number;
  setAiSettings: (lang: string, model: string) => Promise<void>;
  fonts: [string, string][];
  textTemplates: Record<string, TextStyle>;
  saveTextTemplate: (name: string, style: TextStyle) => Promise<void>;
  effectsCatalog: EffectDef[];
  generatorsCatalog: GeneratorDef[];
  addGeneratorClip: (generatorId: string) => Promise<void>;
  setClipGenerator: (
    clipId: Id,
    generatorId: string,
    params: Record<string, Param>,
    colorParams: Record<string, string>,
  ) => Promise<void>;
  setClipEffects: (clipId: Id, effects: EffectInstance[]) => Promise<void>;
  setClipTransition: (clipId: Id, transition: TransitionRef | null) => Promise<void>;
  reloadEffectPacks: () => Promise<void>;
  addTextClip: () => Promise<void>;
  removeSilences: (
    clipId: Id,
    mode: "delete" | "speedup" | "split",
    params?: { thresholdDb?: number; minSilenceMs?: number; padMs?: number },
  ) => Promise<void>;
  setClipSpeed: (clipId: Id, speed: number) => Promise<void>;
  unlinkClip: (clipId: Id) => Promise<void>;
  transcribeAsset: (assetId: Id) => Promise<void>;
  addSubtitlesClip: (clipId: Id) => Promise<void>;
  generateVertical: () => Promise<void>;
  addAvatarClip: (clipId: Id) => Promise<void>;
  setActiveSequence: (sequenceId: Id) => Promise<void>;
  cutTimelineRanges: (ranges: [TimeUs, TimeUs][]) => Promise<void>;
  moveTimelineRange: (fromUs: TimeUs, toUs: TimeUs, destUs: TimeUs) => Promise<void>;
  setClipText: (clipId: Id, content: string, style: TextStyle) => Promise<void>;
  setSubtitlesProps: (
    clipId: Id,
    style: TextStyle,
    mode: "phrase" | "word" | "karaoke",
  ) => Promise<void>;
  toggleTrack: (trackId: Id, prop: "muted" | "solo" | "locked") => Promise<void>;
  addTrack: (kind: "video" | "audio") => Promise<void>;
  removeSequence: (sequenceId: Id) => Promise<void>;
  setWordText: (transcriptId: Id, index: number, text: string) => Promise<void>;
  replaceWords: (transcriptId: Id, from: string, to: string) => Promise<void>;
  setSequenceProps: (
    sequenceId: Id,
    width: number,
    height: number,
    fpsNum: number,
    fpsDen: number,
  ) => Promise<void>;
  /** Assets currently being transcribed (background jobs). */
  transcribingIds: Id[];
  removeTrack: (trackId: Id) => Promise<void>;
  renameTrack: (trackId: Id, name: string) => Promise<void>;
  setTrackVolume: (trackId: Id, db: number) => Promise<void>;
  undo: () => Promise<void>;
  redo: () => Promise<void>;
}

/** The initial empty project is replaced in init() with the engine state. */
const emptyProject: Project = {
  schema_version: 1,
  id: "pending",
  name: "Loading…",
  created_at: "",
  settings: { whisper_language: "auto", whisper_model: "base", autosave_secs: 60 },
  assets: [],
  transcripts: [],
  sequences: [
    {
      id: "seq_pending",
      name: "Main",
      resolution: [1920, 1080],
      fps: [30, 1],
      sample_rate: 48000,
      tracks: [],
      markers: [],
    },
  ],
  active_sequence: "seq_pending",
};

export const useStore = create<UiState>((set, get) => {
  const applySnapshot = (snap: StateSnapshot, label?: string) =>
    set((st) => ({
      project: snap.project,
      version: snap.version,
      dirty: snap.dirty,
      canUndo: snap.can_undo,
      canRedo: snap.can_redo,
      lastActionLabel: label,
      ready: true,
      // a finished transcription shows up in the snapshot
      transcribingIds: st.transcribingIds.filter(
        (id) => !snap.project.transcripts.some((d) => d.asset_id === id),
      ),
    }));

  /** Runs an engine op; errors go to the status bar. */
  const run = async (label: string, op: () => Promise<StateSnapshot>) => {
    try {
      applySnapshot(await op(), label);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      engine.uiLog("error", `${label}: ${msg}`);
      set({ lastActionLabel: `⚠ ${msg}` });
    }
  };

  return {
    ready: false,
    project: emptyProject,
    version: -1,
    dirty: false,
    canUndo: false,
    canRedo: false,
    selection: [],
    playheadUs: 12_400_000,
    playing: false,
    engineClock: false,
    viewStartUs: 0,
    pxPerSec: 26,
    lastActionLabel: undefined,

    init: async () => {
      applySnapshot(await engine.getState());
      // is there a recovery copy left from a previous session?
      try {
        const autosave = await engine.checkRecovery();
        if (autosave) {
          if (window.confirm("There is a recovery copy newer than the project. Load it?")) {
            applySnapshot(
              await engine.recoverProject(autosave, null),
              "Project recovered from autosave",
            );
          } else {
            await engine.discardRecovery();
          }
        }
      } catch {
        /* no recovery */
      }
      // refresh when the backend finishes jobs (conform, etc.)
      void engine.onStateChanged(async () => {
        applySnapshot(await engine.getState());
      });
      void engine.onExportProgress((p) => set({ exportProgress: p }));
      try {
        set({ effectsCatalog: await engine.getEffectsCatalog() });
      } catch {
        /* no catalog: the effects panel stays empty */
      }
      try {
        set({ generatorsCatalog: await engine.getGenerators() });
      } catch {
        /* no generators */
      }
      try {
        const status = await engine.mcpStatus();
        if (status) set({ mcpPort: status[0], mcpToken: status[1] });
      } catch {
        /* no MCP */
      }
      try {
        set({
          fonts: await engine.listFonts(),
          textTemplates: await engine.listTextTemplates(),
        });
      } catch {
        /* no fonts/templates */
      }
    },

    seek: (us) => {
      const clamped = Math.max(0, Math.round(us));
      set({ playheadUs: clamped });
      // keep the audio clock aligned if it is playing
      if (engine.kind === "tauri" && get().playing) {
        void engine.playbackSeek(clamped).catch(() => {});
      }
    },

    select: (ids, additive = false) =>
      set((s) => ({ selection: additive ? [...new Set([...s.selection, ...ids])] : ids })),

    shuttleRate: 1,
    shuttle: (direction) => {
      const s = get();
      if (direction === 0) {
        set({ shuttleRate: 1 });
        if (s.playing) s.togglePlay();
        return;
      }
      // repeating the key doubles the rate: 1→2→4→8 (or starts at 1)
      const prev = s.playing ? s.shuttleRate : 0;
      const sameDir = Math.sign(prev) === direction;
      const next = sameDir ? Math.min(Math.abs(prev) * 2, 8) * direction : direction;
      if (engine.kind !== "tauri") {
        set({ shuttleRate: next, playing: true, engineClock: false });
        return;
      }
      engine
        .playbackSetRate(next, s.playheadUs)
        .then(() => set({ shuttleRate: next, playing: true, engineClock: true }))
        .catch(() => {
          set({ shuttleRate: next, playing: true, engineClock: false });
        });
    },
    togglePlay: () => {
      const s = get();
      if (engine.kind !== "tauri") {
        set({ playing: !s.playing, engineClock: false });
        return;
      }
      if (!s.playing) {
        set({ shuttleRate: 1 });
        engine
          .playbackPlay(s.playheadUs)
          .then(() => set({ playing: true, engineClock: true }))
          .catch((e) => {
            // no audio device → silent local clock
            set({
              playing: true,
              engineClock: false,
              lastActionLabel: `⚠ audio unavailable: ${e instanceof Error ? e.message : e}`,
            });
          });
      } else {
        set({ playing: false });
        engine
          .playbackPause()
          .then((t) => set({ playheadUs: t }))
          .catch(() => {});
      }
    },

    setView: (viewStartUs, pxPerSec) =>
      set({
        viewStartUs: Math.max(0, viewStartUs),
        pxPerSec: Math.min(600, Math.max(2, pxPerSec)),
      }),

    splitAtPlayhead: async () => {
      const { playheadUs, selection, project } = get();
      const seq = activeSequence(project);
      const candidates = seq.tracks
        .flatMap((t) => t.clips)
        .filter((c) => c.start < playheadUs && playheadUs < c.start + c.duration)
        .filter((c) => selection.length === 0 || selection.includes(c.id));
      if (!candidates.length) return;
      const newSelection: Id[] = [];
      for (const c of candidates) {
        try {
          const snap = await engine.splitClip(c.id, playheadUs);
          applySnapshot(snap, "Split clip");
        } catch {
          /* clip not splittable at that point */
        }
      }
      set({ selection: newSelection });
    },

    deleteSelection: async (ripple) => {
      const { selection } = get();
      if (!selection.length) return;
      await run(ripple ? "Delete (ripple)" : "Delete", () =>
        engine.deleteClips(selection, ripple),
      );
      set({ selection: [] });
    },

    moveClip: (clipId, toTrackId, toStartUs) =>
      run("Move clip", () => engine.moveClip(clipId, toTrackId, toStartUs, false)),

    trimClip: (clipId, left, newEdgeUs) =>
      run("Trim clip", () => engine.trimClip(clipId, left, newEdgeUs)),

    setClipAudio: (clipId, audio) =>
      run("Edit audio", () => engine.setClipAudio(clipId, audio)),

    setClipTransform: (clipId, transform) =>
      run("Edit transform", () => engine.setClipTransform(clipId, transform)),

    importMedia: async () => {
      const paths = await engine.pickMediaFiles();
      if (!paths) {
        set({ lastActionLabel: "⚠ Import requires the desktop app (npx tauri dev)" });
        return;
      }
      await run(`Import ${paths.length} file(s)`, () => engine.importMedia(paths));
    },

    addClipFromAsset: (assetId) =>
      run("Add clip", () => engine.addClip(assetId, get().playheadUs)),

    exporting: false,
    rangeInUs: null,
    rangeOutUs: null,
    setRangeIn: (us) => set({ rangeInUs: us }),
    setRangeOut: (us) => set({ rangeOutUs: us }),
    exportRanges: [],
    addExportRange: () => {
      const { rangeInUs, rangeOutUs, exportRanges } = get();
      if (rangeInUs == null || rangeOutUs == null || rangeOutUs <= rangeInUs) {
        set({ lastActionLabel: "⚠ mark a range with I and O first" });
        return;
      }
      const pair: [number, number] = [rangeInUs, rangeOutUs];
      const next: [number, number][] = [...exportRanges, pair].sort((a, b) => a[0] - b[0]);
      set({ exportRanges: next, lastActionLabel: `Piece ${next.length} added` });
    },
    removeExportRange: (index) =>
      set((st) => ({ exportRanges: st.exportRanges.filter((_, i) => i !== index) })),
    clearExportRanges: () => set({ exportRanges: [] }),
    showExportDialog: false,
    setShowExportDialog: (v) => set({ showExportDialog: v }),
    exportProgress: null,
    exportVideo: async (settings) => {
      try {
        const ext = settings?.format ?? "mp4";
        const name = `${get().project.name.replace(/[^\p{L}\p{N} _-]/gu, "").trim() || "export"}.${ext}`;
        const path = await engine.pickSavePath(name, ext);
        if (!path) {
          if (engine.kind === "mock")
            set({ lastActionLabel: "⚠ Export requires the desktop app (npx tauri dev)" });
          return;
        }
        set({
          exporting: true,
          exportProgress: 0,
          showExportDialog: false,
          lastActionLabel: "Exporting…",
        });
        const written = await engine.exportVideo(path, settings);
        set({ exporting: false, exportProgress: null, lastActionLabel: `Exported to ${written}` });
      } catch (e) {
        set({
          exporting: false,
    rangeInUs: null,
    rangeOutUs: null,
    setRangeIn: (us) => set({ rangeInUs: us }),
    setRangeOut: (us) => set({ rangeOutUs: us }),
    exportRanges: [],
    addExportRange: () => {
      const { rangeInUs, rangeOutUs, exportRanges } = get();
      if (rangeInUs == null || rangeOutUs == null || rangeOutUs <= rangeInUs) {
        set({ lastActionLabel: "⚠ mark a range with I and O first" });
        return;
      }
      const pair: [number, number] = [rangeInUs, rangeOutUs];
      const next: [number, number][] = [...exportRanges, pair].sort((a, b) => a[0] - b[0]);
      set({ exportRanges: next, lastActionLabel: `Piece ${next.length} added` });
    },
    removeExportRange: (index) =>
      set((st) => ({ exportRanges: st.exportRanges.filter((_, i) => i !== index) })),
    clearExportRanges: () => set({ exportRanges: [] }),
    showExportDialog: false,
    setShowExportDialog: (v) => set({ showExportDialog: v }),
          exportProgress: null,
          lastActionLabel: `⚠ ${e instanceof Error ? e.message : String(e)}`,
        });
      }
    },

    cancelExport: async () => {
      try {
        await engine.cancelExport();
      } catch {
        /* no export in progress */
      }
    },

    mcpPort: null,
    mcpToken: null,
    meterL: 0,
    meterR: 0,
    visualsBump: 0,
    setAiSettings: (lang, model) =>
      run("AI settings", () => engine.setProjectSettings(lang, model)),
    fonts: [],
    textTemplates: {},
    saveTextTemplate: async (name, style) => {
      try {
        set({ textTemplates: await engine.saveTextTemplate(name, style) });
        set({ lastActionLabel: `Template "${name}" saved` });
      } catch (e) {
        set({ lastActionLabel: `⚠ ${e instanceof Error ? e.message : String(e)}` });
      }
    },
    effectsCatalog: [],
    generatorsCatalog: [],
    addGeneratorClip: (generatorId) =>
      run("Add generator", () => engine.addGeneratorClip(generatorId, get().playheadUs)),
    setClipGenerator: (clipId, generatorId, params, colorParams) =>
      run("Edit generator", () =>
        engine.setClipGenerator(clipId, generatorId, params, colorParams),
      ),
    setClipEffects: (clipId, effects) =>
      run("Edit effects", () => engine.setClipEffects(clipId, effects)),
    setClipTransition: (clipId, transition) =>
      run("Edit transition", () => engine.setClipTransition(clipId, transition)),

    moveTimelineRange: async (fromUs, toUs, destUs) => {
      const seqId = activeSequence(get().project).id;
      await run("Move range by text", () =>
        engine.moveRange(seqId, fromUs, toUs, destUs),
      );
    },

    cutTimelineRanges: async (ranges) => {
      if (!ranges.length) return;
      const seqId = activeSequence(get().project).id;
      await run(`Cut ${ranges.length} range(s) by text`, () =>
        engine.cutRanges(seqId, ranges, true),
      );
    },

    generateVertical: () => run("Generate vertical", () => engine.generateVertical()),

    addAvatarClip: async (clipId) => {
      const path = await engine.pickAvatarConfig();
      if (!path) {
        if (engine.kind === "mock")
          set({ lastActionLabel: "⚠ The avatar requires the desktop app (npx tauri dev)" });
        return;
      }
      await run("Add avatar", () => engine.addAvatarClip(clipId, path));
    },
    setActiveSequence: (sequenceId) =>
      run("Change sequence", () => engine.setActiveSequence(sequenceId)),

    addSubtitlesClip: (clipId) =>
      run("Auto subtitles", () => engine.addSubtitlesClip(clipId)),

    transcribeAsset: async (assetId) => {
      try {
        set((st) => ({ transcribingIds: [...st.transcribingIds, assetId] }));
        await engine.transcribeAsset(assetId);
        set({
          lastActionLabel:
            "Transcribing in the background… (downloads the model the first time)",
        });
      } catch (e) {
        set((st) => ({
          transcribingIds: st.transcribingIds.filter((id) => id !== assetId),
          lastActionLabel: `⚠ ${e instanceof Error ? e.message : String(e)}`,
        }));
      }
    },

    removeSilences: async (clipId, mode, params) => {
      try {
        set({ lastActionLabel: "Analyzing silences…" });
        const r = await engine.removeSilences(clipId, mode, params);
        const secs = (r.removed_us / 1e6).toFixed(1);
        applySnapshot(
          r.snapshot,
          r.removed > 0
            ? mode === "speedup"
              ? `Sped up ${r.removed} silences 4× (${secs} s)`
              : `Removed ${r.removed} silences (${secs} s)`
            : "No silences found",
        );
      } catch (e) {
        set({ lastActionLabel: `⚠ ${e instanceof Error ? e.message : String(e)}` });
      }
    },

    setClipSpeed: (clipId, speed) =>
      run(`Speed ${speed}×`, () => engine.setClipSpeed(clipId, speed)),
    unlinkClip: (clipId) => run("Unlink", () => engine.unlinkClip(clipId)),

    addTextClip: async () => {
      await run("Add title", () => engine.addTextClip("Title", get().playheadUs));
    },
    setClipText: (clipId, content, style) =>
      run("Edit text", () => engine.setClipText(clipId, content, style)),
    setSubtitlesProps: (clipId, style, mode) =>
      run("Edit subtitles", () => engine.setSubtitlesProps(clipId, style, mode)),

    reloadEffectPacks: async () => {
      try {
        const r = await engine.reloadEffectPacks();
        set({
          effectsCatalog: r.catalog,
          lastActionLabel: r.errors.length
            ? `⚠ packs with errors: ${r.errors.join("; ")}`
            : `Packs reloaded (${r.catalog.length})${r.dir ? ` from ${r.dir}` : ""}`,
        });
      } catch (e) {
        set({ lastActionLabel: `⚠ ${e instanceof Error ? e.message : String(e)}` });
      }
    },

    toggleTrack: async (trackId, prop) => {
      const track = activeSequence(get().project).tracks.find((t) => t.id === trackId);
      if (!track) return;
      await run("Track", () => engine.setTrackProp(trackId, prop, !track[prop]));
    },
    addTrack: (kind) => run("Add track", () => engine.addTrack(kind)),
    removeSequence: (sequenceId) =>
      run("Delete sequence", () => engine.removeSequence(sequenceId)),
    setWordText: (transcriptId, index, text) =>
      run("Correct word", () => engine.setWordText(transcriptId, index, text)),
    replaceWords: async (transcriptId, from, to) => {
      try {
        const r = await engine.replaceWords(transcriptId, from, to);
        applySnapshot(r.snapshot, `Replaced ${r.replaced} occurrence(s)`);
      } catch (e) {
        set({ lastActionLabel: `⚠ ${e instanceof Error ? e.message : String(e)}` });
      }
    },
    setSequenceProps: (sequenceId, width, height, fpsNum, fpsDen) =>
      run("Sequence settings", () =>
        engine.setSequenceProps(sequenceId, width, height, fpsNum, fpsDen),
      ),
    transcribingIds: [],
    removeTrack: (trackId) => run("Delete track", () => engine.removeTrack(trackId)),
    renameTrack: (trackId, name) =>
      run("Rename track", () => engine.renameTrack(trackId, name)),
    setTrackVolume: (trackId, db) =>
      run("Track volume", () => engine.setTrackVolume(trackId, db)),

    saveProject: async () => {
      try {
        let written: string;
        try {
          written = await engine.saveProject(null);
        } catch (e) {
          const msg = e instanceof Error ? e.message : String(e);
          if (!msg.includes("no save path")) throw e;
          const name = `${get().project.name.replace(/[^\p{L}\p{N} _-]/gu, "").trim() || "project"}.uep`;
          const path = await engine.pickProjectSavePath(name);
          if (!path) return;
          written = await engine.saveProject(path);
        }
        applySnapshot(await engine.getState(), `Saved to ${written}`);
      } catch (e) {
        set({ lastActionLabel: `⚠ ${e instanceof Error ? e.message : String(e)}` });
      }
    },

    newProject: async () => {
      const s = get();
      if (s.dirty && !window.confirm("There are unsaved changes. Discard them and create a new project?"))
        return;
      try {
        const snap = await engine.newProject("Untitled project");
        set({ selection: [], playheadUs: 0, playing: false, viewStartUs: 0 });
        applySnapshot(snap, "New project");
      } catch (e) {
        set({ lastActionLabel: `⚠ ${e instanceof Error ? e.message : String(e)}` });
      }
    },

    relinkAsset: async (assetId) => {
      const paths = await engine.pickMediaFiles();
      if (!paths?.length) return;
      await run("Relink media", () => engine.relinkAsset(assetId, paths[0]));
    },

    openProject: async () => {
      try {
        const path = await engine.pickProjectOpenPath();
        if (!path) {
          if (engine.kind === "mock")
            set({ lastActionLabel: "⚠ Open requires the desktop app (npx tauri dev)" });
          return;
        }
        const snap = await engine.openProject(path);
        set({ selection: [], playheadUs: 0, playing: false });
        applySnapshot(snap, `Opened ${path}`);
      } catch (e) {
        set({ lastActionLabel: `⚠ ${e instanceof Error ? e.message : String(e)}` });
      }
    },

    undo: async () => {
      const snap = await engine.undo();
      applySnapshot(snap, "Undo");
    },

    redo: async () => {
      const snap = await engine.redo();
      applySnapshot(snap, "Redo");
    },
  };
});
