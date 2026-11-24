import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  RelayClient,
  resetRelaySocketFactory,
  setRelaySocketFactory,
  type WebSocketFactory,
} from "../api/relay-client";

// jsdom has no usable WebSocket, so every test injects this fake and drives
// it by hand (open / receive / fail).
class FakeWebSocket {
  static instances: FakeWebSocket[] = [];

  url: string;
  readyState = 0; // CONNECTING
  sent: string[] = [];
  closed = false;
  onopen: (() => void) | null = null;
  onmessage: ((e: { data: string }) => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;

  constructor(url: string) {
    this.url = url;
    FakeWebSocket.instances.push(this);
  }

  static last(): FakeWebSocket {
    const ws = FakeWebSocket.instances.at(-1);
    if (!ws) throw new Error("no socket created");
    return ws;
  }

  send(data: string): void {
    this.sent.push(data);
  }

  close(): void {
    this.closed = true;
    this.readyState = 3;
    this.onclose?.();
  }

  open(): void {
    this.readyState = 1;
    this.onopen?.();
  }

  receive(frame: unknown): void {
    this.onmessage?.({ data: JSON.stringify(frame) });
  }

  lastSent(): { id: string; op: string; payload: unknown } {
    const raw = this.sent.at(-1);
    if (!raw) throw new Error("nothing sent");
    return JSON.parse(raw);
  }
}

const factory: WebSocketFactory = (url) => new FakeWebSocket(url) as unknown as WebSocket;

// Fake timers make promise-chain draining deterministic (no wall-clock waits)
// and let the request-timeout test jump the clock instead of sleeping.
// queueMicrotask stays real: pumping awaits drain the genuine microtask queue.
const pump = () => vi.advanceTimersByTimeAsync(0);

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "setInterval", "clearInterval"] });
  FakeWebSocket.instances = [];
  setRelaySocketFactory(factory);
});

afterEach(() => {
  vi.useRealTimers();
  resetRelaySocketFactory();
});

function makeClient(): RelayClient {
  return new RelayClient({ base: "http://test-api", requestTimeoutMs: 200 });
}

/** connect + open, returning the driven socket ready to receive frames. */
async function connected(client: RelayClient): Promise<FakeWebSocket> {
  const opening = client.connect("dev-1", "tok-1");
  const ws = FakeWebSocket.last();
  ws.open();
  await opening;
  return ws;
}

describe("RelayClient", () => {
  it("connects to the ide endpoint with the device id and token in the query", async () => {
    const client = makeClient();
    await connected(client);
    expect(FakeWebSocket.last().url).toBe("ws://test-api/api/relay/dev-1?role=ide&token=tok-1");
    client.close();
  });

  it("rejects connect when the socket errors before opening", async () => {
    const client = makeClient();
    const p = client.connect("dev-1", "tok-1");
    FakeWebSocket.last().onerror?.();
    await expect(p).rejects.toThrow("websocket error");
  });

  it("rejects request when not connected", async () => {
    const client = makeClient();
    await expect(client.request("status")).rejects.toThrow("not connected");
  });

  it("correlates responses by id and resolves the payload", async () => {
    const client = makeClient();
    const ws = await connected(client);

    const p = client.request("browse");
    const frame = ws.lastSent();
    expect(frame.op).toBe("browse");
    expect(frame.id).toBe("r1");

    ws.receive({ id: "r1", ok: true, payload: { variables: [] } });
    await expect(p).resolves.toEqual({ variables: [] });
    client.close();
  });

  it("rejects with the server error on ok:false (device offline propagation)", async () => {
    const client = makeClient();
    const ws = await connected(client);

    const p = client.request("deploy");
    ws.receive({ id: "r1", ok: false, error: "device offline" });
    await expect(p).rejects.toThrow("device offline");
    client.close();
  });

  it("times out a request that never gets a response", async () => {
    const client = makeClient();
    await connected(client);

    // Attach the rejection handler before firing the timer, so vitest never
    // sees the rejection as unhandled mid-advance.
    const p = expect(client.request("status")).rejects.toThrow("timed out");
    await vi.advanceTimersByTimeAsync(201);
    await p;
    client.close();
  });

  it("ignores DO heartbeat frames and still routes the real response", async () => {
    const client = makeClient();
    const ws = await connected(client);

    let settled = false;
    const p = client.request("status").then(
      (v) => {
        settled = true;
        return v;
      },
      () => {
        settled = true;
      },
    );
    ws.receive({ op: "ping" });
    await pump();
    expect(settled).toBe(false);

    ws.receive({ id: "r1", ok: true, payload: { running: true } });
    await expect(p).resolves.toEqual({ running: true });
    client.close();
  });

  it("fans unsolicited device events out to listeners", async () => {
    const client = makeClient();
    const ws = await connected(client);

    const seen: unknown[] = [];
    const unlisten = client.onEvent((e) => seen.push(e));
    ws.receive({ deviceId: "dev-1", event: "variable_update", payload: { name: "x", value: 1 } });
    expect(seen).toEqual([{ deviceId: "dev-1", event: "variable_update", payload: { name: "x", value: 1 } }]);

    unlisten();
    ws.receive({ deviceId: "dev-1", event: "variable_update", payload: { name: "x", value: 2 } });
    expect(seen).toHaveLength(1);
    client.close();
  });

  it("rejects pending requests and notifies close listeners when the socket drops", async () => {
    const client = makeClient();
    const ws = await connected(client);

    let closed = false;
    client.onClose(() => {
      closed = true;
    });
    const p = client.request("status");
    ws.onclose?.();
    await expect(p).rejects.toThrow("connection closed");
    expect(closed).toBe(true);
  });

  it("rejects pending requests on close()", async () => {
    const client = makeClient();
    const ws = await connected(client);

    const p = client.request("status");
    client.close();
    await expect(p).rejects.toThrow("closed");
    expect(ws.closed).toBe(true);
  });
});
