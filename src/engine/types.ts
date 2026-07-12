/**
 * TypeScript mirror of the real ue-core model (crates/ue-core/src/model.rs),
 * with the same JSON shape serde produces (snake_case, tagged payload,
 * tuples as arrays). Automatic generation (ts-rs) pending; keep it in
 * sync by hand until then.
 */

export type Id = string;
export type TimeUs = number;

// -- keyframes ---------------------------------------------------------------

export type Interp =
  | { kind: "hold" }
  | { kind: "linear" }
  | { kind: "smooth"; tan_out?: number; tan_in?: number };

export interface Keyframe {
  t: TimeUs;
  value: number;
  interp: Interp;
}

/** plain number ⇔ Const; object {keys} ⇔ Curve (serde untagged) */
export type Param = number | { keys: Keyframe[] };

export function paramValue(p: Param, tUs: TimeUs = 0): number {
  if (typeof p === "number") return p;
  const keys = p.keys;
  if (!keys.length) return 0;
  if (tUs <= keys[0].t) return keys[0].value;
  const last = keys[keys.length - 1];
  if (tUs >= last.t) return last.value;
  for (let i = 0; i < keys.length - 1; i++) {
    const a = keys[i];
    const b = keys[i + 1];
    if (a.t <= tUs && tUs < b.t) {
      if (a.interp.kind === "hold") return a.value;
      const u = (tUs - a.t) / (b.t - a.t);
      return a.value + (b.value - a.value) * u; // linear (smooth ≈ linear in the UI)
    }
  }
  return last.value;
}

export function isCurve(p: Param): boolean {
  return typeof p !== "number";
}

/** Is there a keyframe within ±eps of tUs? */
export function hasKeyAt(p: Param, tUs: TimeUs, epsUs = 20_000): boolean {
  if (typeof p === "number") return false;
  return p.keys.some((k) => Math.abs(k.t - tUs) <= epsUs);
}

/** Adds (or updates) a keyframe at tUs. A Const becomes a curve. */
export function withKeyAt(p: Param, tUs: TimeUs, value: number, epsUs = 20_000): Param {
  const keys = typeof p === "number" ? [] : [...p.keys];
  const idx = keys.findIndex((k) => Math.abs(k.t - tUs) <= epsUs);
  const key = { t: Math.max(0, Math.round(tUs)), value, interp: { kind: "linear" as const } };
  if (idx >= 0) keys[idx] = { ...keys[idx], value };
  else {
    keys.push(key);
    keys.sort((a, b) => a.t - b.t);
  }
  return { keys };
}

/**
 * Removes the keyframe within ±eps of tUs. Only an EMPTY curve reverts to a
 * constant (= animation switched off). A one-key curve stays a curve: it is
 * how a freshly animated property starts, and collapsing it was what made the
 * diamond button appear to do nothing (see Param::sanitized in ue-core).
 */
export function removeKeyAt(p: Param, tUs: TimeUs, epsUs = 20_000): Param {
  if (typeof p === "number") return p;
  const keys = p.keys.filter((k) => Math.abs(k.t - tUs) > epsUs);
  if (keys.length === 0) return paramValue(p, tUs);
  return { keys };
}

/** Times (µs relative to the clip) of all of a clip's keyframes. */
export function clipKeyframeTimes(clip: Clip): TimeUs[] {
  const times: TimeUs[] = [];
  const collect = (p: Param) => {
    if (typeof p !== "number") for (const k of p.keys) times.push(k.t);
  };
  const t = clip.transform;
  [t.position[0], t.position[1], t.scale[0], t.scale[1], t.rotation, t.opacity, ...t.crop].forEach(
    collect,
  );
  collect(clip.audio.gain_db);
  collect(clip.audio.pan);
  for (const fx of clip.effects) Object.values(fx.params).forEach(collect);
  return [...new Set(times)].sort((a, b) => a - b);
}

// -- media -------------------------------------------------------------------

export type MediaKind = "video" | "audio" | "image";
export type TrackKind = "video" | "audio";

