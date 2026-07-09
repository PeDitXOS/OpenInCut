// Caché de visuales del timeline (waveforms y miniaturas reales por asset).
// Vive fuera de zustand: los datos son pesados y no serializables; el store
// solo lleva un contador (visualsBump) para disparar el redibujado.

import type { Id, Project, ThumbStrip } from "../engine/types";
import { engine, useStore } from "./store";

export interface AssetVisuals {
  /** Picos |amp| 0..1 a 25 bins/s (mezcla mono). */
  peaks?: Float32Array;
  /** Tira de miniaturas decodificada + su meta. */
  strip?: ImageBitmap;
  stripMeta?: ThumbStrip;
}

export const PEAKS_PER_SEC = 25;

const cache = new Map<Id, AssetVisuals>();
const pending = new Set<string>();

export function assetVisuals(assetId: Id): AssetVisuals | undefined {
  return cache.get(assetId);
}

function bump() {
  useStore.setState((s) => ({ visualsBump: s.visualsBump + 1 }));
}

function merge(assetId: Id, patch: Partial<AssetVisuals>) {
  cache.set(assetId, { ...cache.get(assetId), ...patch });
  bump();
}

/**
 * Pide (una sola vez por asset) los visuales que falten. Idempotente y barata:
 * llamar en cada render del timeline. Solo motor de escritorio.
 */
export function requestVisuals(project: Project) {
  if (engine.kind !== "tauri") return;
  for (const a of project.assets) {
    if (a.offline) continue;
    const key = a.id;
    if (a.audio_conform && !cache.get(key)?.peaks && !pending.has(`${key}:p`)) {
      pending.add(`${key}:p`);
      engine
        .getAudioPeaks(key)
        .then((peaks) => {
          if (peaks?.length) merge(key, { peaks: Float32Array.from(peaks) });
        })
        .catch(() => {
          /* p. ej. conformado aún en curso: reintentar en el próximo render */
          pending.delete(`${key}:p`);
        });
    }
    if (a.kind !== "audio" && !cache.get(key)?.strip && !pending.has(`${key}:t`)) {
      pending.add(`${key}:t`);
      void (async () => {
        try {
          const meta = await engine.ensureThumbs(key);
          if (!meta) return;
          const bytes = await engine.getThumbStrip(key);
          if (!bytes) return;
          const bitmap = await createImageBitmap(
            new Blob([bytes.slice().buffer], { type: "image/jpeg" }),
          );
          merge(key, { strip: bitmap, stripMeta: meta });
        } catch {
          /* sin miniaturas para este asset (no reintentar en bucle) */
        }
      })();
    }
  }
}
