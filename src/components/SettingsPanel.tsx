
import { useState, useCallback, useEffect } from "react";
import { useStore } from "../state/store";
import { engine } from "../state/store";

type Tab = "ai" | "mcp" | "language" | "shortcuts" | "midi" | "export";

export function getSetting(key: string, fallback: string): string {
  try { return localStorage.getItem(`opencut_$` + key + ``) ?? fallback; } catch { return fallback; }
}
function setSetting(key: string, value: string) {
  try { localStorage.setItem(`opencut_$` + key + ``, value); } catch { /* ignore */ }
}

type Endpoint = { name: string; url: string; key: string; model: string };
export function getActiveEndpoint(): Endpoint {
  try {
    const list: Endpoint[] = JSON.parse(localStorage.getItem("opencut_ai_endpoints") ?? "[]");
    const idx = parseInt(localStorage.getItem("opencut_ai_active_endpoint") ?? "0", 10);
    if (list.length) return list[Math.min(idx, list.length - 1)];
  } catch {}
  return { name: "Default", url: localStorage.getItem("opencut_ai_url") ?? "", key: localStorage.getItem("opencut_ai_key") ?? "", model: localStorage.getItem("opencut_ai_model") ?? "" };
}
export function getSystemPrompt(): string {
  try { return localStorage.getItem("opencut_ai_system_prompt") ?? ""; } catch { return ""; }
}

export default function SettingsPanel({ onClose }: { onClose: () => void }) {
  const [tab, setTab] = useState<Tab>("ai");
  const mcpPort = useStore((s) => s.mcpPort);
  const mcpToken = useStore((s) => s.mcpToken);

  const tabs: { id: Tab; label: string; icon: string }[] = [
    { id: "ai", label: "AI Assistant", icon: "🤖" },
    { id: "mcp", label: "MCP Server", icon: "🔌" },
    { id: "language", label: "Language", icon: "🌐" },
    { id: "shortcuts", label: "Shortcuts", icon: "⌨" },
    { id: "midi", label: "MIDI Controller", icon: "🎹" },
    { id: "export", label: "Export", icon: "📤" },
  ];

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60">
      <div className="card w-[600px] max-h-[80vh] flex flex-col shadow-2xl">
        {/* Header */}
        <div className="flex items-center justify-between border-b border-line px-4 py-3">
          <h2 className="text-[14px] font-semibold text-ink">Settings</h2>
          <button onClick={onClose} className="text-ink-faint hover:text-ink text-[16px]">&times;</button>
        </div>

        <div className="flex min-h-0 flex-1">
          {/* Sidebar tabs */}
          <div className="w-[160px] border-r border-line p-2 space-y-1">
            {tabs.map((t) => (
              <button
                key={t.id}
                className={`w-full flex items-center gap-2 px-3 py-2 rounded-md text-[12px] text-left transition-colors ${
                  tab === t.id ? "bg-bg3 text-ink" : "text-ink-dim hover:text-ink hover:bg-bg2"
                }`}
                onClick={() => setTab(t.id)}
              >
                <span>{t.icon}</span>
                <span>{t.label}</span>
              </button>
            ))}
          </div>

          {/* Content */}
          <div className="flex-1 overflow-auto p-4">
            {tab === "ai" && <AISettings />}
            {tab === "mcp" && <MCPSettings port={mcpPort} token={mcpToken} />}
            {tab === "language" && <LanguageSettings />}
            {tab === "shortcuts" && <ShortcutSettings />}
            {tab === "midi" && <MIDISettings />}
            {tab === "export" && <ExportSettings />}
          </div>
        </div>
      </div>
    </div>
  );
}

