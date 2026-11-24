// AI route tests. fetchMock is unavailable in vitest-pool-workers 0.22, so the
// upstream boundary is a FetchLike injected via createAiRoutes() and stubbed
// here; everything else (auth, D1 keys, rate limiting) runs against real
// bindings from cloudflare:test via Hono's app.request(env).

import { describe, expect, it } from "vitest";
import { env } from "cloudflare:test";
import { Hono } from "hono";
import { createAiRoutes, type FetchLike } from "../src/routes/ai";
import { issueToken } from "../src/auth";
import { setPlanForTest } from "../src/entitlements";
import type { AppEnv, Env } from "../src/env";

const testEnv = env as unknown as Env;
const JSON_HEADERS = { "Content-Type": "application/json" };
const CLAUDE_KEY = "sk-ant-api03-totally-secret-value-9f2e";
const OLLAMA_KEY = "ollama-token-abcd1234";

type StubCall = { url: string; init?: RequestInit };

function stubFetch(handler: (url: string, init?: RequestInit) => Response): FetchLike & { calls: StubCall[] } {
  const calls: StubCall[] = [];
  const fn = async (input: string | URL | Request, init?: RequestInit): Promise<Response> => {
    calls.push({ url: input.toString(), init });
    return handler(input.toString(), init);
  };
  return Object.assign(fn, { calls });
}

/** Chunked upstream response; chunks arrive as separate stream enqueues. */
function streamResponse(chunks: string[], init: { status?: number; contentType: string }): Response {
  const encoder = new TextEncoder();
  let index = 0;
  const stream = new ReadableStream<Uint8Array>({
    pull(controller) {
      if (index < chunks.length) {
        controller.enqueue(encoder.encode(chunks[index++]!));
      } else {
        controller.close();
      }
    },
  });
  return new Response(stream, { status: init.status ?? 200, headers: { "Content-Type": init.contentType } });
}

async function readAll(res: Response): Promise<string> {
  const reader = res.body!.getReader();
  const decoder = new TextDecoder();
  let text = "";
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    text += decoder.decode(value, { stream: true });
  }
  return text;
}

function initBody(init?: RequestInit): string {
  const body = init!.body;
  if (typeof body === "string") return body;
  return new TextDecoder().decode(body as ArrayBuffer);
}

async function authedUser(): Promise<{ userId: string; token: string }> {
  const userId = crypto.randomUUID();
  // The proxy routes are solo-gated; provision a user row (FK) + solo entitlement.
  await testEnv.DB.prepare(
    "INSERT INTO users (id, email, password_hash, salt, created_at) VALUES (?, ?, 'x', 'x', ?)",
  )
    .bind(userId, `ai-${userId}@example.com`, new Date().toISOString())
    .run();
  await setPlanForTest(testEnv.DB, userId, "solo");
  return { userId, token: await issueToken(testEnv.JWT_SECRET, userId) };
}

function makeApp(fetchLike: FetchLike = stubFetch(() => new Response("upstream miss", { status: 599 }))): Hono<AppEnv> {
  const app = new Hono<AppEnv>();
  app.route("/api/ai", createAiRoutes(fetchLike));
  return app;
}

