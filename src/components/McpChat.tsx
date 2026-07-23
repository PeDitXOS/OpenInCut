import { useCallback, useEffect, useRef, useState } from "react";
import { useStore } from "../state/store";
import { engine } from "../state/store";

interface McpMessage {
  role: "user" | "system" | "result" | "error";
  text: string;
}

export default function McpChat() {
  const client = engine;
  const mcpPort = useStore((s) => s.mcpPort);
  const [messages, setMessages] = useState<McpMessage[]>([]);
  const [input, setInput] = useState("");
  const [tools, setTools] = useState<string[]>([]);
  const [loading, setLoading] = useState(false);
  const endRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    endRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages]);

  const loadTools = useCallback(async () => {
    try {
      const result = (await client.mcpListTools()) as { tools?: { name: string }[] };
      const names = result?.tools?.map((t) => t.name) ?? [];
      setTools(names);
    } catch { /* ignore */ }
  }, [client]);

  useEffect(() => {
    if (mcpPort) void loadTools();
  }, [mcpPort, loadTools]);

  const handleSend = useCallback(async () => {
    if (!input.trim() || loading) return;
    const text = input.trim();
    setInput("");
    setMessages((m) => [...m, { role: "user", text }]);
    setLoading(true);

    try {
      // Parse: either "toolName jsonArgs" or just "toolName"
      const parts = text.split(/\s+/);
      const toolName = parts[0];
      let args: Record<string, unknown> = {};
      if (parts.length > 1) {
        try {
          args = JSON.parse(parts.slice(1).join(" "));
        } catch {
          // treat as simple string arg
          args = { input: parts.slice(1).join(" ") };
        }
      }

      const result = await client.mcpCall(toolName, args);
      const formatted = typeof result === "string" ? result : JSON.stringify(result, null, 2);
      setMessages((m) => [...m, { role: "result", text: formatted }]);
    } catch (err) {
      setMessages((m) => [...m, { role: "error", text: String(err) }]);
    } finally {
      setLoading(false);
    }
  }, [input, loading, client]);

  if (!mcpPort) {
    return (
      <div className="flex h-full items-center justify-center text-[12px] text-ink-faint">
        MCP server not running
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col">
      <div className="flex-1 overflow-auto p-3 space-y-2">
        {tools.length > 0 && (
          <div className="mb-2 text-[10px] text-ink-faint">
            Tools: {tools.join(", ")}
          </div>
        )}
        {messages.map((msg, i) => (
          <div key={i} className={`rounded-lg px-3 py-2 text-[12px] ${
            msg.role === "user" ? "ml-8 bg-bg3 text-ink" :
            msg.role === "error" ? "bg-red-900/30 text-red-300" :
            "mr-8 bg-bg2 text-ink-dim"
          }`}>
            <pre className="whitespace-pre-wrap break-all font-[var(--font-mono)] text-[11px]">
              {msg.text}
            </pre>
          </div>
        ))}
        <div ref={endRef} />
      </div>
      <div className="border-t border-line p-2">
        <div className="flex gap-2">
          <input
            type="text"
            className="flex-1 rounded-md border border-line bg-bg0 px-3 py-1.5 text-[12px] text-ink placeholder:text-ink-faint focus:outline-none focus:ring-1 focus:ring-(--color-accent)"
            placeholder="tool_name {json args} or tool_name text"
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && void handleSend()}
            disabled={loading}
          />
          <button
            className="rounded-md bg-accent px-3 py-1.5 text-[12px] font-medium text-bg0 hover:bg-accent/80 disabled:opacity-50"
            onClick={() => void handleSend()}
            disabled={loading || !input.trim()}
          >
            {loading ? "..." : "Send"}
          </button>
        </div>
      </div>
    </div>
  );
}
