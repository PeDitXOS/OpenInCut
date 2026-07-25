import { useCallback, useEffect, useState } from "react";
import { useStore } from "../state/store";
import { engine } from "../state/store";
import {
  type MidiMapping,
  type MidiAction,
  type MidiPreset,
  requestMidiAccess,
  listInputDevices,
  parseMidiMessage,
  matchMapping,
  saveMappings,
  loadPresets,
  savePresets,
  DEFAULT_MAPPINGS,
} from "../lib/midi";

const ACTION_LABELS: Record<string, string> = {
  togglePlay: "Play / Pause",
  split: "Split at Playhead",
  delete: "Delete Selection",
  undo: "Undo",
  redo: "Redo",
  seek: "Scrub (CC 0→127)",
  shuttle: "Shuttle (CC)",
  "tool.select": "Tool: Select",
  "tool.blade": "Tool: Blade",
  "tool.trim": "Tool: Trim",
  "tool.position": "Tool: Position",
  "tool.hand": "Tool: Hand",
  "tool.zoom": "Tool: Zoom",
};

function actionKey(a: MidiAction): string {
  return a.kind === "tool" ? `tool.${a.tool}` : a.kind;
}

function parseActionKey(key: string): MidiAction {
  if (key === "togglePlay") return { kind: "togglePlay" };
  if (key === "split") return { kind: "split" };
  if (key === "delete") return { kind: "delete" };
  if (key === "undo") return { kind: "undo" };
  if (key === "redo") return { kind: "redo" };
  if (key === "seek") return { kind: "seek" };
  if (key === "shuttle") return { kind: "shuttle", direction: 1 };
  if (key.startsWith("tool.")) {
    const tool = key.slice(5) as MidiAction extends { tool: infer T } ? T : never;
    return { kind: "tool", tool: tool as "select" };
  }
  return { kind: "togglePlay" };
}

type Tab = "mcp" | "midi";

export default function McpSettings({ onClose }: { onClose: () => void }) {
  const [tab, setTab] = useState<Tab>("mcp");
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60">
      <div className="w-[420px] max-h-[80vh] rounded-lg border border-line bg-bg1 shadow-xl overflow-y-auto">
        <div className="flex border-b border-line">
          {(["mcp", "midi"] as const).map((t) => (
            <button
              key={t}
              className={`flex-1 px-3 py-2 text-[11px] font-medium uppercase tracking-wide ${
                tab === t ? "text-ink border-b-2 border-accent" : "text-ink-faint hover:text-ink"
              }`}
              onClick={() => setTab(t)}
            >
              {t === "mcp" ? "MCP" : "MIDI Controller"}
            </button>
          ))}
          <button className="px-3 text-[11px] text-ink-faint hover:text-ink" onClick={onClose}>✕</button>
        </div>
        {tab === "mcp" ? <McpPane /> : <MidiPane />}
      </div>
    </div>
  );
}

/* ── MCP Tab (existing) ── */
function McpPane() {
  const mcpPort = useStore((s) => s.mcpPort);
  const mcpToken = useStore((s) => s.mcpToken);
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<string | null>(null);

  const testConnection = useCallback(async () => {
    setTesting(true);
    setTestResult(null);
    try {
      await engine.mcpListTools();
      setTestResult("Connected successfully!");
    } catch (err) {
      setTestResult(`Connection failed: ${err}`);
    } finally {
      setTesting(false);
    }
  }, []);

  return (
    <div className="p-4 space-y-3">
      <div className="space-y-3">
        <div>
          <label className="mb-1 block text-[10.5px] font-medium uppercase tracking-wide text-ink-faint">Status</label>
          <div className="flex items-center gap-2">
            <span className={`h-2 w-2 rounded-full ${mcpPort ? "bg-green-500" : "bg-red-500"}`} />
            <span className="text-[12px] text-ink">{mcpPort ? `Active on port ${mcpPort}` : "Inactive"}</span>
          </div>
        </div>
        {mcpPort && (
          <>
            <div>
              <label className="mb-1 block text-[10.5px] font-medium uppercase tracking-wide text-ink-faint">URL</label>
              <div className="rounded-md border border-line bg-bg0 px-2 py-1.5 font-[var(--font-mono)] text-[11px] text-ink">http://127.0.0.1:{mcpPort}/mcp</div>
            </div>
            <div>
              <label className="mb-1 block text-[10.5px] font-medium uppercase tracking-wide text-ink-faint">Token</label>
              <div className="rounded-md border border-line bg-bg0 px-2 py-1.5 font-[var(--font-mono)] text-[11px] text-ink select-all break-all">{mcpToken}</div>
            </div>
            <button className="rounded-md border border-line bg-bg2 px-3 py-1.5 text-[12px] text-ink hover:bg-bg3 disabled:opacity-50" onClick={() => void testConnection()} disabled={testing}>
              {testing ? "Testing..." : "Test Connection"}
            </button>
            {testResult && <div className={`text-[11px] ${testResult.startsWith("Connected") ? "text-green-400" : "text-red-400"}`}>{testResult}</div>}
          </>
        )}
        {!mcpPort && (
          <p className="text-[11px] text-ink-dim">
            The MCP server starts automatically when the desktop app launches. Run <code className="font-[var(--font-mono)] text-ink">npx tauri dev</code> to start the app.
          </p>
        )}
      </div>
    </div>
  );
}

