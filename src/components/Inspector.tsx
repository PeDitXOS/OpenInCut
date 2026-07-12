import { useEffect, useRef, useState } from "react";

import type { Clip, EffectDef, EffectInstance, Param } from "../engine/types";
import { hasKeyAt, removeKeyAt, withKeyAt } from "../engine/types";
import {
  activeSequence,
  assetName,
  instantiateEffect,
  isCurve,
  paramValue,
} from "../engine/types";
import { usToDuration, usToTimecode } from "../lib/time";
import { useStore } from "../state/store";

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="flex items-center justify-between gap-3 py-1">
      <span className="w-20 shrink-0 text-[11px] text-ink-dim">{label}</span>
      <div className="flex min-w-0 flex-1 items-center gap-2">{children}</div>
    </label>
  );
}

function Slider({
  value,
  min,
  max,
  step,
  unit,
  disabled,
  format,
  onChange,
}: {
  value: number;
  min: number;
  max: number;
  step: number;
  unit?: string;
  disabled?: boolean;
  format?: (v: number) => string;
  onChange: (v: number) => void;
}) {
  const decimals = step < 1 ? 2 : 0;
  const shown = format ? format(value) : `${value.toFixed(decimals)}${unit ?? ""}`;
  // A slider alone can't hit an exact number, so the readout is an input:
  // click it and type. ↑/↓ nudge by one step (×10 with Shift), Enter commits,
  // Esc reverts. Typing is NOT clamped to the slider's range — the engine's
  // own limits are the real ones, and a slider maximum shouldn't cap you.
  const [draft, setDraft] = useState<string | null>(null);
  const commit = (raw: string) => {
    const n = Number(raw.replace(/[^\d.eE+-]/g, ""));
    if (Number.isFinite(n)) onChange(n);
    setDraft(null);
  };
  return (
    <>
      <input
        type="range"
        className="h-1 min-w-0 flex-1 cursor-pointer appearance-none rounded-full bg-bg3 accent-(--color-accent) disabled:opacity-40"
        min={min}
        max={max}
        step={step}
        value={Math.min(max, Math.max(min, value))}
        disabled={disabled}
        onChange={(e) => onChange(Number(e.target.value))}
      />
      <input
        className="focus-ring w-14 shrink-0 rounded bg-transparent text-right font-[var(--font-mono)] text-[11px] text-ink hover:bg-bg3 focus:bg-bg3 disabled:opacity-40"
        value={draft ?? shown}
        disabled={disabled}
        title="Type an exact value · ↑/↓ to nudge (Shift ×10)"
        onFocus={(e) => {
          setDraft(String(Number(value.toFixed(decimals))));
          requestAnimationFrame(() => e.target.select());
        }}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={(e) => commit(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            commit((e.target as HTMLInputElement).value);
            (e.target as HTMLInputElement).blur();
          } else if (e.key === "Escape") {
            setDraft(null);
            (e.target as HTMLInputElement).blur();
          } else if (e.key === "ArrowUp" || e.key === "ArrowDown") {
            e.preventDefault();
            const d = (e.key === "ArrowUp" ? 1 : -1) * step * (e.shiftKey ? 10 : 1);
            const next = Number((value + d).toFixed(6));
            onChange(next);
            setDraft(String(Number(next.toFixed(decimals))));
          }
        }}
      />
    </>
  );
}

/** Keyframe button: ◆ if there's a key at the playhead, ◇ if the property animates. */
function KeyBtn({
  param,
  at,
  onChange,
}: {
  param: Param;
  at: number;
  onChange: (p: Param) => void;
}) {
  const keyed = hasKeyAt(param, at);
  const animated = isCurve(param);
  return (
    <button
      className={`focus-ring h-5 w-4 shrink-0 rounded text-[11px] leading-none hover:bg-bg3 ${
        keyed ? "text-accent" : animated ? "text-accent/50" : "text-ink-faint"
      }`}
      title={
        keyed
          ? "Remove the keyframe at the playhead"
          : "Add keyframe at the playhead (animates this property)"
      }
      onClick={() =>
        onChange(keyed ? removeKeyAt(param, at) : withKeyAt(param, at, paramValue(param, at)))
      }
    >
      {keyed ? "◆" : "◇"}
    </button>
  );
}

/** Interpolation of key i (for the selector). */
function keyInterp(k: { interp: { kind: string } }): string {
  return k.interp.kind;
}

/**
 * Mini curve editor: drag keys (time/value), double click adds or
 * removes, click selects (editable interpolation). Commit on release.
 */
