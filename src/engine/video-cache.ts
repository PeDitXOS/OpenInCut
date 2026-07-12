import {
  Input,
  ALL_FORMATS,
  UrlSource,
  CanvasSink,
  type WrappedCanvas,
} from "mediabunny";

import type { Id } from "./types";
import type { LayerPixels } from "./compositor";
import { engine } from "../state/store";

/**
 * Frame decoder for the preview — a port of OpenCut's `VideoCache`.
 *
 * mediabunny (WebCodecs) decodes any supported video to canvas frames at an
 * arbitrary time, with the next frame prefetched during playback. This is the
 * ONE source of pixels for BOTH the paused and the playing preview. Adapted
 * for Tauri: media streams through the asset protocol via `UrlSource` (range
 * requests) instead of a `File` blob.
 *
 * Two departures from the original:
 * - Streams are keyed PER CLIP, not per asset: two clips of the same file on
 *   different tracks each get their own decoder, otherwise their seeks fight
 *   over one iterator and both layers show the wrong frame.
 * - Codecs WebCodecs cannot decode (e.g. the generated avatar's qtrle .mov,
 *   which carries real alpha) fall back to single frames decoded by the
 *   backend with ffmpeg (PNG, alpha preserved), throttled to keep playback
 *   responsive. Correctness first: the layer is visible, exact when paused.
 */

interface VideoSinkData {
  input: Input;
  sink: CanvasSink;
  iterator: AsyncGenerator<WrappedCanvas, void, unknown> | null;
  currentFrame: WrappedCanvas | null;
  nextFrame: WrappedCanvas | null;
  lastTime: number;
  prefetching: boolean;
  prefetchPromise: Promise<void> | null;
  lastUsed: number;
}

/** Streams kept alive at once; ~4 layers × current/adjacent clips. */
const MAX_SINKS = 12;

/** No single decode may stall the (single-flight) preview loop for longer. */
const FRAME_TIMEOUT_MS = 400;
/** Opening a demuxer reads the container over the asset protocol: allow more. */
const INIT_TIMEOUT_MS = 4000;

const TIMED_OUT = Symbol("timed-out");

/** Resolves to TIMED_OUT instead of hanging forever. */
function withTimeout<T>(p: Promise<T>, ms: number, _what: string): Promise<T | typeof TIMED_OUT> {
  let timer = 0;
  const bomb = new Promise<typeof TIMED_OUT>((res) => {
    timer = window.setTimeout(() => res(TIMED_OUT), ms);
  });
  return Promise.race([p, bomb]).finally(() => window.clearTimeout(timer));
}

/** Backend fallback refresh distance (s): ~12 fps for that layer. */
const FALLBACK_STEP_S = 0.08;

interface FallbackFrame {
  bitmap: ImageBitmap | null;
  timeSec: number;
  pending: boolean;
}

class VideoCache {
  private sinks = new Map<Id, VideoSinkData>(); // key: clip id
  private initPromises = new Map<Id, Promise<void>>();
  private frameChain = new Map<Id, Promise<unknown>>();
  private seekGenerations = new Map<Id, number>();
  private failed = new Set<Id>(); // key: asset id (decode failure is per codec)
  private fallback = new Map<Id, FallbackFrame>(); // key: asset id
  /** Last frame that DID decode, per clip: shown while a stream recovers. */
  private lastFrames = new Map<Id, WrappedCanvas>();

  private images = new Map<Id, { el: HTMLImageElement; ready: boolean }>();

  /**
   * Decoded pixels for a video asset at `time` (seconds), or null.
   *
   * EVERY await here is bounded. A decode that never settles used to hang the
   * whole preview forever: the render loop is single-flight, so one stuck
   * promise means no frame ever completes again — CPU at 0%, the picture
   * frozen, looking exactly like "10 seconds per frame". A timeout degrades to
   * the previous frame instead, and says so.
   */
  async getFrameAt(assetId: Id, clipId: Id, time: number): Promise<LayerPixels | null> {
    if (this.failed.has(assetId)) return this.backendFrame(assetId, time);
    const ready = await withTimeout(this.ensureSink(assetId, clipId), INIT_TIMEOUT_MS, "sink init");
    if (ready === TIMED_OUT) {
      engine.uiLog("warn", `decoder for ${assetId} did not open in ${INIT_TIMEOUT_MS} ms`);
      return this.lastGood(clipId);
    }
    if (this.failed.has(assetId)) return this.backendFrame(assetId, time);

    const sinkData = this.sinks.get(clipId);
    if (!sinkData) return null;
    sinkData.lastUsed = performance.now();

    const generation = (this.seekGenerations.get(clipId) ?? 0) + 1;
    this.seekGenerations.set(clipId, generation);

    const previous = this.frameChain.get(clipId) ?? Promise.resolve();
    const current = previous.then(() => {
      if (this.seekGenerations.get(clipId) !== generation) {
        return sinkData.currentFrame ?? null;
      }
      return this.resolveFrame(sinkData, time);
    });
    // the chain must NEVER carry a pending promise forward: bound it here too,
    // or one stuck decode poisons every later frame of this clip
    this.frameChain.set(
      clipId,
      withTimeout(current, FRAME_TIMEOUT_MS, "frame").catch(() => {}),
    );
    const frame = await withTimeout(current, FRAME_TIMEOUT_MS, "frame").catch(() => null);
    if (frame === TIMED_OUT) {
      engine.uiLog("warn", `decode of ${assetId} @ ${time.toFixed(2)}s exceeded ${FRAME_TIMEOUT_MS} ms`);
      // the stream is wedged: drop it so the next frame reopens cleanly
      this.dropSink(clipId);
      return this.lastGood(clipId);
    }
    if (!frame) return null;
    return { source: frame.canvas, sw: frame.canvas.width, sh: frame.canvas.height };
  }

