// Cloud-mode mapping for the AI chat commands (PLC-22) plus the REST helpers
// for server-side AI key management.
//
// The panels invoke chat_claude_stream / chat_ollama_stream and consume
// listen() events; the Tauri commands (src-tauri/src/commands/ai.rs) emit
// exactly two events:
//   "ai-chunk" — payload is the chunk TEXT string (Anthropic
//   content_block_delta delta.text / Ollama message.content, non-empty only)
//   "ai-done"  — payload null, emitted on message_stop / done / stream end /
//   stream error, and NOT emitted when the upstream HTTP status is non-2xx
//   (the command rejects instead; panels render `Error: <rejection>`).
// Tauri rejections are strings, so cloud-mode handler rejections are strings
// too (except BridgeError, which the panels render via toString).
//
// The chat proxies ride the cloud worker (apps/voltrus-plc-cloud
// src/routes/ai.ts): POST /api/ai/claude relays the Anthropic messages body
// verbatim (the worker injects the stored x-api-key), POST /api/ai/ollama
// strips `endpoint` and relays the rest. The apiKey argument the panels pass
// is intentionally ignored in cloud mode — keys live server-side (AI Keys
// panel). Key management REST mirrors cloud-auth.ts conventions (Bearer
// session token, JSON error bodies as CloudApiError).

import { CloudApiError, cloudApiBase } from "./cloud-auth";
import { BridgeError, emitLocalEvent, registerCloudCommands } from "./bridge-core";

export interface CloudAiKey {
  provider: string;
  masked: string;
  updatedAt: string;
}

export type AiProvider = "claude" | "ollama";

interface ChatMessageArg {
  role: string;
  content: string;
}

/** The panels pass camelCase args (no Tauri snake_case rewrite in cloud mode). */
function messagesArg(args: Record<string, unknown> | undefined, cmd: string): ChatMessageArg[] {
  const raw = args?.messages;
  if (!Array.isArray(raw)) {
    throw new BridgeError("bad_request", cmd, `Command ${cmd} requires a 'messages' array`);
  }
  return raw.map((m) => {
    const msg = m as Record<string, unknown>;
    if (typeof msg?.role !== "string" || typeof msg?.content !== "string") {
      throw new BridgeError("bad_request", cmd, `Command ${cmd} messages need string role/content`);
    }
    return { role: msg.role, content: msg.content };
  });
}

function strCmdArg(args: Record<string, unknown> | undefined, key: string, cmd: string): string {
  const v = args?.[key];
  if (typeof v !== "string") {
    throw new BridgeError("bad_request", cmd, `Command ${cmd} requires a string '${key}' argument`);
  }
  return v;
}

async function openStream(cmd: string, res: Response, label: "Claude" | "Ollama"): Promise<Response> {
  if (res.ok) return res;
  const text = await res.text().catch(() => "");
  let serverError: string | null = null;
  try {
    const body = JSON.parse(text) as { error?: unknown };
    if (body && typeof body.error === "string") serverError = body.error;
  } catch {
    // non-JSON error body — fall through to the status/text message
  }
  if (res.status === 402) throw "Upgrade required";
  if (res.status === 429) throw "Rate limited";
  if (res.status === 401 || res.status === 403) {
    throw new BridgeError("unauthorized", cmd, `HTTP ${res.status}`);
  }
  if (res.status === 404 && serverError === "no_key") {
    throw new BridgeError(
      "cloud_unavailable",
      cmd,
      "No API key configured on the server — add one under Cloud → AI Keys",
    );
  }
  // Mirror the Tauri commands' non-2xx rejection string.
  throw `${label} API error ${res.status}: ${text}`;
}

function authHeader(): Record<string, string> {
  const token = localStorage.getItem("vplc_token");
  return token ? { Authorization: `Bearer ${token}` } : {};
}

async function fetchStream(cmd: string, path: string, body: unknown): Promise<Response> {
  let res: Response;
  try {
    res = await fetch(`${cloudApiBase()}${path}`, {
      method: "POST",
      headers: { ...authHeader(), "Content-Type": "application/json" },
      body: JSON.stringify(body),
    });
  } catch (e) {
    throw new BridgeError("network", cmd, e instanceof Error ? e.message : String(e));
  }
  return openStream(cmd, res, cmd === "chat_claude_stream" ? "Claude" : "Ollama");
}

