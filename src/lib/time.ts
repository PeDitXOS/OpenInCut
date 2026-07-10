export const US_PER_SEC = 1_000_000;

export type Fps = [number, number];

export function usToFrame(us: number, fps: Fps): number {
  return Math.round((us * fps[0]) / (fps[1] * US_PER_SEC));
}

export function frameToUs(frame: number, fps: Fps): number {
  return Math.round((frame * fps[1] * US_PER_SEC) / fps[0]);
}

export function quantizeToFrame(us: number, fps: Fps): number {
  return frameToUs(usToFrame(us, fps), fps);
}

/** HH:MM:SS:FF */
export function usToTimecode(us: number, fps: Fps): string {
  const fpsReal = fps[0] / fps[1];
  const totalSec = Math.floor(us / US_PER_SEC);
  const frame = Math.floor(((us % US_PER_SEC) / US_PER_SEC) * fpsReal);
  const h = Math.floor(totalSec / 3600);
  const m = Math.floor((totalSec % 3600) / 60);
  const s = totalSec % 60;
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(h)}:${pad(m)}:${pad(s)}:${pad(frame)}`;
}

/** M:SS or H:MM:SS for readable durations */
export function usToDuration(us: number): string {
  const totalSec = Math.round(us / US_PER_SEC);
  const h = Math.floor(totalSec / 3600);
  const m = Math.floor((totalSec % 3600) / 60);
  const s = totalSec % 60;
  const pad = (n: number) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
}

/** deterministic hash (for the mock's fake waveforms and filmstrips) */
export function hash32(str: string): number {
  let h = 2166136261;
  for (let i = 0; i < str.length; i++) {
    h ^= str.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return h >>> 0;
}

export function mulberry32(seed: number): () => number {
  let a = seed;
  return () => {
    a |= 0;
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}
