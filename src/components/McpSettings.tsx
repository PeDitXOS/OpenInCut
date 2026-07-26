import { useCallback, useEffect, useState } from "react";
import { useStore } from "../state/store";
import { engine } from "../state/store";
import { showToast } from "./Toast";
import {
  type MidiMapping,
  type MidiAction,
  type MidiPreset,
  type MidiDevice,
  requestMidiAccess,
  listInputDevices,
  parseMidiMessage,
  matchMapping,
  dispatchMidi,
  saveMappings,
  savePresets,
  saveDevices,
  sendBacklight,
  sendLedFeedback,
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

type Tab = "mcp" | "hermes" | "midi";

export default function McpSettings({ onClose }: { onClose: () => void }) {
  const [tab, setTab] = useState<Tab>("mcp");
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60">
      <div className="w-[420px] max-h-[80vh] rounded-lg border border-line bg-bg1 shadow-xl overflow-y-auto">
        <div className="flex border-b border-line">
          {([ "mcp", "hermes", "midi" ] as const).map((t) => (
                                <button
                                  key={t}
                                  className={`flex-1 px-3 py-2 text-[11px] font-medium uppercase tracking-wide ${
                                    tab === t ? "text-ink border-b-2 border-accent" : "text-ink-faint hover:text-ink"
                                  }`}
                                  onClick={() => setTab(t)}
                                >
                                  {t === "mcp" ? "MCP" : t === "hermes" ? "Hermes" : "MIDI Controller"}
                                </button>
                              ))}
                    <button className="px-3 text-[11px] text-ink-faint hover:text-ink" onClick={onClose}>✕</button>
                  </div>
        {tab === "mcp" ? <McpPane /> : tab === "hermes" ? <HermesPane /> : <MidiPane />}
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
  const [copiedField, setCopiedField] = useState<string | null>(null);

  // Client-side token management (for future auth use)
  const [clientToken, setClientToken] = useState(() => {
    try { return localStorage.getItem("opencut_hermes_token") ?? ""; } catch { return ""; }
  });
  const [tokenRegeneratedAt, setTokenRegeneratedAt] = useState(() => {
    try { return localStorage.getItem("opencut_hermes_token_ts") ?? ""; } catch { return ""; }
  });

  const copyToClipboard = useCallback((text: string, field: string) => {
    navigator.clipboard.writeText(text).then(() => {
      setCopiedField(field);
      setTimeout(() => setCopiedField(null), 1500);
    }).catch(() => {});
  }, []);

  const regenerateToken = useCallback(() => {
    const newToken = crypto.randomUUID();
    setClientToken(newToken);
    const ts = new Date().toLocaleString();
    setTokenRegeneratedAt(ts);
    try {
      localStorage.setItem("opencut_hermes_token", newToken);
      localStorage.setItem("opencut_hermes_token_ts", ts);
    } catch { /* ignore */ }
  }, []);

  const testConnection = useCallback(async () => {
    setTesting(true);
    setTestResult(null);
    try {
      await engine.mcpListTools();
      setTestResult("Connected successfully!");
      showToast("MCP connected successfully", "success");
    } catch (err) {
      setTestResult(`Connection failed: ${err}`);
      showToast(`MCP connection failed`, "error");
    } finally {
      setTesting(false);
    }
  }, []);

  const mcpUrl = mcpPort ? `http://127.0.0.1:${mcpPort}/mcp` : "";
  const hermesConnectionCmd = mcpPort
    ? `claude mcp add --transport http opencut ${mcpUrl} --header "Authorization: Bearer ${mcpToken ?? ""}"`
    : "";

  return (
    <div className="p-4 space-y-3">
      {/* ── Connection Status ── */}
      <div className="space-y-2">
        <div>
          <label className="mb-1 block text-[10.5px] font-medium uppercase tracking-wide text-ink-faint">Status</label>
          <div className="flex items-center gap-2">
            <span className={`h-2 w-2 rounded-full ${mcpPort ? "bg-green-500" : "bg-red-500"}`} />
            <span className="text-[12px] text-ink">{mcpPort ? `Active on port ${mcpPort}` : "Inactive"}</span>
            <span className={`ml-auto text-[10px] px-1.5 py-0.5 rounded-full ${mcpPort ? "bg-green-900/40 text-green-400" : "bg-red-900/40 text-red-400"}`}>
              {mcpPort ? "Connected" : "Disconnected"}
            </span>
          </div>
        </div>
      </div>

      {mcpPort && (
        <>
          {/* ── Hermes WebUI Connection ── */}
          <div className="border-t border-line pt-2 space-y-2">
            <label className="mb-1 block text-[10.5px] font-medium uppercase tracking-wide text-ink-faint">Hermes WebUI Connection</label>
            <div>
              <label className="mb-0.5 block text-[10px] text-ink-faint">MCP URL</label>
              <div className="flex items-center gap-1">
                <div className="flex-1 rounded-md border border-line bg-bg0 px-2 py-1.5 font-[var(--font-mono)] text-[11px] text-ink truncate">{mcpUrl}</div>
                <button className="rounded border border-line bg-bg2 px-2 py-1 text-[10px] text-ink hover:bg-bg3" onClick={() => copyToClipboard(mcpUrl, "url")}>
                  {copiedField === "url" ? "✓" : "Copy"}
                </button>
              </div>
            </div>
            <div>
              <label className="mb-0.5 block text-[10px] text-ink-faint">Auth Token</label>
              <div className="flex items-center gap-1">
                <div className="flex-1 rounded-md border border-line bg-bg0 px-2 py-1.5 font-[var(--font-mono)] text-[11px] text-ink truncate select-all">{mcpToken}</div>
                <button className="rounded border border-line bg-bg2 px-2 py-1 text-[10px] text-ink hover:bg-bg3" onClick={() => copyToClipboard(mcpToken ?? "", "token")}>
                  {copiedField === "token" ? "✓" : "Copy"}
                </button>
              </div>
            </div>
            <button
              className="w-full rounded-md border border-line bg-bg2 px-3 py-1.5 text-[11px] text-ink hover:bg-bg3"
              onClick={() => copyToClipboard(hermesConnectionCmd, "cmd")}
            >
              {copiedField === "cmd" ? "✓ Copied!" : "Copy Hermes Connection"}
            </button>
          </div>

          {/* ── Token Auth ── */}
          <div className="border-t border-line pt-2 space-y-2">
            <label className="mb-1 block text-[10.5px] font-medium uppercase tracking-wide text-ink-faint">Token Auth</label>
            <div>
              <label className="mb-0.5 block text-[10px] text-ink-faint">Client Token</label>
              <div className="flex items-center gap-1">
                <div className="flex-1 rounded-md border border-line bg-bg0 px-2 py-1.5 font-[var(--font-mono)] text-[11px] text-ink truncate select-all">
                  {clientToken || "No token generated"}
                </div>
                <button className="rounded border border-line bg-bg2 px-2 py-1 text-[10px] text-ink hover:bg-bg3" onClick={() => copyToClipboard(clientToken, "clientToken")}>
                  {copiedField === "clientToken" ? "✓" : "Copy"}
                </button>
              </div>
            </div>
            {tokenRegeneratedAt && (
              <div className="text-[10px] text-ink-faint">Last regenerated: {tokenRegeneratedAt}</div>
            )}
            <button
              className="rounded-md border border-line bg-bg2 px-3 py-1.5 text-[12px] text-ink hover:bg-bg3"
              onClick={regenerateToken}
            >
              Regenerate Token
            </button>
          </div>

          {/* ── Test ── */}
          <div className="border-t border-line pt-2 space-y-2">
            <button className="rounded-md border border-line bg-bg2 px-3 py-1.5 text-[12px] text-ink hover:bg-bg3 disabled:opacity-50" onClick={() => void testConnection()} disabled={testing}>
              {testing ? "Testing..." : "Test Connection"}
            </button>
            {testResult && <div className={`text-[11px] ${testResult.startsWith("Connected") ? "text-green-400" : "text-red-400"}`}>{testResult}</div>}
          </div>
        </>
      )}
      {!mcpPort && (
        <p className="text-[11px] text-ink-dim">
          The MCP server starts automatically when the desktop app launches. Run <code className="font-[var(--font-mono)] text-ink">npx tauri dev</code> to start the app.
        </p>
      )}
    </div>
  );
}

/* ── MIDI Tab ── */
function MidiPane() {
  const mappings = useStore((s) => s.midiMappings);
  const deviceIds = useStore((s) => s.midiDeviceIds);
  const midiDevices = useStore((s) => s.midiDevices);
  const midiPresets = useStore((s) => s.midiPresets);
  const [availableInputs, setAvailableInputs] = useState<MIDIInput[]>([]);
  const [midiAvailable, setMidiAvailable] = useState<boolean | null>(null);
  const [lastEvent, setLastEvent] = useState<string | null>(null);
  const [presets, setPresets] = useState<MidiPreset[]>(midiPresets);
  const [newPresetName, setNewPresetName] = useState("");
  const [backlightCc, setBacklightCc] = useState(() => {
    try { return parseInt(localStorage.getItem("opencut_midi_backlight_cc") ?? "0", 10); } catch { return 0; }
  });
  const [backlightChannel, setBacklightChannel] = useState(() => {
    try { return parseInt(localStorage.getItem("opencut_midi_backlight_ch") ?? "0", 10); } catch { return 0; }
  });
  const [backlightValue, setBacklightValue] = useState(() => {
    try { return parseInt(localStorage.getItem("opencut_midi_backlight_val") ?? "127", 10); } catch { return 127; }
  });

  const setMappings = (updater: MidiMapping[] | ((prev: MidiMapping[]) => MidiMapping[])) => {
    useStore.setState((st) => {
      const next = typeof updater === "function" ? updater(st.midiMappings) : updater;
      saveMappings(next);
      return { midiMappings: next };
    });
  };

  const setDeviceIds = (ids: string[]) => useStore.setState({ midiDeviceIds: ids });
  const setMidiDevices = (devices: MidiDevice[]) => {
    useStore.setState({ midiDevices: devices });
    saveDevices(devices);
  };

  const connectMidi = useCallback(async () => {
    const access = await requestMidiAccess();
    if (!access) { setMidiAvailable(false); return; }
    setMidiAvailable(true);
    setAvailableInputs(listInputDevices(access));
    access.onstatechange = () => setAvailableInputs(listInputDevices(access));
  }, []);

  useEffect(() => { void connectMidi(); }, [connectMidi]);

  // Sync devices with store
  useEffect(() => {
    // Filter out disconnected devices
    const availableIds = new Set(availableInputs.map(d => d.id));
    const filteredDevices = midiDevices.filter(d => availableIds.has(d.id));
    // Add new available devices
    for (const input of availableInputs) {
      if (!filteredDevices.find(d => d.id === input.id)) {
        filteredDevices.push({ id: input.id, name: String(input.name ?? input.id), manufacturer: input.manufacturer ?? "", enabled: true });
      }
    }
    setMidiDevices(filteredDevices);
  }, [availableInputs, midiDevices, setMidiDevices]);

  // Attach input listeners for all enabled devices
  useEffect(() => {
    if (!midiAvailable || !deviceIds.length) return;
    let cleanup: (() => void) | undefined;
    void requestMidiAccess().then((access) => {
      if (!access) return;
      const cleanupFns: (() => void)[] = [];
      const currentDevices = useStore.getState().midiDevices;

      for (const deviceId of deviceIds) {
        const deviceConfig = currentDevices.find(d => d.id === deviceId && d.enabled);
        if (!deviceConfig) continue;
        const input = access.inputs.get(deviceId);
        if (!input) continue;
        input.onmidimessage = (e: MIDIMessageEvent) => {
          if (!e.data) return;
          const msg = parseMidiMessage(e.data);
          if (!msg) return;
          const typeStr = msg.command === 0xb0 ? "CC" : msg.command === 0x90 ? "Note" : null;
          if (!typeStr) return;
          setLastEvent(`${typeStr} ch${msg.channel + 1} #${msg.data1} v=${msg.data2}`);
          const mappings = useStore.getState().midiMappings;
          const devices = useStore.getState().midiDevices;
          for (const m of mappings) {
            if (matchMapping(msg, m)) {
              dispatchMidi(m.action, msg.data2, () => useStore.getState());
              void sendLedFeedback(access, devices, m, msg.data2);
              break;
            }
          }
        };
        cleanupFns.push(() => { input.onmidimessage = null; });
      }
      cleanup = () => { cleanupFns.forEach(fn => fn()); };
    });
    return () => { cleanup?.(); };
  }, [deviceIds, midiAvailable]);

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
    const state = useStore.getState();
    const next = [...presets.filter((p) => p.name !== name), { 
      name, 
      mappings: state.midiMappings, 
      devices: state.midiDevices,
      backlightCc: backlightCc || undefined,
      backlightChannel: backlightChannel || undefined,
      backlightValue: backlightValue || undefined,
    }];
    setPresets(next);
    useStore.setState({ midiPresets: next });
    savePresets(next);
    setNewPresetName("");
  };

  const loadPreset = (name: string) => {
    const preset = presets.find((p) => p.name === name);
    if (preset) {
      setMappings(preset.mappings);
      setMidiDevices(preset.devices);
      setDeviceIds(preset.devices.filter(d => d.enabled).map(d => d.id));
      if (preset.backlightCc !== undefined) setBacklightCc(preset.backlightCc);
      if (preset.backlightChannel !== undefined) setBacklightChannel(preset.backlightChannel);
      if (preset.backlightValue !== undefined) setBacklightValue(preset.backlightValue);
    }
  };

  const deletePreset = (name: string) => {
    const next = presets.filter((p) => p.name !== name);
    setPresets(next);
    useStore.setState({ midiPresets: next });
    savePresets(next);
  };

  const applyBacklight = async () => {
    const access = await requestMidiAccess();
    if (!access) return;
    const devices = useStore.getState().midiDevices;
    await sendBacklight(access, devices, backlightCc, backlightValue, backlightChannel);
  };

  const toggleDevice = (id: string) => {
    setMidiDevices(midiDevices.map(d => d.id === id ? { ...d, enabled: !d.enabled } : d));
    const newIds = deviceIds.includes(id) 
      ? deviceIds.filter(d => d !== id)
      : [...deviceIds, id];
    setDeviceIds(newIds);
  };

  if (midiAvailable === null) return <div className="p-4 text-[11px] text-ink-dim">Checking MIDI availability…</div>;
  if (!midiAvailable) return <div className="p-4 text-[11px] text-ink-dim">Web MIDI API not available. Requires HTTPS or localhost.</div>;

  const actionKeys = Object.keys(ACTION_LABELS);

  return (
    <div className="p-4 space-y-3 text-[12px]">
      {/* Device List with multi-select */}
      <div>
        <div className="flex items-center justify-between mb-2">
          <label className="text-[10.5px] font-medium uppercase tracking-wide text-ink-faint">MIDI Devices</label>
          <button className="btn-secondary text-[10px]" onClick={() => void connectMidi()} disabled={!midiAvailable}>
            Refresh
          </button>
        </div>
        {availableInputs.length === 0 ? (
          <p className="text-[11px] text-ink-faint">No MIDI devices found. Connect a controller and click Refresh.</p>
        ) : (
          <div className="space-y-1 max-h-40 overflow-y-auto">
            {availableInputs.map((input) => {
              const config = midiDevices.find(d => d.id === input.id);
              const isEnabled = config?.enabled ?? false;
              const isSelected = deviceIds.includes(input.id);
              return (
                <div key={input.id} className="flex items-center gap-2 p-2 bg-bg2 rounded">
                  <input
                    type="checkbox"
                    checked={isEnabled}
                    onChange={() => toggleDevice(input.id)}
                    className="w-4 h-4"
                  />
                  <span className={`status-dot ${isEnabled ? "online" : "offline"}`} />
                  <span className="text-[12px] text-ink flex-1 truncate">{input.name ?? input.id}</span>
                  {input.manufacturer && <span className="text-[10px] text-ink-faint">{input.manufacturer}</span>}
                  {isSelected && <span className="text-[10px] text-accent">● Active</span>}
                </div>
              );
            })}
          </div>
        )}
      </div>

      {/* Active device indicator */}
      <div className="flex items-center gap-2 p-2 bg-bg2 rounded">
        <span className={`h-2 w-2 rounded-full ${deviceIds.length > 0 ? "bg-green-500 animate-pulse" : "bg-red-500"}`} />
        <span className="text-[11px] text-ink-faint">
          {deviceIds.length > 0 
            ? `Listening on ${deviceIds.length} device(s)` 
            : "No device enabled"}
        </span>
        {lastEvent && <span className="ml-auto text-[10px] font-[var(--font-mono)] text-accent">{lastEvent}</span>}
      </div>

      {/* Mappings */}
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

      {/* Backlight / LED Control */}
      <div className="border-t border-line pt-2 space-y-2">
        <label className="mb-1 block text-[10.5px] font-medium uppercase tracking-wide text-ink-faint">Backlight / LED Control</label>
        <div className="grid grid-cols-3 gap-2">
          <div>
            <label className="mb-0.5 block text-[10px] text-ink-faint">CC Number</label>
            <input type="number" min={0} max={127} className="input" value={backlightCc} onChange={(e) => { const v = Number(e.target.value); setBacklightCc(v); localStorage.setItem("opencut_midi_backlight_cc", String(v)); }} />
          </div>
          <div>
            <label className="mb-0.5 block text-[10px] text-ink-faint">Channel</label>
            <input type="number" min={0} max={15} className="input" value={backlightChannel} onChange={(e) => { const v = Number(e.target.value); setBacklightChannel(v); localStorage.setItem("opencut_midi_backlight_ch", String(v)); }} />
          </div>
          <div>
            <label className="mb-0.5 block text-[10px] text-ink-faint">Value (0-127)</label>
            <input type="number" min={0} max={127} className="input" value={backlightValue} onChange={(e) => { const v = Number(e.target.value); setBacklightValue(v); localStorage.setItem("opencut_midi_backlight_val", String(v)); }} />
          </div>
        </div>
        <button className="btn-secondary text-[11px]" onClick={applyBacklight}>Send Backlight</button>
      </div>

      {/* Presets */}
      <div className="border-t border-line pt-2">
        <label className="mb-1 block text-[10.5px] font-medium uppercase tracking-wide text-ink-faint">Presets</label>
        <div className="flex gap-1">
          <input className="flex-1 rounded border border-line bg-bg0 px-2 py-1 text-[11px] text-ink" placeholder="Preset name" value={newPresetName} onChange={(e) => setNewPresetName(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter") saveCurrentAsPreset(); }} />
          <button className="rounded border border-line bg-bg2 px-2 py-1 text-[10px] text-ink hover:bg-bg3" onClick={saveCurrentAsPreset}>Save</button>
        </div>
        <div className="mt-1 space-y-0.5 max-h-32 overflow-y-auto">
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

/* ── Hermes Agent Tab ── */
function HermesPane() {
  const [gatewayUrl, setGatewayUrl] = useState(() => {
    try { return localStorage.getItem("opencut_hermes_gateway_url") ?? "http://localhost:8080"; } catch { return "http://localhost:8080"; }
  });
  const [apiKey, setApiKey] = useState(() => {
    try { return localStorage.getItem("opencut_hermes_api_key") ?? ""; } catch { return ""; }
  });
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<string | null>(null);

  const testConnection = useCallback(async () => {
    setTesting(true);
    setTestResult(null);
    try {
      const resp = await fetch(`${gatewayUrl}/v1/models`, {
        method: "GET",
        headers: { Authorization: `Bearer ${apiKey}` },
      });
      if (resp.ok) {
        const data = await resp.json();
        const models = data.data?.map((m: { id: string }) => m.id).join(", ") ?? "OK";
        setTestResult(`Connected! Models: ${models}`);
        showToast("Hermes Gateway connected", "success");
      } else {
        setTestResult(`Failed: ${resp.status}`);
        showToast("Hermes connection failed", "error");
      }
    } catch (err) {
      setTestResult(`Error: ${err instanceof Error ? err.message : String(err)}`);
      showToast("Hermes connection error", "error");
    } finally {
      setTesting(false);
    }
  }, [gatewayUrl, apiKey]);

  const saveSettings = useCallback(() => {
    try {
      localStorage.setItem("opencut_hermes_gateway_url", gatewayUrl);
      localStorage.setItem("opencut_hermes_api_key", apiKey);
      showToast("Hermes settings saved", "success");
    } catch { /* ignore */ }
  }, [gatewayUrl, apiKey]);

  return (
    <div className="p-4 space-y-3">
      <div className="space-y-2">
        <label className="mb-1 block text-[10.5px] font-medium uppercase tracking-wide text-ink-faint">Hermes Gateway URL</label>
        <input
          type="text"
          className="w-full rounded-md border border-line bg-bg0 px-2 py-1.5 text-[11px] text-ink"
          value={gatewayUrl}
          onChange={(e) => setGatewayUrl(e.target.value)}
          placeholder="http://localhost:8080"
        />
      </div>

      <div className="space-y-2">
        <label className="mb-1 block text-[10.5px] font-medium uppercase tracking-wide text-ink-faint">API Key</label>
        <input
          type="password"
          className="w-full rounded-md border border-line bg-bg0 px-2 py-1.5 text-[11px] text-ink"
          value={apiKey}
          onChange={(e) => setApiKey(e.target.value)}
          placeholder="Enter Hermes API key"
        />
      </div>

      <div className="flex gap-2">
        <button
          className="flex-1 rounded-md border border-line bg-bg2 px-3 py-1.5 text-[12px] text-ink hover:bg-bg3 disabled:opacity-50"
          onClick={() => void testConnection()}
          disabled={testing}
        >
          {testing ? "Testing..." : "Test Connection"}
        </button>
        <button
          className="flex-1 rounded-md bg-accent px-3 py-1.5 text-[12px] font-medium text-bg0 hover:bg-accent/80"
          onClick={saveSettings}
        >
          Save
        </button>
      </div>

      {testResult && (
        <div className={`text-[11px] ${testResult.startsWith("Connected") ? "text-green-400" : "text-red-400"}`}>
          {testResult}
        </div>
      )}

      <div className="border-t border-line pt-2 text-[10px] text-ink-faint space-y-1">
        <p>Configure your Hermes Agent Gateway URL and API key above.</p>
        <p>Then add it as an AI endpoint in the AI Assistant settings tab.</p>
        <p className="font-[var(--font-mono)]">URL: {gatewayUrl}/v1/chat/completions</p>
      </div>
    </div>
  );
}

