import { useStore } from "../state/store";

export function StatusBar() {
  const project = useStore((s) => s.project);
  const lastAction = useStore((s) => s.lastActionLabel);
  const dirty = useStore((s) => s.dirty);

  const caching = project.assets.find((a) => a.caching !== undefined);
  const seq = project.sequences[0];

  return (
    <footer className="flex h-7 shrink-0 items-center gap-4 border-t border-line bg-bg1 px-3 text-[10.5px] text-ink-faint">
      <span>{dirty ? "Cambios sin guardar" : "Todo guardado"}</span>
      {lastAction && <span className="text-ink-dim">· {lastAction}</span>}
      <div className="flex-1" />
      {caching && (
        <span className="flex items-center gap-2">
          Generando proxy · {caching.path.split("/").pop()}
          <span className="inline-block h-[3px] w-24 overflow-hidden rounded-full bg-bg3 align-middle">
            <span
              className="block h-full rounded-full bg-accent/70"
              style={{ width: `${(caching.caching ?? 0) * 100}%` }}
            />
          </span>
          <span className="font-[var(--font-mono)]">{Math.round((caching.caching ?? 0) * 100)}%</span>
        </span>
      )}
      <span className="font-[var(--font-mono)]">
        {seq.resolution[0]}×{seq.resolution[1]} · {Math.round(seq.fps[0] / seq.fps[1])} fps · 48 kHz
      </span>
    </footer>
  );
}