function AISettings() {
  const [endpoints, setEndpoints] = useState<Endpoint[]>(() => {
    try { return JSON.parse(localStorage.getItem("opencut_ai_endpoints") ?? "[]"); } catch { return []; }
  });
  const [activeIdx, setActiveIdx] = useState(() => {
    try { return Math.max(0, parseInt(localStorage.getItem("opencut_ai_active_endpoint") ?? "0", 10)); } catch { return 0; }
  });
  const [editIdx, setEditIdx] = useState<number | null>(null);
  const [draft, setDraft] = useState<Endpoint>({ name: "", url: "", key: "", model: "" });
  const [systemPrompt, setSystemPrompt] = useState(() => getSystemPrompt());
  const [enabled, setEnabled] = useState(() => getSetting("ai_enabled", "") === "true");
  const [saved, setSaved] = useState(false);

  const presets: Endpoint[] = [
    { name: "9Router", url: "https://9router.peditx.ir/v1", key: "", model: "hermes-3-llama-3.1-70b" },
    { name: "Ollama (Local)", url: "http://localhost:11434/v1", key: "", model: "llama3.1" },
    { name: "OpenAI", url: "https://api.openai.com/v1", key: "", model: "gpt-4o" },
    { name: "Anthropic", url: "https://api.anthropic.com/v1", key: "", model: "claude-3-5-sonnet" },
  ];

  const saveAll = useCallback(() => {
    localStorage.setItem("opencut_ai_endpoints", JSON.stringify(endpoints));
    localStorage.setItem("opencut_ai_active_endpoint", String(activeIdx));
    setSetting("ai_system_prompt", systemPrompt);
    setSetting("ai_enabled", String(enabled));
    setSaved(true);
    setTimeout(() => setSaved(false), 2000);
  }, [endpoints, activeIdx, systemPrompt, enabled]);

  const startEdit = (idx: number | null) => {
    if (idx === null) {
      const ep = { name: "", url: "", key: "", model: "" };
      setEndpoints((prev) => [...prev, ep]);
      setDraft(ep);
      // editIdx will be set after state update via effect or directly
      setEditIdx(endpoints.length);
    } else {
      setEditIdx(idx);
      setDraft({ ...endpoints[idx] });
    }
  };

  const saveEdit = () => {
    if (editIdx === null) return;
    setEndpoints((prev) => {
      const next = [...prev];
      next[editIdx] = { ...draft };
      return next;
    });
    setEditIdx(null);
  };

  const removeEndpoint = (idx: number) => {
    setEndpoints((prev) => prev.filter((_, i) => i !== idx));
    if (activeIdx >= endpoints.length - 1) setActiveIdx(Math.max(0, endpoints.length - 2));
    if (editIdx === idx) setEditIdx(null);
  };

  return (
    <div className="space-y-4">
      <h3 className="panel-label">Agent Configuration</h3>

      <div className="flex items-center gap-2">
        <input type="checkbox" id="ai-enabled" checked={enabled}
          onChange={(e) => setEnabled(e.target.checked)} className="w-4 h-4" />
        <label htmlFor="ai-enabled" className="text-[12px] text-ink">Enable AI Assistant</label>
      </div>

      <div>
        <label className="mb-1 block text-[11px] text-ink-faint">System Prompt (optional)</label>
        <textarea
          className="input min-h-[80px] w-full resize-y font-mono text-[11px]"
          value={systemPrompt}
          onChange={(e) => setSystemPrompt(e.target.value)}
          placeholder="You are a helpful video editing assistant..."
        />
      </div>

      <h3 className="panel-label">Endpoints</h3>

      <div className="flex flex-wrap gap-1">
        {presets.map((p, i) => (
          <button key={i} className="btn-secondary text-[10px]"
            onClick={() => setEndpoints((prev) => [...prev, { ...p }])}>
            + {p.name}
          </button>
        ))}
      </div>

      {endpoints.length === 0 && (
        <p className="text-[11px] text-ink-faint">No endpoints saved. Add one above or create custom.</p>
      )}

      <div className="space-y-1">
        {endpoints.map((ep, i) => (
          <div key={i} className="flex items-center gap-2 p-2 rounded-md bg-bg2">
            <input type="radio" name="active-ep" checked={activeIdx === i}
              onChange={() => setActiveIdx(i)} className="w-3 h-3" />
            <div className="flex-1 min-w-0">
              <div className="text-[12px] text-ink truncate">{ep.name || "Unnamed"}</div>
              <div className="text-[10px] text-ink-faint truncate">{ep.url} · {ep.model}</div>
            </div>
            <button className="text-[10px] text-ink-faint hover:text-ink" onClick={() => startEdit(i)}>Edit</button>
            <button className="text-[10px] text-red-400 hover:text-red-300" onClick={() => removeEndpoint(i)}>×</button>
          </div>
        ))}
      </div>

      <button className="btn-secondary text-[11px]" onClick={() => startEdit(null)}>+ Custom Endpoint</button>

      {editIdx !== null && (
        <div className="space-y-2 rounded-md border border-line p-3">
          <div>
            <label className="mb-1 block text-[11px] text-ink-faint">Name</label>
            <input className="input" value={draft.name} onChange={(e) => setDraft((d) => ({ ...d, name: e.target.value }))} placeholder="My Backend" />
          </div>
          <div>
            <label className="mb-1 block text-[11px] text-ink-faint">API URL</label>
            <input className="input" value={draft.url} onChange={(e) => setDraft((d) => ({ ...d, url: e.target.value }))} placeholder="https://api.example.com/v1" />
          </div>
          <div>
            <label className="mb-1 block text-[11px] text-ink-faint">API Key</label>
            <input type="password" className="input" value={draft.key} onChange={(e) => setDraft((d) => ({ ...d, key: e.target.value }))} placeholder="sk-..." />
          </div>
          <div>
            <label className="mb-1 block text-[11px] text-ink-faint">Model</label>
            <input className="input" value={draft.model} onChange={(e) => setDraft((d) => ({ ...d, model: e.target.value }))} placeholder="gpt-4o" />
          </div>
          <div className="flex gap-2">
            <button className="btn-primary" onClick={saveEdit}>Save Endpoint</button>
            <button className="btn-secondary" onClick={() => setEditIdx(null)}>Cancel</button>
          </div>
        </div>
      )}

      <div className="flex items-center gap-2 pt-2">
        <button className="btn-primary" onClick={saveAll}>
          {saved ? "Saved!" : "Save Settings"}
        </button>
      </div>
    </div>
  );
}