export interface ProbeInfo {
  duration_us: TimeUs;
  fps: [number, number] | null;
  width: number;
  height: number;
  rotation: number;
  vcodec: string | null;
  acodec: string | null;
  audio_channels: number;
  vfr: boolean;
}

/** One avatar expression: a still image or a video loop. */
export interface AvatarExpression {
  name: string;
  path: string;
}

/** A complete avatar setup, stored in the project and exportable. */
export interface AvatarConfig {
  id: Id;
  name: string;
  expressions: AvatarExpression[];
  shake_factor: number;
  scale: number;
  model: string;
  api_base: string;
  api_key: string;
}

export function newAvatarConfig(): AvatarConfig {
  return {
    id: "",
    name: "My avatar",
    expressions: [],
    shake_factor: 1,
    scale: 0.25,
    model: "",
    api_base: "",
    api_key: "",
  };
}

/** One selectable TTS voice (`say -v` name or kokoro voice id). */
export interface TtsVoice {
  id: string;
  name: string;
  lang: string;
}

/** How one engine understands its rate knob; the slider renders from this. */
export interface TtsRateSpec {
  min: number;
  max: number;
  default: number;
  step: number;
  /** Slider caption ("words/min", "speed ×"). */
  label: string;
}

/** One TTS engine (built-in or user manifest), voices included. */
export interface TtsEngineInfo {
  id: string;
  name: string;
  available: boolean;
  /** Why it is unavailable, or where it was found. */
  detail: string;
  voices: TtsVoice[];
  rate: TtsRateSpec | null;
}

/** Engine catalog (list_tts_voices). */
export interface TtsCatalog {
  engines: TtsEngineInfo[];
  /** Folder where user engine manifests (*.json) are discovered. */
  engines_dir: string | null;
}

/** Meta for the thumbnail strip generated by the backend. */
export interface ThumbStrip {
  path: string;
  tile_w: number;
  tile_h: number;
  count: number;
  interval_us: number;
}

export interface MediaAsset {
  id: Id;
  kind: MediaKind;
  path: string;
  content_hash: string;
  probe: ProbeInfo;
  proxy: string | null;
  audio_conform: string | null;
  peaks: string | null;
  thumbnails: string | null;
  transcript: Id | null;
  offline: boolean;
}

export function assetName(asset: MediaAsset | undefined): string {
  return asset?.path.split("/").pop() ?? "(file)";
}

// -- clips -------------------------------------------------------------------

export type SubtitleMode = "phrase" | "word" | "karaoke";
export type TextAlign = "left" | "center" | "right";

export interface TextStyle {
  font: string;
  size: number;
  color: string;
  bg: string | null;
  stroke_color: string | null;
  stroke_width: number;
  highlight_color: string | null;
  x_offset: number;
  y_offset: number;
  align: TextAlign;
  /** Line spacing as a multiple of the font size (wrapped captions). */
  line_height: number;
}

export const DEFAULT_TEXT_STYLE: TextStyle = {
  font: "sans-serif",
  size: 60,
  color: "#ffffff",
  bg: null,
  stroke_color: null,
  stroke_width: 0,
  highlight_color: null,
  x_offset: 0,
  y_offset: 0,
  align: "center",
  line_height: 1.2,
};

export type ClipPayload =
  | { type: "media"; asset_id: Id; src_in: TimeUs; src_out: TimeUs }
  | { type: "text"; content: string; style: TextStyle }
  | {
      type: "subtitles";
      transcript_id: Id;
      style: TextStyle;
      mode: SubtitleMode;
      /** Hard cap on the words a caption holds; null = fit to the frame width. */
      max_words?: number | null;
    }
  | {
      type: "generator";
      generator_id: string;
      params: Record<string, Param>;
      color_params: Record<string, string>;
    };

export interface Transform2D {
  position: [Param, Param];
  scale: [Param, Param];
  rotation: Param;
  crop: [Param, Param, Param, Param];
  opacity: Param;
  flip_h: boolean;
  flip_v: boolean;
}