function CurveEditor({
  param,
  durationUs,
  playheadUs,
  min,
  max,
  onChange,
}: {
  param: Param;
  durationUs: number;
  playheadUs: number;
  min: number;
  max: number;
  onChange: (p: Param) => void;
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [draft, setDraft] = useState<
    { t: number; value: number; interp: { kind: "hold" | "linear" | "smooth" } }[] | null
  >(null);
  const [selected, setSelected] = useState<number | null>(null);
  const dragIdx = useRef<number | null>(null);
  const H = 64;

  const keys =
    draft ?? (typeof param === "number" ? [] : (param.keys as NonNullable<typeof draft>));

  const toX = (t: number, w: number) => (t / Math.max(1, durationUs)) * w;
  const toY = (v: number) => 5 + (1 - (v - min) / (max - min)) * (H - 10);
  const fromXY = (x: number, y: number, w: number) => ({
    t: Math.round(Math.max(0, Math.min(1, x / w)) * durationUs),
    value: Math.max(min, Math.min(max, min + (1 - (y - 5) / (H - 10)) * (max - min))),
  });
  const hitKey = (x: number, y: number, w: number) =>
    keys.findIndex((k) => Math.abs(toX(k.t, w) - x) < 7 && Math.abs(toY(k.value) - y) < 7);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const w = canvas.clientWidth;
    const dpr = window.devicePixelRatio || 1;
    canvas.width = w * dpr;
    canvas.height = H * dpr;
    const ctx = canvas.getContext("2d")!;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, H);
    // zero line (if the range crosses it)
    if (min < 0 && max > 0) {
      ctx.strokeStyle = "rgba(233,228,219,0.12)";
      ctx.beginPath();
      ctx.moveTo(0, toY(0) + 0.5);
      ctx.lineTo(w, toY(0) + 0.5);
      ctx.stroke();
    }
    // curve (sampled with paramValue: same interpolation as the engine)
    const evalAt = (t: number) => paramValue(keys.length ? { keys } : 0, t);
    ctx.strokeStyle = "#ffb224";
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    for (let x = 0; x <= w; x += 2) {
      const v = evalAt((x / w) * durationUs);
      const y = toY(v);
      if (x === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }
    ctx.stroke();
    // playhead
    const px = toX(playheadUs, w);
    ctx.strokeStyle = "rgba(255,178,36,0.45)";
    ctx.beginPath();
    ctx.moveTo(px + 0.5, 0);
    ctx.lineTo(px + 0.5, H);
    ctx.stroke();
    // keys
    keys.forEach((k, i) => {
      const x = toX(k.t, w);
      const y = toY(k.value);
      ctx.fillStyle = i === selected ? "#ffb224" : "#e9e4db";
      ctx.beginPath();
      ctx.moveTo(x, y - 4.5);
      ctx.lineTo(x + 4.5, y);
      ctx.lineTo(x, y + 4.5);
      ctx.lineTo(x - 4.5, y);
      ctx.closePath();
      ctx.fill();
    });
  }, [keys, selected, playheadUs, durationUs, min, max]);

  const commit = (ks: NonNullable<typeof draft>) => {
    const sorted = [...ks].sort((a, b) => a.t - b.t);
    if (sorted.length === 0) onChange(paramValue(param, playheadUs));
    else if (sorted.length === 1) onChange(sorted[0].value);
    else onChange({ keys: sorted });
  };

  return (
    <div className="mt-1 rounded-md border border-line bg-bg2/50">
      <canvas
        ref={canvasRef}
        aria-label="Curve editor"
        className="block h-16 w-full cursor-crosshair touch-none"
        onPointerDown={(e) => {
          const rect = e.currentTarget.getBoundingClientRect();
          const [x, y] = [e.clientX - rect.left, e.clientY - rect.top];
          const i = hitKey(x, y, rect.width);
          setSelected(i >= 0 ? i : null);
          if (i >= 0) {
            dragIdx.current = i;
            setDraft(keys.map((k) => ({ ...k })));
            e.currentTarget.setPointerCapture(e.pointerId);
          }
        }}
        onPointerMove={(e) => {
          if (dragIdx.current === null || !draft) return;
          const rect = e.currentTarget.getBoundingClientRect();
          const { t, value } = fromXY(e.clientX - rect.left, e.clientY - rect.top, rect.width);
          const next = draft.map((k, i) => (i === dragIdx.current ? { ...k, t, value } : k));
          setDraft(next);
        }}
        onPointerUp={() => {
          if (dragIdx.current !== null && draft) commit(draft);
          dragIdx.current = null;
          setDraft(null);
        }}
        onDoubleClick={(e) => {
          const rect = e.currentTarget.getBoundingClientRect();
          const [x, y] = [e.clientX - rect.left, e.clientY - rect.top];
          const i = hitKey(x, y, rect.width);
          if (i >= 0) {
            commit(keys.filter((_, k) => k !== i));
            setSelected(null);
          } else {
            const { t, value } = fromXY(x, y, rect.width);
            commit([...keys, { t, value, interp: { kind: "linear" } }]);
          }
        }}
      />
      <div className="flex items-center gap-2 border-t border-line-soft px-2 py-1 text-[10.5px] text-ink-faint">
        {selected !== null && keys[selected] ? (
          <>
            <span>Key {selected + 1}</span>
            <select
              className="focus-ring cursor-pointer rounded border border-line bg-bg1 px-1 py-0.5 text-[10.5px] text-ink"
              value={keyInterp(keys[selected])}
              onChange={(e) => {
                const kind = e.target.value as "hold" | "linear" | "smooth";
                commit(keys.map((k, i) => (i === selected ? { ...k, interp: { kind } } : k)));
              }}
              title="Interpolation of the segment starting at this key"
            >
              <option value="linear">Linear</option>
              <option value="hold">Hold</option>
              <option value="smooth">Smooth</option>
            </select>
            <button
              className="focus-ring rounded px-1 text-ink-faint hover:text-danger"
              onClick={() => {
                commit(keys.filter((_, k) => k !== selected));
                setSelected(null);
              }}
              title="Delete this keyframe"
            >
              ✕ key
            </button>
          </>
        ) : (
          <span>Double click: add/remove · drag the diamonds</span>
        )}
      </div>
    </div>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="border-b border-line-soft px-3 py-3">
      <h3 className="panel-eyebrow mb-2">{title}</h3>
      {children}
    </section>
  );
}