function MCPSettings({ port, token }: { port: number | null; token: string | null }) {
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<string | null>(null);

  const test = useCallback(async () => {
    setTesting(true);
    setTestResult(null);
    try {
      await engine.mcpListTools();
      setTestResult("Connected successfully!");
    } catch (err) {
      setTestResult(`Failed: ${err}`);
    } finally {
      setTesting(false);
    }
  }, []);

  return (
    <div className="space-y-4">
      <h3 className="panel-label">MCP Server</h3>
      
      <div className="space-y-3">
        <div className="flex items-center gap-2">
          <span className={`status-dot ${port ? "online" : "offline"}`} />
          <span className="text-[12px] text-ink">
            {port ? `Active on port ${port}` : "Inactive"}
          </span>
        </div>

        {port && (
          <>
            <div>
              <label className="mb-1 block text-[11px] text-ink-faint">URL</label>
              <div className="input font-mono text-[11px]">
                http://127.0.0.1:{port}/mcp
              </div>
            </div>
            <div>
              <label className="mb-1 block text-[11px] text-ink-faint">Token</label>
              <div className="input font-mono text-[11px] select-all break-all">
                {token}
              </div>
            </div>
            <button className="btn-secondary" onClick={() => void test()} disabled={testing}>
              {testing ? "Testing..." : "Test Connection"}
            </button>
            {testResult && (
              <div className={`text-[11px] ${testResult.startsWith("Connected") ? "text-green-400" : "text-red-400"}`}>
                {testResult}
              </div>
            )}
          </>
        )}
      </div>
    </div>
  );
}