/* ── MIDI Tab ── */
function MidiPane() {
  const mappings = useStore((s) => s.midiMappings);
  const deviceId = useStore((s) => s.midiDeviceId);
  const [devices, setDevices] = useState<MIDIInput[]>([]);
  const [midiAvailable, setMidiAvailable] = useState<boolean | null>(null);
  const [lastEvent, setLastEvent] = useState<string | null>(null);
  const [presets, setPresets] = useState<MidiPreset[]>(loadPresets);
  const [newPresetName, setNewPresetName] = useState("");

  const setMappings = (updater: MidiMapping[] | ((prev: MidiMapping[]) => MidiMapping[])) => {
    useStore.setState((st) => {
      const next = typeof updater === "function" ? updater(st.midiMappings) : updater;
      saveMappings(next);
      return { midiMappings: next };
    });
  };

  const setDeviceId = (id: string | null) => useStore.setState({ midiDeviceId: id });

  const connectMidi = useCallback(async () => {
    const access = await requestMidiAccess();
    if (!access) { setMidiAvailable(false); return; }
    setMidiAvailable(true);
    setDevices(listInputDevices(access));
    access.onstatechange = () => setDevices(listInputDevices(access));
  }, []);

  useEffect(() => { void connectMidi(); }, [connectMidi]);

  // Attach input listener
  useEffect(() => {
    if (!deviceId || !midiAvailable) return;
    let cleanup: (() => void) | undefined;
    void requestMidiAccess().then((access) => {
      if (!access) return;
      const input = access.inputs.get(deviceId);
      if (!input) return;
      input.onmidimessage = (e: MIDIMessageEvent) => {
        if (!e.data) return;
        const msg = parseMidiMessage(e.data);
        if (!msg) return;
        const typeStr = msg.command === 0xb0 ? "CC" : msg.command === 0x90 ? "Note" : null;
        if (!typeStr) return;
        setLastEvent(`${typeStr} ch${msg.channel + 1} #${msg.data1} v=${msg.data2}`);
        const currentMappings = useStore.getState().midiMappings;
        for (const m of currentMappings) {
          if (matchMapping(msg, m)) {
            dispatchMidiAction(m.action, msg.data2);
            break;
          }
        }
      };
      cleanup = () => { input.onmidimessage = null; };
    });
    return () => { cleanup?.(); };
  }, [deviceId, midiAvailable]);

  const updateMapping = (index: number, partial: Partial<MidiMapping>) => {
    setMappings((prev) => {
      const next = [...prev];
      next[index] = { ...next[index], ...partial };
      return next;
    });
  };

  const removeMapping = (index: number) => setMappings((prev) => prev.filter((_, i) => i !== index));
  const addMapping = () => setMappings((prev) => [...prev, { type: "cc", channel: -1, number: 0, action: { kind: "togglePlay" } }]);

  const saveCurrentAsPreset = () => {
    const name = newPresetName.trim();
    if (!name) return;
    const next = [...presets.filter((p) => p.name !== name), { name, mappings: useStore.getState().midiMappings }];
    setPresets(next);
    savePresets(next);
    setNewPresetName("");
  };

  const loadPreset = (name: string) => {
    const preset = presets.find((p) => p.name === name);
    if (preset) setMappings(preset.mappings);
  };

  const deletePreset = (name: string) => {
    const next = presets.filter((p) => p.name !== name);
    setPresets(next);
    savePresets(next);
  };

  if (midiAvailable === null) return <div className="p-4 text-[11px] text-ink-dim">Checking MIDI availability…</div>;
  if (!midiAvailable) return <div className="p-4 text-[11px] text-ink-dim">Web MIDI API not available. Requires HTTPS or localhost.</div>;

  const actionKeys = Object.keys(ACTION_LABELS);

  return (
    <div className="p-4 space-y-3 text-[12px]">
      <div>
        <label className="mb-1 block text-[10.5px] font-medium uppercase tracking-wide text-ink-faint">Input Device</label>
        <select className="w-full rounded-md border border-line bg-bg0 px-2 py-1.5 text-[11px] text-ink" value={deviceId ?? ""} onChange={(e) => setDeviceId(e.target.value || null)}>
          <option value="">None selected</option>
          {devices.map((d) => <option key={d.id} value={d.id}>{d.name ?? d.id}</option>)}
        </select>
      </div>

      <div className="flex items-center gap-2">
        <span className={`h-2 w-2 rounded-full ${deviceId ? "bg-green-500 animate-pulse" : "bg-red-500"}`} />
        <span className="text-[11px] text-ink-faint">{deviceId ? "Listening" : "No device"}</span>
        {lastEvent && <span className="ml-auto text-[10px] font-[var(--font-mono)] text-accent">{lastEvent}</span>}
      </div>

      <div>
        <div className="flex items-center justify-between mb-2">
          <label className="text-[10.5px] font-medium uppercase tracking-wide text-ink-faint">Mappings</label>
          <button className="rounded border border-line bg-bg2 px-2 py-0.5 text-[10px] text-ink hover:bg-bg3" onClick={addMapping}>+ Add</button>
        </div>
        <div className="space-y-1.5 max-h-[200px] overflow-y-auto">
          {mappings.map((m, i) => (
            <div key={i} className="flex items-center gap-1 rounded border border-line bg-bg0 p-1.5">
              <select className="w-14 rounded border border-line bg-bg1 px-1 py-0.5 text-[10px] text-ink" value={m.type} onChange={(e) => updateMapping(i, { type: e.target.value as "cc" | "note" })}>
                <option value="cc">CC</option>
                <option value="note">Note</option>
              </select>
              <select className="w-16 rounded border border-line bg-bg1 px-1 py-0.5 text-[10px] text-ink" value={m.channel} onChange={(e) => updateMapping(i, { channel: Number(e.target.value) })}>
                <option value={-1}>Any</option>
                {Array.from({ length: 16 }, (_, ch) => <option key={ch} value={ch}>Ch{ch + 1}</option>)}
              </select>
              <input type="number" min={0} max={127} className="w-12 rounded border border-line bg-bg1 px-1 py-0.5 text-[10px] text-ink font-[var(--font-mono)]" value={m.number} onChange={(e) => updateMapping(i, { number: Math.min(127, Math.max(0, Number(e.target.value))) })} />
              <select className="flex-1 rounded border border-line bg-bg1 px-1 py-0.5 text-[10px] text-ink" value={actionKey(m.action)} onChange={(e) => updateMapping(i, { action: parseActionKey(e.target.value) })}>
                {actionKeys.map((k) => <option key={k} value={k}>{ACTION_LABELS[k]}</option>)}
              </select>
              <button className="px-1 text-[10px] text-red-400 hover:text-red-300" onClick={() => removeMapping(i)}>✕</button>
            </div>
          ))}
        </div>
        {mappings.length === 0 && <button className="text-[11px] text-accent hover:underline" onClick={() => setMappings(DEFAULT_MAPPINGS)}>Load default mappings</button>}
      </div>

      <div className="border-t border-line pt-2">
        <label className="mb-1 block text-[10.5px] font-medium uppercase tracking-wide text-ink-faint">Presets</label>
        <div className="flex gap-1">
          <input className="flex-1 rounded border border-line bg-bg0 px-2 py-1 text-[11px] text-ink" placeholder="Preset name" value={newPresetName} onChange={(e) => setNewPresetName(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter") saveCurrentAsPreset(); }} />
          <button className="rounded border border-line bg-bg2 px-2 py-1 text-[10px] text-ink hover:bg-bg3" onClick={saveCurrentAsPreset}>Save</button>
        </div>
        <div className="mt-1 space-y-0.5">
          {presets.map((p) => (
            <div key={p.name} className="flex items-center justify-between rounded px-2 py-1 hover:bg-bg2">
              <button className="text-[11px] text-ink hover:underline" onClick={() => loadPreset(p.name)}>{p.name}</button>
              <button className="text-[10px] text-red-400 hover:text-red-300" onClick={() => deletePreset(p.name)}>✕</button>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

function dispatchMidiAction(action: MidiAction, value: number) {
  const s = useStore.getState();
  switch (action.kind) {
    case "togglePlay": s.togglePlay(); break;
    case "split": void s.splitAtPlayhead(); break;
    case "delete": void s.deleteSelection(false); break;
    case "undo": void s.undo(); break;
    case "redo": void s.redo(); break;
    case "seek": {
      const viewLen = Math.max(1, s.pxPerSec * 10);
      const target = s.viewStartUs + (value / 127) * viewLen * 1_000_000 / s.pxPerSec;
      s.seek(target);
      break;
    }
    case "shuttle": {
      const dir = value > 64 ? action.direction : value < 63 ? -action.direction : 0;
      s.shuttle(dir as -1 | 0 | 1);
      break;
    }
    case "tool":
      s.setTool(action.tool);
      break;
  }
}