function ClipInspector({ clip }: { clip: Clip }) {
  const [silenceDb, setSilenceDb] = useState(-38);
  const [speedDraft, setSpeedDraft] = useState<number | null>(null);
  const [silenceMs, setSilenceMs] = useState(400);
  const [padMs, setPadMs] = useState(150);
  const project = useStore((s) => s.project);
  const setClipAudio = useStore((s) => s.setClipAudio);
  const setClipTransform = useStore((s) => s.setClipTransform);
  const removeSilences = useStore((s) => s.removeSilences);
  const setClipSpeed = useStore((s) => s.setClipSpeed);
  const unlinkClip = useStore((s) => s.unlinkClip);
  const addSubtitlesClip = useStore((s) => s.addSubtitlesClip);
  const openAvatarDialog = useStore((s) => s.openAvatarDialog);
  const fps = activeSequence(project).fps;

  const asset =
    clip.payload.type === "media"
      ? project.assets.find((a) => a.id === (clip.payload as { asset_id: string }).asset_id)
      : undefined;

  const playheadUs = useStore((s) => s.playheadUs);
  const transcribeAsset = useStore((s) => s.transcribeAsset);
  const transcribingIds = useStore((s) => s.transcribingIds);
  const transcribing = asset ? transcribingIds.includes(asset.id) : false;
  const relUs = Math.max(0, Math.min(Math.round(playheadUs - clip.start), clip.duration));
  const opacity = paramValue(clip.transform.opacity, relUs);
  const scale = paramValue(clip.transform.scale[0], relUs);
  const rotation = paramValue(clip.transform.rotation, relUs);
  const gain = paramValue(clip.audio.gain_db, relUs);
  /** Writes a value: keyframe at the playhead if the property animates. */
  const drive = (p: Param, v: number): Param => (isCurve(p) ? withKeyAt(p, relUs, v) : v);

  return (
    <>
      <div className="border-b border-line-soft px-3 py-3">
        <div className="truncate text-[13px] font-medium text-ink">
          {asset ? assetName(asset) : clip.payload.type === "text" ? "Text" : "Clip"}
        </div>
        <div className="mt-1 font-[var(--font-mono)] text-[10px] text-ink-faint">
          {usToTimecode(clip.start, fps)} → {usToTimecode(clip.start + clip.duration, fps)} ·{" "}
          {usToDuration(clip.duration)}
        </div>
      </div>

      <Section title="Transform">
        <Row label="Position X">
          <Slider
            value={paramValue(clip.transform.position[0], relUs)}
            min={-1920}
            max={1920}
            step={2}
            unit=" px"
            onChange={(v) =>
              void setClipTransform(clip.id, {
                ...clip.transform,
                position: [drive(clip.transform.position[0], v), clip.transform.position[1]],
              })
            }
          />
          <KeyBtn
            param={clip.transform.position[0]}
            at={relUs}
            onChange={(p) =>
              void setClipTransform(clip.id, {
                ...clip.transform,
                position: [p, clip.transform.position[1]],
              })
            }
          />
        </Row>
        {isCurve(clip.transform.position[0]) && (
          <CurveEditor
            param={clip.transform.position[0]}
            durationUs={clip.duration}
            playheadUs={relUs}
            min={-1920}
            max={1920}
            onChange={(p) =>
              void setClipTransform(clip.id, {
                ...clip.transform,
                position: [p, clip.transform.position[1]],
              })
            }
          />
        )}
        <Row label="Position Y">
          <Slider
            value={paramValue(clip.transform.position[1], relUs)}
            min={-1080}
            max={1080}
            step={2}
            unit=" px"
            onChange={(v) =>
              void setClipTransform(clip.id, {
                ...clip.transform,
                position: [clip.transform.position[0], drive(clip.transform.position[1], v)],
              })
            }
          />
          <KeyBtn
            param={clip.transform.position[1]}
            at={relUs}
            onChange={(p) =>
              void setClipTransform(clip.id, {
                ...clip.transform,
                position: [clip.transform.position[0], p],
              })
            }
          />
        </Row>
        {isCurve(clip.transform.position[1]) && (
          <CurveEditor
            param={clip.transform.position[1]}
            durationUs={clip.duration}
            playheadUs={relUs}
            min={-1080}
            max={1080}
            onChange={(p) =>
              void setClipTransform(clip.id, {
                ...clip.transform,
                position: [clip.transform.position[0], p],
              })
            }
          />
        )}
        <Row label="Opacity">
          <Slider
            value={opacity}
            min={0}
            max={1}
            step={0.01}
            onChange={(v) =>
              void setClipTransform(clip.id, {
                ...clip.transform,
                opacity: drive(clip.transform.opacity, v),
              })
            }
          />
          <KeyBtn
            param={clip.transform.opacity}
            at={relUs}
            onChange={(p) => void setClipTransform(clip.id, { ...clip.transform, opacity: p })}
          />
        </Row>
        {isCurve(clip.transform.opacity) && (
          <CurveEditor
            param={clip.transform.opacity}
            durationUs={clip.duration}
            playheadUs={relUs}
            min={0}
            max={1}
            onChange={(p) => void setClipTransform(clip.id, { ...clip.transform, opacity: p })}
          />
        )}
        <Row label="Scale">
          <Slider
            value={scale}
            min={0.1}
            max={4}
            step={0.01}
            onChange={(v) => {
              const p = drive(clip.transform.scale[0], v);
              void setClipTransform(clip.id, { ...clip.transform, scale: [p, p] });
            }}
          />
          <KeyBtn
            param={clip.transform.scale[0]}
            at={relUs}
            onChange={(p) => void setClipTransform(clip.id, { ...clip.transform, scale: [p, p] })}
          />
        </Row>
        {isCurve(clip.transform.scale[0]) && (
          <CurveEditor
            param={clip.transform.scale[0]}
            durationUs={clip.duration}
            playheadUs={relUs}
            min={0.1}
            max={4}
            onChange={(p) => void setClipTransform(clip.id, { ...clip.transform, scale: [p, p] })}
          />
        )}
        <Row label="Rotation">
          <Slider
            value={rotation}
            min={-180}
            max={180}
            step={1}
            unit="°"
            onChange={(v) =>
              void setClipTransform(clip.id, {
                ...clip.transform,
                rotation: drive(clip.transform.rotation, v),
              })
            }
          />
          <KeyBtn
            param={clip.transform.rotation}
            at={relUs}
            onChange={(p) => void setClipTransform(clip.id, { ...clip.transform, rotation: p })}
          />
        </Row>
        {isCurve(clip.transform.rotation) && (
          <CurveEditor
            param={clip.transform.rotation}
            durationUs={clip.duration}
            playheadUs={relUs}
            min={-180}
            max={180}
            onChange={(p) => void setClipTransform(clip.id, { ...clip.transform, rotation: p })}
          />
        )}
        <Row label="Flip">
          <div className="flex min-w-0 flex-1 items-center gap-1.5">
            {(
              [
                ["flip_h", "Horizontal", "↔"],
                ["flip_v", "Vertical", "↕"],
              ] as const
            ).map(([key, title, glyph]) => (
              <button
                key={key}
                className={`focus-ring flex h-6 flex-1 items-center justify-center gap-1 rounded-md border text-[11px] ${
                  clip.transform[key]
                    ? "border-(--color-accent) bg-accent/15 text-accent"
                    : "border-line text-ink-dim hover:bg-bg3 hover:text-ink"
                }`}
                title={`Flip ${title.toLowerCase()}`}
                onClick={() =>
                  void setClipTransform(clip.id, {
                    ...clip.transform,
                    [key]: !clip.transform[key],
                  })
                }
              >
                {glyph} {title.slice(0, 1)}
              </button>
            ))}
          </div>
        </Row>
      </Section>

      <Section title="Audio">
        <Row label="Gain">
          <Slider
            value={gain}
            min={-60}
            max={12}
            step={0.5}
            unit=" dB"
            onChange={(v) =>
              void setClipAudio(clip.id, { ...clip.audio, gain_db: drive(clip.audio.gain_db, v) })
            }
          />
          <KeyBtn
            param={clip.audio.gain_db}
            at={relUs}
            onChange={(p) => void setClipAudio(clip.id, { ...clip.audio, gain_db: p })}
          />
        </Row>
        {isCurve(clip.audio.gain_db) && (
          <CurveEditor
            param={clip.audio.gain_db}
            durationUs={clip.duration}
            playheadUs={relUs}
            min={-60}
            max={12}
            onChange={(p) => void setClipAudio(clip.id, { ...clip.audio, gain_db: p })}
          />
        )}
        <Row label="Pan">
          <Slider
            value={paramValue(clip.audio.pan)}
            min={-1}
            max={1}
            step={0.05}
            unit=""
            format={(v) => (v === 0 ? "C" : v < 0 ? `L ${Math.round(-v * 100)}%` : `R ${Math.round(v * 100)}%`)}
            disabled={isCurve(clip.audio.pan)}
            onChange={(v) => void setClipAudio(clip.id, { ...clip.audio, pan: v })}
          />
        </Row>
        <Row label="Denoise">
          <label
            className="flex min-w-0 flex-1 cursor-pointer items-center gap-2 text-[11px] text-ink-dim"
            title="DNS64 neural denoiser (same engine as the toolkit); renders in the background — playback and export switch to the clean audio when ready"
          >
            <input
              type="checkbox"
              className="accent-(--color-accent)"
              checked={clip.audio.denoise}
              onChange={(e) =>
                void setClipAudio(clip.id, { ...clip.audio, denoise: e.target.checked })
              }
            />
            Reduce background noise
          </label>
        </Row>
        <Row label="Fade in">
          <Slider
            value={clip.audio.fade_in_us / 1e6}
            min={0}
            max={Math.min(5, clip.duration / 1e6)}
            step={0.1}
            unit=" s"
            onChange={(v) =>
              void setClipAudio(clip.id, { ...clip.audio, fade_in_us: Math.round(v * 1e6) })
            }
          />
        </Row>
        <Row label="Fade out">
          <Slider
            value={clip.audio.fade_out_us / 1e6}
            min={0}
            max={Math.min(5, clip.duration / 1e6)}
            step={0.1}
            unit=" s"
            onChange={(v) =>
              void setClipAudio(clip.id, { ...clip.audio, fade_out_us: Math.round(v * 1e6) })
            }
          />
        </Row>
      </Section>

      {clip.group && (
        <Section title="Link">
          <div className="flex items-center justify-between gap-2">
            <span className="text-[11px] text-ink-dim">
              🔗 Video and audio linked: move, split, trim, speed and
              delete affect both.
            </span>
            <button
              className="focus-ring shrink-0 rounded-md border border-line px-2 py-1.5 text-[12px] text-ink-dim hover:text-ink"
              onClick={() => void unlinkClip(clip.id)}
              title="Break the link to edit video and audio separately"
            >
              Unlink
            </button>
          </div>
        </Section>
      )}

      {clip.payload.type === "media" && (
        <Section title="Speed">
          <div className="flex items-center gap-2">
            <input
              type="range"
              className="h-1 min-w-0 flex-1 cursor-pointer appearance-none rounded-full bg-bg3 accent-(--color-accent)"
              min={0.25}
              max={4}
              step={0.05}
              value={speedDraft ?? clip.speed}
              onChange={(e) => setSpeedDraft(Number(e.target.value))}
              onPointerUp={() => {
                if (speedDraft != null && Math.abs(speedDraft - clip.speed) > 1e-9)
                  void setClipSpeed(clip.id, speedDraft);
                setSpeedDraft(null);
              }}
              title="Playback speed (export keeps the voice pitch, like YouTube)"
            />
            <span className="w-12 shrink-0 text-right font-[var(--font-mono)] text-[11px] text-ink">
              {(speedDraft ?? clip.speed).toFixed(2)}×
            </span>
            <button
              className="focus-ring rounded-md border border-line px-1.5 py-0.5 text-[10.5px] text-ink-dim hover:text-ink"
              onClick={() => void setClipSpeed(clip.id, 1)}
              title="Back to normal speed"
            >
              1×
            </button>
          </div>
          <p className="mt-1.5 text-[10px] leading-snug text-ink-faint">
            The clip {clip.speed > 1 ? "gets shorter" : clip.speed < 1 ? "gets longer" : "doesn't change"};
            the voice pitch is preserved live (WSOLA) and on export (atempo).
          </p>
        </Section>
      )}

      {clip.payload.type === "media" && asset && (
        <Section title="Source">
          <div className="space-y-1 text-[11px]">
            <div className="truncate text-ink" title={asset.path}>
              {asset.path}
            </div>
            <div className="font-[var(--font-mono)] text-[10px] text-ink-faint">
              {usToDuration(clip.payload.src_in)} → {usToDuration(clip.payload.src_out)} of the
              file · speed {clip.speed}×
            </div>
          </div>
        </Section>
      )}

      {clip.payload.type === "text" && <TextPanel clip={clip} />}
      {clip.payload.type === "generator" && <GeneratorPanel clip={clip} />}
      {clip.payload.type === "subtitles" && <SubtitlesPanel clip={clip} />}

      {clip.payload.type === "media" && asset && asset.probe.audio_channels > 0 && (
        <Section title="Silences">
          <Row label="Threshold">
            <Slider
              value={silenceDb}
              min={-70}
              max={-15}
              step={1}
              unit=" dB"
              onChange={setSilenceDb}
            />
          </Row>
          <Row label="Min. silence">
            <Slider
              value={silenceMs}
              min={100}
              max={1500}
              step={50}
              unit=" ms"
              onChange={setSilenceMs}
            />
          </Row>
          <Row label="Padding">
            <Slider value={padMs} min={0} max={500} step={10} unit=" ms" onChange={setPadMs} />
          </Row>
          <div className="mt-1 flex gap-1.5">
            <button
              className="focus-ring flex-1 rounded-md border border-line bg-bg2 px-2 py-2 text-[12px] text-ink hover:bg-bg3"
              onClick={() =>
                void removeSilences(clip.id, "delete", {
                  thresholdDb: silenceDb,
                  minSilenceMs: silenceMs,
                  padMs,
                })
              }
              title="Detects silences and cuts them, closing the gaps (1 undo)"
            >
              🔇 Delete
            </button>
            <button
              className="focus-ring flex-1 rounded-md border border-line bg-bg2 px-2 py-2 text-[12px] text-ink hover:bg-bg3"
              onClick={() =>
                void removeSilences(clip.id, "speedup", {
                  thresholdDb: silenceDb,
                  minSilenceMs: silenceMs,
                  padMs,
                })
              }
              title="Detects silences and speeds them up 4× instead of cutting them (1 undo)"
            >
              ⏩ Speed up 4×
            </button>
            <button
              className="focus-ring flex-1 rounded-md border border-line bg-bg2 px-2 py-2 text-[12px] text-ink hover:bg-bg3"
              onClick={() =>
                void removeSilences(clip.id, "split", {
                  thresholdDb: silenceDb,
                  minSilenceMs: silenceMs,
                  padMs,
                })
              }
              title="Only cuts at silence boundaries (silence/speech segments); you decide what to delete (1 undo)"
            >
              ✂ Split
            </button>
          </div>
        </Section>
      )}

      {/* AI tools: driven by the clip's AUDIO, so any clip with sound gets them */}
      {clip.payload.type === "media" && asset && asset.probe.audio_channels > 0 && (
        <Section title="AI">
          {!asset.transcript && (
            <button
              className="focus-ring w-full rounded-md border border-accent/60 bg-bg2 px-2.5 py-2 text-[12px] font-medium text-accent hover:bg-bg3 disabled:opacity-50"
              disabled={transcribing}
              onClick={() => void transcribeAsset(asset.id)}
              title="Word-by-word transcription with Whisper (downloads the model the first time). Enables text-based editing, subtitles and the avatar."
            >
              {transcribing ? "⏳ Transcribing…" : "🎙 Transcribe (Whisper)"}
            </button>
          )}
          {asset.transcript && (
            <button
              className="focus-ring w-full rounded-md border border-line bg-bg2 px-2.5 py-2 text-[12px] text-ink hover:bg-bg3"
              onClick={() => void addSubtitlesClip(clip.id)}
              title="Creates an auto-subtitles clip (by phrases) over this clip"
            >
              💬 Auto subtitles
            </button>
          )}
          {/* the avatar is driven by the VOICE: always reachable, it asks for
              the transcript from inside the dialog if it is missing */}
          <button
            className="focus-ring mt-1.5 w-full rounded-md border border-line bg-bg2 px-2.5 py-2 text-[12px] text-ink hover:bg-bg3"
            onClick={() => openAvatarDialog(asset.id)}
            title="Emotion-reactive avatar: expressions, look and classifier. Driven by this clip's voice."
          >
            🧑‍🎤 Reactive avatar…
          </button>
        </Section>
      )}

      {clip.payload.type === "media" && <TransitionPanel clip={clip} />}
      <EffectsPanel clip={clip} />
    </>
  );
}

