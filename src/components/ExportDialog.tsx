import { useState } from "react";

import type { ExportUiSettings } from "../engine/client";
import { usToDuration } from "../lib/time";
import { useStore } from "../state/store";

interface Preset {
  name: string;
  hint: string;
  maxHeight: number | null;
  crf: number;
  preset: string;
  audioBitrateK: number;
}

const PRESETS: Preset[] = [
  { name: "YouTube 1080p", hint: "H.264 · CRF 18", maxHeight: 1080, crf: 18, preset: "veryfast", audioBitrateK: 256 },
  { name: "YouTube 4K", hint: "H.264 · CRF 17", maxHeight: 2160, crf: 17, preset: "veryfast", audioBitrateK: 320 },
  { name: "Máxima calidad", hint: "resolución nativa · CRF 15", maxHeight: null, crf: 15, preset: "medium", audioBitrateK: 320 },
  { name: "Borrador rápido", hint: "720p · CRF 26", maxHeight: 720, crf: 26, preset: "ultrafast", audioBitrateK: 128 },
];

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="flex items-center gap-2">
      <span className="w-24 shrink-0 text-[11px] text-ink-dim">{label}</span>
      {children}
    </label>
  );
}

const selectCls =
  "focus-ring min-w-0 flex-1 cursor-pointer rounded-md border border-line bg-bg2 px-2 py-1 text-[12px] text-ink";