// Same SSE handling as chat_claude_stream: skip empty and `event:` lines,
// parse `data: ` JSON, emit ai-chunk per content_block_delta, stop on
// message_stop, emit ai-done before rejecting on an SSE error event.
async function chatClaudeStream(cmd: string, args?: Record<string, unknown>): Promise<void> {
  const messages = messagesArg(args, cmd);
  const model = strCmdArg(args, "model", cmd);

  let system: string | undefined;
  const anthropicMessages: ChatMessageArg[] = [];
  for (const msg of messages) {
    if (msg.role === "system") system = msg.content;
    else anthropicMessages.push(msg);
  }
  const body: Record<string, unknown> = {
    model,
    max_tokens: 4096,
    stream: true,
    messages: anthropicMessages,
  };
  if (system !== undefined) body.system = system;

  const res = await fetchStream(cmd, "/api/ai/claude", body);
  const reader = res.body?.getReader();
  if (!reader) throw "Claude API error: empty response body";

  const decoder = new TextDecoder();
  let buffer = "";
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    buffer += decoder.decode(value, { stream: true });

    for (;;) {
      const lineEnd = buffer.indexOf("\n");
      if (lineEnd === -1) break;
      const line = buffer.slice(0, lineEnd).trim();
      buffer = buffer.slice(lineEnd + 1);

      if (line.length === 0 || line.startsWith("event:")) continue;
      if (!line.startsWith("data: ")) continue;
      let event: {
        type?: unknown;
        delta?: { text?: unknown };
        error?: { message?: unknown };
      };
      try {
        event = JSON.parse(line.slice("data: ".length));
      } catch {
        continue;
      }
      if (event.type === "content_block_delta") {
        if (typeof event.delta?.text === "string") emitLocalEvent("ai-chunk", event.delta.text);
      } else if (event.type === "message_stop") {
        emitLocalEvent("ai-done", null);
        return;
      } else if (event.type === "error") {
        const msg =
          typeof event.error?.message === "string" ? event.error.message : "Unknown error";
        emitLocalEvent("ai-done", null);
        throw msg;
      }
    }
  }
  emitLocalEvent("ai-done", null);
}

// Same NDJSON handling as chat_ollama_stream: one JSON object per line,
// ai-chunk per non-empty message.content, stop on done:true.
async function chatOllamaStream(cmd: string, args?: Record<string, unknown>): Promise<void> {
  const messages = messagesArg(args, cmd);
  const endpoint = strCmdArg(args, "endpoint", cmd);
  const model = strCmdArg(args, "model", cmd);

  const res = await fetchStream(cmd, "/api/ai/ollama", {
    endpoint,
    model,
    messages,
    stream: true,
  });
  const reader = res.body?.getReader();
  if (!reader) throw "Ollama API error: empty response body";

  const decoder = new TextDecoder();
  let buffer = "";
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    buffer += decoder.decode(value, { stream: true });

    for (;;) {
      const lineEnd = buffer.indexOf("\n");
      if (lineEnd === -1) break;
      const line = buffer.slice(0, lineEnd).trim();
      buffer = buffer.slice(lineEnd + 1);

      if (line.length === 0) continue;
      let event: { message?: { content?: unknown }; done?: unknown };
      try {
        event = JSON.parse(line);
      } catch {
        continue;
      }
      if (typeof event.message?.content === "string" && event.message.content.length > 0) {
        emitLocalEvent("ai-chunk", event.message.content);
      }
      if (event.done === true) {
        emitLocalEvent("ai-done", null);
        return;
      }
    }
  }
  emitLocalEvent("ai-done", null);
}

registerCloudCommands({
  chat_claude_stream: chatClaudeStream,
  chat_ollama_stream: chatOllamaStream,
});

// --- AI key management REST (mirrors cloud-auth.ts request conventions) ---

async function aiRequest<T>(path: string, init: RequestInit, token: string): Promise<T> {
  let res: Response;
  try {
    res = await fetch(`${cloudApiBase()}${path}`, {
      ...init,
      headers: {
        ...(init.body ? { "Content-Type": "application/json" } : {}),
        Authorization: `Bearer ${token}`,
      },
    });
  } catch (e) {
    throw new CloudApiError(0, e instanceof Error ? e.message : String(e));
  }
  if (!res.ok) {
    let message = `HTTP ${res.status}`;
    try {
      const body = (await res.json()) as { error?: unknown };
      if (typeof body.error === "string") message = body.error;
    } catch {
      // non-JSON error body — keep the HTTP status fallback
    }
    throw new CloudApiError(res.status, message);
  }
  return (await res.json()) as T;
}

export function listAiKeys(token: string): Promise<CloudAiKey[]> {
  return aiRequest("/api/ai/keys", {}, token);
}

export function putAiKey(
  token: string,
  provider: AiProvider,
  key: string,
): Promise<{ provider: string; masked: string }> {
  return aiRequest("/api/ai/keys", { method: "PUT", body: JSON.stringify({ provider, key }) }, token);
}

export async function removeAiKey(token: string, provider: AiProvider): Promise<void> {
  await aiRequest(`/api/ai/keys/${encodeURIComponent(provider)}`, { method: "DELETE" }, token);
}