function TextPanel({ clip }: { clip: Clip }) {
  const setClipText = useStore((s) => s.setClipText);
  const fonts = useStore((s) => s.fonts);
  const textTemplates = useStore((s) => s.textTemplates);
  const saveTextTemplate = useStore((s) => s.saveTextTemplate);
  if (clip.payload.type !== "text") return null;
  const { content, style } = clip.payload;

  return (
    <Section title="Text">
      <textarea
        className="focus-ring w-full resize-y rounded-md border border-line bg-bg2 px-2.5 py-2 text-[12px] text-ink"
        rows={2}
        value={content}
        onChange={(e) => void setClipText(clip.id, e.target.value, style)}
        placeholder="Type the title…"
      />
      <Row label="Font">
        <select
          className="focus-ring min-w-0 flex-1 cursor-pointer rounded-md border border-line bg-bg2 px-2 py-1 text-[12px] text-ink"
          value={style.font}
          onChange={(e) => void setClipText(clip.id, content, { ...style, font: e.target.value })}
          style={{ fontFamily: style.font }}
        >
          <option value="sans-serif">Default</option>
          {fonts.map(([family]) => (
            <option key={family} value={family} style={{ fontFamily: family }}>
              {family}
            </option>
          ))}
        </select>
      </Row>
      <Row label="Alignment">
        <select
          className="focus-ring min-w-0 flex-1 cursor-pointer rounded-md border border-line bg-bg2 px-2 py-1 text-[12px] text-ink"
          value={style.align}
          onChange={(e) =>
            void setClipText(clip.id, content, {
              ...style,
              align: e.target.value as "left" | "center" | "right",
            })
          }
        >
          <option value="left">Left</option>
          <option value="center">Center</option>
          <option value="right">Right</option>
        </select>
      </Row>
      <Row label="Position X">
        <Slider
          value={style.x_offset}
          min={-800}
          max={800}
          step={5}
          unit=" px"
          onChange={(v) => void setClipText(clip.id, content, { ...style, x_offset: v })}
        />
      </Row>
      <Row label="Size">
        <Slider
          value={style.size}
          min={16}
          max={200}
          step={1}
          unit=" px"
          onChange={(v) => void setClipText(clip.id, content, { ...style, size: v })}
        />
      </Row>
      <Row label="Color">
        <input
          type="color"
          className="h-6 w-10 cursor-pointer rounded border border-line bg-transparent"
          value={style.color}
          onChange={(e) => void setClipText(clip.id, content, { ...style, color: e.target.value })}
        />
        <span className="font-[var(--font-mono)] text-[10px] text-ink-faint">{style.color}</span>
      </Row>
      <Row label="Height">
        <Slider
          value={style.y_offset}
          min={-500}
          max={500}
          step={5}
          unit=" px"
          onChange={(v) =>
            void setClipText(clip.id, content, { ...style, y_offset: v })
          }
        />
      </Row>
      <p className="mt-1 text-[10px] leading-snug text-ink-faint">
        Size and positions are relative to 1080p; they scale on export.
      </p>
      <div className="mt-2 flex gap-1.5">
        <select
          className="focus-ring min-w-0 flex-1 cursor-pointer rounded-md border border-line bg-bg2 px-2 py-1 text-[11px] text-ink-dim"
          value=""
          onChange={(e) => {
            const tpl = textTemplates[e.target.value];
            if (tpl) void setClipText(clip.id, content, tpl);
          }}
          title="Apply a saved template"
        >
          <option value="">Templates…</option>
          {Object.keys(textTemplates).map((name) => (
            <option key={name} value={name}>
              {name}
            </option>
          ))}
        </select>
        <button
          className="focus-ring shrink-0 rounded-md border border-line px-2 py-1 text-[11px] text-ink-dim hover:text-ink"
          onClick={() => {
            const name = window.prompt("Template name:");
            if (name?.trim()) void saveTextTemplate(name.trim(), style);
          }}
          title="Save the current style as a template"
        >
          Save
        </button>
      </div>
      <Row label="Line height">
        <Slider
          value={style.line_height ?? 1.2}
          min={0.8}
          max={2}
          step={0.05}
          unit="×"
          onChange={(v) =>
            void setClipText(clip.id, content, { ...style, line_height: v })
          }
        />
      </Row>
    </Section>
  );
}

