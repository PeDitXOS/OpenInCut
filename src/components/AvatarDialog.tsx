import { useEffect, useState } from "react";

import type { AvatarConfig, AvatarExpression, Id } from "../engine/types";
import { assetName, newAvatarConfig } from "../engine/types";
import { engine, useStore } from "../state/store";

const inputCls =
  "focus-ring min-w-0 flex-1 rounded-md border border-line bg-bg2 px-2 py-1 text-[12px] text-ink placeholder:text-ink-faint";

function Field({ label, hint, children }: { label: string; hint?: string; children: React.ReactNode }) {
  return (
    <label className="flex items-center gap-2 py-0.5">
      <span className="w-28 shrink-0 text-[11px] text-ink-dim" title={hint}>
        {label}
      </span>
      {children}
    </label>
  );
}

function fileName(path: string): string {
  return path.split("/").pop() ?? path;
}

function isVideo(path: string): boolean {
  return /\.(mp4|mov|webm|mkv|avi)$/i.test(path);
}

/**
 * Full avatar setup: expressions (image or video) with the text the emotion
 * classifier matches, look (shake/size), and the model that classifies.
 * Saved with the project; exportable as JSON. Generating renders the video in
 * the background and drops it into Media.
 */
export function AvatarDialog() {
  const show = useStore((s) => s.showAvatarDialog);
  const setShow = useStore((s) => s.setShowAvatarDialog);
  const project = useStore((s) => s.project);
  const driverAssetId = useStore((s) => s.avatarDriverAsset);
  const saveAvatarConfig = useStore((s) => s.saveAvatarConfig);
  const removeAvatarConfig = useStore((s) => s.removeAvatarConfig);
  const generateAvatarVideo = useStore((s) => s.generateAvatarVideo);
  const importAvatarConfig = useStore((s) => s.importAvatarConfig);
  const exportAvatarConfig = useStore((s) => s.exportAvatarConfig);
  const avatarProgress = useStore((s) => s.avatarProgress);
  const setAvatarDriver = useStore((s) => s.setAvatarDriver);
  const transcribeAsset = useStore((s) => s.transcribeAsset);
  const transcribingIds = useStore((s) => s.transcribingIds);

  const saved = project.avatars ?? [];
  const [draft, setDraft] = useState<AvatarConfig>(newAvatarConfig());
  const [showKey, setShowKey] = useState(false);

  // load the first saved setup when the dialog opens
  useEffect(() => {
    if (show) setDraft(saved.length ? structuredClone(saved[0]) : newAvatarConfig());
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [show]);

  if (!show) return null;

  // the avatar is driven by the VOICE: any asset with audio qualifies
  const voiceAssets = project.assets.filter((a) => a.probe.audio_channels > 0);
  const driver = project.assets.find((a) => a.id === driverAssetId);
  const hasTranscript = project.transcripts.some((t) => t.asset_id === driverAssetId);
  const transcribing = driverAssetId ? transcribingIds.includes(driverAssetId) : false;
  const canGenerate = draft.id !== "" && draft.expressions.length > 0 && hasTranscript;

  const patch = (p: Partial<AvatarConfig>) => setDraft((d) => ({ ...d, ...p }));
  const patchExpr = (i: number, p: Partial<AvatarExpression>) =>
    setDraft((d) => ({
      ...d,
      expressions: d.expressions.map((e, k) => (k === i ? { ...e, ...p } : e)),
    }));

  const addExpressions = async () => {
    const paths = await engine.pickAvatarMedia();
    if (!paths.length) return;
    setDraft((d) => ({
      ...d,
      expressions: [
        ...d.expressions,
        ...paths.map((path) => ({
          // "avatar_angry.mp4" → "angry"
          name: fileName(path)
            .replace(/\.[^.]+$/, "")
            .replace(/^avatar[_-]?/i, "")
            .toLowerCase(),
          path,
          description: "",
        })),
      ],
    }));
  };

  const move = (i: number, dir: -1 | 1) => {
    const j = i + dir;
    if (j < 0 || j >= draft.expressions.length) return;
    const next = [...draft.expressions];
    [next[i], next[j]] = [next[j], next[i]];
    patch({ expressions: next });
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50"
      onClick={() => setShow(false)}
    >
      <div
        className="flex max-h-[86vh] w-[560px] flex-col rounded-xl border border-line bg-bg1 p-4 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="mb-3 flex items-center gap-2">
          <h2 className="flex-1 font-[var(--font-display)] text-[15px] font-semibold text-ink">
            🧑‍🎤 Reactive avatar
          </h2>
          {saved.length > 0 && (
            <select
              className="focus-ring cursor-pointer rounded-md border border-line bg-bg2 px-2 py-1 text-[11px] text-ink"
              value={draft.id}
              onChange={(e) => {
                const found = saved.find((c) => c.id === e.target.value);
                setDraft(found ? structuredClone(found) : newAvatarConfig());
              }}
              title="Saved setups (stored in the project)"
            >
              {saved.map((c) => (
                <option key={c.id} value={c.id}>
                  {c.name}
                </option>
              ))}
              {draft.id === "" && <option value="">(new)</option>}
            </select>
          )}
          <button
            className="focus-ring rounded-md border border-line px-2 py-1 text-[11px] text-ink-dim hover:text-ink"
            onClick={() => setDraft(newAvatarConfig())}
            title="Start a new setup"
          >
            New
          </button>
          <button
            className="focus-ring rounded-md border border-line px-2 py-1 text-[11px] text-ink-dim hover:text-ink"
            onClick={() => void importAvatarConfig()}
            title="Import a JSON setup (also reads the Youtubers-toolkit config.json)"
          >
            Import…
          </button>
          <button
            className="focus-ring rounded-md border border-line px-2 py-1 text-[11px] text-ink-dim hover:text-ink disabled:opacity-40"
            disabled={draft.id === ""}
            onClick={() => void exportAvatarConfig(draft.id)}
            title="Export this setup as JSON (the API key is never written)"
          >
            Export…
          </button>
        </div>

        <div className="min-h-0 flex-1 space-y-3 overflow-y-auto pr-1">
          <div className="rounded-lg border border-line-soft bg-bg2/40 p-2.5">
            <Field label="Name">
              <input
                className={inputCls}
                value={draft.name}
                onChange={(e) => patch({ name: e.target.value })}
              />
            </Field>
            <Field label="Voice" hint="The audio whose speech and emotion drive the avatar">
              <select
                className="focus-ring min-w-0 flex-1 cursor-pointer rounded-md border border-line bg-bg2 px-2 py-1 text-[12px] text-ink"
                value={driverAssetId ?? ""}
                onChange={(e) => setAvatarDriver(e.target.value)}
              >
                {voiceAssets.length === 0 && <option value="">— no audio in the project —</option>}
                {voiceAssets.map((a) => (
                  <option key={a.id} value={a.id}>
                    {assetName(a)}
                    {project.transcripts.some((t) => t.asset_id === a.id) ? " ✓" : ""}
                  </option>
                ))}
              </select>
            </Field>
            {driver && !hasTranscript && (
              <div className="mt-1 flex items-center gap-2 rounded-md border border-accent/40 bg-bg2 px-2 py-1.5">
                <span className="flex-1 text-[11px] text-ink-dim">
                  The avatar needs the words and emotions of this voice.
                </span>
                <button
                  className="focus-ring shrink-0 rounded-md border border-accent/60 px-2 py-1 text-[11px] font-medium text-accent hover:bg-bg3 disabled:opacity-50"
                  disabled={transcribing}
                  onClick={() => void transcribeAsset(driver.id)}
                >
                  {transcribing ? "⏳ Transcribing…" : "🎙 Transcribe"}
                </button>
              </div>
            )}
          </div>

          <section className="rounded-lg border border-line-soft bg-bg2/40 p-2.5">
            <div className="mb-1.5 flex items-center">
              <h3 className="panel-eyebrow flex-1">Expressions</h3>
              <button
                className="focus-ring rounded-md border border-accent/60 px-2 py-1 text-[11px] text-accent hover:bg-bg3"
                onClick={() => void addExpressions()}
              >
                + Add images / videos
              </button>
            </div>
            {draft.expressions.length === 0 && (
              <p className="px-1 py-2 text-[11px] leading-relaxed text-ink-faint">
                Add one file per expression. The <b>first one is the default</b> (used when nothing
                else matches). The description helps the classifier: “angry → furious, complaining”.
              </p>
            )}
            <div className="space-y-1">
              {draft.expressions.map((e, i) => (
                <div
                  key={`${e.path}-${i}`}
                  className="flex items-center gap-1.5 rounded-md border border-line bg-bg2 p-1.5"
                >
                  <span
                    className="w-6 shrink-0 text-center text-[10px] text-ink-faint"
                    title={i === 0 ? "Default expression" : `Expression ${i + 1}`}
                  >
                    {i === 0 ? "★" : i + 1}
                  </span>
                  <span className="shrink-0 text-[13px]" title={e.path}>
                    {isVideo(e.path) ? "🎬" : "🖼"}
                  </span>
                  <input
                    className="focus-ring w-24 shrink-0 rounded border border-line bg-bg1 px-1.5 py-0.5 text-[11px] text-ink"
                    value={e.name}
                    placeholder="name"
                    onChange={(ev) => patchExpr(i, { name: ev.target.value })}
                    title="Label the classifier returns"
                  />
                  <input
                    className="focus-ring min-w-0 flex-1 rounded border border-line bg-bg1 px-1.5 py-0.5 text-[11px] text-ink placeholder:text-ink-faint"
                    value={e.description}
                    placeholder="when to use it (furious, upset…)"
                    onChange={(ev) => patchExpr(i, { description: ev.target.value })}
                  />
                  <button
                    className="focus-ring rounded px-1 text-[10px] text-ink-faint hover:text-ink disabled:opacity-30"
                    disabled={i === 0}
                    onClick={() => move(i, -1)}
                    title="Move up"
                  >
                    ▲
                  </button>
                  <button
                    className="focus-ring rounded px-1 text-[10px] text-ink-faint hover:text-ink disabled:opacity-30"
                    disabled={i === draft.expressions.length - 1}
                    onClick={() => move(i, 1)}
                    title="Move down"
                  >
                    ▼
                  </button>
                  <button
                    className="focus-ring rounded px-1 text-[11px] text-ink-faint hover:text-danger"
                    onClick={() =>
                      patch({ expressions: draft.expressions.filter((_, k) => k !== i) })
                    }
                  >
                    ✕
                  </button>
                </div>
              ))}
            </div>
          </section>

          <section className="rounded-lg border border-line-soft bg-bg2/40 p-2.5">
            <h3 className="panel-eyebrow mb-1.5">Look</h3>
            <Field label="Size">
              <input
                type="range"
                className="h-1 min-w-0 flex-1 cursor-pointer appearance-none rounded-full bg-bg3 accent-(--color-accent)"
                min={0.05}
                max={1}
                step={0.05}
                value={draft.scale}
                onChange={(e) => patch({ scale: Number(e.target.value) })}
              />
              <span className="w-12 shrink-0 text-right font-[var(--font-mono)] text-[11px] text-ink">
                {Math.round(draft.scale * 100)}%
              </span>
            </Field>
            <Field label="Shake" hint="How much the avatar bounces with your voice">
              <input
                type="range"
                className="h-1 min-w-0 flex-1 cursor-pointer appearance-none rounded-full bg-bg3 accent-(--color-accent)"
                min={0}
                max={3}
                step={0.1}
                value={draft.shake_factor}
                onChange={(e) => patch({ shake_factor: Number(e.target.value) })}
              />
              <span className="w-12 shrink-0 text-right font-[var(--font-mono)] text-[11px] text-ink">
                {draft.shake_factor.toFixed(1)}×
              </span>
            </Field>
          </section>

          <section className="rounded-lg border border-line-soft bg-bg2/40 p-2.5">
            <h3 className="panel-eyebrow mb-1.5">Emotion classifier</h3>
            <Field label="Model" hint="Empty = offline heuristic (volume + speech rate)">
              <input
                className={inputCls}
                value={draft.model}
                placeholder="gpt-4o-mini (empty = offline)"
                onChange={(e) => patch({ model: e.target.value })}
              />
            </Field>
            <Field label="API base">
              <input
                className={inputCls}
                value={draft.api_base}
                placeholder="https://api.openai.com/v1"
                onChange={(e) => patch({ api_base: e.target.value })}
              />
            </Field>
            <Field label="API key" hint="Stored in the project; never written to exported JSON">
              <input
                className={inputCls}
                type={showKey ? "text" : "password"}
                value={draft.api_key}
                placeholder="empty = use OPENAI_API_KEY"
                onChange={(e) => patch({ api_key: e.target.value })}
              />
              <button
                className="focus-ring shrink-0 rounded border border-line px-1.5 py-0.5 text-[10.5px] text-ink-faint hover:text-ink"
                onClick={() => setShowKey((v) => !v)}
              >
                {showKey ? "hide" : "show"}
              </button>
            </Field>
          </section>
        </div>

        {avatarProgress && (
          <div className="mt-2 rounded-md border border-line bg-bg2 p-2">
            <div className="flex items-center gap-2 text-[11px] text-ink">
              <span className="flex-1">{avatarProgress.message}</span>
              <span className="font-[var(--font-mono)] text-ink-faint">
                {Math.round(avatarProgress.progress * 100)}%
              </span>
            </div>
            <div className="mt-1 h-1 overflow-hidden rounded-full bg-bg3">
              <div
                className={`h-full rounded-full ${
                  avatarProgress.stage === "error" ? "bg-danger" : "bg-accent"
                }`}
                style={{ width: `${Math.round(avatarProgress.progress * 100)}%` }}
              />
            </div>
          </div>
        )}

        <div className="mt-3 flex items-center gap-2">
          {draft.id !== "" && (
            <button
              className="focus-ring rounded-md border border-line px-3 py-1.5 text-[12px] text-ink-dim hover:text-danger"
              onClick={() => {
                void removeAvatarConfig(draft.id);
                setDraft(newAvatarConfig());
              }}
            >
              Delete
            </button>
          )}
          <div className="flex-1" />
          <button
            className="focus-ring rounded-md border border-line px-3 py-1.5 text-[12px] text-ink hover:bg-bg2"
            onClick={() => setShow(false)}
          >
            Close
          </button>
          <button
            className="focus-ring rounded-md border border-line px-3 py-1.5 text-[12px] text-ink hover:bg-bg2 disabled:opacity-40"
            disabled={draft.expressions.length === 0 || !draft.name.trim()}
            onClick={() => void saveAvatarConfig(draft)}
            title="Save the setup in the project"
          >
            Save
          </button>
          <button
            className="focus-ring rounded-md bg-(--color-accent) px-3 py-1.5 text-[12px] font-semibold text-black hover:brightness-110 disabled:opacity-40"
            disabled={!canGenerate || avatarProgress?.stage === "classifying"}
            onClick={() => void generateAvatarVideo(draft.id, driverAssetId as Id)}
            title={
              !hasTranscript
                ? "Transcribe the voice first"
                : draft.id === ""
                  ? "Save the setup first"
                  : "Render the avatar video in the background and add it to Media"
            }
          >
            Generate video
          </button>
        </div>
      </div>
    </div>
  );
}
