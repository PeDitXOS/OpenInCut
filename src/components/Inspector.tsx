import type { Clip } from "../engine/types";
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
  onChange,
}: {
  value: number;
  min: number;
  max: number;
  step: number;
  unit?: string;
  onChange: (v: number) => void;
}) {
  return (
    <>
      <input
        type="range"
        className="h-1 min-w-0 flex-1 cursor-pointer appearance-none rounded-full bg-bg3 accent-(--color-accent)"
        min={min}
        max={max}
        step={step}
        value={value}
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
  const setClipProp = useStore((s) => s.setClipProp);
  const fps = project.sequences[0].fps;

  const asset =
    clip.payload.type === "media"
      ? project.assets.find((a) => a.id === (clip.payload as { asset_id: string }).asset_id)
      : undefined;

  return (
    <>
      <div className="border-b border-line-soft px-3 py-3">
        <div className="truncate text-[13px] font-medium text-ink">
          {clip.label ?? (clip.payload.type === "text" ? "Texto" : "Clip")}
        </div>
        <div className="mt-1 font-[var(--font-mono)] text-[10px] text-ink-faint">
          {usToTimecode(clip.start, fps)} → {usToTimecode(clip.start + clip.duration, fps)} ·{" "}
          {usToDuration(clip.duration)}
        </div>
      </div>

      <Section title="Transformación">
        <Row label="Opacidad">
          <Slider
            value={clip.opacity}
            min={0}
            max={1}
            step={0.01}
            onChange={(v) => setClipProp(clip.id, { opacity: v })}
          />
        </Row>
        <Row label="Escala">
          <Slider value={1} min={0.1} max={4} step={0.01} onChange={() => {}} />
        </Row>
        <Row label="Rotación">
          <Slider value={0} min={-180} max={180} step={1} unit="°" onChange={() => {}} />
        </Row>
      </Section>

      <Section title="Audio">
        <Row label="Ganancia">
          <Slider
            value={clip.gain_db}
            min={-60}
            max={12}
            step={0.5}
            unit=" dB"
            onChange={(v) => setClipProp(clip.id, { gain_db: v })}
          />
        </Row>
        <Row label="Fade in">
          <span className="font-[var(--font-mono)] text-[11px] text-ink">
            {(clip.fade_in_us / 1e6).toFixed(1)} s
          </span>
        </Row>
        <Row label="Fade out">
          <span className="font-[var(--font-mono)] text-[11px] text-ink">
            {(clip.fade_out_us / 1e6).toFixed(1)} s
          </span>
        </Row>
      </Section>

      {clip.payload.type === "media" && asset && (
        <Section title="Fuente">
          <div className="space-y-1 text-[11px]">
            <div className="truncate text-ink">{asset.path}</div>
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

      <Section title="Efectos">
        <div className="rounded-md border border-dashed border-line px-2.5 py-3 text-center text-[11px] text-ink-faint">
          Sin efectos. El sistema de packs llega en la Fase 2.
        </div>
      </Section>
    </>
  );
}

export function Inspector() {
  const selection = useStore((s) => s.selection);
  const project = useStore((s) => s.project);
  useStore((s) => s.version);

  const seq = project.sequences[0];
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
              <div>{seq.tracks.length} pistas · 48 kHz</div>
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