function LanguageSettings() {
  const [lang, setLang] = useState(() => getSetting("ui_lang", "en"));
  const [rtl, setRtl] = useState(() => localStorage.getItem("ui_rtl") === "true");
  const [saved, setSaved] = useState(false);

  const langs = [
    { code: "en", name: "English", native: "English" },
    { code: "fa", name: "Persian", native: "فارسی" },
    { code: "ar", name: "Arabic", native: "العربية" },
    { code: "ru", name: "Russian", native: "Русский" },
    { code: "zh", name: "Chinese", native: "中文" },
    { code: "es", name: "Spanish", native: "Español" },
    { code: "fr", name: "French", native: "Français" },
    { code: "de", name: "German", native: "Deutsch" },
  ];

  const saveLanguage = (code: string) => {
    setLang(code);
    setSetting("ui_lang", code);
    setSaved(true);
    setTimeout(() => setSaved(false), 2000);
  };

  const saveRtl = (value: boolean) => {
    setRtl(value);
    localStorage.setItem("ui_rtl", String(value));
    document.documentElement.classList.toggle("rtl", value);
    setSaved(true);
    setTimeout(() => setSaved(false), 2000);
  };

  return (
    <div className="space-y-4">
      <h3 className="panel-label">Language</h3>
      <div className="space-y-2">
        {langs.map((l) => (
          <label key={l.code} className="flex items-center gap-3 p-2 rounded-md hover:bg-bg2 cursor-pointer">
            <input
              type="radio"
              name="lang"
              checked={lang === l.code}
              onChange={() => saveLanguage(l.code)}
              className="w-4 h-4"
            />
            <div>
              <div className="text-[12px] text-ink">{l.native}</div>
              <div className="text-[10px] text-ink-faint">{l.name}</div>
            </div>
            {(l.code === "fa" || l.code === "ar") && (
              <span className="ml-auto text-[9px] bg-bg3 px-2 py-0.5 rounded text-ink-faint">RTL</span>
            )}
          </label>
        ))}
      </div>
      <div className="border-t border-line pt-4 space-y-3">
        <h4 className="panel-label">RTL Layout</h4>
        <label className="flex items-center gap-3 p-2 rounded-md hover:bg-bg2 cursor-pointer">
          <input
            type="checkbox"
            checked={rtl}
            onChange={(e) => saveRtl(e.target.checked)}
            className="w-4 h-4"
          />
          <div>
            <div className="text-[12px] text-ink">Right-to-Left Layout</div>
            <div className="text-[10px] text-ink-faint">For RTL languages (Arabic, Persian, Hebrew)</div>
          </div>
        </label>
      </div>
      {saved && <div className="text-[11px] text-green-400">Settings saved!</div>}
    </div>
  );
}

function ShortcutSettings() {
  const defaults: Record<string, string> = {
    "Play/Pause": "Space",
    "Split": "S",
    "Delete": "Delete",
    "Undo": "Ctrl+Z",
    "Redo": "Ctrl+Shift+Z",
    "Save": "Ctrl+S",
    "Open": "Ctrl+O",
    "Shuttle Back": "J",
    "Shuttle Stop": "K",
    "Shuttle Forward": "L",
    "Mark In": "I",
    "Mark Out": "O",
  };

  const [shortcuts, setShortcuts] = useState(() => {
    const saved = getSetting("shortcuts", "");
    return saved ? JSON.parse(saved) : defaults;
  });

  return (
    <div className="space-y-4">
      <h3 className="panel-label">Keyboard Shortcuts</h3>
      <div className="space-y-2">
        {Object.entries(shortcuts).map(([action, key]) => (
          <div key={action} className="flex items-center justify-between p-2 rounded-md hover:bg-bg2">
            <span className="text-[12px] text-ink">{action}</span>
            <kbd className="bg-bg3 border border-line px-2 py-1 rounded text-[11px] font-mono text-ink-dim">
              <>{String(key)}</>
            </kbd>
          </div>
        ))}
      </div>
      <button className="btn-secondary text-[11px]" onClick={() => { setShortcuts(defaults); setSetting("shortcuts", JSON.stringify(defaults)); }}>
        Reset to Defaults
      </button>
    </div>
  );
}

