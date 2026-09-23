// RelayRoom — one Durable Object per device (idFromName(deviceId)).
//
// Holds a single device socket (the PLC-side agent, latest wins) and N ide
// sockets (browsers). Ops are opaque here: the DO only validates envelope
// shape and routes frames. See test/relay.test.ts for the wire examples.
//
// Envelopes (JSON text frames):
//   ide    -> device: {id, op, payload}  DO wraps as {deviceId, id, op, payload}
//   device -> ide:    {id, ok, payload|error}  routed by id to the requesting ide
//   device -> ides:   {event, ...}  (no id) broadcast as {deviceId, event, ...}
//   DO     -> all:    {op: "ping"} heartbeat; a failed send closes the socket
import { DurableObject } from "cloudflare:workers";
import type { Env } from "./env";

const TAG_DEVICE = "role:device";
const TAG_IDE = "role:ide";
const HEARTBEAT_MS = 30_000;

// ── Envelope parsing (pure, unit-testable) ──

export type IdeRequest = { id: string; op: string; payload?: unknown };
export type DeviceResponse = { id: string; ok: boolean; payload?: unknown; error?: unknown };
export type DeviceEvent = { event: string; [key: string]: unknown };

/** Canonical guard for a parsed JSON frame: a non-null, non-array object. */
export function isJsonObject(frame: unknown): frame is Record<string, unknown> {
  return typeof frame === "object" && frame !== null && !Array.isArray(frame);
}

/** `{id, op}` with non-empty string fields; `payload` optional. */
export function parseIdeRequest(frame: unknown): IdeRequest | null {
  if (!isJsonObject(frame)) return null;
  const { id, op, payload } = frame;
  if (typeof id !== "string" || id.length === 0) return null;
  if (typeof op !== "string" || op.length === 0) return null;
  return payload === undefined ? { id, op } : { id, op, payload };
}

/** `{id, ok}` with boolean `ok`; `payload`/`error` optional extras. */
export function parseDeviceResponse(frame: unknown): DeviceResponse | null {
  if (!isJsonObject(frame)) return null;
  const { id, ok, payload, error } = frame;
  if (typeof id !== "string" || id.length === 0) return null;
  if (typeof ok !== "boolean") return null;
  const out: DeviceResponse = { id, ok };
  if (payload !== undefined) out.payload = payload;
  if (error !== undefined) out.error = error;
  return out;
}

/** Unsolicited device frame: `{event, ...}` with a non-empty `event` and no `id`. */
export function parseDeviceEvent(frame: unknown): DeviceEvent | null {
  if (!isJsonObject(frame)) return null;
  if ("id" in frame) return null;
  if (typeof frame.event !== "string" || frame.event.length === 0) return null;
  return frame as DeviceEvent;
}

function parseJson(text: string): unknown {
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}

type DeviceAttachment = { pending: Record<string, string> };

export class RelayRoom extends DurableObject {
  private readonly deviceId: string;
  private heartbeatScheduled = false;

  constructor(ctx: DurableObjectState, env: Env) {
    super(ctx, env);
    // The room is always created via idFromName(deviceId), so the name is set.
    this.deviceId = ctx.id.name ?? "unknown";
  }

  async fetch(request: Request): Promise<Response> {
    const role = new URL(request.url).searchParams.get("role") === "device" ? "device" : "ide";
    const pair = new WebSocketPair();

    if (role === "device") {
      // Latest device wins: any older device socket is evicted.
      for (const stale of this.ctx.getWebSockets(TAG_DEVICE)) {
        try {
          stale.close(4000, "replaced by a newer device connection");
        } catch {
          // Already gone.
        }
      }
      this.ctx.acceptWebSocket(pair[1], [TAG_DEVICE]);
      pair[1].serializeAttachment({ pending: {} } satisfies DeviceAttachment);
    } else {
      this.ctx.acceptWebSocket(pair[1], [TAG_IDE, `ide:${crypto.randomUUID()}`]);
    }

    this.scheduleHeartbeat();
    return new Response(null, { status: 101, webSocket: pair[0] });
  }

  override async webSocketMessage(ws: WebSocket, message: string | ArrayBuffer): Promise<void> {
    if (typeof message !== "string") return; // Binary frames are not part of the protocol.
    const frame = parseJson(message);
    if (frame === null) return;

    if (this.ctx.getTags(ws).includes(TAG_DEVICE)) {
      this.onDeviceFrame(ws, frame);
    } else {
      this.onIdeFrame(ws, frame);
    }
  }

