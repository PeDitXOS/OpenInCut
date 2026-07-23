import { useCallback, useState } from "react";
import { useStore } from "../state/store";
import { engine } from "../state/store";

export default function McpSettings({ onClose }: { onClose: () => void }) {
  const mcpPort = useStore((s) => s.mcpPort);
  const mcpToken = useStore((s) => s.mcpToken);
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<string | null>(null);

  const testConnection = useCallback(async () => {
    const client = engine;
    setTesting(true);
    setTestResult(null);
    try {
      await client.mcpListTools();
      setTestResult("Connected successfully!");
    } catch (err) {
      setTestResult(`Connection failed: ${err}`);
    } finally {
      setTesting(false);
    }
  }, []);

  return (
    <div className="p-4 space-y-4">
      <div className="flex items-center justify-between">
        <h3 className="text-[13px] font-semibold text-ink">MCP Settings</h3>
        <button
          className="text-[11px] text-ink-faint hover:text-ink"
          onClick={onClose}
        >
          ✕
        </button>
      </div>

      <div className="space-y-3">
        <div>
          <label className="mb-1 block text-[10.5px] font-medium uppercase tracking-wide text-ink-faint">
            Status
          </label>
          <div className="flex items-center gap-2">
            <span className={`h-2 w-2 rounded-full ${mcpPort ? "bg-green-500" : "bg-red-500"}`} />
            <span className="text-[12px] text-ink">
              {mcpPort ? `Active on port ${mcpPort}` : "Inactive"}
            </span>
          </div>
        </div>

        {mcpPort && (
          <>
            <div>
              <label className="mb-1 block text-[10.5px] font-medium uppercase tracking-wide text-ink-faint">
                URL
              </label>
              <div className="rounded-md border border-line bg-bg0 px-2 py-1.5 font-[var(--font-mono)] text-[11px] text-ink">
                http://127.0.0.1:{mcpPort}/mcp
              </div>
            </div>

            <div>
              <label className="mb-1 block text-[10.5px] font-medium uppercase tracking-wide text-ink-faint">
                Token
              </label>
              <div className="rounded-md border border-line bg-bg0 px-2 py-1.5 font-[var(--font-mono)] text-[11px] text-ink select-all break-all">
                {mcpToken}
              </div>
            </div>

            <button
              className="rounded-md border border-line bg-bg2 px-3 py-1.5 text-[12px] text-ink hover:bg-bg3 disabled:opacity-50"
              onClick={() => void testConnection()}
              disabled={testing}
            >
              {testing ? "Testing..." : "Test Connection"}
            </button>

            {testResult && (
              <div className={`text-[11px] ${testResult.startsWith("Connected") ? "text-green-400" : "text-red-400"}`}>
                {testResult}
              </div>
            )}
          </>
        )}

        {!mcpPort && (
          <p className="text-[11px] text-ink-dim">
            The MCP server starts automatically when the desktop app launches.
            Run <code className="font-[var(--font-mono)] text-ink">npx tauri dev</code> to start the app.
          </p>
        )}
      </div>
    </div>
  );
}
