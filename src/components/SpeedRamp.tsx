import { useCallback, useEffect, useRef, useState } from "react";

/** Speed Ramp: bezier curve editor for variable-speed playback. */

const W = 260;
const H = 120;
const PAD = 8;

interface Point {
  x: number; // 0..1 (time)
  y: number; // 0..1 → mapped to speed range
}

interface RampState {
  points: Point[];
  minSpeed: number;
  maxSpeed: number;
}

const PRESETS: Record<string, Point[]> = {
  constant: [
    { x: 0, y: 0.5 },
    { x: 1, y: 0.5 },
  ],
  "ease-in": [
    { x: 0, y: 0.9 },
    { x: 0.5, y: 0.7 },
    { x: 0.8, y: 0.3 },
    { x: 1, y: 0.1 },
  ],
  "ease-out": [
    { x: 0, y: 0.1 },
    { x: 0.2, y: 0.3 },
    { x: 0.5, y: 0.7 },
    { x: 1, y: 0.9 },
  ],
  "ease-in-out": [
    { x: 0, y: 0.9 },
    { x: 0.3, y: 0.8 },
    { x: 0.7, y: 0.2 },
    { x: 1, y: 0.1 },
  ],
  ramp: [
    { x: 0, y: 0.5 },
    { x: 0.33, y: 0.5 },
    { x: 0.66, y: 0.1 },
    { x: 1, y: 0.1 },
  ],
};

const DEFAULT: RampState = {
  points: PRESETS.constant.map((p) => ({ ...p })),
  minSpeed: 0.25,
  maxSpeed: 4,
};

function speedAt(t: number, points: Point[], min: number, max: number): number {
  if (points.length < 2) return 1;
  // find segment
  for (let i = 0; i < points.length - 1; i++) {
    if (t >= points[i].x && t <= points[i + 1].x) {
      const p0 = points[i];
      const p1 = points[i + 1];
      const local = (t - p0.x) / Math.max(1e-6, p1.x - p0.x);
      // simple hermite-like interpolation
      const v = p0.y + (p1.y - p0.y) * (3 - 2 * local) * local;
      return min + (1 - v) * (max - min); // y=0 → max speed (top), y=1 → min speed (bottom)
    }
  }
  return points[points.length - 1].y * (max - min) + min;
}

