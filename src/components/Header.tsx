import { useStore } from "../state/store";

export function Header() {
  const project = useStore((s) => s.project);
  const dirty = useStore((s) => s.dirty);
  const canUndo = useStore((s) => s.canUndo);
  const canRedo = useStore((s) => s.canRedo);
  const undo = useStore((s) => s.undo);
  const redo = useStore((s) => s.redo);
  const exportVideo = useStore((s) => s.exportVideo);
  const exporting = useStore((s) => s.exporting);
  const saveProject = useStore((s) => s.saveProject);
  const openProject = useStore((s) => s.openProject);
  const exportProgress = useStore((s) => s.exportProgress);
  const cancelExport = useStore((s) => s.cancelExport);

  return (
    <header className="flex h-12 shrink-0 items-center gap-3 border-b border-line bg-bg1 px-3">
      <div className="flex items-center gap-2.5">
        <div className="flex h-7 w-7 items-center justify-center rounded-md bg-accent font-[var(--font-display)] text-[13px] font-bold text-bg0">
          UE
        </div>
        <span className="font-[var(--font-display)] text-[15px] font-semibold tracking-tight">
          UberEditor
        </span>
      </div>

      <div className="mx-2 h-5 w-px bg-line" />

      <div className="flex items-center gap-2 text-ink-dim">
        <span className="max-w-[360px] truncate text-[13px] text-ink">{project.name}</span>
        {dirty && <span className="h-1.5 w-1.5 rounded-full bg-accent" title="Cambios sin guardar" />}
      </div>

      <div className="flex-1" />

      <button
        className="focus-ring rounded-md px-2.5 py-1.5 text-[12px] text-ink-dim hover:bg-bg3 hover:text-ink"
        onClick={() => void openProject()}
        title="Abrir proyecto (⌘O)"
      >
        Abrir…
      </button>
      <button
        className="focus-ring rounded-md px-2.5 py-1.5 text-[12px] text-ink-dim hover:bg-bg3 hover:text-ink"
        onClick={() => void saveProject()}
        title="Guardar proyecto (⌘S)"
      >
        Guardar
      </button>

      <div className="mx-1 h-5 w-px bg-line" />

      <button
        className="focus-ring rounded-md px-2.5 py-1.5 text-[12px] text-ink-dim enabled:hover:bg-bg3 enabled:hover:text-ink disabled:opacity-40"
        onClick={() => void undo()}
        disabled={!canUndo}
        title="Deshacer (⌘Z)"
      >
        ↶ Deshacer
      </button>
      <button
        className="focus-ring rounded-md px-2.5 py-1.5 text-[12px] text-ink-dim enabled:hover:bg-bg3 enabled:hover:text-ink disabled:opacity-40"
        onClick={() => void redo()}
        disabled={!canRedo}
        title="Rehacer (⇧⌘Z)"
      >
        ↷ Rehacer
      </button>

      <div className="mx-1 h-5 w-px bg-line" />

      <span
        className="flex items-center gap-1.5 rounded-full border border-line px-2.5 py-1 text-[11px] text-ink-faint"
        title="Servidor MCP: llega en la Fase 4"
      >
        <span className="h-1.5 w-1.5 rounded-full bg-ink-faint" />
        MCP inactivo
      </span>

      {exporting && (
        <button
          className="focus-ring rounded-md border border-line px-2 py-1.5 text-[12px] text-ink-dim hover:text-danger"
          onClick={() => void cancelExport()}
          title="Cancelar la exportación"
        >
          Cancelar
        </button>
      )}
      <button
        className="focus-ring relative overflow-hidden rounded-md bg-accent px-3.5 py-1.5 text-[12px] font-semibold text-bg0 enabled:hover:bg-accent-deep disabled:opacity-80"
        onClick={() => void exportVideo()}
        disabled={exporting}
        title="Exportar la secuencia a MP4"
      >
        {exporting && (
          <span
            className="absolute inset-y-0 left-0 bg-accent-deep"
            style={{ width: `${Math.round((exportProgress ?? 0) * 100)}%` }}
          />
        )}
        <span className="relative">
          {exporting ? `Exportando ${Math.round((exportProgress ?? 0) * 100)}%` : "Exportar…"}
        </span>
      </button>
    </header>
  );
}
