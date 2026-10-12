import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke, listen } from "../api/bridge";
// Side-effect import: registers the relay command handlers on module load.
import "../api/relay-bridge";
import { resetRelaySocketFactory, setRelaySocketFactory, type WebSocketFactory } from "../api/relay-client";
import type { DeployResult, TargetStatus } from "../types";

// jsdom has no usable WebSocket, so tests inject this fake and drive it by
// hand (open / receive / fail).
class FakeWebSocket {
  static instances: FakeWebSocket[] = [];

  url: string;
  readyState = 0;
  sent: string[] = [];
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

// Fake timers drain promise chains deterministically; the request timeout
// never has to be waited out in real time. queueMicrotask stays real: pumping
// awaits drain the genuine microtask queue, which is what flushes
// relay-bridge's deferred command registration.
const pump = () => vi.advanceTimersByTimeAsync(0);

// Node/jsdom localStorage is shared across tests in the file; cloud-mode
// tests run against an in-memory stub for isolation.
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

/** Connect through the bridge and return the driven socket. */
async function connectFakeDevice(): Promise<FakeWebSocket> {
  const connecting = invoke("target_connect", { ip: "1.2.3.4" });
  const ws = FakeWebSocket.last();
  ws.open();
  await pump();
  ws.receive({ id: "r1", ok: true, payload: { driver: "echo", running: true } });
  await connecting;
  return ws;
}

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "setInterval", "clearInterval"] });
  vi.stubGlobal("localStorage", memStorage());
  localStorage.setItem("vplc_base", "http://test-api");
  localStorage.setItem("vplc_token", "tok-1");
  localStorage.setItem("vplc_device", "dev-1");
  setRelaySocketFactory(factory);
});

afterEach(async () => {
  await invoke("target_disconnect").catch(() => {});
  vi.unstubAllGlobals();
  vi.useRealTimers();
  resetRelaySocketFactory();
});