function MIDISettings() {
  const [devices, setDevices] = useState<string[]>([]);
  // mapping state removed for now
  const [scanning, setScanning] = useState(false);

  const scanDevices = useCallback(async () => {
    setScanning(true);
    try {
      if (navigator.requestMIDIAccess) {
        const access = await navigator.requestMIDIAccess();
        const names = Array.from(access.inputs.values()).map((d) => d.name ?? "Unknown MIDI Device");
        setDevices(names);
      }
    } catch { /* ignore */ }
    setScanning(false);
  }, []);

  useEffect(() => { void scanDevices(); }, [scanDevices]);

  const mapActions = [
    "Play/Pause", "Split", "Delete", "Undo", "Redo",
    "Volume", "Brightness", "Saturation", "Contrast",
    "Shuttle", "Zoom Timeline", "Seek Forward", "Seek Back",
  ];

  return (
    <div className="space-y-4">
      <h3 className="panel-label">MIDI Controller</h3>
      
      <div className="space-y-3">
        <div>
          <div className="flex items-center justify-between mb-2">
            <label className="text-[11px] text-ink-faint">Connected Devices</label>
            <button className="btn-secondary text-[10px]" onClick={() => void scanDevices()} disabled={scanning}>
              {scanning ? "Scanning..." : "Refresh"}
            </button>
          </div>
          {devices.length === 0 ? (
            <p className="text-[11px] text-ink-faint">No MIDI devices found. Connect a controller and click Refresh.</p>
          ) : (
            <div className="space-y-1">
              {devices.map((d) => (
                <div key={d} className="flex items-center gap-2 p-2 bg-bg2 rounded">
                  <span className="status-dot online" />
                  <span className="text-[12px] text-ink">{d}</span>
                </div>
              ))}
            </div>
          )}
        </div>

        <div>
          <label className="mb-2 block text-[11px] text-ink-faint">MIDI Mapping</label>
          <div className="space-y-2">
            {mapActions.map((action) => (
              <div key={action} className="flex items-center justify-between p-2 rounded-md bg-bg2">
                <span className="text-[12px] text-ink">{action}</span>
                <span className="text-[10px] text-ink-faint italic">Click to map</span>
              </div>
            ))}
          </div>
          <button className="btn-secondary text-[11px] mt-2">Save Mapping</button>
        </div>
      </div>
    </div>
  );
}

function ExportSettings() {
  const [format, setFormat] = useState(() => getSetting("export_format", "mp4"));
  const [quality, setQuality] = useState(() => getSetting("export_quality", "high"));
  const [resolution, setResolution] = useState(() => getSetting("export_resolution", "1080"));

  return (
    <div className="space-y-4">
      <h3 className="panel-label">Export Settings</h3>
      <div className="space-y-3">
        <div>
          <label className="mb-1 block text-[11px] text-ink-faint">Format</label>
          <select className="input" value={format} onChange={(e) => { setFormat(e.target.value); setSetting("export_format", e.target.value); }}>
            <option value="mp4">MP4 (H.264)</option>
            <option value="webm">WebM (VP9)</option>
            <option value="gif">GIF</option>
          </select>
        </div>
        <div>
          <label className="mb-1 block text-[11px] text-ink-faint">Quality</label>
          <select className="input" value={quality} onChange={(e) => { setQuality(e.target.value); setSetting("export_quality", e.target.value); }}>
            <option value="draft">Draft (fast)</option>
            <option value="standard">Standard</option>
            <option value="high">High Quality</option>
          </select>
        </div>
        <div>
          <label className="mb-1 block text-[11px] text-ink-faint">Resolution</label>
          <select className="input" value={resolution} onChange={(e) => { setResolution(e.target.value); setSetting("export_resolution", e.target.value); }}>
            <option value="720">720p</option>
            <option value="1080">1080p</option>
            <option value="1440">1440p</option>
            <option value="2160">4K</option>
          </select>
        </div>
      </div>
    </div>
  );
}