  /** Latest frame we successfully decoded for this clip (avoids a black flash). */
  private lastGood(clipId: Id): LayerPixels | null {
    const f = this.sinks.get(clipId)?.currentFrame ?? this.lastFrames.get(clipId);
    if (!f) return null;
    return { source: f.canvas, sw: f.canvas.width, sh: f.canvas.height };
  }

  private dropSink(clipId: Id): void {
    const s = this.sinks.get(clipId);
    if (!s) return;
    if (s.currentFrame) this.lastFrames.set(clipId, s.currentFrame);
    this.sinks.delete(clipId);
    this.frameChain.delete(clipId);
    this.seekGenerations.delete(clipId);
    void s.iterator?.return().catch(() => {});
    try {
      s.input.dispose();
    } catch {
      /* already gone */
    }
  }

  /** Single frames decoded by the backend (ffmpeg → PNG with alpha), for
   *  codecs the webview cannot decode. Throttled: returns the latest frame
   *  immediately and refreshes it in the background. */
  private backendFrame(assetId: Id, time: number): LayerPixels | null {
    let f = this.fallback.get(assetId);
    if (!f) {
      f = { bitmap: null, timeSec: -1, pending: false };
      this.fallback.set(assetId, f);
    }
    if (!f.pending && (f.bitmap === null || Math.abs(time - f.timeSec) > FALLBACK_STEP_S)) {
      f.pending = true;
      const want = time;
      void engine
        .renderAssetFrame(assetId, Math.round(want * 1e6), 1280)
        .then(async (bytes) => {
          if (!bytes) return;
          const bitmap = await createImageBitmap(new Blob([bytes as BlobPart]));
          const cur = this.fallback.get(assetId);
          if (cur) {
            cur.bitmap?.close();
            cur.bitmap = bitmap;
            cur.timeSec = want;
          }
        })
        .catch((e) => engine.uiLog("warn", `fallback frame ${assetId}: ${e}`))
        .finally(() => {
          const cur = this.fallback.get(assetId);
          if (cur) cur.pending = false;
        });
    }
    if (!f.bitmap) return null;
    return { source: f.bitmap, sw: f.bitmap.width, sh: f.bitmap.height };
  }

  private async resolveFrame(sinkData: VideoSinkData, time: number): Promise<WrappedCanvas | null> {
    if (sinkData.nextFrame && sinkData.nextFrame.timestamp <= time) {
      sinkData.currentFrame = sinkData.nextFrame;
      sinkData.nextFrame = null;
      this.startPrefetch(sinkData);
    }

    if (sinkData.currentFrame && this.isFrameValid(sinkData.currentFrame, time)) {
      if (!sinkData.nextFrame && !sinkData.prefetching) this.startPrefetch(sinkData);
      return sinkData.currentFrame;
    }

    // small forward step: iterate instead of a full seek (keeps playback smooth)
    if (
      sinkData.iterator &&
      sinkData.currentFrame &&
      time >= sinkData.lastTime &&
      time < sinkData.lastTime + 2.0
    ) {
      const frame = await this.iterateToTime(sinkData, time);
      if (frame) {
        if (!sinkData.nextFrame && !sinkData.prefetching) this.startPrefetch(sinkData);
        return frame;
      }
    }

    const frame = await this.seekToTime(sinkData, time);
    if (frame && !sinkData.nextFrame && !sinkData.prefetching) this.startPrefetch(sinkData);
    return frame;
  }

  private isFrameValid(frame: WrappedCanvas, time: number): boolean {
    return time >= frame.timestamp && time < frame.timestamp + frame.duration;
  }

