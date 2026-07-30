import { useCallback, useEffect, useRef, useState } from "react";
import { useStore } from "../state/store";
import { engine } from "../state/store";
import { showToast } from "./Toast";
import { detectCompositionRequest, createTitleComposition, createLowerThird, renderComposition } from "../lib/hyperframes";
import { renderToTimeline } from "../lib/hyperframes";
import { t } from "../lib/i18n";
import { getActiveEndpoint } from "./SettingsPanel";
import { activeSequence } from "../engine/types";
import { callAI, parseAIResponse } from "../lib/ai";

interface McpMessage {
  role: "user" | "assistant" | "tool" | "error";
  text: string;
}

export default function McpChat() {
  const mcpPort = useStore((s) => s.mcpPort);
  const selection = useStore((s) => s.selection);
  const project = useStore((s) => s.project);
  const playheadUs = useStore((s) => s.playheadUs);
  const [messages, setMessages] = useState<McpMessage[]>(() => {
    try {
      const raw = localStorage.getItem("opencut_chat_history");
      return raw ? (JSON.parse(raw) as McpMessage[]) : [];
    } catch { return []; }
  });
  const [input, setInput] = useState("");
  const [loading, setLoading] = useState(false);
  const [pendingFrame, setPendingFrame] = useState<string | null>(null);
  const endRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    endRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages]);

  // Persist chat history to localStorage (cap at 50)
  useEffect(() => {
    const capped = messages.length > 50 ? messages.slice(-50) : messages;
    try { localStorage.setItem("opencut_chat_history", JSON.stringify(capped)); } catch { /* ignore */ }
    if (messages.length > 50) setMessages(capped);
  }, [messages]);


  /** Attach the current frame from the selected clip for the next message */
  const attachFrame = useCallback(async () => {
    if (!selection.length) { showToast("Select a clip first", "error"); return; }
    const clipId = selection[0];
    const seq = activeSequence(project);
    const clip = seq.tracks.flatMap((t) => t.clips).find((c) => c.id === clipId);
    if (!clip || clip.payload.type !== "media") { showToast("Selected clip has no media", "error"); return; }
    try {
      const result = (await engine.mcpCall("analyze_frame", { asset_id: clip.payload.asset_id, time_us: 0, prompt: "frame capture" })) as { frame_base64?: string };
      if (result?.frame_base64) {
        setPendingFrame(result.frame_base64);
        showToast("Frame attached for next message", "success");
      } else {
        showToast("Failed to extract frame", "error");
      }
    } catch (e) {
      showToast(`Frame error: ${e instanceof Error ? e.message : String(e)}`, "error");
    }
  }, [selection, project]);

  const handleSend = useCallback(async () => {
    if (!input.trim() || loading) return;
    const text = input.trim();
    setInput("");
    const frameForThisMsg = pendingFrame;
    setPendingFrame(null);
    setMessages((m) => [...m, { role: "user", text: frameForThisMsg ? `${text} [📸 frame attached]` : text }]);
    setLoading(true);

    // HyperFrames: detect composition requests
    const composition = detectCompositionRequest(text);
    if (composition) {
      try {
        if (composition.type === "title") {
          const frame = createTitleComposition(composition.args.text);
          const html = renderComposition(frame);
          setMessages((m) => [...m, { role: "assistant", text: `Composition created: ${frame.name}\n\n${html.slice(0, 200)}...` }]);
          showToast(`Composition "${frame.name}" created`, "success");
          try { await renderToTimeline(frame, engine, playheadUs); showToast("Added to timeline", "success"); } catch (e) { showToast(`Timeline error: ${e instanceof Error ? e.message : String(e)}`, "error"); }
        } else if (composition.type === "lower_third") {
          const frame = createLowerThird(composition.args.title, composition.args.subtitle);
          setMessages((m) => [...m, { role: "assistant", text: `Lower third created: ${frame.name}` }]);
          showToast(`Lower third "${frame.name}" created`, "success");
          try { await renderToTimeline(frame, engine, playheadUs); showToast("Added to timeline", "success"); } catch (e) { showToast(`Timeline error: ${e instanceof Error ? e.message : String(e)}`, "error"); }
        } else {
          const frame = createTitleComposition(composition.args.text ?? composition.args.title ?? "Title");
          setMessages((m) => [...m, { role: "assistant", text: `Composition created: ${frame.name}` }]);
          showToast(`Composition created`, "success");
          try { await renderToTimeline(frame, engine, playheadUs); showToast("Added to timeline", "success"); } catch (e) { showToast(`Timeline error: ${e instanceof Error ? e.message : String(e)}`, "error"); }
        }
      } catch (err) {
        setMessages((m) => [...m, { role: "error", text: String(err) }]);
      } finally {
        setLoading(false);
      }
      return;
    }

    // Voice-over / dubbing: detect keyword triggers
    const dubMatch = text.match(/\b(voice\s*over|dub(?:bing)?|نریشن|دوبلاژ)\b/i);
    if (dubMatch) {
      // Extract the dubbing text (everything after the trigger word)
      const dubText = text.replace(dubMatch[0], "").trim();
      if (!dubText) {
        setMessages((m) => [...m, { role: "assistant", text: "Provide text to generate voice-over. Example: voice over سلام دنیا" }]);
        setLoading(false);
        return;
      }
      // Detect language from script
      const langMatch = dubText.match(/[\u0600-\u06FF]/) ? "fa" : dubText.match(/[\u0590-\u05FF]/) ? "he" : "en-US";
      const voiceMap: Record<string, string> = { fa: "fj_parisa", ar: "ej_fatima", "en-US": "af_heart", he: "af_heart" };
      const engineId = "kokoro";
      const voice = voiceMap[langMatch] ?? "af_heart";
      try {
        setMessages((m) => [...m, { role: "assistant", text: `Generating voice-over (${langMatch})...` }]);
        const result = await engine.mcpCall("generate_speech", {
          engine: engineId,
          voice,
          text: dubText,
        });
        const formatted = typeof result === "string" ? result : JSON.stringify(result, null, 2);
        setMessages((m) => [...m, { role: "tool", text: formatted }]);
        showToast("Voice-over generated", "success");
      } catch (err) {
        setMessages((m) => [...m, { role: "error", text: String(err) }]);
        showToast(`Voice-over error: ${err instanceof Error ? err.message : String(err)}`, "error");
      } finally {
        setLoading(false);
      }
      return;
    }

    try {
      const ep = getActiveEndpoint();
      if (ep.url) {
        // AI mode: send to LLM, parse tool_call, execute
        setMessages((m) => [...m, { role: "assistant", text: t("Thinking...") }]);
        const history = [...messages.map((m) => ({ role: m.role === "user" ? "user" : "assistant", text: m.text })), { role: "user" as const, text }];
        const aiResponse = await callAI(ep, history, frameForThisMsg);
        const parsed = parseAIResponse(aiResponse);

        if (parsed.toolCall) {
          setMessages((m) => {
            const updated = [...m];
            updated[updated.length - 1] = { role: "assistant", text: `Executing: ${parsed.toolCall!.name}(${JSON.stringify(parsed.toolCall!.arguments)})` };
            return updated;
          });

          const result = await engine.mcpCall(parsed.toolCall.name, parsed.toolCall.arguments);
          const formatted = typeof result === "string" ? result : JSON.stringify(result, null, 2);
          setMessages((m) => [...m, { role: "tool", text: formatted }]);
          showToast(`Tool executed: ${parsed.toolCall.name}`, "success");
        } else {
          setMessages((m) => {
            const updated = [...m];
            updated[updated.length - 1] = { role: "assistant", text: parsed.text ?? aiResponse };
            return updated;
          });
        }
      } else {
        // Manual mode
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
      showToast(`Error: ${err instanceof Error ? err.message : String(err)}`, "error");
    } finally {
      setLoading(false);
    }
  }, [input, loading, messages, pendingFrame, playheadUs]);

  const ep = getActiveEndpoint();
  const hasAI = !!ep.url;

  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center justify-between border-b border-line px-3 py-1.5">
        <div className="flex items-center gap-2">
          <span className={`h-2 w-2 rounded-full ${hasAI ? "bg-green-500" : "bg-yellow-500"}`} />
          <span className="text-[10px] text-ink-faint">
            {hasAI ? `AI: ${ep.model}` : "Manual mode"}
          </span>
        </div>
        <div className="flex items-center gap-2">
          {messages.length > 0 && (
            <button
              className="text-[10px] text-ink-faint hover:text-red-400"
              onClick={() => { setMessages([]); localStorage.removeItem("opencut_chat_history"); }}
            >
              {t("Clear History")}
            </button>
          )}
        </div>
      </div>

      {/* Messages */}
      <div className="flex-1 overflow-auto p-3 space-y-2">
        {messages.length === 0 && (
          <div className="text-center text-[11px] text-ink-faint mt-8">
            {hasAI ? "Ask the AI to edit your project..." : mcpPort ? "Type tool_name {json} to call MCP tools" : "Configure an AI endpoint in Settings to get started"}
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
        {pendingFrame && (
          <div className="mb-1 flex items-center gap-1 text-[10px] text-green-400">
            📸 Frame attached — will be sent with next message
            <button className="ml-1 text-ink-faint hover:text-ink" onClick={() => setPendingFrame(null)}>✕</button>
          </div>
        )}
        <div className="flex gap-2">
          <button
            className="shrink-0 rounded-md border border-line bg-bg0 px-2 py-1.5 text-[12px] hover:bg-bg2 disabled:opacity-40"
            onClick={() => void attachFrame()}
            disabled={loading || !selection.length}
            title="Attach current frame from selected clip"
          >📸</button>
          <input
            type="text"
            className="flex-1 rounded-md border border-line bg-bg0 px-3 py-1.5 text-[12px] text-ink placeholder:text-ink-faint focus:outline-none focus:ring-1 focus:ring-(--color-accent)"
            placeholder={hasAI ? "Describe what you want to do..." : "tool_name {json args}"}
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
            {loading ? <span className="inline-flex gap-0.5"><span className="animate-[dot_1s_infinite_0s]">.</span><span className="animate-[dot_1s_infinite_0.2s]">.</span><span className="animate-[dot_1s_infinite_0.4s]">.</span></span> : t("Send")}
          </button>
        </div>
      </div>
    </div>
  );
}