function call(
  app: Hono<AppEnv>,
  token: string,
  method: string,
  path: string,
  body?: unknown,
): Promise<Response> {
  return Promise.resolve(app.request("/api/ai" + path, {
    method,
    headers: {
      Authorization: `Bearer ${token}`,
      ...(body === undefined ? {} : JSON_HEADERS),
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  }, testEnv));
}

async function storeKey(userId: string, provider: "claude" | "ollama", key: string): Promise<void> {
  await makeApp().request("/api/ai/keys", {
    method: "PUT",
    headers: { Authorization: `Bearer ${await issueToken(testEnv.JWT_SECRET, userId)}`, ...JSON_HEADERS },
    body: JSON.stringify({ provider, key }),
  }, testEnv);
}

describe("auth", () => {
  it("rejects key endpoints without a bearer token", async () => {
    const app = makeApp();
    const get = await app.request("/api/ai/keys");
    expect(get.status).toBe(401);
    const put = await app.request("/api/ai/keys", { method: "PUT", headers: JSON_HEADERS, body: "{}" });
    expect(put.status).toBe(401);
    const claude = await app.request("/api/ai/claude", { method: "POST" });
    expect(claude.status).toBe(401);
  });
});

describe("key management", () => {
  it("upserts a key and lists masked-only entries", async () => {
    const { token } = await authedUser();
    const app = makeApp();

    const put = await call(app, token, "PUT", "/keys", { provider: "claude", key: CLAUDE_KEY });
    expect(put.status).toBe(200);
    const putText = await put.text();
    expect(JSON.parse(putText)).toEqual({ provider: "claude", masked: "sk-ant..." + CLAUDE_KEY.slice(-4) });
    expect(putText).not.toContain(CLAUDE_KEY);

    const list = await call(app, token, "GET", "/keys");
    expect(list.status).toBe(200);
    const listText = await list.text();
    const parsed = JSON.parse(listText) as { provider: string; masked: string; updatedAt: string }[];
    expect(parsed).toEqual([
      { provider: "claude", masked: "sk-ant..." + CLAUDE_KEY.slice(-4), updatedAt: expect.any(String) },
    ]);
    expect(parsed[0]!.updatedAt).toMatch(/^\d{4}-\d{2}-\d{2}T/);
    expect(listText).not.toContain(CLAUDE_KEY);
  });

  it("stores ciphertext, never plaintext, in D1", async () => {
    const { userId, token } = await authedUser();
    await call(makeApp(), token, "PUT", "/keys", { provider: "claude", key: CLAUDE_KEY });
    const row = await testEnv.DB.prepare("SELECT key_encrypted, key_iv, masked FROM ai_keys WHERE user_id = ? AND provider = 'claude'")
      .bind(userId)
      .first<{ key_encrypted: string; key_iv: string; masked: string }>();
    expect(row).not.toBeNull();
    expect(row!.key_encrypted).not.toContain(CLAUDE_KEY);
    expect(row!.masked).not.toContain(CLAUDE_KEY);
  });

  it("re-upserting replaces the stored key", async () => {
    const { token } = await authedUser();
    const app = makeApp();
    await call(app, token, "PUT", "/keys", { provider: "claude", key: CLAUDE_KEY });
    const rotated = "sk-ant-rotated-key-777";
    await call(app, token, "PUT", "/keys", { provider: "claude", key: rotated });

    const parsed = (await (await call(app, token, "GET", "/keys")).json()) as { provider: string; masked: string }[];
    expect(parsed).toHaveLength(1);
    expect(parsed[0]!.provider).toBe("claude");
    expect(parsed[0]!.masked).toBe("sk-ant..." + rotated.slice(-4));
  });

  it("validates provider and key", async () => {
    const { token } = await authedUser();
    const app = makeApp();
    const unknownProvider = await call(app, token, "PUT", "/keys", { provider: "gpt", key: "x" });
    expect(unknownProvider.status).toBe(400);
    const missingKey = await call(app, token, "PUT", "/keys", { provider: "claude" });
    expect(missingKey.status).toBe(400);
    const emptyKey = await call(app, token, "PUT", "/keys", { provider: "claude", key: "   " });
    expect(emptyKey.status).toBe(400);
    const notObject = await call(app, token, "PUT", "/keys", ["claude"]);
    expect(notObject.status).toBe(400);
  });

  it("lists an empty array when no keys are stored", async () => {
    const { token } = await authedUser();
    const res = await call(makeApp(), token, "GET", "/keys");
    expect(res.status).toBe(200);
    expect(await res.json()).toEqual([]);
  });

  it("deletes an existing key and 404s otherwise", async () => {
    const { token } = await authedUser();
    const app = makeApp();
    await call(app, token, "PUT", "/keys", { provider: "claude", key: CLAUDE_KEY });

    const del = await call(app, token, "DELETE", "/keys/claude");
    expect(del.status).toBe(200);
    expect(await del.json()).toEqual({ ok: true });
    expect(await (await call(app, token, "GET", "/keys")).json()).toEqual([]);

    const again = await call(app, token, "DELETE", "/keys/claude");
    expect(again.status).toBe(404);
    expect(await again.json()).toEqual({ error: "no_key" });

    const unknown = await call(app, token, "DELETE", "/keys/gpt");
    expect(unknown.status).toBe(404);
  });
});

describe("POST /claude", () => {
  it("returns no_key without touching upstream when no key is stored", async () => {
    const { token } = await authedUser();
    const upstream = stubFetch(() => new Response("{}"));
    const res = await call(makeApp(upstream), token, "POST", "/claude", { model: "claude-sonnet-4-5" });
    expect(res.status).toBe(404);
    expect(await res.json()).toEqual({ error: "no_key" });
    expect(upstream.calls).toEqual([]);
  });

  it("forwards the body with server-side auth headers and relays the SSE stream in order", async () => {
    const { userId, token } = await authedUser();
    await storeKey(userId, "claude", CLAUDE_KEY);
    const upstream = stubFetch(() =>
      streamResponse(
        ['event: message_start\ndata: {"type":"message_start"}\n\n', 'data: {"type":"content_block_delta"}\n\n', "data: [DONE]\n\n"],
        { contentType: "text/event-stream" },
      ),
    );
    const requestBody = { model: "claude-sonnet-4-5", stream: true, messages: [{ role: "user", content: "hi" }] };
    const res = await call(makeApp(upstream), token, "POST", "/claude", requestBody);

    expect(res.status).toBe(200);
    expect(res.headers.get("content-type")).toBe("text/event-stream");
    expect(await readAll(res)).toBe(
      'event: message_start\ndata: {"type":"message_start"}\n\ndata: {"type":"content_block_delta"}\n\ndata: [DONE]\n\n',
    );

    expect(upstream.calls.length).toBe(1);
    const { url, init } = upstream.calls[0]!;
    expect(url).toBe("https://api.anthropic.com/v1/messages");
    expect(init!.method).toBe("POST");
    const headers = new Headers(init!.headers);
    expect(headers.get("x-api-key")).toBe(CLAUDE_KEY);
    expect(headers.get("anthropic-version")).toBe("2023-06-01");
    expect(JSON.parse(initBody(init))).toEqual(requestBody);
  });

  it("relays upstream error statuses and bodies", async () => {
    const { userId, token } = await authedUser();
    await storeKey(userId, "claude", CLAUDE_KEY);
    const upstreamError = { type: "error", error: { type: "authentication_error", message: "invalid x-api-key" } };
    const upstream = stubFetch(() => new Response(JSON.stringify(upstreamError), { status: 401, headers: { "Content-Type": "application/json" } }));
    const res = await call(makeApp(upstream), token, "POST", "/claude", { model: "claude-sonnet-4-5", messages: [] });

    expect(res.status).toBe(401);
    expect(await res.json()).toEqual(upstreamError);
  });
});

describe("rate limiting", () => {
  it("429s after 20 proxied requests in a window, per user", async () => {
    const { userId, token } = await authedUser();
    await storeKey(userId, "claude", CLAUDE_KEY);
    const upstream = stubFetch(() => new Response('{"ok":true}', { headers: { "Content-Type": "application/json" } }));
    const app = makeApp(upstream);

    for (let i = 0; i < 20; i++) {
      const res = await call(app, token, "POST", "/claude", { model: "claude-sonnet-4-5", messages: [] });
      expect(res.status).toBe(200);
    }
    expect(upstream.calls.length).toBe(20);

    const over = await call(app, token, "POST", "/claude", { model: "claude-sonnet-4-5", messages: [] });
    expect(over.status).toBe(429);
    expect(await over.json()).toEqual({ error: "rate_limited" });
    expect(upstream.calls.length).toBe(20);

    // Another user's window is independent.
    const other = await authedUser();
    await storeKey(other.userId, "claude", CLAUDE_KEY);
    const otherRes = await call(app, other.token, "POST", "/claude", { model: "claude-sonnet-4-5", messages: [] });
    expect(otherRes.status).toBe(200);
    expect(upstream.calls.length).toBe(21);
  });

  it("does not count key management against the proxy budget", async () => {
    const { userId, token } = await authedUser();
    const upstream = stubFetch(() => new Response("{}"));
    const app = makeApp(upstream);
    await call(app, token, "PUT", "/keys", { provider: "claude", key: CLAUDE_KEY });
    await call(app, token, "GET", "/keys");
    // Management calls left the full 20-request budget intact...
    for (let i = 0; i < 20; i++) {
      const res = await call(app, token, "POST", "/claude", { model: "m" });
      expect(res.status).toBe(200);
    }
    expect(upstream.calls.length).toBe(20);
    // ...so only the 21st proxy call trips the limit.
    const over = await call(app, token, "POST", "/claude", { model: "m" });
    expect(over.status).toBe(429);
    expect(await over.json()).toEqual({ error: "rate_limited" });
    expect(upstream.calls.length).toBe(20);
    expect(await testEnv.DB.prepare("SELECT count FROM ai_usage WHERE user_id = ?").bind(userId).first<{ count: number }>()).toEqual({ count: 21 });
  });
});

describe("POST /ollama", () => {
  const SSRF_BLOCKED = [
    "http://localhost:11434",
    "http://localhost",
    "http://something.localhost:8080",
    "http://127.0.0.1:1234",
    "http://127.255.0.7",
    "http://10.0.0.5:11434",
    "http://10.255.255.255",
    "http://172.16.1.1",
    "http://172.31.255.254",
    "http://192.168.0.20:11434",
    "http://192.168.255.1",
    "http://my-box.internal/api",
    "http://0.0.0.0:11434",
    "http://[::1]:11434",
    "http://[::ffff:127.0.0.1]:11434",
    "ftp://77.10.20.30",
    "not a url",
    "http://",
  ];

  it.each(SSRF_BLOCKED)("rejects unsafe endpoint %s", async (endpoint) => {
    const { token } = await authedUser();
    const upstream = stubFetch(() => new Response("{}"));
    const res = await call(makeApp(upstream), token, "POST", "/ollama", { endpoint, model: "llama3", messages: [] });
    expect(res.status).toBe(400);
    expect(upstream.calls).toEqual([]);
  });

  it("requires a JSON object body with an endpoint", async () => {
    const { token } = await authedUser();
    const upstream = stubFetch(() => new Response("{}"));
    const app = makeApp(upstream);
    expect((await call(app, token, "POST", "/ollama", { model: "llama3" })).status).toBe(400);
    expect((await call(app, token, "POST", "/ollama", [1])).status).toBe(400);
    expect((await call(app, token, "POST", "/ollama", "http://77.10.20.30")).status).toBe(400);
    expect(upstream.calls).toEqual([]);
  });

  it("forwards to {endpoint}/api/chat without the endpoint field and streams the reply", async () => {
    const { userId, token } = await authedUser();
    await storeKey(userId, "claude", CLAUDE_KEY); // unrelated provider must not leak into ollama
    const upstream = stubFetch(() =>
      streamResponse(['{"message":{"role":"assistant"},"done":false}\n', '{"message":{"role":"assistant"},"done":true}\n'], {
        contentType: "application/x-ndjson",
      }),
    );
    const chatBody = { model: "llama3", stream: true, messages: [{ role: "user", content: "hi" }] };
    const res = await call(makeApp(upstream), token, "POST", "/ollama", {
      endpoint: "http://77.10.20.30:11434/",
      ...chatBody,
    });

    expect(res.status).toBe(200);
    expect(res.headers.get("content-type")).toBe("application/x-ndjson");
    expect(await readAll(res)).toBe('{"message":{"role":"assistant"},"done":false}\n{"message":{"role":"assistant"},"done":true}\n');

    expect(upstream.calls.length).toBe(1);
    const { url, init } = upstream.calls[0]!;
    expect(url).toBe("http://77.10.20.30:11434/api/chat");
    expect(new Headers(init!.headers).get("authorization")).toBeNull();
    expect(JSON.parse(initBody(init))).toEqual(chatBody);
  });

  it("attaches the stored ollama key as a bearer token", async () => {
    const { userId, token } = await authedUser();
    await storeKey(userId, "ollama", OLLAMA_KEY);
    const upstream = stubFetch(() => new Response("{}"));
    await call(makeApp(upstream), token, "POST", "/ollama", { endpoint: "https://ollama.example.com", model: "llama3" });

    expect(upstream.calls[0]!.url).toBe("https://ollama.example.com/api/chat");
    expect(new Headers(upstream.calls[0]!.init!.headers).get("authorization")).toBe(`Bearer ${OLLAMA_KEY}`);
  });

  it("relays upstream error statuses", async () => {
    const { userId, token } = await authedUser();
    await storeKey(userId, "ollama", OLLAMA_KEY);
    const upstream = stubFetch(() => new Response('{"error":"model not found"}', { status: 404 }));
    const res = await call(makeApp(upstream), token, "POST", "/ollama", { endpoint: "http://77.10.20.30:11434", model: "missing" });
    expect(res.status).toBe(404);
    expect(await res.json()).toEqual({ error: "model not found" });
  });
});
