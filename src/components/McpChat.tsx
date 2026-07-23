import { useCallback, useEffect, useRef, useState } from "react";
import { useStore } from "../state/store";
import { engine } from "../state/store";

/** Read/write from localStorage */
function getSetting(key: string, fallback: string): string {
  try { return localStorage.getItem(`opencut_${key}`) ?? fallback; } catch { return fallback; }
}
function setSetting(key: string, value: string) {
  try { localStorage.setItem(`opencut_${key}`, value); } catch { /* ignore */ }
}

/** The tools we expose to the AI as function-calling schema */

function buildSystemPrompt(toolList: string): string {
  return `You are an AI assistant inside the OpenInCut video editor. You control the editor through MCP tools.

Available tools: ${toolList}

Rules:
- When the user asks to do something, pick the right tool and return a JSON object with tool_call
- Always respond with EXACTLY one of these formats:
  1. {"tool_call": {"name": "tool_name", "arguments": {arg1: val1, ...}}}
  2. A plain text answer if no tool is needed
- For color correction: use set_clip_properties with brightness/contrast/saturation
- For titles: use add_text_clip
- For cutting: use split_clip or delete_clips
- For removing silence: use remove_silences
- For subtitles: transcribe_asset then add_subtitles_clip
- Be concise. Prefer action over explanation.`;
}

interface McpMessage {
  role: "user" | "assistant" | "tool" | "error";
  text: string;
}

