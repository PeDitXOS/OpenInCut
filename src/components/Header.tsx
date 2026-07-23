import { useEffect, useRef, useState } from "react";

import { useStore } from "../state/store";

/** Copy text to the clipboard, falling back to a hidden textarea when the
 *  async Clipboard API is unavailable (older webviews / no secure context). */
async function copyText(text: string): Promise<boolean> {
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(text);
      return true;
    }
  } catch {
    // fall through to the legacy path
  }
  try {
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.style.position = "fixed";
    ta.style.opacity = "0";
    document.body.appendChild(ta);
    ta.select();
    const ok = document.execCommand("copy");
    document.body.removeChild(ta);
    return ok;
  } catch {
    return false;
  }
}

/** One labelled, copyable value (the connect command, the token, the URL). */
function CopyRow({ label, value }: { label: string; value: string }) {
  const [copied, setCopied] = useState(false);
  const onCopy = async () => {
    if (await copyText(value)) {
      setCopied(true);
      setTimeout(() => setCopied(false), 1400);
    }
  };
  return (
    <div>
      <div className="mb-1 flex items-center justify-between">
        <span className="text-[10.5px] font-medium uppercase tracking-wide text-ink-faint">
          {label}
        </span>
        <button
          className="focus-ring rounded px-1.5 py-0.5 text-[10.5px] text-ink-dim hover:bg-bg3 hover:text-ink"
          onClick={() => void onCopy()}
        >
          {copied ? "✓ Copied" : "Copy"}
        </button>
      </div>
      <div className="max-h-24 overflow-auto rounded-md border border-line bg-bg0 px-2 py-1.5 font-[var(--font-mono)] text-[11px] text-ink select-all break-all whitespace-pre-wrap">
        {value}
      </div>
    </div>
  );
}

import McpChat from "./McpChat";
import McpSettings from "./McpSettings";

function McpPill() {
  const mcpPort = useStore((s) => s.mcpPort);
  const [open, setOpen] = useState(false);
  const [tab, setTab] = useState<"info" | "chat" | "settings">("info");
  const wrapRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    const onClick = (e: MouseEvent) => {
      if (wrapRef.current && !wrapRef.current.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener("keydown", onKey);
    const t = setTimeout(() => window.addEventListener("mousedown", onClick), 0);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("mousedown", onClick);
      clearTimeout(t);
    };
  }, [open]);

  const tabs = [
    { id: "info" as const, label: "Info" },
    { id: "chat" as const, label: "Chat" },
    { id: "settings" as const, label: "Settings" },
  ];

  return (
    <div className="relative" ref={wrapRef}>
      <button
        className={`focus-ring flex items-center gap-1.5 rounded-full border px-2.5 py-1 text-[11px] ${
          open ? "border-(--color-accent) bg-bg2" : "border-line"
        } ${mcpPort ? "text-ink hover:bg-bg2" : "text-ink-faint hover:bg-bg2"}`}
        onClick={() => setOpen((o) => !o)}
        title="MCP server — click for the connection details"
      >
        <span className={`h-1.5 w-1.5 rounded-full ${mcpPort ? "bg-clip-audio-hi" : "bg-ink-faint"}`} />
        {mcpPort ? `MCP :${mcpPort}` : "MCP inactive"}
        <span className="text-ink-faint">▾</span>
      </button>

      {open && (
        <div className="absolute right-0 top-full z-50 mt-2 w-[420px] rounded-xl border border-line bg-bg1 shadow-2xl">
          <div className="flex border-b border-line">
            {tabs.map((t) => (
              <button
                key={t.id}
                className={`flex-1 px-3 py-2 text-[11px] font-medium ${
                  tab === t.id ? "border-b-2 border-accent text-ink" : "text-ink-faint hover:text-ink"
                }`}
                onClick={() => setTab(t.id)}
              >
                {t.label}
              </button>
            ))}
          </div>
          <div className="max-h-[500px] overflow-auto">
            {tab === "info" && <McpInfo />}
            {tab === "chat" && <McpChat />}
            {tab === "settings" && <McpSettings onClose={() => setOpen(false)} />}
          </div>
        </div>
      )}
    </div>
  );
}