describe("relay bridge", () => {
  it("target_connect opens the ide socket and maps the status payload", async () => {
    const p = invoke<TargetStatus>("target_connect", { ip: "1.2.3.4" });
    const ws = FakeWebSocket.last();
    expect(ws.url).toBe("ws://test-api/api/relay/dev-1?role=ide&token=tok-1");

    ws.open();
    await pump();
    expect(ws.lastSent()).toEqual({ id: "r1", op: "status", payload: null });
    ws.receive({ id: "r1", ok: true, payload: { driver: "echo", running: true, variables: 4 } });

    await expect(p).resolves.toEqual({
      connected: true,
      ip: "dev-1",
      runtime_state: "running",
      cycle_count: null,
      deployed_crc32: null,
      running_crc32: null,
    });
  });

  it("target_connect maps the grpc driver status shape", async () => {
    const p = invoke<TargetStatus>("target_connect", {});
    const ws = FakeWebSocket.last();
    ws.open();
    await pump();
    ws.receive({
      id: "r1",
      ok: true,
      payload: {
        driver: "grpc",
        addr: "http://10.0.0.1:50051",
        runtime: { status: "running", cycle_count: 42 },
        connections: 2,
        deployed_crc32: 3735928559,
      },
    });

    await expect(p).resolves.toEqual({
      connected: true,
      ip: "dev-1",
      runtime_state: "running",
      cycle_count: 42,
      deployed_crc32: null, // no deploy in this session yet
      running_crc32: 3735928559,
    });
  });

  it("target_connect fails with a network BridgeError when the socket cannot open", async () => {
    const p = invoke<TargetStatus>("target_connect", {});
    FakeWebSocket.last().onerror?.();
    await expect(p).rejects.toThrow("relay: websocket error");
  });

  it("target_status rejects when not connected", async () => {
    await expect(invoke("target_status")).rejects.toThrow("Not connected to a relay device");
  });

  it("deploy sends the {module_json, config_json} envelope and maps the result", async () => {
    const ws = await connectFakeDevice();
    const module = { name: "Main", instructions: [{ Load: 1 }], variable_slots: [["x", { Dint: 1 }]] };
    const p = invoke<DeployResult>("deploy_to_target", {
      ip: "ignored-over-relay",
      ioMappings: [
        { variable: "x", direction: "input", address: "%IW0", driver_id: "modbus-tcp", protocol_address: '{"host":"10.0.0.5"}' },
      ],
      module,
    });
    await pump();

    const frame = ws.lastSent();
    expect(frame.id).toBe("r2");
    expect(frame.op).toBe("deploy");
    expect(frame.payload).toEqual({
      module_json: JSON.stringify(module),
      config_json: JSON.stringify({
        io_mappings: [{ variable_name: "x", address: "%IW0", driver: "modbus-tcp", connection_json: '{"host":"10.0.0.5"}' }],
      }),
    });

    ws.receive({ id: "r2", ok: true, payload: { deployed: true, bytes: 128, crc32: 4029621930 } });
    await expect(p).resolves.toEqual({
      success: true,
      message: "Deployed via relay",
      byte_count: 128,
      crc32: 4029621930,
    });

    // The deploy fingerprint becomes the project-side CRC for drift polling.
    const status = invoke<TargetStatus>("target_status");
    await pump();
    FakeWebSocket.last().receive({ id: "r3", ok: true, payload: { driver: "echo", running: true, deployed_crc32: 4029621930 } });
    await expect(status).resolves.toMatchObject({ deployed_crc32: 4029621930, running_crc32: 4029621930 });
  });

  it("deploy propagates device_offline from the ok:false response", async () => {
    const ws = await connectFakeDevice();
    const p = invoke<DeployResult>("deploy_via_grpc", { module: { name: "Main" } });
    await pump();
    ws.receive({ id: "r2", ok: false, error: "device offline" });
    await expect(p).rejects.toThrow("device offline");
  });

  it("deploy without a connection rejects instead of silently failing", async () => {
    await expect(invoke("deploy_to_target", { module: { name: "Main" } })).rejects.toThrow(
      "Not connected to a relay device",
    );
  });

  it("ignores DO heartbeat frames while a request is in flight", async () => {
    const ws = await connectFakeDevice();
    let settled = false;
    const p = invoke("target_status").then(
      (v) => {
        settled = true;
        return v;
      },
      () => {
        settled = true;
      },
    );
    await pump();
    ws.receive({ op: "ping" });
    await pump();
    expect(settled).toBe(false);

    ws.receive({ id: "r2", ok: true, payload: { driver: "echo", running: false } });
    await p;
  });

  it("routes unsolicited device events into the local event registry", async () => {
    const ws = await connectFakeDevice();
    const seen: unknown[] = [];
    const unlisten = await listen("variable_update", (e) => seen.push(e.payload));

    ws.receive({ deviceId: "dev-1", event: "variable_update", payload: { name: "x", value: 7 } });
    expect(seen).toEqual([{ name: "x", value: 7 }]);
    unlisten();
  });

  it("start/stop/step are rejected as unsupported over the relay", async () => {
    await connectFakeDevice();
    for (const cmd of ["target_start", "target_stop", "target_step", "grpc_start", "grpc_stop", "grpc_step"]) {
      await expect(invoke(cmd, { cycleMs: 10 })).rejects.toThrow("not supported over the device relay");
    }
  });

  it("set_target_kind resolves and scan_mdns_targets reports no LAN devices", async () => {
    await expect(invoke("set_target_kind", { kind: "Grpc" })).resolves.toBeUndefined();
    await expect(invoke("scan_mdns_targets")).resolves.toEqual([]);
  });

  it("target_disconnect closes the socket and clears the session", async () => {
    const ws = await connectFakeDevice();
    await invoke("target_disconnect");
    expect(ws.readyState).toBe(3);
    await expect(invoke("target_status")).rejects.toThrow("Not connected");
  });

  it("register_scada_device mirrors the desktop endpoint and payload", async () => {
    const fetchMock = vi.fn(async () => ({ ok: true, status: 200, json: async () => ({ id: 7 }) }));
    vi.stubGlobal("fetch", fetchMock);

    const result = await invoke("register_scada_device", {
      registration: {
        scada_url: "http://scada.local:3000/",
        device_name: "ESP32-1.2.3.4",
        device_url: "ws://1.2.3.4/ws/plc",
        transport: "websocket",
        interval_ms: 1000,
      },
    });
    expect(result).toEqual({ id: 7 });
    const [url, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toBe("http://scada.local:3000/api/v1/voltrus-plc/devices");
    expect(init.method).toBe("POST");
    expect(JSON.parse(String(init.body))).toEqual({
      name: "ESP32-1.2.3.4",
      url: "ws://1.2.3.4/ws/plc",
      transport: "websocket",
      interval_ms: 1000,
    });
  });

  it("register_scada_device rejects non-http URLs", async () => {
    await expect(
      invoke("register_scada_device", {
        registration: {
          scada_url: "ftp://scada.local",
          device_name: "n",
          device_url: "u",
          transport: "websocket",
          interval_ms: 1000,
        },
      }),
    ).rejects.toThrow("SCADA URL must start with http");
  });
});