  override async webSocketClose(ws: WebSocket): Promise<void> {
    if (this.ctx.getTags(ws).includes(TAG_IDE)) {
      this.detachIde(ws);
      return;
    }
    // Device gone (crash, network drop, or replaced by a newer device): every
    // in-flight request would otherwise hang forever — fail them explicitly.
    const attachment = (ws.deserializeAttachment() as DeviceAttachment | null) ?? { pending: {} };
    for (const [id, ideTag] of Object.entries(attachment.pending)) {
      const ide = this.ctx.getWebSockets(ideTag)[0];
      if (ide) this.sendOrClose(ide, JSON.stringify({ id, ok: false, error: "device_offline" }));
    }
  }

  override async webSocketError(_ws: WebSocket): Promise<void> {
    // The heartbeat alarm or the next send cleans the socket up; nothing to do here.
  }

  /** Liveness sweep: any socket that cannot accept a ping is closed. */
  override async alarm(): Promise<void> {
    this.heartbeatScheduled = false;
    for (const ws of this.ctx.getWebSockets()) {
      try {
        ws.send(JSON.stringify({ op: "ping" }));
      } catch {
        try {
          ws.close(1011, "heartbeat timeout");
        } catch {
          // Already gone.
        }
      }
    }
    if (this.ctx.getWebSockets().length > 0) this.scheduleHeartbeat();
  }

  private scheduleHeartbeat(): void {
    if (this.heartbeatScheduled) return;
    this.heartbeatScheduled = true;
    void this.ctx.storage.setAlarm(Date.now() + HEARTBEAT_MS);
  }

  // Device frames are either routed responses ({id, ok, ...}) or unsolicited
  // events ({event, ...}) broadcast to every ide.
  private onDeviceFrame(device: WebSocket, frame: unknown): void {
    const response = parseDeviceResponse(frame);
    if (response) {
      const attachment = (device.deserializeAttachment() as DeviceAttachment | null) ?? { pending: {} };
      const ideTag = attachment.pending[response.id];
      if (ideTag === undefined) return; // Unknown request (or the requesting ide already left).
      delete attachment.pending[response.id];
      device.serializeAttachment(attachment);
      const ide = this.ctx.getWebSockets(ideTag)[0];
      if (ide) this.sendOrClose(ide, JSON.stringify(response));
      return;
    }

    const event = parseDeviceEvent(frame);
    if (event) {
      const wire = JSON.stringify({ deviceId: this.deviceId, ...event });
      for (const ide of this.ctx.getWebSockets(TAG_IDE)) {
        this.sendOrClose(ide, wire);
      }
    }
    // Anything else is malformed; drop silently.
  }

  private onIdeFrame(ide: WebSocket, frame: unknown): void {
    const request = parseIdeRequest(frame);
    if (!request) {
      this.sendOrClose(ide, JSON.stringify({ id: null, ok: false, error: "invalid envelope" }));
      return;
    }

    const device = this.ctx.getWebSockets(TAG_DEVICE)[0];
    if (!device) {
      this.sendOrClose(ide, JSON.stringify({ id: request.id, ok: false, error: "device_offline" }));
      return;
    }

    const attachment = (device.deserializeAttachment() as DeviceAttachment | null) ?? { pending: {} };
    attachment.pending[request.id] = this.ideTag(ide);
    device.serializeAttachment(attachment);

    const wire = JSON.stringify({
      deviceId: this.deviceId,
      id: request.id,
      op: request.op,
      ...(request.payload === undefined ? {} : { payload: request.payload }),
    });
    if (!this.sendOrClose(device, wire)) {
      // Device socket died between lookup and send; fail the request fast.
      const fresh = (device.deserializeAttachment() as DeviceAttachment | null) ?? { pending: {} };
      delete fresh.pending[request.id];
      device.serializeAttachment(fresh);
      this.sendOrClose(ide, JSON.stringify({ id: request.id, ok: false, error: "device_offline" }));
    }
  }

  /** Drop pending request entries that point at a departing ide. */
  private detachIde(ide: WebSocket): void {
    const tag = this.ideTag(ide);
    const device = this.ctx.getWebSockets(TAG_DEVICE)[0];
    if (!device) return;
    const attachment = (device.deserializeAttachment() as DeviceAttachment | null) ?? { pending: {} };
    let changed = false;
    for (const [id, pendingTag] of Object.entries(attachment.pending)) {
      if (pendingTag === tag) {
        delete attachment.pending[id];
        changed = true;
      }
    }
    if (changed) device.serializeAttachment(attachment);
  }

  private ideTag(ide: WebSocket): string {
    return this.ctx.getTags(ide).find((tag) => tag.startsWith("ide:")) ?? "";
  }

  /** Send one text frame; a socket that cannot deliver is closed for cleanup. */
  private sendOrClose(ws: WebSocket, text: string): boolean {
    try {
      ws.send(text);
      return true;
    } catch {
      try {
        ws.close(1011, "failed to deliver relay frame");
      } catch {
        // Already gone.
      }
      return false;
    }
  }
}