export function ExportDialog() {
  const show = useStore((s) => s.showExportDialog);
  const setShow = useStore((s) => s.setShowExportDialog);
  const exportVideo = useStore((s) => s.exportVideo);
  const rangeInUs = useStore((s) => s.rangeInUs);
  const rangeOutUs = useStore((s) => s.rangeOutUs);

  const [presetIdx, setPresetIdx] = useState(0);
  const [maxHeight, setMaxHeight] = useState<number | null>(1080);
  const [crf, setCrf] = useState(18);
  const [codecPreset, setCodecPreset] = useState("veryfast");
  const [audioK, setAudioK] = useState(256);
  const [loudnorm, setLoudnorm] = useState(false);
  const [useRange, setUseRange] = useState(false);

  if (!show) return null;

  const hasRange = rangeInUs != null && rangeOutUs != null && rangeOutUs > rangeInUs;
  const applyPreset = (i: number) => {
    const p = PRESETS[i];
    setPresetIdx(i);
    setMaxHeight(p.maxHeight);
    setCrf(p.crf);
    setCodecPreset(p.preset);
    setAudioK(p.audioBitrateK);
  };
  const settings: ExportUiSettings = {
    maxHeight,
    crf,
    preset: codecPreset,
    audioBitrateK: audioK,
    loudnorm,
    rangeInUs: useRange && hasRange ? rangeInUs : null,
    rangeOutUs: useRange && hasRange ? rangeOutUs : null,
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50"
      onClick={() => setShow(false)}
    >
      <div
        className="w-[440px] rounded-xl border border-line bg-bg1 p-4 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <h2 className="mb-3 font-[var(--font-display)] text-[15px] font-semibold text-ink">
          Exportar video
        </h2>

        <div className="mb-3 grid grid-cols-2 gap-1.5">
          {PRESETS.map((p, i) => (
            <button
              key={p.name}
              className={`focus-ring rounded-lg border px-2.5 py-1.5 text-left ${
                presetIdx === i
                  ? "border-(--color-accent) bg-bg2"
                  : "border-line bg-bg2/50 hover:bg-bg2"
              }`}
              onClick={() => applyPreset(i)}
            >
              <div className="text-[12px] font-medium text-ink">{p.name}</div>
              <div className="text-[10.5px] text-ink-faint">{p.hint}</div>
            </button>
          ))}
        </div>

        <div className="flex flex-col gap-2 rounded-lg border border-line-soft bg-bg2/40 p-2.5">
          <Field label="Resolución">
            <select
              className={selectCls}
              value={maxHeight ?? 0}
              onChange={(e) => {
                setMaxHeight(Number(e.target.value) || null);
                setPresetIdx(-1);
              }}
            >
              <option value={0}>Nativa (secuencia)</option>
              <option value={2160}>Hasta 2160p (4K)</option>
              <option value={1440}>Hasta 1440p</option>
              <option value={1080}>Hasta 1080p</option>
              <option value={720}>Hasta 720p</option>
            </select>
          </Field>
          <Field label={`Calidad CRF ${crf}`}>
            <input
              type="range"
              className="h-1 min-w-0 flex-1 cursor-pointer appearance-none rounded-full bg-bg3 accent-(--color-accent)"
              min={14}
              max={28}
              step={1}
              value={crf}
              onChange={(e) => {
                setCrf(Number(e.target.value));
                setPresetIdx(-1);
              }}
            />
            <span className="w-16 shrink-0 text-right text-[10.5px] text-ink-faint">
              {crf <= 17 ? "más calidad" : crf >= 24 ? "más ligero" : "equilibrado"}
            </span>
          </Field>
          <Field label="Códec">
            <select
              className={selectCls}
              value={codecPreset}
              onChange={(e) => {
                setCodecPreset(e.target.value);
                setPresetIdx(-1);
              }}
            >
              <option value="ultrafast">Ultra rápido</option>
              <option value="veryfast">Rápido</option>
              <option value="medium">Medio</option>
              <option value="slow">Lento (mejor compresión)</option>
            </select>
          </Field>
          <Field label="Audio">
            <select
              className={selectCls}
              value={audioK}
              onChange={(e) => {
                setAudioK(Number(e.target.value));
                setPresetIdx(-1);
              }}
            >
              {[128, 192, 256, 320].map((k) => (
                <option key={k} value={k}>
                  AAC {k} kbps
                </option>
              ))}
            </select>
          </Field>
          <label className="flex cursor-pointer items-center gap-2 text-[12px] text-ink">
            <input
              type="checkbox"
              className="accent-(--color-accent)"
              checked={loudnorm}
              onChange={(e) => setLoudnorm(e.target.checked)}
            />
            Normalizar sonoridad (R128, −14 LUFS)
          </label>
        </div>

        <div className="mt-2.5 flex items-center gap-3 text-[12px] text-ink">
          <span className="w-24 shrink-0 text-[11px] text-ink-dim">Rango</span>
          <label className="flex cursor-pointer items-center gap-1.5">
            <input
              type="radio"
              className="accent-(--color-accent)"
              checked={!useRange}
              onChange={() => setUseRange(false)}
            />
            Todo
          </label>
          <label
            className={`flex items-center gap-1.5 ${hasRange ? "cursor-pointer" : "opacity-40"}`}
            title={hasRange ? "" : "Marca el rango con las teclas I y O sobre la línea de tiempo"}
          >
            <input
              type="radio"
              className="accent-(--color-accent)"
              disabled={!hasRange}
              checked={useRange && hasRange}
              onChange={() => setUseRange(true)}
            />
            Rango I–O
            {hasRange && (
              <span className="font-[var(--font-mono)] text-[11px] text-ink-faint">
                {usToDuration(rangeInUs)}–{usToDuration(rangeOutUs)}
              </span>
            )}
          </label>
        </div>

        <div className="mt-4 flex justify-end gap-2">
          <button
            className="focus-ring rounded-md border border-line px-3 py-1.5 text-[12px] text-ink hover:bg-bg2"
            onClick={() => setShow(false)}
          >
            Cancelar
          </button>
          <button
            className="focus-ring rounded-md bg-(--color-accent) px-3 py-1.5 text-[12px] font-semibold text-black hover:brightness-110"
            onClick={() => void exportVideo(settings)}
          >
            Exportar…
          </button>
        </div>
      </div>
    </div>
  );
}
