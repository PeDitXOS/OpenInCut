import type { Clip, EffectDef, EffectInstance } from "../engine/types";
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
  onChange,
}: {
  value: number;
  min: number;
  max: number;
  step: number;
  unit?: string;
  disabled?: boolean;
  onChange: (v: number) => void;
}) {
  return (
    <>
      <input
        type="range"
        className="h-1 min-w-0 flex-1 cursor-pointer appearance-none rounded-full bg-bg3 accent-(--color-accent) disabled:opacity-40"
        min={min}
        max={max}
        step={step}
        value={value}
        disabled={disabled}
        onChange={(e) => onChange(Number(e.target.value))}
      />
      <span className="w-14 shrink-0 text-right font-[var(--font-mono)] text-[11px] text-ink">
        {value.toFixed(step < 1 ? 2 : 0)}
        {unit}
      </span>
    </>
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
  const project = useStore((s) => s.project);
  const setClipAudio = useStore((s) => s.setClipAudio);
  const setClipTransform = useStore((s) => s.setClipTransform);
  const fps = activeSequence(project).fps;

  const asset =
    clip.payload.type === "media"
      ? project.assets.find((a) => a.id === (clip.payload as { asset_id: string }).asset_id)
      : undefined;

  const opacity = paramValue(clip.transform.opacity);
  const scale = paramValue(clip.transform.scale[0]);
  const rotation = paramValue(clip.transform.rotation);
  const gain = paramValue(clip.audio.gain_db);

  return (
    <>
      <div className="border-b border-line-soft px-3 py-3">
        <div className="truncate text-[13px] font-medium text-ink">
          {asset ? assetName(asset) : clip.payload.type === "text" ? "Texto" : "Clip"}
        </div>
        <div className="mt-1 font-[var(--font-mono)] text-[10px] text-ink-faint">
          {usToTimecode(clip.start, fps)} → {usToTimecode(clip.start + clip.duration, fps)} ·{" "}
          {usToDuration(clip.duration)}
        </div>
      </div>

      <Section title="Transformación">
        <Row label="Opacidad">
          <Slider
            value={opacity}
            min={0}
            max={1}
            step={0.01}
            disabled={isCurve(clip.transform.opacity)}
            onChange={(v) => void setClipTransform(clip.id, { ...clip.transform, opacity: v })}
          />
        </Row>
        <Row label="Escala">
          <Slider
            value={scale}
            min={0.1}
            max={4}
            step={0.01}
            disabled={isCurve(clip.transform.scale[0])}
            onChange={(v) => void setClipTransform(clip.id, { ...clip.transform, scale: [v, v] })}
          />
        </Row>
        <Row label="Rotación">
          <Slider
            value={rotation}
            min={-180}
            max={180}
            step={1}
            unit="°"
            disabled={isCurve(clip.transform.rotation)}
            onChange={(v) => void setClipTransform(clip.id, { ...clip.transform, rotation: v })}
          />
        </Row>
      </Section>

      <Section title="Audio">
        <Row label="Ganancia">
          <Slider
            value={gain}
            min={-60}
            max={12}
            step={0.5}
            unit=" dB"
            disabled={isCurve(clip.audio.gain_db)}
            onChange={(v) => void setClipAudio(clip.id, { ...clip.audio, gain_db: v })}
          />
        </Row>
        <Row label="Fade in">
          <span className="font-[var(--font-mono)] text-[11px] text-ink">
            {(clip.audio.fade_in_us / 1e6).toFixed(1)} s
          </span>
        </Row>
        <Row label="Fade out">
          <span className="font-[var(--font-mono)] text-[11px] text-ink">
            {(clip.audio.fade_out_us / 1e6).toFixed(1)} s
          </span>
        </Row>
      </Section>

      {clip.payload.type === "media" && asset && (
        <Section title="Fuente">
          <div className="space-y-1 text-[11px]">
            <div className="truncate text-ink" title={asset.path}>
              {asset.path}
            </div>
            <div className="font-[var(--font-mono)] text-[10px] text-ink-faint">
              {usToDuration(clip.payload.src_in)} → {usToDuration(clip.payload.src_out)} del
              archivo · velocidad {clip.speed}×
            </div>
          </div>
        </Section>
      )}

      {clip.payload.type === "text" && (
        <Section title="Texto">
          <div className="rounded-md border border-line bg-bg2 px-2.5 py-2 text-[12px] text-ink">
            {clip.payload.content}
          </div>
        </Section>
      )}

      <EffectsPanel clip={clip} />
    </>
  );
}

function EffectRow({
  inst,
  def,
  onChange,
  onRemove,
}: {
  inst: EffectInstance;
  def: EffectDef | undefined;
  onChange: (next: EffectInstance) => void;
  onRemove: () => void;
}) {
  return (
    <div className="rounded-lg border border-line bg-bg2 p-2">
      <div className="flex items-center gap-2">
        <input
          type="checkbox"
          className="accent-(--color-accent)"
          checked={inst.enabled}
          onChange={(e) => onChange({ ...inst, enabled: e.target.checked })}
          title="Activar/desactivar efecto"
        />
        <span className="flex-1 truncate text-[12px] font-medium text-ink">
          {def?.name ?? inst.effect_id}
        </span>
        <button
          className="focus-ring rounded px-1.5 text-[12px] text-ink-faint hover:text-danger"
          onClick={onRemove}
          title="Quitar efecto"
        >
          ✕
        </button>
      </div>
      {def && (
        <div className="mt-1.5 space-y-0.5">
          {def.params.map((p) =>
            p.type === "float" ? (
              <Row key={p.key} label={p.label ?? p.key}>
                <Slider
                  value={
                    inst.params[p.key] !== undefined
                      ? paramValue(inst.params[p.key])
                      : (p.default as number)
                  }
                  min={p.min ?? 0}
                  max={p.max ?? 1}
                  step={((p.max ?? 1) - (p.min ?? 0)) / 100}
                  onChange={(v) =>
                    onChange({ ...inst, params: { ...inst.params, [p.key]: v } })
                  }
                />
              </Row>
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

function EffectsPanel({ clip }: { clip: Clip }) {
  const catalog = useStore((s) => s.effectsCatalog);
  const setClipEffects = useStore((s) => s.setClipEffects);

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
    <Section title="Efectos">
      <div className="space-y-1.5">
        {clip.effects.map((e, i) => (
          <EffectRow
            key={`${e.effect_id}-${i}`}
            inst={e}
            def={catalog.find((d) => d.id === e.effect_id)}
            onChange={(next) => update(i, next)}
            onRemove={() => remove(i)}
          />
        ))}
        <select
          className="focus-ring w-full cursor-pointer rounded-md border border-line bg-bg2 px-2 py-1.5 text-[12px] text-ink-dim"
          value=""
          onChange={(e) => e.target.value && add(e.target.value)}
          title="Añadir un efecto al clip"
        >
          <option value="">+ Añadir efecto…</option>
          {catalog.map((d) => (
            <option key={d.id} value={d.id}>
              {d.name}
            </option>
          ))}
        </select>
      </div>
    </Section>
  );
}

export function Inspector() {
  const selection = useStore((s) => s.selection);
  const project = useStore((s) => s.project);
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
            <div className="mt-1.5 space-y-0.5 font-[var(--font-mono)] text-[10px] text-ink-faint">
              <div>
                {seq.resolution[0]}×{seq.resolution[1]} · {Math.round(seq.fps[0] / seq.fps[1])} fps
              </div>
              <div>
                {seq.tracks.length} pistas · {(seq.sample_rate / 1000).toFixed(0)} kHz
              </div>
            </div>
          </div>
          <p className="mt-3 text-[11px] leading-relaxed text-ink-faint">
            Selecciona un clip en la línea de tiempo para editar sus propiedades.
          </p>
        </div>
      )}
    </div>
  );
}
