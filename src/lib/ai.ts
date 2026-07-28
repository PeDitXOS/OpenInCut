import { getSystemPrompt } from "../components/SettingsPanel";
import { engine } from "../state/store";

export interface AiMessage {
  role: "system" | "user" | "assistant" | "tool";
  content: string | Array<{ type: "text"; text: string } | { type: "image_url"; image_url: { url: string } } | { type: "image"; source: { type: "base64"; media_type: string; data: string } }>;
}

export interface OpenAIMessage {
  role: "system" | "user" | "assistant";
  content: string | Array<{ type: "text"; text: string } | { type: "image_url"; image_url: { url: string } }>;
}

export interface AnthropicMessage {
  role: "user" | "assistant";
  content: string | Array<{ type: "text"; text: string } | { type: "image"; source: { type: "base64"; media_type: string; data: string } }>;
}

export interface Endpoint {
  name: string;
  url: string;
  key: string;
  model: string;
}

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
- Be concise. Prefer action over explanation.
- When a frame image is attached, analyze it to answer questions about visual content.
When the user asks about colors, brightness, or visual properties, you can see frames. Describe what you observe.`;
}

/** Normalize endpoint URL: ensure it ends with the right path. */
function normalizeUrl(url: string): string {
  let u = url.trim().replace(/\/+$/, "");
  // Already has /v1 → keep as-is (e.g. https://api.openai.com/v1)
  if (/\/v1$/.test(u)) return u;
  // Hermes / 9Router / local gateways need /v1
  if (u.includes("9router") || u.includes("hermes") || /localhost:\d+/.test(u) || /127\.0\.0\.1:\d+/.test(u)) {
    return u + "/v1";
  }
  // OpenAI-style: if it's api.openai.com without /v1, add it
  if (u.includes("api.openai.com") && !u.includes("/v1")) return u + "/v1";
  return u;
}

/** Call Anthropic API directly */
async function callAnthropic(
  ep: Endpoint,
  messages: AnthropicMessage[],
  system: string,
  model: string,
  maxTokens: number,
  temperature: number
): Promise<string> {
  const resp = await fetch("https://api.anthropic.com/v1/messages", {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "x-api-key": ep.key,
      "anthropic-version": "2023-06-01",
    },
    body: JSON.stringify({
      model,
      max_tokens: maxTokens,
      temperature,
      system,
      messages: messages.map((m) => ({
        role: m.role === "assistant" ? "assistant" : "user",
        content: typeof m.content === "string" ? m.content : m.content.map((c: { type: string; image_url?: { url: string } }) =>
          c.type === "image_url" && c.image_url
            ? { type: "image", source: { type: "base64", media_type: "image/jpeg", data: c.image_url.url.split(",")[1] } }
            : c
        ),
      })),
    }),
  });

  if (!resp.ok) throw new Error(`Anthropic API error: ${resp.status}`);
  const data = await resp.json();
  return data.content?.[0]?.text ?? "";
}

/** Call Hermes Agent Gateway */
async function callHermes(
  ep: Endpoint,
  messages: AiMessage[],
  model: string,
  maxTokens: number,
  temperature: number
): Promise<string> {
  const resp = await fetch(`${normalizeUrl(ep.url)}/chat/completions`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      Authorization: `Bearer ${ep.key}`,
    },
    body: JSON.stringify({
      model,
      messages,
      temperature,
      max_tokens: maxTokens,
    }),
  });

  if (!resp.ok) throw new Error(`Hermes API error: ${resp.status}`);
  const data = await resp.json();
  return data.choices?.[0]?.message?.content ?? "";
}

/** Call OpenAI-compatible API (default) */
async function callOpenAI(
  ep: Endpoint,
  messages: AiMessage[],
  model: string,
  maxTokens: number,
  temperature: number
): Promise<string> {
  const resp = await fetch(`${normalizeUrl(ep.url)}/chat/completions`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      Authorization: `Bearer ${ep.key}`,
    },
    body: JSON.stringify({
      model,
      messages,
      temperature,
      max_tokens: maxTokens,
    }),
  });

  if (!resp.ok) throw new Error(`OpenAI API error: ${resp.status}`);
  const data = await resp.json();
  return data.choices?.[0]?.message?.content ?? "";
}

/** Main AI caller - detects provider by URL and calls appropriate API */
export async function callAI(
  ep: Endpoint,
  history: Array<{ role: string; text: string }>,
  frameBase64?: string | null
): Promise<string> {
  const toolList = (await engine.mcpListTools()) as { tools?: { name: string }[] } | undefined;
  const tools = toolList?.tools?.map((t) => t.name) ?? [];
  const customPrompt = getSystemPrompt();
  const defaultPrompt = buildSystemPrompt(tools.join(", "));
  const fullSystem = customPrompt ? `${customPrompt}\n\n${defaultPrompt}` : defaultPrompt;

  // Build messages for OpenAI format (used by default and Hermes)
  const openAIMessages: OpenAIMessage[] = [{ role: "system", content: fullSystem }];

  for (const m of history) {
    openAIMessages.push({
      role: m.role === "user" ? "user" : "assistant",
      content: m.text,
    });
  }

  // Add user message with optional vision
  const lastUserMsg = history[history.length - 1]?.text ?? "";
  const userContent: Array<{ type: "text"; text: string } | { type: "image_url"; image_url: { url: string } }> = [
    { type: "text", text: lastUserMsg },
  ];
  if (frameBase64) {
    userContent.push({ type: "image_url", image_url: { url: `data:image/jpeg;base64,${frameBase64}` } });
  }

  // Build Anthropic messages if needed
  let anthropicMessages: AnthropicMessage[] = [];
  if (ep.url.includes("anthropic.com")) {
    for (const m of history) {
      anthropicMessages.push({
        role: m.role === "user" ? "user" : "assistant",
        content: m.text,
      });
    }
    const lastUser = history[history.length - 1]?.text ?? "";
    anthropicMessages.push({
      role: "user",
      content: frameBase64
        ? [
            { type: "text", text: lastUser },
            { type: "image", source: { type: "base64", media_type: "image/jpeg", data: frameBase64 } },
          ]
        : lastUser,
    });
  }

  const model = ep.model || "gpt-4o";
  const maxTokens = 2048;
  const temperature = 0.3;

  // Detect provider by URL
  const url = normalizeUrl(ep.url);
  if (ep.url.includes("anthropic.com")) {
    return callAnthropic(ep, anthropicMessages, fullSystem, model, maxTokens, temperature);
  }
  if (url.includes("9router") || url.includes("hermes") || /localhost:\d+/.test(url) || /127\.0\.0\.1:\d+/.test(url)) {
    openAIMessages.push({ role: "user", content: userContent });
    return callHermes(ep, openAIMessages, model, maxTokens, temperature);
  }
  openAIMessages.push({ role: "user", content: userContent });
  return callOpenAI(ep, openAIMessages, model, maxTokens, temperature);
}

export function parseAIResponse(response: string): {
  toolCall?: { name: string; arguments: Record<string, unknown> };
  text?: string;
} {
  const trimmed = response.trim();
  const jsonMatch = trimmed.match(/\{[\s\S]*"tool_call"[\s\S]*\}/);
  if (jsonMatch) {
    try {
      const parsed = JSON.parse(jsonMatch[0]);
      if (parsed.tool_call?.name) {
        return { toolCall: parsed.tool_call };
      }
    } catch { /* fall through */ }
  }
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
}

export interface HyperFramesClip {
  type: "title" | "subtitle" | "lower_third" | "text";
  text?: string;
  duration?: number;
  style?: Record<string, string>;
}

export interface HyperFramesComposition {
  clips: HyperFramesClip[];
}

/** Detect and parse HyperFrames commands from user text */
export function generateHyperFramesComposition(text: string): HyperFramesComposition | null {
  const lower = text.toLowerCase();
  
  // Detect HyperFrames commands
  if (!lower.includes("create composition") && !lower.includes("add title") && 
      !lower.includes("add subtitle") && !lower.includes("add lower third") &&
      !lower.includes("hyperframes")) {
    return null;
  }

  const clips: HyperFramesClip[] = [];

  // Parse "create composition" - could be multiple clips
  if (lower.includes("create composition")) {
    // Extract title text if specified
    const titleMatch = text.match(/title[:\s]+["']?([^"']+)["']?/i);
    if (titleMatch) {
      clips.push({ type: "title", text: titleMatch[1].trim() });
    }
    
    // Extract subtitle text
    const subtitleMatch = text.match(/subtitle[:\s]+["']?([^"']+)["']?/i);
    if (subtitleMatch) {
      clips.push({ type: "subtitle", text: subtitleMatch[1].trim() });
    }
    
    // Extract lower third text
    const lowerThirdMatch = text.match(/lower[\s-]?third[:\s]+["']?([^"']+)["']?/i);
    if (lowerThirdMatch) {
      clips.push({ type: "lower_third", text: lowerThirdMatch[1].trim() });
    }

    // Default: add a title if nothing specified
    if (clips.length === 0) {
      clips.push({ type: "title", text: "New Composition" });
    }
  }

  // Parse "add title" command
  if (lower.includes("add title")) {
    const match = text.match(/add title[:\s]+["']?([^"']+)["']?/i);
    clips.push({ type: "title", text: match?.[1]?.trim() || "Title" });
  }

  // Parse "add subtitle" command
  if (lower.includes("add subtitle")) {
    const match = text.match(/add subtitle[:\s]+["']?([^"']+)["']?/i);
    clips.push({ type: "subtitle", text: match?.[1]?.trim() || "Subtitle" });
  }

  // Parse "add lower third" command
  if (lower.includes("add lower third")) {
    const match = text.match(/add lower[\s-]?third[:\s]+["']?([^"']+)["']?/i);
    clips.push({ type: "lower_third", text: match?.[1]?.trim() || "Lower Third" });
  }

  return clips.length > 0 ? { clips } : null;
}