export const DEFAULT_TRANSFORM: Transform2D = {
  position: [0, 0],
  scale: [1, 1],
  rotation: 0,
  crop: [0, 0, 0, 0],
  opacity: 1,
  flip_h: false,
  flip_v: false,
};

export interface AudioProps {
  gain_db: Param;
  pan: Param;
  fade_in_us: TimeUs;
  fade_out_us: TimeUs;
  muted: boolean;
  denoise: boolean;
}

export const DEFAULT_AUDIO: AudioProps = {
  gain_db: 0,
  pan: 0,
  fade_in_us: 0,
  fade_out_us: 0,
  muted: false,
  denoise: false,
};

export interface EffectInstance {
  effect_id: string;
  enabled: boolean;
  params: Record<string, Param>;
  color_params: Record<string, string>;
}

export interface TransitionRef {
  effect_id: string;
  duration: TimeUs;
  params: Record<string, Param>;
}

export interface Clip {
  id: Id;
  payload: ClipPayload;
  start: TimeUs;
  duration: TimeUs;
  speed: number;
  effects: EffectInstance[];
  transform: Transform2D;
  audio: AudioProps;
  transition_in: TransitionRef | null;
  transition_out: TransitionRef | null;
  label_color: string | null;
  /** linked clips (video+audio from the same media) share a group */
  group: Id | null;
}

// -- sequence / project ------------------------------------------------------

export interface Track {
  id: Id;
  kind: TrackKind;
  name: string;
  muted: boolean;
  solo: boolean;
  locked: boolean;
  volume_db: number;
  clips: Clip[];
}

export interface Marker {
  id: Id;
  t: TimeUs;
  name: string;
  color: string | null;
}

export interface Sequence {
  id: Id;
  name: string;
  resolution: [number, number];
  fps: [number, number];
  sample_rate: number;
  tracks: Track[];
  markers: Marker[];
}

export interface ProjectSettings {
  whisper_language: string;
  whisper_model: string;
  autosave_secs: number;
}

/** What captions and the text editor show (user correction wins). */
export function wordLabel(w: TranscriptWord): string {
  return w.display ?? w.text;
}

export interface TranscriptWord {
  text: string;
  start_us: TimeUs;
  end_us: TimeUs;
  confidence: number;
  rejected: boolean;
  display?: string | null;
}

export interface TranscriptSegment {
  text: string;
  start_us: TimeUs;
  end_us: TimeUs;
  word_range: [number, number];
  emotion: string | null;
  volume_rms: number;
}

export interface TranscriptDoc {
  id: Id;
  asset_id: Id;
  language: string;
  model: string;
  words: TranscriptWord[];
  segments: TranscriptSegment[];
  global_avg_volume: number;
}

export interface Project {
  schema_version: number;
  id: Id;
  name: string;
  created_at: string;
  settings: ProjectSettings;
  assets: MediaAsset[];
  transcripts: TranscriptDoc[];
  avatars: AvatarConfig[];
  sequences: Sequence[];
  active_sequence: Id;
}

/** Position of an asset word on the timeline (via the first clip that contains it). */
export function wordTimelineRange(
  project: Project,
  assetId: Id,
  word: TranscriptWord,
): [TimeUs, TimeUs] | null {
  const seq = activeSequence(project);
  for (const track of seq.tracks) {
    for (const clip of track.clips) {
      if (clip.payload.type !== "media" || clip.payload.asset_id !== assetId) continue;
      const { src_in, src_out } = clip.payload;
      if (word.start_us >= src_in && word.start_us < src_out) {
        const s = clip.start + Math.round((word.start_us - src_in) / clip.speed);
        const e = clip.start + Math.round((Math.min(word.end_us, src_out) - src_in) / clip.speed);
        return [s, Math.max(e, s + 1000)];
      }
    }
  }
  return null;
}

