// Timeline visuals cache (real waveforms and thumbnails per asset).
// Lives outside zustand: the data is heavy and non-serializable; the store
// only carries a counter (visualsBump) to trigger a redraw.

import type { Id, Project, ThumbStrip } from "../engine/types";
import { engine, useStore } from "./store";

export interface AssetVisuals {
  /** Peaks |amp| 0..1 at 25 bins/s (mono mix). */
  peaks?: Float32Array;
  /** Decoded thumbnail strip + its meta. */
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
 * Requests (once per asset) the missing visuals. Idempotent and cheap:
 * call it on every timeline render. Desktop engine only.
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
          /* e.g. conform still in progress: retry on the next render */
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
          /* no thumbnails for this asset (don't retry in a loop) */
        }
      })();
    }
  }
}
