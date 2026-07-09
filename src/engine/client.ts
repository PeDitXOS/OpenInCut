import type {
  AudioProps,
  EffectDef,
  EffectInstance,
  Id,
  StateSnapshot,
  TimeUs,
  Transform2D,
  TransitionRef,
} from "./types";

/**
 * Contrato del motor de edición. Dos implementaciones:
 * - TauriEngine: la app de escritorio real (ue-core vía IPC).
 * - MockEngine: navegador, para desarrollo de UI y pruebas visuales.
 */
export interface EngineClient {
  readonly kind: "tauri" | "mock";

  getState(): Promise<StateSnapshot>;
  splitClip(clipId: Id, tUs: TimeUs): Promise<StateSnapshot>;
  deleteClips(ids: Id[], ripple: boolean): Promise<StateSnapshot>;
  moveClip(
    clipId: Id,
    toTrack: Id,
    toStartUs: TimeUs,
    overwrite: boolean,
  ): Promise<StateSnapshot>;
  trimClip(clipId: Id, left: boolean, newEdgeUs: TimeUs): Promise<StateSnapshot>;
  undo(): Promise<StateSnapshot>;
  redo(): Promise<StateSnapshot>;
  setClipAudio(clipId: Id, audio: AudioProps): Promise<StateSnapshot>;
  setClipTransform(clipId: Id, transform: Transform2D): Promise<StateSnapshot>;

  /** Diálogo nativo de selección de archivos (null si no está disponible). */
  pickMediaFiles(): Promise<string[] | null>;
  importMedia(paths: string[]): Promise<StateSnapshot>;
  /** Añade un clip del asset al timeline (playhead o final de pista). */
  addClip(assetId: Id, atUs: TimeUs): Promise<StateSnapshot>;

  /** Frame real JPEG del tiempo dado, o null si no hay señal / no soportado. */
  renderFrame(tUs: TimeUs, maxWidth: number): Promise<Uint8Array | null>;

  /** Diálogo nativo "guardar como" (null si el usuario cancela o no hay soporte). */
  pickSavePath(defaultName: string): Promise<string | null>;
  /** Exporta la secuencia activa a MP4. Devuelve la ruta escrita. */
  exportVideo(path: string): Promise<string>;

  // -- transporte con el audio como reloj maestro (solo escritorio) --
  /** Arranca la reproducción de audio desde `fromUs`. Rechaza si no hay dispositivo. */
  playbackPlay(fromUs: TimeUs): Promise<void>;
  /** Pausa y devuelve la posición exacta del reloj de audio. */
  playbackPause(): Promise<TimeUs>;
  playbackSeek(tUs: TimeUs): Promise<void>;
  /** [posición µs, reproduciendo] según el reloj de audio. */
  playbackPosition(): Promise<[TimeUs, boolean]>;

  /** Suscripción a cambios de estado originados en el backend (jobs). */
  onStateChanged(cb: () => void): Promise<() => void>;

  // -- proyecto en disco (solo escritorio) --
  /** Guarda a la ruta dada (o a la última usada si es null). Devuelve la ruta. */
  saveProject(path: string | null): Promise<string>;
  openProject(path: string): Promise<StateSnapshot>;
  pickProjectSavePath(defaultName: string): Promise<string | null>;
  pickProjectOpenPath(): Promise<string | null>;

  /** Último frame JPEG del stream de reproducción (vacío = sin señal). */
  playbackFrame(): Promise<Uint8Array | null>;

  // -- efectos modulares --
  getEffectsCatalog(): Promise<EffectDef[]>;
  /** Recarga packs de usuario desde disco; devuelve el catálogo actualizado. */
  reloadEffectPacks(): Promise<{ catalog: EffectDef[]; errors: string[]; dir: string | null }>;
  setClipEffects(clipId: Id, effects: EffectInstance[]): Promise<StateSnapshot>;
  setClipTransition(clipId: Id, transition: TransitionRef | null): Promise<StateSnapshot>;

  // -- progreso de export --
  cancelExport(): Promise<void>;
  onExportProgress(cb: (progress: number) => void): Promise<() => void>;
}