function SubtitlesPanel({ clip }: { clip: Clip }) {
  const setSubtitlesProps = useStore((s) => s.setSubtitlesProps);
  const subFonts = useStore((s) => s.fonts);
  if (clip.payload.type !== "subtitles") return null;
  const { style, mode } = clip.payload;
  const maxWords = clip.payload.max_words ?? null;

  return (
    <Section title="Subtitles">
      <Row label="Mode">
        <select
          className="focus-ring min-w-0 flex-1 cursor-pointer rounded-md border border-line bg-bg2 px-2 py-1 text-[12px] text-ink"
          value={mode}
          onChange={(e) =>
            void setSubtitlesProps(
              clip.id,
              style,
              e.target.value as "phrase" | "word" | "karaoke",
              maxWords,
            )
          }
          title="Full phrase, word by word (shorts style) or karaoke"
        >
          <option value="phrase">By phrases</option>
          <option value="word">Word by word</option>
          <option value="karaoke">Karaoke (highlighted word)</option>
        </select>
      </Row>
      <Row label="Font">
        <select
          className="focus-ring min-w-0 flex-1 cursor-pointer rounded-md border border-line bg-bg2 px-2 py-1 text-[12px] text-ink"
          value={style.font}
          onChange={(e) =>
            void setSubtitlesProps(clip.id, { ...style, font: e.target.value }, mode, maxWords)
          }
          style={{ fontFamily: style.font }}
        >
          <option value="sans-serif">Default</option>
          {subFonts.map(([family]) => (
            <option key={family} value={family} style={{ fontFamily: family }}>
              {family}
            </option>
          ))}
        </select>
      </Row>
      {mode === "karaoke" && (
        <Row label="Highlight">
          <input
            type="color"
            className="h-6 w-10 cursor-pointer rounded border border-line bg-transparent"
            value={style.highlight_color ?? "#FFB224"}
            onChange={(e) =>
              void setSubtitlesProps(clip.id, { ...style, highlight_color: e.target.value }, mode, maxWords)
            }
          />
        </Row>
      )}
      <Row label="Words">
        <div className="flex min-w-0 flex-1 items-center gap-2">
          <button
            className={`focus-ring h-6 shrink-0 rounded-md border px-2 text-[11px] ${
              maxWords === null
                ? "border-(--color-accent) bg-accent/15 text-accent"
                : "border-line text-ink-dim hover:bg-bg3 hover:text-ink"
            }`}
            title="Fit as many words as the frame width allows"
            onClick={() => void setSubtitlesProps(clip.id, style, mode, null)}
          >
            Auto
          </button>
          <Slider
            value={maxWords ?? 0}
            min={1}
            max={12}
            step={1}
            unit=""
            format={(v) => (maxWords === null ? "auto" : `${v}`)}
            onChange={(v) =>
              void setSubtitlesProps(clip.id, style, mode, Math.max(1, Math.round(v)))
            }
          />
        </div>
      </Row>
      <Row label="Size">
        <Slider
          value={style.size}
          min={16}
          max={160}
          step={1}
          unit=" px"
          onChange={(v) => void setSubtitlesProps(clip.id, { ...style, size: v }, mode, maxWords)}
        />
      </Row>
      <Row label="Color">
        <input
          type="color"
          className="h-6 w-10 cursor-pointer rounded border border-line bg-transparent"
          value={style.color}
          onChange={(e) => void setSubtitlesProps(clip.id, { ...style, color: e.target.value }, mode, maxWords)}
        />
        <span className="font-[var(--font-mono)] text-[10px] text-ink-faint">{style.color}</span>
      </Row>
      <Row label="Height">
        <Slider
          value={style.y_offset}
          min={-500}
          max={500}
          step={5}
          unit=" px"
          onChange={(v) => void setSubtitlesProps(clip.id, { ...style, y_offset: v }, mode, maxWords)}
        />
      </Row>
      <Row label="Line height">
        <Slider
          value={style.line_height ?? 1.2}
          min={0.8}
          max={2}
          step={0.05}
          unit="×"
          onChange={(v) =>
            void setSubtitlesProps(clip.id, { ...style, line_height: v }, mode, maxWords)
          }
        />
      </Row>
    </Section>
  );
}

