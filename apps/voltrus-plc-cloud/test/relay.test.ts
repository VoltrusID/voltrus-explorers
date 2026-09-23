import { describe, expect, it } from "vitest";
import { env } from "cloudflare:test";
import { Hono } from "hono";
import { authRoutes } from "../src/routes/auth";
import { relayRoutes } from "../src/routes/relay";
import { setPlanForTest } from "../src/entitlements";
import type { AppEnv } from "../src/env";
import { parseDeviceEvent, parseDeviceResponse, parseIdeRequest } from "../src/relay";

// Route mounting is the integrator's job (src/app.ts); the test wires the same
// shape itself so the module is exercised end to end through the real bindings.
const app = new Hono<AppEnv>();
app.route("/api/auth", authRoutes);
app.route("/api", relayRoutes);

// Miniflare injects the same bindings the deployed worker sees (DB, RELAY, JWT_SECRET).
const workerEnv = env as unknown as AppEnv["Bindings"];

type Json = Record<string, unknown>;

function post(path: string, body: unknown, headers: Record<string, string> = {}) {
  return app.request("https://example.com" + path, {
    method: "POST",
    body: JSON.stringify(body),
    headers: { "Content-Type": "application/json", ...headers },
  }, workerEnv);
}

function authed(path: string, init: RequestInit & { method?: string; token?: string }) {
  const headers = new Headers(init.headers);
  headers.set("Authorization", `Bearer ${init.token ?? ""}`);
  return app.request("https://example.com" + path, { ...init, headers }, workerEnv);
}

async function registerUser(): Promise<string> {
  const res = await post("/api/auth/register", {
    email: `relay-${crypto.randomUUID()}@example.com`,
    password: "correct horse battery",
  });
  expect(res.status).toBe(201);
  const body = (await res.json()) as { token: string; user: { id: string } };
  // ide WebSocket upgrades are solo-gated; every relay test connects as ide.
  await setPlanForTest(workerEnv.DB, body.user.id, "solo");
  return body.token;
}

async function createDevice(token: string, name = "bench-plc"): Promise<{ id: string; pairingToken: string }> {
  const res = await post("/api/devices", { name }, { Authorization: `Bearer ${token}` });
  expect(res.status).toBe(201);
  return (await res.json()) as { id: string; pairingToken: string };
}

/** WebSocket upgrade through the mounted app; returns the client-side socket (unaccepted). */
async function connectWs(path: string, token?: string): Promise<Response> {
  const headers = new Headers({ Upgrade: "websocket" });
  if (token) headers.set("Authorization", `Bearer ${token}`);
  return app.request("https://example.com" + path, { headers }, workerEnv);
}

function nextMessage(ws: WebSocket, timeoutMs = 2000): Promise<Json> {
  const { promise, resolve, reject } = Promise.withResolvers<Json>();
  const timer = setTimeout(() => reject(new Error("timed out waiting for websocket message")), timeoutMs);
  ws.addEventListener(
    "message",
    (ev) => {
      clearTimeout(timer);
      resolve(JSON.parse(String(ev.data)) as Json);
    },
    { once: true },
  );
  return promise;
}

function nextClose(ws: WebSocket, timeoutMs = 2000): Promise<{ code: number; reason: string }> {
  const { promise, resolve, reject } = Promise.withResolvers<{ code: number; reason: string }>();
  const timer = setTimeout(() => reject(new Error("timed out waiting for websocket close")), timeoutMs);
  ws.addEventListener(
    "close",
    (ev) => {
      clearTimeout(timer);
      resolve({ code: ev.code, reason: ev.reason });
    },
    { once: true },
  );
  return promise;
}

describe("envelope parsing", () => {
  it("accepts well-formed ide requests", () => {
    expect(parseIdeRequest({ id: "1", op: "browse", payload: { a: 1 } })).toEqual({ id: "1", op: "browse", payload: { a: 1 } });
    expect(parseIdeRequest({ id: "1", op: "status" })).toEqual({ id: "1", op: "status" });
  });

  it("rejects malformed ide requests", () => {
    expect(parseIdeRequest({ id: "", op: "browse" })).toBeNull();
    expect(parseIdeRequest({ id: "1" })).toBeNull();
    expect(parseIdeRequest({ id: 7, op: "browse" })).toBeNull();
    expect(parseIdeRequest("browse")).toBeNull();
    expect(parseIdeRequest(null)).toBeNull();
    expect(parseIdeRequest(["1", "browse"])).toBeNull();
  });

  it("accepts only boolean-ok device responses", () => {
    expect(parseDeviceResponse({ id: "1", ok: true, payload: { x: 1 } })).toEqual({ id: "1", ok: true, payload: { x: 1 } });
    expect(parseDeviceResponse({ id: "1", ok: false, error: "boom" })).toEqual({ id: "1", ok: false, error: "boom" });
    expect(parseDeviceResponse({ id: "1", ok: "yes" })).toBeNull();
    expect(parseDeviceResponse({ ok: true })).toBeNull();
  });

  it("treats id-less event frames as events and nothing else", () => {
    expect(parseDeviceEvent({ event: "variable_update", name: "motor_running" })).toEqual({
      event: "variable_update",
      name: "motor_running",
    });
    expect(parseDeviceEvent({ id: "1", ok: true })).toBeNull();
    expect(parseDeviceEvent({ event: "" })).toBeNull();
  });
});

