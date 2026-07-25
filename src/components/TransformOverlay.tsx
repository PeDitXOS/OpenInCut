import { useRef, useEffect, useState } from "react";
import { useStore } from "../state/store";
import {
  activeSequence,
  paramValue,
  isCurve,
  withKeyAt,
  type Param,
  type Transform2D,
} from "../engine/types";

const HANDLE = 8;
const ROT_ARM = 24;

function drive(p: Param, tUs: number, v: number): Param {
  return isCurve(p) ? withKeyAt(p, tUs, v) : v;
}

interface Drag {
  kind: "move" | "scale" | "rotate";
  mx0: number;
  my0: number;
  tf: Transform2D;
  cornerX: 1 | -1;
  cornerY: 1 | -1;
  prevAngle: number;
  accDeg: number;
}

const CORNER_CURSORS = ["nwse-resize", "nesw-resize", "nwse-resize", "nesw-resize"];

export function TransformOverlay() {
  const rootRef = useRef<HTMLDivElement>(null);
  const [sz, setSz] = useState({ w: 0, h: 0 });
  const dragRef = useRef<Drag | null>(null);

  const selection = useStore((s) => s.selection);
  const project = useStore((s) => s.project);
  const playheadUs = useStore((s) => s.playheadUs);
  const setClipTransform = useStore((s) => s.setClipTransform);

  useEffect(() => {
    const el = rootRef.current;
    if (!el) return;
    const ro = new ResizeObserver((e) => {
      const r = e[0].contentRect;
      setSz({ w: r.width, h: r.height });
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const clipId = selection[0];
  if (!clipId) return null;

  const seq = activeSequence(project);
  const clip = seq.tracks.flatMap((t) => t.clips).find((c) => c.id === clipId);
  if (!clip) return null;

  const relUs = Math.max(0, playheadUs - clip.start);
  const t = clip.transform;
  const tpx = paramValue(t.position[0], relUs);
  const tpy = paramValue(t.position[1], relUs);
  const tsx = Math.max(0.01, paramValue(t.scale[0], relUs));
  const tsy = Math.max(0.01, paramValue(t.scale[1], relUs));
  const rot = paramValue(t.rotation, relUs);

  if (sz.w === 0 || sz.h === 0) return null;
  const { w, h } = sz;
  const k = w / seq.resolution[0];

  // Bounding box in overlay CSS px
  const cx = w / 2 + tpx * k;
  const cy = h / 2 + tpy * k;
  const bw = w * tsx;
  const bh = h * tsy;
  const bx = cx - bw / 2;
  const by = cy - bh / 2;

  const corners = [
    { x: bx, y: by },
    { x: bx + bw, y: by },
    { x: bx + bw, y: by + bh },
    { x: bx, y: by + bh },
  ];
  const edges = [
    { x: cx, y: by },
    { x: bx + bw, y: cy },
    { x: cx, y: by + bh },
    { x: bx, y: cy },
  ];
  const rotY = by - ROT_ARM;

  const onDown = (
    e: React.PointerEvent,
    kind: Drag["kind"],
    hx: number,
    hy: number,
  ) => {
    e.preventDefault();
    e.stopPropagation();
    (e.target as HTMLElement).setPointerCapture(e.pointerId);
    const a = Math.atan2(e.clientY - cy, e.clientX - cx);
    dragRef.current = {
      kind,
      mx0: e.clientX,
      my0: e.clientY,
      tf: { ...t },
      cornerX: hx > cx ? 1 : -1,
      cornerY: hy > cy ? 1 : -1,
      prevAngle: a,
      accDeg: 0,
    };

    const onMove = (ev: PointerEvent) => {
      const d = dragRef.current;
      if (!d) return;
      const dx = ev.clientX - d.mx0;
      const dy = ev.clientY - d.my0;

      if (d.kind === "move") {
        void setClipTransform(clipId, {
          ...d.tf,
          position: [
            drive(d.tf.position[0], relUs, paramValue(d.tf.position[0], relUs) + dx / k),
            drive(d.tf.position[1], relUs, paramValue(d.tf.position[1], relUs) + dy / k),
          ],
        });
      } else if (d.kind === "scale") {
        const nsx = Math.min(10, Math.max(0.01,
          paramValue(d.tf.scale[0], relUs) + (dx * d.cornerX) / Math.max(1, bw) * 2,
        ));
        const nsy = Math.min(10, Math.max(0.01,
          paramValue(d.tf.scale[1], relUs) + (dy * d.cornerY) / Math.max(1, bh) * 2,
        ));
        void setClipTransform(clipId, {
          ...d.tf,
          scale: [drive(d.tf.scale[0], relUs, nsx), drive(d.tf.scale[1], relUs, nsy)],
        });
      } else {
        const ma = Math.atan2(ev.clientY - cy, ev.clientX - cx);
        let delta = ma - d.prevAngle;
        if (delta > Math.PI) delta -= 2 * Math.PI;
        if (delta < -Math.PI) delta += 2 * Math.PI;
        d.accDeg += (delta * 180) / Math.PI;
        d.prevAngle = ma;
        void setClipTransform(clipId, {
          ...d.tf,
          rotation: drive(d.tf.rotation, relUs, paramValue(d.tf.rotation, relUs) + d.accDeg),
        });
      }
    };

    const onUp = () => {
      dragRef.current = null;
      document.removeEventListener("pointermove", onMove);
      document.removeEventListener("pointerup", onUp);
    };
    document.addEventListener("pointermove", onMove);
    document.addEventListener("pointerup", onUp);
  };

  return (
    <div ref={rootRef} style={{ position: "absolute", inset: 0, pointerEvents: "none" }}>
      <svg width={w} height={h} style={{ position: "absolute", top: 0, left: 0 }}>
        <g transform={`rotate(${rot},${cx},${cy})`}>
          {/* box */}
          <rect x={bx} y={by} width={bw} height={bh} fill="none"
            stroke="var(--color-accent, #e5a832)" strokeWidth={1.5} />
          {/* drag-to-move area */}
          <rect x={bx} y={by} width={bw} height={bh} fill="transparent"
            style={{ pointerEvents: "auto", cursor: "move" }}
            onPointerDown={(e) => onDown(e, "move", cx, cy)} />
          {/* corner scale handles */}
          {corners.map((c, i) => (
            <rect key={`c${i}`} x={c.x - HANDLE / 2} y={c.y - HANDLE / 2}
              width={HANDLE} height={HANDLE} rx={1}
              fill="var(--color-accent, #e5a832)" stroke="var(--color-bg0, #0a0908)" strokeWidth={1}
              style={{ pointerEvents: "auto", cursor: CORNER_CURSORS[i] }}
              onPointerDown={(e) => onDown(e, "scale", c.x, c.y)} />
          ))}
          {/* edge position handles */}
          {edges.map((c, i) => (
            <circle key={`e${i}`} cx={c.x} cy={c.y} r={HANDLE / 2}
              fill="var(--color-accent, #e5a832)" stroke="var(--color-bg0, #0a0908)" strokeWidth={1}
              style={{ pointerEvents: "auto", cursor: "grab" }}
              onPointerDown={(e) => onDown(e, "move", cx, cy)} />
          ))}
          {/* rotation arm + handle */}
          <line x1={cx} y1={by} x2={cx} y2={rotY}
            stroke="var(--color-accent, #e5a832)" strokeWidth={1.5} />
          <circle cx={cx} cy={rotY} r={HANDLE / 2}
            fill="var(--color-accent, #e5a832)" stroke="var(--color-bg0, #0a0908)" strokeWidth={1}
            style={{ pointerEvents: "auto", cursor: "crosshair" }}
            onPointerDown={(e) => onDown(e, "rotate", cx, cy)} />
        </g>
      </svg>
    </div>
  );
}