const TRANSITION_KINDS: [string, string][] = [
  ["core.crossfade", "Cross fade"],
  ["core.wipeleft", "Wipe ←"],
  ["core.wiperight", "Wipe →"],
  ["core.slideleft", "Slide ←"],
  ["core.slideright", "Slide →"],
  ["core.slideup", "Slide ↑"],
  ["core.circleopen", "Circle open"],
  ["core.circleclose", "Circle close"],
  ["core.dissolve", "Dissolve"],
  ["core.pixelize", "Pixelize"],
  ["core.radial", "Radial"],
];

function TransitionPanel({ clip }: { clip: Clip }) {
  const setClipTransition = useStore((s) => s.setClipTransition);
  const durS = (clip.transition_in?.duration ?? 500_000) / 1e6;

  return (
    <Section title="Transition in">
      <Row label="Type">
        <select
          className="focus-ring min-w-0 flex-1 cursor-pointer rounded-md border border-line bg-bg2 px-2 py-1 text-[12px] text-ink"
          value={clip.transition_in?.effect_id ?? ""}
          onChange={(e) =>
            void setClipTransition(
              clip.id,
              e.target.value
                ? {
                    effect_id: e.target.value,
                    duration: clip.transition_in?.duration ?? 500_000,
                    params: {},
                  }
                : null,
            )
          }
        >
          <option value="">Cut (none)</option>
          {TRANSITION_KINDS.map(([id, label]) => (
            <option key={id} value={id}>
              {label}
            </option>
          ))}
        </select>
      </Row>
      {clip.transition_in && (
        <Row label="Duration">
          <Slider
            value={durS}
            min={0.1}
            max={2}
            step={0.05}
            unit=" s"
            onChange={(v) =>
              void setClipTransition(clip.id, {
                ...clip.transition_in!,
                duration: Math.round(v * 1e6),
              })
            }
          />
        </Row>
      )}
      <p className="mt-1 text-[10px] leading-snug text-ink-faint">
        Needs extra material on both sides of the cut; if there isn't any, it
        shrinks. Also works between clips with different speeds.
      </p>
    </Section>
  );
}

