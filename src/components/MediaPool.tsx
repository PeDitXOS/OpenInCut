import type { MediaAsset } from "../engine/types";
import { assetName } from "../engine/types";
import { usToDuration } from "../lib/time";
import { useStore } from "../state/store";

const KIND_LABEL: Record<MediaAsset["kind"], string> = {
  video: "VIDEO",
  audio: "AUDIO",
  image: "IMAGE",
};

function AssetThumb({ asset }: { asset: MediaAsset }) {
  if (asset.kind === "video")
    return (
      <div className="relative h-12 w-20 shrink-0 overflow-hidden rounded bg-clip-video">
        <div className="absolute inset-0 opacity-60 [background:repeating-linear-gradient(90deg,#0000_0_14px,#0003_14px_16px)]" />
        <div className="absolute bottom-1 left-1 rounded-sm bg-black/50 px-1 font-[var(--font-mono)] text-[9px] text-ink">
          {usToDuration(asset.probe.duration_us)}
        </div>
      </div>
    );
  if (asset.kind === "audio")
    return (
      <div className="flex h-12 w-20 shrink-0 items-end gap-[2px] overflow-hidden rounded bg-clip-audio px-1.5 pb-1.5">
        {[5, 9, 14, 8, 16, 11, 6, 13, 9, 15, 7, 11].map((h, i) => (
          <div key={i} className="w-[3px] rounded-sm bg-clip-audio-hi/80" style={{ height: h * 2 }} />
        ))}
      </div>
    );
  return (
    <div className="flex h-12 w-20 shrink-0 items-center justify-center rounded bg-clip-text">
      <div className="h-6 w-6 rounded-sm border-2 border-clip-text-hi/70" />
    </div>
  );
}

export function MediaPool() {
  const assets = useStore((s) => s.project.assets);
  const importMedia = useStore((s) => s.importMedia);
  const addClipFromAsset = useStore((s) => s.addClipFromAsset);
  const transcribeAsset = useStore((s) => s.transcribeAsset);
  const transcribingIds = useStore((s) => s.transcribingIds);
  const relinkAsset = useStore((s) => s.relinkAsset);

  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center justify-between px-3 pb-2 pt-3">
        <h2 className="panel-eyebrow">Media</h2>
        <button
          className="focus-ring rounded-md border border-line px-2 py-1 text-[11px] text-ink-dim hover:bg-bg3 hover:text-ink"
          onClick={() => void importMedia()}
          title="Import video, audio, or image files"
        >
          + Import
        </button>
      </div>
      <div className="min-h-0 flex-1 space-y-1 overflow-y-auto px-2 pb-2">
        {assets.length === 0 && (
          <div className="mx-1 mt-2 rounded-lg border border-dashed border-line px-3 py-6 text-center text-[11px] leading-relaxed text-ink-faint">
            No media yet.
            <br />
            Use "+ Import" to add video, audio, or images.
          </div>
        )}
        {assets.map((a) => (
          <div
            key={a.id}
            className="group flex cursor-grab items-center gap-2.5 rounded-lg p-1.5 hover:bg-bg2"
            title={`${a.path}\nDouble-click: add to the timeline at the playhead`}
            onDoubleClick={() => void addClipFromAsset(a.id)}
          >
            <AssetThumb asset={a} />
            <div className="min-w-0 flex-1">
              <div
                className={`truncate text-[12px] leading-tight ${
                  a.offline ? "text-danger" : "text-ink"
                }`}
              >
                {a.offline && "⚠ "}
                {assetName(a)}
              </div>
              <div className="mt-0.5 font-[var(--font-mono)] text-[10px] text-ink-faint">
                {KIND_LABEL[a.kind]}
                {a.probe.width > 0 && ` · ${a.probe.width}×${a.probe.height}`}
                {a.probe.fps && ` · ${Math.round(a.probe.fps[0] / a.probe.fps[1])}fps`}
                {a.kind === "audio" && ` · ${usToDuration(a.probe.duration_us)}`}
                {a.probe.vfr && " · VFR"}
              </div>
              {a.offline && (
                <button
                  className="focus-ring mt-1 rounded border border-danger/60 px-1.5 py-0.5 text-[10px] text-danger hover:bg-danger/10"
                  onClick={(e) => {
                    e.stopPropagation();
                    void relinkAsset(a.id);
                  }}
                  title="The file is not at its path: select its new location"
                >
                  Relink…
                </button>
              )}
            </div>
            {a.probe.audio_channels > 0 &&
              (a.transcript ? (
                <span
                  className="shrink-0 rounded bg-clip-audio px-1.5 py-0.5 font-[var(--font-mono)] text-[9px] text-clip-audio-hi"
                  title="Word-level transcript ready"
                >
                  T✓
                </span>
              ) : transcribingIds.includes(a.id) ? (
                <span
                  className="shrink-0 rounded border border-line px-1.5 py-0.5 text-[10px] text-ink-faint"
                  title="Transcribing in the background…"
                >
                  ⏳
                </span>
              ) : (
                <button
                  className="focus-ring shrink-0 rounded border border-line px-1.5 py-0.5 text-[10px] text-ink-dim hover:border-accent/60 hover:text-accent"
                  title="Transcribe with Whisper (word by word): enables text-based editing and subtitles"
                  onClick={(e) => {
                    e.stopPropagation();
                    void transcribeAsset(a.id);
                  }}
                >
                  🎙 Transcribe
                </button>
              ))}
          </div>
        ))}
      </div>
      <div className="border-t border-line-soft px-3 py-2 text-[11px] text-ink-faint">
        Double-click a media item to add it to the timeline
      </div>
    </div>
  );
}
