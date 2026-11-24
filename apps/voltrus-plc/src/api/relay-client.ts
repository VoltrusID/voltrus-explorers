// Browser-side client for the device relay (PLC-23): one WebSocket per
// device session, JSON envelopes per the shared wire contract.
//
//   ide    -> device: {id, op, payload}        (correlated request)
//   device -> ide:    {id, ok, payload|error}  (response, routed by id)
//   device -> ides:   {deviceId, event, ...}   (unsolicited broadcast)
//   DO     -> all:    {op: "ping"}             (heartbeat; ignored here)
//
// Reconnection is deliberately NOT here — the UI owns connection status and
// retry policy. This class only correlates requests and fans out events.

export type RelayEvent = {
  deviceId?: string;
  event: string;
  payload?: unknown;
};

export type RelayEventListener = (event: RelayEvent) => void;

export type WebSocketFactory = (url: string) => WebSocket;

// Same-origin relay in production; localhost dev targets the local worker.
// vplc_base overrides. An empty base means "this origin" — made absolute in
// connect() because the WebSocket constructor rejects relative URLs.
const DEFAULT_BASE = /^localhost$|^127\.0\.0\.1$/.test(location.hostname)
  ? "http://localhost:8787"
  : "";
const DEFAULT_TIMEOUT_MS = 10_000;

// jsdom (and node) provide no WebSocket implementation, so tests inject a
// fake via setRelaySocketFactory — the same injectable-worker pattern
// sim-host.ts uses.
const defaultSocketFactory: WebSocketFactory = (url) => new WebSocket(url);
let socketFactory: WebSocketFactory = defaultSocketFactory;

export function setRelaySocketFactory(factory: WebSocketFactory): void {
  socketFactory = factory;
}

export function resetRelaySocketFactory(): void {
  socketFactory = defaultSocketFactory;
}

export interface RelayClientOptions {
  /** http(s) relay base; defaults to localStorage `vplc_base`. */
  base?: string;
  socketFactory?: WebSocketFactory;
  requestTimeoutMs?: number;
}

interface Pending {
  resolve: (value: unknown) => void;
  reject: (reason: Error) => void;
  timer: ReturnType<typeof setTimeout>;
}

type WireFrame = {
  id?: unknown;
  op?: unknown;
  ok?: unknown;
  payload?: unknown;
  error?: unknown;
  event?: unknown;
  deviceId?: unknown;
};

export class RelayClient {
  private ws: WebSocket | null = null;
  private nextId = 1;
  private readonly pending = new Map<string, Pending>();
  private readonly eventListeners = new Set<RelayEventListener>();
  private readonly closeListeners = new Set<() => void>();
  private readonly base: string;
  private readonly factory: WebSocketFactory;
  private readonly requestTimeoutMs: number;

  constructor(options: RelayClientOptions = {}) {
    this.base = options.base ?? localStorage.getItem("vplc_base") ?? DEFAULT_BASE;
    this.factory = options.socketFactory ?? socketFactory;
    this.requestTimeoutMs = options.requestTimeoutMs ?? DEFAULT_TIMEOUT_MS;
  }

  /** Open the ide socket; resolves on upgrade, rejects on error/early close. */
  connect(deviceId: string, token: string): Promise<void> {
    const raw = this.base.replace(/\/+$/, "");
    // Same-origin deployments leave the base empty; WebSocket needs absolute.
    const absolute = /^[a-z]+:\/\//.test(raw) ? raw : `${location.origin}${raw}`;
    const base = absolute.replace(/^http/, "ws");
    const url = `${base}/api/relay/${encodeURIComponent(deviceId)}?role=ide&token=${encodeURIComponent(token)}`;
    return new Promise<void>((resolve, reject) => {
      const ws = this.factory(url);
      this.ws = ws;
      let settled = false;
      const fail = (error: Error) => {
        if (!settled) {
          settled = true;
          reject(error);
        }
      };
      ws.onopen = () => {
        settled = true;
        resolve();
      };
      ws.onerror = () => fail(new Error("relay: websocket error"));
      ws.onclose = () => {
        fail(new Error("relay: closed before the upgrade completed"));
        this.handleClose();
      };
      ws.onmessage = (e: MessageEvent) => this.handleMessage(e.data);
    });
  }

  /**
   * Send one {id, op, payload} frame and await the correlated response.
   * Rejects on ok:false, timeout (10s default), or a closed socket.
   */
  request(op: string, payload?: unknown): Promise<unknown> {
    const ws = this.ws;
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      return Promise.reject(new Error("relay: not connected"));
    }
    const id = `r${this.nextId++}`;
    return new Promise<unknown>((resolve, reject) => {
      const entry: Pending = {
        resolve,
        reject,
        timer: setTimeout(() => {
          this.pending.delete(id);
          reject(new Error(`relay: request '${op}' timed out`));
        }, this.requestTimeoutMs),
      };
      this.pending.set(id, entry);
      ws.send(JSON.stringify({ id, op, payload: payload ?? null }));
    });
  }

  /** Register a listener for unsolicited device events; returns an unlisten fn. */
  onEvent(listener: RelayEventListener): () => void {
    this.eventListeners.add(listener);
    return () => {
      this.eventListeners.delete(listener);
    };
  }

  /** Register a listener notified when the socket drops (UI owns reconnect). */
  onClose(listener: () => void): () => void {
    this.closeListeners.add(listener);
    return () => {
      this.closeListeners.delete(listener);
    };
  }

  /** Close the socket; pending requests are rejected. */
  close(): void {
    const ws = this.ws;
    if (!ws) return;
    this.ws = null;
    ws.onopen = null;
    ws.onmessage = null;
    ws.onclose = null;
    ws.onerror = null;
    ws.close();
    for (const [id, entry] of this.pending) {
      clearTimeout(entry.timer);
      entry.reject(new Error("relay: closed"));
      this.pending.delete(id);
    }
  }

  private handleMessage(raw: unknown): void {
    if (typeof raw !== "string") return;
    let frame: WireFrame;
    try {
      frame = JSON.parse(raw) as WireFrame;
    } catch {
      return; // Malformed junk: ignored.
    }
    if (frame.op === "ping") return; // DO heartbeat.
    if (typeof frame.id === "string") {
      const entry = this.pending.get(frame.id);
      if (!entry) return;
      this.pending.delete(frame.id);
      clearTimeout(entry.timer);
      if (frame.ok === true) entry.resolve(frame.payload);
      else entry.reject(new Error(typeof frame.error === "string" ? frame.error : "relay: request failed"));
      return;
    }
    if (typeof frame.event === "string") {
      const event: RelayEvent = { event: frame.event };
      if (typeof frame.deviceId === "string") event.deviceId = frame.deviceId;
      if (frame.payload !== undefined) event.payload = frame.payload;
      for (const listener of this.eventListeners) listener(event);
    }
  }

  private handleClose(): void {
    this.ws = null;
    for (const [id, entry] of this.pending) {
      clearTimeout(entry.timer);
      entry.reject(new Error("relay: connection closed"));
      this.pending.delete(id);
    }
    for (const listener of this.closeListeners) listener();
  }
}