function EffectRow({
  inst,
  def,
  onChange,
  onRemove,
  durationUs,
  relUs,
}: {
  inst: EffectInstance;
  def: EffectDef | undefined;
  onChange: (next: EffectInstance) => void;
  onRemove: () => void;
  durationUs: number;
  relUs: number;
}) {
  return (
    <div className="rounded-lg border border-line bg-bg2 p-2">
      <div className="flex items-center gap-2">
        <input
          type="checkbox"
          className="accent-(--color-accent)"
          checked={inst.enabled}
          onChange={(e) => onChange({ ...inst, enabled: e.target.checked })}
          title="Enable/disable effect"
        />
        <span className="flex-1 truncate text-[12px] font-medium text-ink">
          {def?.name ?? inst.effect_id}
        </span>
        <button
          className="focus-ring rounded px-1.5 text-[12px] text-ink-faint hover:text-danger"
          onClick={onRemove}
          title="Remove effect"
        >
          ✕
        </button>
      </div>
      {def && (
        <div className="mt-1.5 space-y-0.5">
          {def.params.map((p) =>
            p.type === "float" ? (
              <div key={p.key}>
                <Row label={p.label ?? p.key}>
                  <Slider
                    value={
                      inst.params[p.key] !== undefined
                        ? paramValue(inst.params[p.key], relUs)
                        : (p.default as number)
                    }
                    min={p.min ?? 0}
                    max={p.max ?? 1}
                    step={((p.max ?? 1) - (p.min ?? 0)) / 100}
                    onChange={(v) => {
                      const cur = inst.params[p.key] ?? (p.default as number);
                      const next = isCurve(cur) ? withKeyAt(cur, relUs, v) : v;
                      onChange({ ...inst, params: { ...inst.params, [p.key]: next } });
                    }}
                  />
                  <KeyBtn
                    param={inst.params[p.key] ?? (p.default as number)}
                    at={relUs}
                    onChange={(np) =>
                      onChange({ ...inst, params: { ...inst.params, [p.key]: np } })
                    }
                  />
                </Row>
                {inst.params[p.key] !== undefined && isCurve(inst.params[p.key]) && (
                  <CurveEditor
                    param={inst.params[p.key]}
                    durationUs={durationUs}
                    playheadUs={relUs}
                    min={p.min ?? 0}
                    max={p.max ?? 1}
                    onChange={(np) =>
                      onChange({ ...inst, params: { ...inst.params, [p.key]: np } })
                    }
                  />
                )}
              </div>
            ) : (
              <Row key={p.key} label={p.label ?? p.key}>
                <input
                  type="color"
                  className="h-6 w-10 cursor-pointer rounded border border-line bg-transparent"
                  value={inst.color_params[p.key] ?? (p.default as string)}
                  onChange={(e) =>
                    onChange({
                      ...inst,
                      color_params: { ...inst.color_params, [p.key]: e.target.value },
                    })
                  }
                />
                <span className="font-[var(--font-mono)] text-[10px] text-ink-faint">
                  {inst.color_params[p.key] ?? p.default}
                </span>
              </Row>
            ),
          )}
        </div>
      )}
    </div>
  );
}

/** Panel for a generator clip: type + params auto-generated from the manifest. */
function GeneratorPanel({ clip }: { clip: Clip }) {
  const catalog = useStore((s) => s.generatorsCatalog);
  const setClipGenerator = useStore((s) => s.setClipGenerator);
  if (clip.payload.type !== "generator") return null;
  const { generator_id, params, color_params } = clip.payload;
  const def = catalog.find((d) => d.id === generator_id);

  return (
    <Section title="Generator">
      <Row label="Type">
        <select
          className="focus-ring min-w-0 flex-1 cursor-pointer rounded-md border border-line bg-bg2 px-2 py-1 text-[12px] text-ink"
          value={generator_id}
          onChange={(e) => void setClipGenerator(clip.id, e.target.value, {}, {})}
          title="Generator type (changing it resets the parameters)"
        >
          {catalog.map((d) => (
            <option key={d.id} value={d.id}>
              {d.name}
            </option>
          ))}
        </select>
      </Row>
      {def?.params.map((p) =>
        p.type === "float" ? (
          <Row key={p.key} label={p.label ?? p.key}>
            <Slider
              value={
                params[p.key] !== undefined ? paramValue(params[p.key]) : (p.default as number)
              }
              min={p.min ?? 0}
              max={p.max ?? 1}
              step={1}
              unit=" px"
              onChange={(v) =>
                void setClipGenerator(
                  clip.id,
                  generator_id,
                  { ...params, [p.key]: v },
                  color_params,
                )
              }
            />
          </Row>
        ) : (
          <Row key={p.key} label={p.label ?? p.key}>
            <input
              type="color"
              className="h-6 w-10 cursor-pointer rounded border border-line bg-transparent"
              value={color_params[p.key] ?? (p.default as string)}
              onChange={(e) =>
                void setClipGenerator(clip.id, generator_id, params, {
                  ...color_params,
                  [p.key]: e.target.value,
                })
              }
            />
            <span className="font-[var(--font-mono)] text-[10.5px] text-ink-faint">
              {(color_params[p.key] ?? (p.default as string)).toUpperCase()}
            </span>
          </Row>
        ),
      )}
      {def?.notes && <p className="mt-1.5 text-[10.5px] text-ink-faint">{def.notes}</p>}
    </Section>
  );
}

