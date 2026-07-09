/**
 * Espejo TypeScript del modelo de ue-core (crates/ue-core/src/model.rs).
 * Cuando exista el backend Tauri, estos tipos se generarán automáticamente;
 * mientras tanto se mantienen a mano y el MockEngine los implementa.
 */

export type Id = string;
export type TimeUs = number;

export type MediaKind = "video" | "audio" | "image";
export type TrackKind = "video" | "audio";

export interface ProbeInfo {
  duration_us: TimeUs;
  fps?: [number, number];
  width: number;
  height: number;
  audio_channels: number;
  vcodec?: string;
  acodec?: string;
}

export interface MediaAsset {
  id: Id;
  kind: MediaKind;
  path: string;
  probe: ProbeInfo;
  /** progreso 0..1 de jobs de caché (mock); undefined = listo */
  caching?: number;
}

export type ClipPayload =
  | { type: "media"; asset_id: Id; src_in: TimeUs; src_out: TimeUs }
  | { type: "text"; content: string }
  | { type: "solid"; color: string };

export interface Clip {
  id: Id;
  payload: ClipPayload;
  start: TimeUs;
  duration: TimeUs;
  speed: number;
  gain_db: number;
  fade_in_us: number;
  fade_out_us: number;
  opacity: number;
  label?: string;
}

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
  color?: string;
}

export interface Sequence {
  id: Id;
  name: string;
  resolution: [number, number];
  fps: [number, number];
  tracks: Track[];
  markers: Marker[];
}

export interface Project {
  name: string;
  assets: MediaAsset[];
  sequences: Sequence[];
  active_sequence: Id;
}

export interface Job {
  id: Id;
  label: string;
  progress: number; // 0..1
}