describe("device pairing REST", () => {
  it("creates a device, shows the pairing token once, and lists it", async () => {
    const token = await registerUser();
    const created = await createDevice(token, "line-1 plc");
    expect(created.id).toBeTruthy();
    expect(created.pairingToken).toBeTruthy();

    const listed = await authed("/api/devices", { method: "GET", token });
    expect(listed.status).toBe(200);
    const devices = (await listed.json()) as Array<Json>;
    const mine = devices.find((d) => d.id === created.id);
    expect(mine).toMatchObject({ name: "line-1 plc", lastSeenAt: null });
    expect(mine).not.toHaveProperty("pairingToken");
    expect(mine).not.toHaveProperty("pairing_token_hash");
  });

  it("rejects nameless devices and unauthenticated access", async () => {
    expect((await post("/api/devices", {}, { Authorization: "Bearer x" })).status).toBe(401);
    const token = await registerUser();
    expect((await post("/api/devices", {}, { Authorization: `Bearer ${token}` })).status).toBe(400);
    expect((await post("/api/devices", { name: "  " }, { Authorization: `Bearer ${token}` })).status).toBe(400);
  });

  it("404s foreign deletes and deletes owned devices", async () => {
    const owner = await registerUser();
    const stranger = await registerUser();
    const device = await createDevice(owner);

    const foreign = await authed(`/api/devices/${device.id}`, { method: "DELETE", token: stranger });
    expect(foreign.status).toBe(404);

    const unknown = await authed(`/api/devices/${crypto.randomUUID()}`, { method: "DELETE", token: owner });
    expect(unknown.status).toBe(404);

    const owned = await authed(`/api/devices/${device.id}`, { method: "DELETE", token: owner });
    expect(owned.status).toBe(204);
    const listed = await authed("/api/devices", { method: "GET", token: owner });
    expect((await listed.json()) as Array<Json>).toHaveLength(0);
  });
});

describe("relay websocket auth", () => {
  it("rejects device upgrades with a bad pairing token", async () => {
    const owner = await registerUser();
    const device = await createDevice(owner);
    const res = await connectWs(`/api/relay/${device.id}?role=device&token=not-the-token`);
    expect(res.status).toBe(401);
  });

  it("rejects ide upgrades without a session and 404s foreign devices", async () => {
    const owner = await registerUser();
    const stranger = await registerUser();
    const device = await createDevice(owner);

    const anonymous = await connectWs(`/api/relay/${device.id}?role=ide`);
    expect(anonymous.status).toBe(401);

    const foreign = await connectWs(`/api/relay/${device.id}?role=ide`, stranger);
    expect(foreign.status).toBe(404);

    const missing = await connectWs(`/api/relay/${crypto.randomUUID()}?role=ide`, owner);
    expect(missing.status).toBe(404);
  });

  it("rejects upgrades for unknown devices on the device side", async () => {
    const res = await connectWs(`/api/relay/${crypto.randomUUID()}?role=device&token=whatever`);
    expect(res.status).toBe(401);
  });
});