function EffectsPanel({ clip }: { clip: Clip }) {
  const catalog = useStore((s) => s.effectsCatalog);
  const fxPlayhead = useStore((s) => s.playheadUs);
  const fxRelUs = Math.max(0, Math.min(Math.round(fxPlayhead - clip.start), clip.duration));
  const setClipEffects = useStore((s) => s.setClipEffects);
  const reloadEffectPacks = useStore((s) => s.reloadEffectPacks);

  const update = (i: number, next: EffectInstance) => {
    const effects = clip.effects.map((e, k) => (k === i ? next : e));
    void setClipEffects(clip.id, effects);
  };
  const remove = (i: number) => {
    void setClipEffects(clip.id, clip.effects.filter((_, k) => k !== i));
  };
  const add = (id: string) => {
    const def = catalog.find((d) => d.id === id);
    if (!def) return;
    void setClipEffects(clip.id, [...clip.effects, instantiateEffect(def)]);
  };

  return (
    <section className="border-b border-line-soft px-3 py-3">
      <div className="mb-2 flex items-center justify-between">
        <h3 className="panel-eyebrow">Effects</h3>
        <button
          className="focus-ring rounded px-1.5 text-[11px] text-ink-faint hover:text-ink"
          onClick={() => void reloadEffectPacks()}
          title="Reload effect packs from disk"
        >
          ↻ packs
        </button>
      </div>
      <div className="space-y-1.5">
        {clip.effects.map((e, i) => (
          <EffectRow
            key={`${e.effect_id}-${i}`}
            inst={e}
            def={catalog.find((d) => d.id === e.effect_id)}
            onChange={(next) => update(i, next)}
            onRemove={() => remove(i)}
            durationUs={clip.duration}
            relUs={fxRelUs}
          />
        ))}
        <select
          className="focus-ring w-full cursor-pointer rounded-md border border-line bg-bg2 px-2 py-1.5 text-[12px] text-ink-dim"
          value=""
          onChange={(e) => e.target.value && add(e.target.value)}
          title="Add an effect to the clip"
        >
          <option value="">+ Add effect…</option>
          {catalog.map((d) => (
            <option key={d.id} value={d.id}>
              {d.name}
            </option>
          ))}
        </select>
      </div>
    </section>
  );
}

export function Inspector() {
  const selection = useStore((s) => s.selection);
  const project = useStore((s) => s.project);
  const setAiSettings = useStore((s) => s.setAiSettings);
  const setSequenceProps = useStore((s) => s.setSequenceProps);
  useStore((s) => s.version);

  const seq = activeSequence(project);
  const clip = seq.tracks.flatMap((t) => t.clips).find((c) => selection.includes(c.id));

  return (
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="px-3 pb-1 pt-3">
        <h2 className="panel-eyebrow">Inspector</h2>
      </div>
      {clip ? (
        <ClipInspector clip={clip} />
      ) : (
        <div className="px-3 py-4">
          <div className="rounded-lg border border-line bg-bg2 p-3">
            <div className="text-[12px] font-medium text-ink">{seq.name}</div>
            <div className="mt-1.5 font-[var(--font-mono)] text-[10px] text-ink-faint">
              {seq.tracks.length} tracks · {(seq.sample_rate / 1000).toFixed(0)} kHz
            </div>
            <Row label="Resolution">
              <select
                className="focus-ring min-w-0 flex-1 cursor-pointer rounded-md border border-line bg-bg1 px-2 py-1 text-[12px] text-ink"
                value={`${seq.resolution[0]}x${seq.resolution[1]}`}
                onChange={(e) => {
                  const [w, h] = e.target.value.split("x").map(Number);
                  void setSequenceProps(seq.id, w, h, seq.fps[0], seq.fps[1]);
                }}
                title="Sequence canvas: clips are fitted into it; exports use this size by default"
              >
                {[
                  [1920, 1080, "1080p (FHD)"],
                  [2560, 1440, "1440p (QHD)"],
                  [3840, 2160, "2160p (4K)"],
                  [1280, 720, "720p"],
                  [1080, 1920, "1080×1920 (vertical)"],
                  [2160, 3840, "2160×3840 (vertical 4K)"],
                ].map(([w, h, label]) => (
                  <option key={`${w}x${h}`} value={`${w}x${h}`}>
                    {label}
                  </option>
                ))}
                {!["1920x1080", "2560x1440", "3840x2160", "1280x720", "1080x1920", "2160x3840"].includes(
                  `${seq.resolution[0]}x${seq.resolution[1]}`,
                ) && (
                  <option value={`${seq.resolution[0]}x${seq.resolution[1]}`}>
                    {seq.resolution[0]}×{seq.resolution[1]}
                  </option>
                )}
              </select>
            </Row>
            <Row label="Frame rate">
              <select
                className="focus-ring min-w-0 flex-1 cursor-pointer rounded-md border border-line bg-bg1 px-2 py-1 text-[12px] text-ink"
                value={`${seq.fps[0]}/${seq.fps[1]}`}
                onChange={(e) => {
                  const [n, d] = e.target.value.split("/").map(Number);
                  void setSequenceProps(seq.id, seq.resolution[0], seq.resolution[1], n, d);
                }}
              >
                {[
                  [24, 1, "24 fps"],
                  [25, 1, "25 fps"],
                  [30, 1, "30 fps"],
                  [30000, 1001, "29.97 fps"],
                  [50, 1, "50 fps"],
                  [60, 1, "60 fps"],
                ].map(([n, d, label]) => (
                  <option key={`${n}/${d}`} value={`${n}/${d}`}>
                    {label}
                  </option>
                ))}
              </select>
            </Row>
          </div>
          <div className="mt-3 rounded-lg border border-line bg-bg2 p-3">
            <h3 className="panel-eyebrow mb-2">AI · Whisper</h3>
            <Row label="Model">
              <select
                className="focus-ring min-w-0 flex-1 cursor-pointer rounded-md border border-line bg-bg1 px-2 py-1 text-[12px] text-ink"
                value={project.settings.whisper_model}
                onChange={(e) =>
                  void setAiSettings(project.settings.whisper_language, e.target.value)
                }
              >
                {["tiny", "base", "small", "medium", "large-v3-turbo"].map((m) => (
                  <option key={m} value={m}>
                    {m}
                  </option>
                ))}
              </select>
            </Row>
            <Row label="Language">
              <select
                className="focus-ring min-w-0 flex-1 cursor-pointer rounded-md border border-line bg-bg1 px-2 py-1 text-[12px] text-ink"
                value={project.settings.whisper_language}
                onChange={(e) =>
                  void setAiSettings(e.target.value, project.settings.whisper_model)
                }
              >
                {[
                  ["auto", "Detect"],
                  ["es", "Spanish"],
                  ["en", "English"],
                  ["pt", "Portuguese"],
                  ["fr", "French"],
                  ["de", "German"],
                ].map(([v, l]) => (
                  <option key={v} value={v}>
                    {l}
                  </option>
                ))}
              </select>
            </Row>
          </div>
          <p className="mt-3 text-[11px] leading-relaxed text-ink-faint">
            Select a clip in the timeline to edit its properties.
          </p>
        </div>
      )}
    </div>
  );
}