function McpInfo() {
  const mcpPort = useStore((s) => s.mcpPort);
  const mcpToken = useStore((s) => s.mcpToken);
  const url = mcpPort ? `http://127.0.0.1:${mcpPort}/mcp` : "";
  const command = mcpPort
    ? `claude mcp add --transport http opencut ${url} --header "Authorization: Bearer ${mcpToken}"`
    : "";

  if (!mcpPort) {
    return (
      <div className="p-4">
        <p className="text-[11px] leading-relaxed text-ink-dim">
          The MCP server runs inside the desktop app. Launch it with{" "}
          <span className="font-[var(--font-mono)] text-ink">npx tauri dev</span> to expose
          it on <span className="font-[var(--font-mono)] text-ink">127.0.0.1:4599</span>.
        </p>
      </div>
    );
  }

  return (
    <div className="p-4 space-y-3">
      <p className="text-[11px] leading-relaxed text-ink-dim">
        An agent (Claude Code, etc.) can edit this project through the editor. The
        server is <b>loopback-only</b> and needs the token below.
      </p>
      <CopyRow label="Connect command" value={command} />
      <CopyRow label="Token" value={mcpToken ?? ""} />
      <CopyRow label="URL" value={url} />
      <p className="text-[10px] leading-relaxed text-ink-faint">
        A new token is generated every time the app starts. 53 tools available — see <b>docs/MCP.md</b>.
      </p>
    </div>
  );
}


export function Header() {
  const project = useStore((s) => s.project);
  const dirty = useStore((s) => s.dirty);
  const canUndo = useStore((s) => s.canUndo);
  const canRedo = useStore((s) => s.canRedo);
  const undo = useStore((s) => s.undo);
  const redo = useStore((s) => s.redo);
  const setShowExportDialog = useStore((s) => s.setShowExportDialog);
  const exporting = useStore((s) => s.exporting);
  const saveProject = useStore((s) => s.saveProject);
  const openProject = useStore((s) => s.openProject);
  const newProject = useStore((s) => s.newProject);
  const exportProgress = useStore((s) => s.exportProgress);
  const cancelExport = useStore((s) => s.cancelExport);

  return (
    <header className="flex h-12 shrink-0 items-center gap-3 border-b border-line bg-bg1 px-3">
      <div className="flex items-center gap-2.5">
        <div className="flex h-7 w-7 items-center justify-center rounded-md bg-accent font-[var(--font-display)] text-[13px] font-bold text-bg0">
          UE
        </div>
        <span className="font-[var(--font-display)] text-[15px] font-semibold tracking-tight">
          OpenInCut
        </span>
      </div>

      <div className="mx-2 h-5 w-px bg-line" />

      <div className="flex items-center gap-2 text-ink-dim">
        <span className="max-w-[360px] truncate text-[13px] text-ink">{project.name}</span>
        {dirty && <span className="h-1.5 w-1.5 rounded-full bg-accent" title="Unsaved changes" />}
      </div>

      <div className="flex-1" />

      <button
        className="focus-ring rounded-md px-2.5 py-1.5 text-[12px] text-ink-dim hover:bg-bg3 hover:text-ink"
        onClick={() => void newProject()}
        title="New project (discards the current one if unsaved)"
      >
        New
      </button>
      <button
        className="focus-ring rounded-md px-2.5 py-1.5 text-[12px] text-ink-dim hover:bg-bg3 hover:text-ink"
        onClick={() => void openProject()}
        title="Open project (⌘O)"
      >
        Open…
      </button>
      <button
        className="focus-ring rounded-md px-2.5 py-1.5 text-[12px] text-ink-dim hover:bg-bg3 hover:text-ink"
        onClick={() => void saveProject()}
        title="Save project (⌘S)"
      >
        Save
      </button>

      <div className="mx-1 h-5 w-px bg-line" />

      <button
        className="focus-ring rounded-md px-2.5 py-1.5 text-[12px] text-ink-dim enabled:hover:bg-bg3 enabled:hover:text-ink disabled:opacity-40"
        onClick={() => void undo()}
        disabled={!canUndo}
        title="Undo (⌘Z)"
      >
        ↶ Undo
      </button>
      <button
        className="focus-ring rounded-md px-2.5 py-1.5 text-[12px] text-ink-dim enabled:hover:bg-bg3 enabled:hover:text-ink disabled:opacity-40"
        onClick={() => void redo()}
        disabled={!canRedo}
        title="Redo (⇧⌘Z)"
      >
        ↷ Redo
      </button>

      <div className="mx-1 h-5 w-px bg-line" />

      <McpPill />

      {exporting && (
        <button
          className="focus-ring rounded-md border border-line px-2 py-1.5 text-[12px] text-ink-dim hover:text-danger"
          onClick={() => void cancelExport()}
          title="Cancel the export"
        >
          Cancel
        </button>
      )}
      <button
        className="focus-ring relative overflow-hidden rounded-md bg-accent px-3.5 py-1.5 text-[12px] font-semibold text-bg0 enabled:hover:bg-accent-deep disabled:opacity-80"
        onClick={() => setShowExportDialog(true)}
        disabled={exporting}
        title="Export the sequence to MP4"
      >
        {exporting && (
          <span
            className="absolute inset-y-0 left-0 bg-accent-deep"
            style={{ width: `${Math.round((exportProgress ?? 0) * 100)}%` }}
          />
        )}
        <span className="relative">
          {exporting ? `Exporting ${Math.round((exportProgress ?? 0) * 100)}%` : "Export…"}
        </span>
      </button>
    </header>
  );
}
