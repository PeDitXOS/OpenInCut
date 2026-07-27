import { activeSequence } from "../engine/types";
import { useStore } from "../state/store";
import { usToTimecode, usToDuration, usToFrame } from "../lib/time";

export function StatusBar() {
  const project = useStore((s) => s.project);
  const lastAction = useStore((s) => s.lastActionLabel);
  const dirty = useStore((s) => s.dirty);
  const selectionCount = useStore((s) => s.selection.length);
  const playheadUs = useStore((s) => s.playheadUs);
  const mcpPort = useStore((s) => s.mcpPort);

  const seq = activeSequence(project);
  const isError = lastAction?.startsWith("⚠");
  const fpsRounded = Math.round(seq.fps[0] / seq.fps[1]);
  const totalClips = seq.tracks.reduce((n, t) => n + t.clips.length, 0);
  // total duration = latest clip end across all tracks
  const totalDuration = seq.tracks.reduce(
    (max, t) => t.clips.reduce((m, c) => Math.max(m, c.start + c.duration), max),
    0,
  );

  return (
    <footer className="flex h-7 shrink-0 items-center gap-4 border-t border-line bg-bg1 px-3 text-[10.5px] text-ink-faint">
      <span>{dirty ? "Unsaved changes" : "All saved"}</span>
      {selectionCount > 1 && <span>· {selectionCount} clips selected</span>}
      {lastAction && (
        <span className={isError ? "text-danger" : "text-ink-dim"}>· {lastAction}</span>
      )}
      <div className="flex-1" />
      <span className="font-[var(--font-mono)]">
        {usToTimecode(playheadUs, seq.fps)} · F{usToFrame(playheadUs, seq.fps)} / {totalDuration > 0 ? usToFrame(totalDuration, seq.fps) : 0}
      </span>
      <span className="font-[var(--font-mono)]">
        {seq.resolution[0]}×{seq.resolution[1]} · {fpsRounded} fps
      </span>
      <span>{seq.tracks.length} tracks · {totalClips} clips</span>
      <span className="font-[var(--font-mono)]">
        {usToDuration(totalDuration)}
      </span>
      <span className="flex items-center gap-1">
        <span className={`inline-block h-1.5 w-1.5 rounded-full ${mcpPort !== null ? "bg-green-500" : "bg-red-500"}`} />
        MCP
      </span>
    </footer>
  );
}