describe("relay websocket routing", () => {
  it("routes ide requests to the device and responses back by id", async () => {
    const owner = await registerUser();
    const device = await createDevice(owner);

    const deviceRes = await connectWs(`/api/relay/${device.id}?role=device&token=${device.pairingToken}`);
    expect(deviceRes.status).toBe(101);
    const deviceWs = deviceRes.webSocket!;
    deviceWs.accept();

    const ideRes = await connectWs(`/api/relay/${device.id}?role=ide`, owner);
    expect(ideRes.status).toBe(101);
    const ideWs = ideRes.webSocket!;
    ideWs.accept();

    ideWs.send(JSON.stringify({ id: "req-1", op: "browse", payload: { path: "/" } }));

    const wrapped = await nextMessage(deviceWs);
    expect(wrapped).toMatchObject({
      deviceId: device.id,
      id: "req-1",
      op: "browse",
      payload: { path: "/" },
    });

    deviceWs.send(JSON.stringify({ id: "req-1", ok: true, payload: { variables: ["motor_running"] } }));

    const answered = await nextMessage(ideWs);
    expect(answered).toMatchObject({ id: "req-1", ok: true, payload: { variables: ["motor_running"] } });
  });

  it("answers requests with device_offline when no device is connected", async () => {
    const owner = await registerUser();
    const device = await createDevice(owner);

    const ideRes = await connectWs(`/api/relay/${device.id}?role=ide`, owner);
    const ideWs = ideRes.webSocket!;
    ideWs.accept();
    ideWs.send(JSON.stringify({ id: "r", op: "status" }));

    const reply = await nextMessage(ideWs);
    expect(reply).toMatchObject({ id: "r", ok: false, error: "device_offline" });
  });

  it("fails in-flight ide requests with device_offline when the device disconnects", async () => {
    const owner = await registerUser();
    const device = await createDevice(owner);

    const deviceRes = await connectWs(`/api/relay/${device.id}?role=device&token=${device.pairingToken}`);
    const deviceWs = deviceRes.webSocket!;
    deviceWs.accept();

    const ideRes = await connectWs(`/api/relay/${device.id}?role=ide`, owner);
    const ideWs = ideRes.webSocket!;
    ideWs.accept();

    ideWs.send(JSON.stringify({ id: "req-9", op: "browse" }));
    await nextMessage(deviceWs); // The request reached the device; it never replies.

    deviceWs.close(1001, "agent crashed");

    // The stranded request must be failed explicitly, not left hanging.
    const failed = await nextMessage(ideWs);
    expect(failed).toMatchObject({ id: "req-9", ok: false, error: "device_offline" });
  });

  it("broadcasts unsolicited device events to every ide", async () => {
    const owner = await registerUser();
    const device = await createDevice(owner);

    const deviceRes = await connectWs(`/api/relay/${device.id}?role=device&token=${device.pairingToken}`);
    const deviceWs = deviceRes.webSocket!;
    deviceWs.accept();

    const ideA = (await connectWs(`/api/relay/${device.id}?role=ide`, owner)).webSocket!;
    const ideB = (await connectWs(`/api/relay/${device.id}?role=ide`, owner)).webSocket!;
    ideA.accept();
    ideB.accept();

    deviceWs.send(JSON.stringify({ event: "variable_update", name: "cycle_count", value: 42 }));

    const received = await Promise.all([nextMessage(ideA), nextMessage(ideB)]);
    for (const frame of received) {
      expect(frame).toMatchObject({ deviceId: device.id, event: "variable_update", name: "cycle_count", value: 42 });
    }
  });

  it("closes an older device socket when a newer one connects", async () => {
    const owner = await registerUser();
    const device = await createDevice(owner);

    const firstRes = await connectWs(`/api/relay/${device.id}?role=device&token=${device.pairingToken}`);
    const firstWs = firstRes.webSocket!;
    firstWs.accept();
    const firstClosed = nextClose(firstWs);

    const secondRes = await connectWs(`/api/relay/${device.id}?role=device&token=${device.pairingToken}`);
    expect(secondRes.status).toBe(101);
    secondRes.webSocket!.accept();

    const closed = await firstClosed;
    expect(closed.code).toBe(4000);
  });

  it("keeps the connection usable after an Origin header (CORS middleware round trip)", async () => {
    const owner = await registerUser();
    const device = await createDevice(owner);

    const ideRes = await app.request(`https://example.com/api/relay/${device.id}?role=ide`, {
      headers: new Headers({ Upgrade: "websocket", Origin: "http://localhost:1440", Authorization: `Bearer ${owner}` }),
    }, workerEnv);
    expect(ideRes.status).toBe(101);
    const ideWs = ideRes.webSocket!;
    ideWs.accept();

    const deviceRes = await connectWs(`/api/relay/${device.id}?role=device&token=${device.pairingToken}`);
    const deviceWs = deviceRes.webSocket!;
    deviceWs.accept();

    ideWs.send(JSON.stringify({ id: "p", op: "ping-op" }));
    const wrapped = await nextMessage(deviceWs);
    expect(wrapped).toMatchObject({ deviceId: device.id, id: "p", op: "ping-op" });
  });
});
