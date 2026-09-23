import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";

// Exercises the cloud-mode AI command mapping in api/ai-bridge.ts through the
// real bridge: registration (side-effect import), streaming via a mocked
// fetch, and the exact listen() event names/payloads the Tauri commands emit
// (see src-tauri/src/commands/ai.rs).

// Node ≥22 exposes a localStorage global that stays undefined without
// --localstorage-file and shadows jsdom's, so — like cloud.test.ts — we run
// against an in-memory stub. The dynamic import is deliberate: bridge.ts
// computes bridgeMode and reads vplc_* keys at module load, so the stub and
// fetch mock must be installed before the module-loading boundary this test
// intentionally exercises.
function memStorage(): Storage {
  const map = new Map<string, string>();
  return {
    get length() {
      return map.size;
    },
    clear: () => map.clear(),
    getItem: (k) => map.get(k) ?? null,
    key: (i) => [...map.keys()][i] ?? null,
    removeItem: (k) => {
      map.delete(k);
    },
    setItem: (k, v) => {
      map.set(k, String(v));
    },
  };
}

vi.stubGlobal("localStorage", memStorage());

const bridge = await import("../api/bridge");
const fetchMock = vi.fn();
vi.stubGlobal("fetch", fetchMock);

localStorage.setItem("vplc_token", "tok-1");
localStorage.setItem("vplc_base", "http://test-api");

// ai-bridge registers its handlers from a macrotask (queued during the
// cyclic side-effect import); await its readiness signal before invoking.
const { aiCommandsRegistered } = await import("../api/ai-bridge");
await aiCommandsRegistered;

const BASE = "http://test-api";

/** A 200 response whose body drips out one chunk per read() call. */
function streamResponse(chunks: string[]): Response {
  const encoder = new TextEncoder();
  let i = 0;
  return {
    ok: true,
    status: 200,
    body: {
      getReader() {
        return {
          read: async () =>
            i < chunks.length
              ? { done: false, value: encoder.encode(chunks[i++]) }
              : { done: true, value: undefined },
        };
      },
    },
  } as unknown as Response;
}

function errorResponse(status: number, body: string): Response {
  return { ok: false, status, text: async () => body } as unknown as Response;
}

interface EventLog {
  chunks: unknown[];
  doneCount: number;
}

async function trackEvents(): Promise<{ log: EventLog; unlisten: () => void }> {
  const log: EventLog = { chunks: [], doneCount: 0 };
  const offChunk = await bridge.listen("ai-chunk", (e: { payload: unknown }) => {
    log.chunks.push(e.payload);
  });
  const offDone = await bridge.listen("ai-done", () => {
    log.doneCount += 1;
  });
  return {
    log,
    unlisten: () => {
      offChunk();
      offDone();
    },
  };
}

beforeEach(() => {
  fetchMock.mockReset();
  localStorage.clear();
  localStorage.setItem("vplc_token", "tok-1");
  localStorage.setItem("vplc_base", "http://test-api");
});