export function SpeedRamp() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [state, setState] = useState<RampState>(DEFAULT);
  const [dragIdx, setDragIdx] = useState<number | null>(null);
  const [hoverSpeed, setHoverSpeed] = useState<number | null>(null);

  const { points, minSpeed, maxSpeed } = state;

  const draw = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const dpr = window.devicePixelRatio || 1;
    canvas.width = W * dpr;
    canvas.height = H * dpr;
    const ctx = canvas.getContext("2d")!;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, W, H);

    const toX = (v: number) => PAD + v * (W - 2 * PAD);
    const toY = (v: number) => PAD + v * (H - 2 * PAD);

    // grid lines
    ctx.strokeStyle = "rgba(233,228,219,0.06)";
    ctx.lineWidth = 1;
    for (let i = 0; i <= 4; i++) {
      const y = toY(i / 4);
      ctx.beginPath();
      ctx.moveTo(PAD, y);
      ctx.lineTo(W - PAD, y);
      ctx.stroke();
    }

    // 1x line
    const oneY = toY(1 - (1 - minSpeed) / (maxSpeed - minSpeed));
    ctx.strokeStyle = "rgba(255,178,36,0.25)";
    ctx.setLineDash([4, 4]);
    ctx.beginPath();
    ctx.moveTo(PAD, oneY);
    ctx.lineTo(W - PAD, oneY);
    ctx.stroke();
    ctx.setLineDash([]);

    // curve
    ctx.strokeStyle = "#ffb224";
    ctx.lineWidth = 2;
    ctx.beginPath();
    for (let px = 0; px <= W - 2 * PAD; px++) {
      const t = px / (W - 2 * PAD);
      const speed = speedAt(t, points, minSpeed, maxSpeed);
      const v = 1 - (speed - minSpeed) / (maxSpeed - minSpeed);
      const x = toX(t);
      const y = toY(v);
      if (px === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }
    ctx.stroke();

    // control points
    points.forEach((p, i) => {
      const x = toX(p.x);
      const y = toY(p.y);
      ctx.fillStyle = i === dragIdx ? "#ffb224" : "#e9e4db";
      ctx.beginPath();
      ctx.arc(x, y, 5, 0, Math.PI * 2);
      ctx.fill();
      ctx.strokeStyle = "rgba(0,0,0,0.4)";
      ctx.lineWidth = 1;
      ctx.stroke();
    });
  }, [points, minSpeed, maxSpeed, dragIdx]);

  useEffect(() => {
    draw();
  }, [draw]);

  const hitPoint = (mx: number, my: number): number => {
    for (let i = 0; i < points.length; i++) {
      const px = PAD + points[i].x * (W - 2 * PAD);
      const py = PAD + points[i].y * (H - 2 * PAD);
      if (Math.hypot(mx - px, my - py) < 10) return i;
    }
    return -1;
  };

  return (
    <section className="border-b border-line-soft px-3 py-3">
      <div className="mb-2 flex items-center justify-between">
        <h3 className="panel-eyebrow">Speed Ramp</h3>
        <select
          className="focus-ring cursor-pointer rounded border border-line bg-bg2 px-1.5 py-0.5 text-[10.5px] text-ink-dim"
          value=""
          onChange={(e) => {
            const pts = PRESETS[e.target.value];
            if (pts) setState((s) => ({ ...s, points: pts.map((p) => ({ ...p })) }));
          }}
        >
          <option value="">Presets…</option>
          {Object.keys(PRESETS).map((n) => (
            <option key={n} value={n}>
              {n}
            </option>
          ))}
        </select>
      </div>

      <div className="rounded-md border border-line bg-bg2/50">
        <canvas
          ref={canvasRef}
          className="block w-full touch-none"
          style={{ height: H }}
          onPointerDown={(e) => {
            const rect = e.currentTarget.getBoundingClientRect();
            const mx = (e.clientX - rect.left) * (W / rect.width);
            const my = (e.clientY - rect.top) * (H / rect.height);
            const i = hitPoint(mx, my);
            if (i >= 0) {
              setDragIdx(i);
              e.currentTarget.setPointerCapture(e.pointerId);
            } else {
              // add point
              const t = Math.max(0, Math.min(1, (mx - PAD) / (W - 2 * PAD)));
              const v = Math.max(0, Math.min(1, (my - PAD) / (H - 2 * PAD)));
              const next = [...points, { x: t, y: v }]
                .sort((a, b) => a.x - b.x)
                .slice(0, 8); // cap at 8 points
              setState((s) => ({ ...s, points: next }));
            }
          }}
          onPointerMove={(e) => {
            const rect = e.currentTarget.getBoundingClientRect();
            const mx = (e.clientX - rect.left) * (W / rect.width);
            const my = (e.clientY - rect.top) * (H / rect.height);
            const t = Math.max(0, Math.min(1, (mx - PAD) / (W - 2 * PAD)));
            const v = Math.max(0, Math.min(1, (my - PAD) / (H - 2 * PAD)));
            const speed = speedAt(t, points, minSpeed, maxSpeed);
            setHoverSpeed(speed);
            if (dragIdx === null) return;
            const next = points.map((p, i) =>
              i === dragIdx ? { x: Math.max(0, Math.min(1, t)), y: v } : p,
            );
            setState((s) => ({ ...s, points: next }));
          }}
          onPointerUp={() => {
            setDragIdx(null);
            setHoverSpeed(null);
          }}
        />
      </div>

      {hoverSpeed !== null && (
        <div className="mt-1 text-center font-[var(--font-mono)] text-[10px] text-ink-faint">
          {hoverSpeed.toFixed(2)}×
        </div>
      )}

      {/* Speed range */}
      <div className="mt-2 space-y-1">
        <label className="flex items-center justify-between gap-2 text-[11px] text-ink-dim">
          <span className="w-20 shrink-0">Min speed</span>
          <input
            type="range"
            className="h-1 min-w-0 flex-1 cursor-pointer appearance-none rounded-full bg-bg3 accent-(--color-accent)"
            min={0.1}
            max={1}
            step={0.05}
            value={minSpeed}
            onChange={(e) =>
              setState((s) => ({
                ...s,
                minSpeed: Math.min(Number(e.target.value), s.maxSpeed - 0.25),
              }))
            }
          />
          <span className="w-8 text-right font-[var(--font-mono)] text-[10px] text-ink-faint">
            {minSpeed.toFixed(2)}×
          </span>
        </label>
        <label className="flex items-center justify-between gap-2 text-[11px] text-ink-dim">
          <span className="w-20 shrink-0">Max speed</span>
          <input
            type="range"
            className="h-1 min-w-0 flex-1 cursor-pointer appearance-none rounded-full bg-bg3 accent-(--color-accent)"
            min={1}
            max={8}
            step={0.25}
            value={maxSpeed}
            onChange={(e) =>
              setState((s) => ({
                ...s,
                maxSpeed: Math.max(Number(e.target.value), s.minSpeed + 0.25),
              }))
            }
          />
          <span className="w-8 text-right font-[var(--font-mono)] text-[10px] text-ink-faint">
            {maxSpeed.toFixed(2)}×
          </span>
        </label>
      </div>

      {/* Controls */}
      <div className="mt-2 flex gap-1.5">
        <button
          className="focus-ring flex-1 rounded-md border border-line bg-bg2 px-2 py-1 text-[11px] text-ink-dim hover:text-ink"
          onClick={() =>
            setState((s) => ({
              ...s,
              points: s.points.filter((_, i) => i !== 0 && i !== s.points.length - 1),
            }))
          }
        >
          − Remove inner points
        </button>
        <button
          className="focus-ring flex-1 rounded-md border border-line bg-bg2 px-2 py-1 text-[11px] text-ink-dim hover:text-ink"
          onClick={() => setState({ ...DEFAULT })}
        >
          ↻ Reset
        </button>
      </div>
    </section>
  );
}