/** Asset time → timeline via the first media clip that contains it. */
export function assetTimeToTimeline(
  project: Project,
  assetId: Id,
  tAsset: TimeUs,
): TimeUs | null {
  const seq = activeSequence(project);
  for (const track of seq.tracks) {
    for (const clip of track.clips) {
      if (clip.payload.type !== "media" || clip.payload.asset_id !== assetId) continue;
      const { src_in, src_out } = clip.payload;
      if (tAsset >= src_in && tAsset < src_out) return clip.start + (tAsset - src_in);
    }
  }
  return null;
}

/** Text of the active subtitle of a Subtitles clip at the given time
 *  (respects the mode: full phrase or a single, larger word). */
// Caption chunking — MUST mirror ue-export/src/graph.rs (preview = export).
const CAPTION_GAP_US = 900_000;
const CAPTION_MAX_DUR_US = 6_000_000;
const CAPTION_LINGER_US = 600_000;

export function captionMaxChars(canvasW: number, fontPx: number): number {
  return Math.min(64, Math.max(12, Math.round((canvasW * 0.86) / (fontPx * 0.52))));
}

/**
 * Builds caption phrases straight from the WORDS (Whisper's own segments can
 * span minutes of continuous speech). New caption on: line full, pause >
 * CAPTION_GAP_US, or duration > CAPTION_MAX_DUR_US.
 */
export function captionPhrases(
  doc: TranscriptDoc,
  maxChars: number,
  maxWords?: number | null,
): { text: string; words: TranscriptWord[]; s: TimeUs; e: TimeUs }[] {
  const words = doc.words.filter((w) => !w.rejected);
  if (!words.length) {
    return doc.segments.map((s) => ({ text: s.text, words: [], s: s.start_us, e: s.end_us }));
  }
  const cap = maxWords ? Math.min(20, Math.max(1, Math.round(maxWords))) : null;
  const cuts = [0];
  let chars = wordLabel(words[0]).length;
  let count = 1;
  let chunkStart = words[0].start_us;
  for (let i = 1; i < words.length; i++) {
    const w = words[i];
    const gap = w.start_us - words[i - 1].end_us;
    // an explicit word cap REPLACES the width heuristic (mirror of graph.rs)
    const full = cap ? count >= cap : chars + 1 + wordLabel(w).length > maxChars;
    const tooSlow = w.end_us - chunkStart > CAPTION_MAX_DUR_US;
    if (full || gap > CAPTION_GAP_US || tooSlow) {
      cuts.push(i);
      chars = wordLabel(w).length;
      count = 1;
      chunkStart = w.start_us;
    } else {
      chars += 1 + wordLabel(w).length;
      count += 1;
    }
  }
  cuts.push(words.length);
  const out = [];
  for (let c = 0; c < cuts.length - 1; c++) {
    const group = words.slice(cuts[c], cuts[c + 1]);
    const naturalEnd = group[group.length - 1].end_us + CAPTION_LINGER_US;
    const next = words[cuts[c + 1]];
    const e = next ? Math.min(naturalEnd, next.start_us) : naturalEnd;
    out.push({
      text: group.map((w) => wordLabel(w)).join(" "),
      words: group,
      s: group[0].start_us,
      e: Math.max(e, group[0].start_us + 1),
    });
  }
  return out;
}