  private async iterateToTime(sinkData: VideoSinkData, targetTime: number): Promise<WrappedCanvas | null> {
    if (!sinkData.iterator) return null;
    try {
      while (true) {
        if (sinkData.prefetching && sinkData.prefetchPromise) await sinkData.prefetchPromise;

        if (sinkData.nextFrame && sinkData.nextFrame.timestamp <= targetTime + 0.05) {
          sinkData.currentFrame = sinkData.nextFrame;
          sinkData.nextFrame = null;
        } else {
          const { value: frame, done } = await sinkData.iterator.next();
          if (done || !frame) break;
          sinkData.currentFrame = frame;
        }

        const frame = sinkData.currentFrame;
        if (!frame) break;
        sinkData.lastTime = frame.timestamp;

        if (this.isFrameValid(frame, targetTime)) return frame;
        if (frame.timestamp > targetTime + 1.0) break;
      }
    } catch (error) {
      console.warn("iterator failed, will restart:", error);
      sinkData.iterator = null;
    }
    return null;
  }

  private async seekToTime(sinkData: VideoSinkData, time: number): Promise<WrappedCanvas | null> {
    try {
      if (sinkData.prefetching && sinkData.prefetchPromise) await sinkData.prefetchPromise;
      if (sinkData.iterator) {
        await sinkData.iterator.return();
        sinkData.iterator = null;
      }
      sinkData.nextFrame = null;
      sinkData.iterator = sinkData.sink.canvases(time);
      sinkData.lastTime = time;
      const { value: frame } = await sinkData.iterator.next();
      if (frame) {
        sinkData.currentFrame = frame;
        this.startPrefetch(sinkData);
        return frame;
      }
    } catch (error) {
      console.warn("failed to seek video:", error);
    }
    return null;
  }

  private startPrefetch(sinkData: VideoSinkData): void {
    if (sinkData.prefetching || !sinkData.iterator || sinkData.nextFrame) return;
    sinkData.prefetching = true;
    sinkData.prefetchPromise = this.prefetchNextFrame(sinkData);
  }

  private async prefetchNextFrame(sinkData: VideoSinkData): Promise<void> {
    if (!sinkData.iterator) {
      sinkData.prefetching = false;
      sinkData.prefetchPromise = null;
      return;
    }
    try {
      const { value: frame, done } = await sinkData.iterator.next();
      sinkData.nextFrame = done || !frame ? null : frame;
    } catch (error) {
      console.warn("prefetch failed:", error);
      sinkData.iterator = null;
    } finally {
      sinkData.prefetching = false;
      sinkData.prefetchPromise = null;
    }
  }

  private async ensureSink(assetId: Id, clipId: Id): Promise<void> {
    if (this.sinks.has(clipId)) return;
    const existing = this.initPromises.get(clipId);
    if (existing) return existing;
    const initPromise = this.initializeSink(assetId, clipId);
    this.initPromises.set(clipId, initPromise);
    try {
      await initPromise;
    } catch {
      this.failed.add(assetId);
    } finally {
      this.initPromises.delete(clipId);
    }
  }

  private evictIfNeeded(): void {
    if (this.sinks.size < MAX_SINKS) return;
    let oldest: Id | null = null;
    let oldestUsed = Infinity;
    for (const [key, s] of this.sinks) {
      if (s.lastUsed < oldestUsed) {
        oldestUsed = s.lastUsed;
        oldest = key;
      }
    }
    if (oldest !== null) {
      const s = this.sinks.get(oldest);
      this.sinks.delete(oldest);
      this.frameChain.delete(oldest);
      this.seekGenerations.delete(oldest);
      void s?.iterator?.return().catch(() => {});
      s?.input.dispose();
    }
  }

  private async initializeSink(assetId: Id, clipId: Id): Promise<void> {
    const url = await engine.resolveAssetUrl(assetId);
    if (!url) throw new Error("no asset url");
    const input = new Input({ source: new UrlSource(url), formats: ALL_FORMATS });
    try {
      const videoTrack = await input.getPrimaryVideoTrack();
      if (!videoTrack) throw new Error("no video track");
      if (!(await videoTrack.canDecode())) throw new Error("codec not decodable");
      const sink = new CanvasSink(videoTrack, { poolSize: 3, fit: "contain" });
      this.evictIfNeeded();
      this.sinks.set(clipId, {
        input,
        sink,
        iterator: null,
        currentFrame: null,
        nextFrame: null,
        lastTime: -1,
        prefetching: false,
        prefetchPromise: null,
        lastUsed: performance.now(),
      });
    } catch (error) {
      input.dispose();
      engine.uiLog(
        "warn",
        `webview cannot decode asset ${assetId} (${error}); using backend frame fallback`,
      );
      throw error;
    }
  }

  /** Loaded image element for an image asset, or null while it loads. */
  getImage(assetId: Id): HTMLImageElement | null {
    let e = this.images.get(assetId);
    if (!e) {
      const el = new Image();
      el.crossOrigin = "anonymous";
      e = { el, ready: false };
      this.images.set(assetId, e);
      void engine.resolveAssetUrl(assetId).then((url) => {
        if (!url) return;
        el.onload = () => {
          const cur = this.images.get(assetId);
          if (cur) cur.ready = true;
        };
        el.src = url;
      });
    }
    return e.ready && e.el.naturalWidth > 0 ? e.el : null;
  }
}

export const videoCache = new VideoCache();