export default function McpChat() {
  const mcpPort = useStore((s) => s.mcpPort);
  const [messages, setMessages] = useState<McpMessage[]>([]);
  const [input, setInput] = useState("");
  const [tools, setTools] = useState<string[]>([]);
  const [loading, setLoading] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  const endRef = useRef<HTMLDivElement>(null);

  // Settings
  const [apiUrl, setApiUrl] = useState(() => getSetting("ai_url", "https://9router.peditx.ir/v1"));
  const [apiKey, setApiKey] = useState(() => getSetting("ai_key", ""));
  const [model, setModel] = useState(() => getSetting("ai_model", "hermes-3-llama-3.1-70b"));
  const [aiEnabled, setAiEnabled] = useState(() => getSetting("ai_enabled", "") === "true");

  useEffect(() => {
    endRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages]);

  const loadTools = useCallback(async () => {
    try {
      const result = (await engine.mcpListTools()) as { tools?: { name: string }[] };
      const names = result?.tools?.map((t) => t.name) ?? [];
      setTools(names);
    } catch { /* ignore */ }
  }, []);

  useEffect(() => {
    if (mcpPort) void loadTools();
  }, [mcpPort, loadTools]);

  const saveSettings = useCallback(() => {
    setSetting("ai_url", apiUrl);
    setSetting("ai_key", apiKey);
    setSetting("ai_model", model);
    setSetting("ai_enabled", String(aiEnabled));
    setShowSettings(false);
  }, [apiUrl, apiKey, model, aiEnabled]);

  /** Call the AI API with OpenAI-compatible format */
  const callAI = useCallback(async (userMessage: string, history: McpMessage[]): Promise<string> => {
    const toolList = tools.join(", ");
    const systemMsg = buildSystemPrompt(toolList);

    const chatMessages = [
      { role: "system", content: systemMsg },
      ...history.map((m) => ({
        role: m.role === "user" ? "user" as const : "assistant" as const,
        content: m.text,
      })),
      { role: "user" as const, content: userMessage },
    ];

    const resp = await fetch(`${apiUrl}/chat/completions`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        "Authorization": `Bearer ${apiKey}`,
      },
      body: JSON.stringify({
        model,
        messages: chatMessages,
        temperature: 0.3,
        max_tokens: 2048,
      }),
    });

    if (!resp.ok) throw new Error(`API error: ${resp.status}`);
    const data = await resp.json();
    return data.choices?.[0]?.message?.content ?? "";
  }, [apiUrl, apiKey, model, tools]);

  /** Parse AI response for tool_call or plain text */
  const parseAIResponse = useCallback((response: string): { toolCall?: { name: string; arguments: Record<string, unknown> }; text?: string } => {
    const trimmed = response.trim();
    // Try to find JSON tool_call in the response
    const jsonMatch = trimmed.match(/\{[\s\S]*"tool_call"[\s\S]*\}/);
    if (jsonMatch) {
      try {
        const parsed = JSON.parse(jsonMatch[0]);
        if (parsed.tool_call?.name) {
          return { toolCall: parsed.tool_call };
        }
      } catch { /* fall through */ }
    }
    // Also handle format: {"name": "...", "arguments": {...}} directly
    const directMatch = trimmed.match(/\{[\s\S]*"name"[\s\S]*"arguments"[\s\S]*\}/);
    if (directMatch) {
      try {
        const parsed = JSON.parse(directMatch[0]);
        if (parsed.name) {
          return { toolCall: { name: parsed.name, arguments: parsed.arguments ?? {} } };
        }
      } catch { /* fall through */ }
    }
    return { text: trimmed };
  }, []);

  const handleSend = useCallback(async () => {
    if (!input.trim() || loading) return;
    const text = input.trim();
    setInput("");
    setMessages((m) => [...m, { role: "user", text }]);
    setLoading(true);

    try {
      if (aiEnabled && apiKey) {
        // AI mode: send to LLM, parse tool_call, execute
        setMessages((m) => [...m, { role: "assistant", text: "Thinking..." }]);
        const aiResponse = await callAI(text, messages);
        const parsed = parseAIResponse(aiResponse);

        if (parsed.toolCall) {
          // Show what AI decided to do
          setMessages((m) => {
            const updated = [...m];
            updated[updated.length - 1] = { role: "assistant", text: `Executing: ${parsed.toolCall!.name}(${JSON.stringify(parsed.toolCall!.arguments)})` };
            return updated;
          });

          // Execute the tool
          const result = await engine.mcpCall(parsed.toolCall.name, parsed.toolCall.arguments);
          const formatted = typeof result === "string" ? result : JSON.stringify(result, null, 2);
          setMessages((m) => [...m, { role: "tool", text: formatted }]);
        } else {
          // Plain text response from AI
          setMessages((m) => {
            const updated = [...m];
            updated[updated.length - 1] = { role: "assistant", text: parsed.text ?? aiResponse };
            return updated;
          });
        }
      } else {
        // Manual mode: parse "toolName {jsonArgs}"
        const parts = text.split(/\s+/);
        const toolName = parts[0];
        let args: Record<string, unknown> = {};
        if (parts.length > 1) {
          try { args = JSON.parse(parts.slice(1).join(" ")); } catch { args = { input: parts.slice(1).join(" ") }; }
        }
        const result = await engine.mcpCall(toolName, args);
        const formatted = typeof result === "string" ? result : JSON.stringify(result, null, 2);
        setMessages((m) => [...m, { role: "tool", text: formatted }]);
      }
    } catch (err) {
      setMessages((m) => [...m, { role: "error", text: String(err) }]);
    } finally {
      setLoading(false);
    }
  }, [input, loading, messages, aiEnabled, apiKey, callAI, parseAIResponse]);

  if (!mcpPort) {
    return (
      <div className="flex h-full items-center justify-center text-[12px] text-ink-faint">
        MCP server not running
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col">
      {/* Header with settings button */}
      <div className="flex items-center justify-between border-b border-line px-3 py-1.5">
        <div className="flex items-center gap-2">
          <span className={`h-2 w-2 rounded-full ${aiEnabled ? "bg-green-500" : "bg-yellow-500"}`} />
          <span className="text-[10px] text-ink-faint">
            {aiEnabled ? `AI: ${model}` : "Manual mode"}
          </span>
        </div>
        <button
          className="text-[10px] text-ink-faint hover:text-ink"
          onClick={() => setShowSettings(!showSettings)}
        >
          ⚙ Settings
        </button>
      </div>

      {/* Settings panel */}
      {showSettings && (
        <div className="border-b border-line bg-bg0 p-3 space-y-2">
          <div>
            <label className="mb-0.5 block text-[10px] text-ink-faint">API URL</label>
            <input
              type="text"
              className="w-full rounded border border-line bg-bg1 px-2 py-1 text-[11px] text-ink"
              value={apiUrl}
              onChange={(e) => setApiUrl(e.target.value)}
              placeholder="https://9router.peditx.ir/v1"
            />
          </div>
          <div>
            <label className="mb-0.5 block text-[10px] text-ink-faint">API Key</label>
            <input
              type="password"
              className="w-full rounded border border-line bg-bg1 px-2 py-1 text-[11px] text-ink"
              value={apiKey}
              onChange={(e) => setApiKey(e.target.value)}
              placeholder="sk-..."
            />
          </div>
          <div>
            <label className="mb-0.5 block text-[10px] text-ink-faint">Model</label>
            <input
              type="text"
              className="w-full rounded border border-line bg-bg1 px-2 py-1 text-[11px] text-ink"
              value={model}
              onChange={(e) => setModel(e.target.value)}
              placeholder="hermes-3-llama-3.1-70b"
            />
          </div>
          <div className="flex items-center gap-2">
            <input
              type="checkbox"
              id="ai-enabled"
              checked={aiEnabled}
              onChange={(e) => setAiEnabled(e.target.checked)}
            />
            <label htmlFor="ai-enabled" className="text-[11px] text-ink">Enable AI assistant</label>
          </div>
          <button
            className="rounded bg-accent px-3 py-1 text-[11px] text-bg0 hover:bg-accent/80"
            onClick={saveSettings}
          >
            Save
          </button>
        </div>
      )}

      {/* Messages */}
      <div className="flex-1 overflow-auto p-3 space-y-2">
        {messages.length === 0 && (
          <div className="text-center text-[11px] text-ink-faint mt-8">
            {aiEnabled ? "Ask the AI to edit your project..." : "Type tool_name {json} to call MCP tools"}
          </div>
        )}
        {messages.map((msg, i) => (
          <div key={i} className={`rounded-lg px-3 py-2 text-[12px] ${
            msg.role === "user" ? "ml-8 bg-accent/20 text-ink border border-accent/30" :
            msg.role === "assistant" ? "mr-8 bg-bg2 text-ink-dim border border-line" :
            msg.role === "tool" ? "mx-4 bg-bg3 text-green-300 border border-green-800/50" :
            "mx-4 bg-red-900/30 text-red-300 border border-red-800/50"
          }`}>
            <div className="text-[9px] text-ink-faint mb-1 uppercase">
              {msg.role === "user" ? "You" : msg.role === "assistant" ? "AI" : msg.role === "tool" ? "Result" : "Error"}
            </div>
            <pre className="whitespace-pre-wrap break-all font-[var(--font-mono)] text-[11px]">
              {msg.text}
            </pre>
          </div>
        ))}
        <div ref={endRef} />
      </div>

      {/* Input */}
      <div className="border-t border-line p-2">
        <div className="flex gap-2">
          <input
            type="text"
            className="flex-1 rounded-md border border-line bg-bg0 px-3 py-1.5 text-[12px] text-ink placeholder:text-ink-faint focus:outline-none focus:ring-1 focus:ring-(--color-accent)"
            placeholder={aiEnabled ? "Describe what you want to do..." : "tool_name {json args}"}
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