describe("chat_claude_stream over the cloud proxy", () => {
  const CLAUDE_SSE = [
    'event: message_start\n\n',
    'data: {"type":"message_start","message":{"role":"assistant"}}\n\n',
    'data: {"type":"content_block_delta","delta":{"text":"He"}}\n',
    'data: {"type":"content_block_delta","delta":{"text":"llo"}}\n\n',
    'event: message_stop\n\n',
    'data: {"type":"message_stop"}\n\n',
  ];

  it("emits ai-chunk with chunk text and ai-done, mirroring the Tauri command", async () => {
    // Split mid-line across chunk boundaries to exercise the line buffer.
    fetchMock.mockResolvedValue(streamResponse([CLAUDE_SSE.slice(0, 3).join(""), CLAUDE_SSE.slice(3).join("")]));
    const { log, unlisten } = await trackEvents();

    await bridge.invoke("chat_claude_stream", {
      messages: [
        { role: "system", content: "be brief" },
        { role: "user", content: "hi" },
      ],
      apiKey: "sk-local-key",
      model: "claude-sonnet-4-6",
    });

    expect(log.chunks).toEqual(["He", "llo"]);
    expect(log.doneCount).toBe(1);
    unlisten();
  });

  it("sends the Anthropic body the Tauri command builds (system lifted, key server-side)", async () => {
    fetchMock.mockResolvedValue(streamResponse([CLAUDE_SSE.join("")]));
    await bridge.invoke("chat_claude_stream", {
      messages: [
        { role: "system", content: "be brief" },
        { role: "user", content: "hi" },
      ],
      apiKey: "sk-local-key",
      model: "claude-sonnet-4-6",
    });

    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(url).toBe(`${BASE}/api/ai/claude`);
    expect(init.method).toBe("POST");
    expect((init.headers as Record<string, string>).Authorization).toBe("Bearer tok-1");
    const body = JSON.parse(init.body as string) as Record<string, unknown>;
    expect(body).toEqual({
      model: "claude-sonnet-4-6",
      max_tokens: 4096,
      stream: true,
      messages: [{ role: "user", content: "hi" }],
      system: "be brief",
    });
  });

  it("emits ai-done before rejecting when the SSE carries an error event", async () => {
    fetchMock.mockResolvedValue(
      streamResponse(['data: {"type":"error","error":{"message":"overloaded"}}\n\n']),
    );
    const { log, unlisten } = await trackEvents();

    await expect(
      bridge.invoke("chat_claude_stream", { messages: [{ role: "user", content: "hi" }], model: "m" }),
    ).rejects.toBe("overloaded");
    expect(log.doneCount).toBe(1);
    unlisten();
  });

  it("resolves with ai-done when the stream ends without message_stop", async () => {
    fetchMock.mockResolvedValue(
      streamResponse(['data: {"type":"content_block_delta","delta":{"text":"tail"}}\n']),
    );
    const { log, unlisten } = await trackEvents();
    await bridge.invoke("chat_claude_stream", { messages: [{ role: "user", content: "hi" }], model: "m" });
    expect(log.chunks).toEqual(["tail"]);
    expect(log.doneCount).toBe(1);
    unlisten();
  });

  it("maps 404 no_key to a cloud_unavailable BridgeError without ai-done", async () => {
    fetchMock.mockResolvedValue(errorResponse(404, '{"error":"no_key"}'));
    const { log, unlisten } = await trackEvents();

    const err = await bridge
      .invoke("chat_claude_stream", { messages: [{ role: "user", content: "hi" }], model: "m" })
      .catch((e: unknown) => e);
    expect(err).toBeInstanceOf(bridge.BridgeError);
    expect((err as InstanceType<typeof bridge.BridgeError>).code).toBe("cloud_unavailable");
    expect(log.doneCount).toBe(0);
    unlisten();
  });

  it("maps 402 to 'Upgrade required' and 429 to 'Rate limited'", async () => {
    fetchMock.mockResolvedValue(errorResponse(402, '{"error":"plan_required","requiredPlan":"solo"}'));
    await expect(
      bridge.invoke("chat_claude_stream", { messages: [{ role: "user", content: "hi" }], model: "m" }),
    ).rejects.toBe("Upgrade required");

    fetchMock.mockResolvedValue(errorResponse(429, '{"error":"rate_limited"}'));
    await expect(
      bridge.invoke("chat_claude_stream", { messages: [{ role: "user", content: "hi" }], model: "m" }),
    ).rejects.toBe("Rate limited");
  });

  it("mirrors the Tauri rejection string for other upstream failures", async () => {
    fetchMock.mockResolvedValue(errorResponse(500, "upstream exploded"));
    await expect(
      bridge.invoke("chat_claude_stream", { messages: [{ role: "user", content: "hi" }], model: "m" }),
    ).rejects.toBe("Claude API error 500: upstream exploded");
  });
});

describe("chat_ollama_stream over the cloud proxy", () => {
  it("emits ai-chunk per non-empty token, stops at done:true, skips the rest", async () => {
    fetchMock.mockResolvedValue(
      streamResponse([
        '{"message":{"content":"he"}}\n' +
          '{"message":{"content":""}}\n' +
          '{"message":{"content":"y"},"done":false}\n' +
          '{"done":true}\n' +
          '{"message":{"content":"after done"}}\n',
      ]),
    );
    const { log, unlisten } = await trackEvents();

    await bridge.invoke("chat_ollama_stream", {
      messages: [
        { role: "system", content: "sys" },
        { role: "user", content: "hi" },
      ],
      endpoint: "localhost:11434",
      model: "llama3.1:8b",
    });

    expect(log.chunks).toEqual(["he", "y"]);
    expect(log.doneCount).toBe(1);

    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(url).toBe(`${BASE}/api/ai/ollama`);
    expect(JSON.parse(init.body as string)).toEqual({
      endpoint: "localhost:11434",
      model: "llama3.1:8b",
      messages: [
        { role: "system", content: "sys" },
        { role: "user", content: "hi" },
      ],
      stream: true,
    });
    unlisten();
  });

  it("maps Ollama proxy failures like the Tauri command strings", async () => {
    fetchMock.mockResolvedValue(errorResponse(502, "bad gateway"));
    await expect(
      bridge.invoke("chat_ollama_stream", {
        messages: [{ role: "user", content: "hi" }],
        endpoint: "localhost:11434",
        model: "llama3.1:8b",
      }),
    ).rejects.toBe("Ollama API error 502: bad gateway");
  });
});

describe("registration", () => {
  it("routes both chat commands away from the default cloud mapping", async () => {
    fetchMock.mockResolvedValue(streamResponse(['data: {"type":"message_stop"}\n\n']));
    // Before this module existed both commands fell through to cloudInvoke's
    // "no cloud mapping" BridgeError; now they must reach the proxy fetch.
    await expect(
      bridge.invoke("chat_claude_stream", { messages: [{ role: "user", content: "hi" }], model: "m" }),
    ).resolves.toBeUndefined();
    await expect(
      bridge.invoke("chat_ollama_stream", {
        messages: [{ role: "user", content: "hi" }],
        endpoint: "localhost:11434",
        model: "m",
      }),
    ).resolves.toBeUndefined();
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });
});
