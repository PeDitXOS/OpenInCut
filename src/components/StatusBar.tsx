import { activeSequence } from "../engine/types";
import { useStore } from "../state/store";

export function StatusBar() {
  const project = useStore((s) => s.project);
  const lastAction = useStore((s) => s.lastActionLabel);
  const dirty = useStore((s) => s.dirty);
  const selectionCount = useStore((s) => s.selection.length);

  const seq = activeSequence(project);
  const isError = lastAction?.startsWith("⚠");

  return (
    <footer className="flex h-7 shrink-0 items-center gap-4 border-t border-line bg-bg1 px-3 text-[10.5px] text-ink-faint">
      <span>{dirty ? "Unsaved changes" : "All saved"}</span>
      {selectionCount > 1 && <span>· {selectionCount} clips selected</span>}
      {lastAction && (
        <span className={isError ? "text-danger" : "text-ink-dim"}>· {lastAction}</span>
      )}
      <div className="flex-1" />
      <span className="font-[var(--font-mono)]">
        {seq.resolution[0]}×{seq.resolution[1]} · {Math.round(seq.fps[0] / seq.fps[1])} fps ·{" "}
        {(seq.sample_rate / 1000).toFixed(0)} kHz
      </span>
    </footer>
  );
}
