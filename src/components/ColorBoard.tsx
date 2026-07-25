import { useCallback, useRef, useState } from "react";

/** Color Board: Lift (shadows), Gamma (midtones), Gain (highlights) wheels + master controls. */

const WHEEL_SIZE = 90;

type WheelState = { r: number; g: number; b: number }; // -1..1 per channel

const defaultWheel: WheelState = { r: 0, g: 0, b: 0 };

const PRESETS: Record<string, Partial<ColorState>> = {
  "Neutral": {},
  "Cool shadows": { lift: { r: -0.15, g: -0.05, b: 0.2 } },
  "Warm highlights": { gain: { r: 0.2, g: 0.1, b: -0.1 } },
  "Cinematic": { lift: { r: -0.05, g: -0.1, b: 0.15 }, gain: { r: 0.15, g: 0.05, b: -0.1 } },
};

interface ColorState {
  brightness: number; // -1..1
  contrast: number; // -1..1
  saturation: number; // -1..1
  gamma: number; // 0.2..5
  lift: WheelState;
  gain: WheelState;
}

function Wheel({
  label,
  value,
  onChange,
  accentColor,
}: {
  label: string;
  value: WheelState;
  onChange: (v: WheelState) => void;
  accentColor: string;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const dragging = useRef(false);

  const cx = value.r / 2; // -0.5..0.5
  const cy = value.g / 2;

  const updateFromPointer = useCallback(
    (clientX: number, clientY: number) => {
      const el = ref.current;
      if (!el) return;
      const rect = el.getBoundingClientRect();
      const x = ((clientX - rect.left) / rect.width) * 2 - 1;
      const y = ((clientY - rect.top) / rect.height) * 2 - 1;
      const clamp = (v: number) => Math.max(-1, Math.min(1, v));
      onChange({
        r: clamp(x * 2),
        g: clamp(y * 2),
        b: clamp(-(x + y)), // blue approximated from opposite diagonal
      });
    },
    [onChange],
  );

  return (
    <div className="flex flex-col items-center gap-1">
      <span className="text-[10px] font-medium" style={{ color: accentColor }}>
        {label}
      </span>
      <div
        ref={ref}
        className="relative cursor-crosshair touch-none rounded-full border border-line"
        style={{ width: WHEEL_SIZE, height: WHEEL_SIZE, background: "var(--color-bg0)" }}
        onPointerDown={(e) => {
          dragging.current = true;
          e.currentTarget.setPointerCapture(e.pointerId);
          updateFromPointer(e.clientX, e.clientY);
        }}
        onPointerMove={(e) => {
          if (dragging.current) updateFromPointer(e.clientX, e.clientY);
        }}
        onPointerUp={() => {
          dragging.current = false;
        }}
      >
        {/* hue ring */}
        <div
          className="absolute inset-0 rounded-full opacity-20"
          style={{
            background:
              "conic-gradient(red, yellow, lime, aqua, blue, magenta, red)",
          }}
        />
        {/* crosshair */}
        <div
          className="pointer-events-none absolute h-0.5 w-0.5 rounded-full"
          style={{
            width: 6,
            height: 6,
            left: `${(cx + 1) * 50}%`,
            top: `${(cy + 1) * 50}%`,
            transform: "translate(-50%, -50%)",
            background: accentColor,
            boxShadow: `0 0 4px ${accentColor}`,
          }}
        />
      </div>
    </div>
  );
}

export function ColorBoard() {
  const [state, setState] = useState<ColorState>({
    brightness: 0,
    contrast: 0,
    saturation: 0,
    gamma: 1,
    lift: { ...defaultWheel },
    gain: { ...defaultWheel },
  });

  const update = (patch: Partial<ColorState>) =>
    setState((s) => ({ ...s, ...patch }));

  return (
    <section className="border-b border-line-soft px-3 py-3">
      <div className="mb-2 flex items-center justify-between">
        <h3 className="panel-eyebrow">Color Board</h3>
        <select
          className="focus-ring cursor-pointer rounded border border-line bg-bg2 px-1.5 py-0.5 text-[10.5px] text-ink-dim"
          value=""
          onChange={(e) => {
            const p = PRESETS[e.target.value];
            if (p) update(p);
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

      <div className="flex justify-around">
        <Wheel
          label="LIFT"
          value={state.lift}
          onChange={(v) => update({ lift: v })}
          accentColor="#64d2ff"
        />
        <Wheel
          label="GAMMA"
          value={{ r: state.brightness, g: state.contrast, b: state.saturation }}
          onChange={(v) =>
            update({ brightness: v.r, contrast: v.g, saturation: v.b })
          }
          accentColor="#ffd60a"
        />
        <Wheel
          label="GAIN"
          value={state.gain}
          onChange={(v) => update({ gain: v })}
          accentColor="#ff9f0a"
        />
      </div>

      {/* Master sliders */}
      <div className="mt-3 space-y-1.5">
        {(
          [
            ["brightness", "Brightness", -1, 1],
            ["contrast", "Contrast", -1, 1],
            ["saturation", "Saturation", -1, 1],
            ["gamma", "Gamma", 0.2, 5],
          ] as const
        ).map(([key, label, min, max]) => (
          <label
            key={key}
            className="flex items-center justify-between gap-2 text-[11px] text-ink-dim"
          >
            <span className="w-16 shrink-0">{label}</span>
            <input
              type="range"
              className="h-1 min-w-0 flex-1 cursor-pointer appearance-none rounded-full bg-bg3 accent-(--color-accent)"
              min={min}
              max={max}
              step={key === "gamma" ? 0.05 : 0.01}
              value={state[key]}
              onChange={(e) => update({ [key]: Number(e.target.value) })}
            />
            <span className="w-8 text-right font-[var(--font-mono)] text-[10px] text-ink-faint">
              {(state[key] as number).toFixed(key === "gamma" ? 1 : 2)}
            </span>
          </label>
        ))}
      </div>

      {/* Reset */}
      <button
        className="focus-ring mt-2 w-full rounded-md border border-line bg-bg2 px-2 py-1.5 text-[11px] text-ink-dim hover:text-ink"
        onClick={() =>
          setState({
            brightness: 0,
            contrast: 0,
            saturation: 0,
            gamma: 1,
            lift: { ...defaultWheel },
            gain: { ...defaultWheel },
          })
        }
      >
        ↻ Reset all
      </button>

      {/* Export summary — JSON for downstream consumption */}
      <details className="mt-2">
        <summary className="cursor-pointer text-[10px] text-ink-faint">
          Export values
        </summary>
        <pre className="mt-1 max-h-24 overflow-auto rounded bg-bg0 p-2 font-[var(--font-mono)] text-[9px] text-ink-faint">
          {JSON.stringify(
            {
              brightness: state.brightness,
              contrast: state.contrast,
              saturation: state.saturation,
              gamma: state.gamma,
              lift_r: state.lift.r,
              lift_g: state.lift.g,
              lift_b: state.lift.b,
              gain_r: state.gain.r,
              gain_g: state.gain.g,
              gain_b: state.gain.b,
            },
            null,
            2,
          )}
        </pre>
      </details>
    </section>
  );
}