export function activeSubtitleText(
  project: Project,
  clip: Clip,
  playheadUs: TimeUs,
): {
  content: string;
  style: TextStyle;
  /** Karaoke: the phrase split into pieces with the current/past words marked. */
  spans?: { text: string; active: boolean }[];
} | null {
  if (clip.payload.type !== "subtitles") return null;
  const { transcript_id, style, mode, max_words } = clip.payload;
  const doc = project.transcripts.find((t) => t.id === transcript_id);
  if (!doc) return null;
  const seq = activeSequence(project);
  const fontPx = style.size * (seq.resolution[1] / 1080);

  if (mode === "phrase" || mode === "karaoke") {
    const phrases = captionPhrases(doc, captionMaxChars(seq.resolution[0], fontPx), max_words);
    for (const ph of phrases) {
      const tlStart = assetTimeToTimeline(project, doc.asset_id, ph.s);
      if (tlStart === null) continue;
      const from = Math.max(tlStart, clip.start);
      const to = Math.min(tlStart + (ph.e - ph.s), clip.start + clip.duration);
      if (playheadUs < from || playheadUs >= to) continue;
      if (mode === "phrase" || !ph.words.length) return { content: ph.text, style };
      const spans = ph.words.map((w) => {
        const wTl = assetTimeToTimeline(project, doc.asset_id, w.start_us) ?? tlStart;
        return { text: wordLabel(w), active: playheadUs >= wTl };
      });
      return { content: ph.text, style, spans };
    }
    return null;
  }

  // word mode: one big word at a time (shorts style)
  const effStyle = { ...style, size: style.size * 1.6 };
  for (const w of doc.words) {
    if (w.rejected) continue;
    const tlStart = assetTimeToTimeline(project, doc.asset_id, w.start_us);
    if (tlStart === null) continue;
    const from = Math.max(tlStart, clip.start);
    const to = Math.min(tlStart + (w.end_us - w.start_us), clip.start + clip.duration);
    if (playheadUs >= from && playheadUs < to) return { content: wordLabel(w), style: effStyle };
  }
  return null;
}

/** Timeline ranges for a set of words, with padding and merging. */
export function wordsToCutRanges(
  project: Project,
  assetId: Id,
  words: TranscriptWord[],
  padUs = 80_000,
  mergeGapUs = 120_000,
): [TimeUs, TimeUs][] {
  const raw: [TimeUs, TimeUs][] = [];
  for (const w of words) {
    const r = wordTimelineRange(project, assetId, w);
    if (r) raw.push([Math.max(0, r[0] - padUs), r[1] + padUs]);
  }
  raw.sort((a, b) => a[0] - b[0]);
  const merged: [TimeUs, TimeUs][] = [];
  for (const r of raw) {
    const last = merged[merged.length - 1];
    if (last && r[0] <= last[1] + mergeGapUs) last[1] = Math.max(last[1], r[1]);
    else merged.push([...r]);
  }
  return merged;
}

export function activeSequence(project: Project): Sequence {
  return (
    project.sequences.find((s) => s.id === project.active_sequence) ??
    project.sequences[0]
  );
}

export function clipDisplayName(clip: Clip, project: Project): string {
  switch (clip.payload.type) {
    case "media":
      return assetName(project.assets.find((a) => a.id === (clip.payload as { asset_id: Id }).asset_id));
    case "text":
      return clip.payload.content.length > 24
        ? "Title"
        : clip.payload.content || "Text";
    case "subtitles":
      return "Subtitles";
    case "generator":
      return clip.payload.generator_id === "core.gradient" ? "Gradient" : "Rectangle";
  }
}

// -- effect catalog (packs) ---------------------------------------------------

export interface EffectParamDef {
  key: string;
  label?: string | null;
  type: "float" | "color";
  default: number | string;
  min?: number;
  max?: number;
}

export interface EffectDef {
  id: string;
  name: string;
  category: string;
  params: EffectParamDef[];
  ffmpeg: string;
  notes?: string | null;
}

/** Manifest of a generator (rectangle, gradient, …). */
export interface GeneratorDef {
  id: string;
  name: string;
  params: EffectParamDef[];
  source: string;
  notes?: string | null;
}

/** A new effect instance with the manifest defaults. */
export function instantiateEffect(def: EffectDef): EffectInstance {
  const params: Record<string, Param> = {};
  const color_params: Record<string, string> = {};
  for (const p of def.params) {
    if (p.type === "float") params[p.key] = p.default as number;
    else color_params[p.key] = p.default as string;
  }
  return { effect_id: def.id, enabled: true, params, color_params };
}

// -- engine snapshot ---------------------------------------------------------

export interface StateSnapshot {
  project: Project;
  version: number;
  dirty: boolean;
  can_undo: boolean;
  can_redo: boolean;
  undo_labels: string[];
}
