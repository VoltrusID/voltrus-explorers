// Device pairing REST + device/ide WebSocket relay endpoints.
// The integrator mounts this module once: `app.route("/api", relayRoutes)`.
//
// REST (Bearer session auth):
//   GET    /api/devices          -> [{id, name, lastSeenAt, createdAt}]
//   POST   /api/devices {name}   -> 201 {id, name, pairingToken, createdAt}
//                                   pairingToken is shown exactly once; only its
//                                   SHA-256 hash is stored.
//   DELETE /api/devices/:id      -> 204 (404 for missing or foreign devices)
//
// WebSocket (same worker):
//   GET /api/relay/:deviceId?role=device&token=<pairingToken>  -> agent side
//   GET /api/relay/:deviceId?role=ide                          -> browser side (Bearer auth)
import { Hono } from "hono";
import { authMiddleware, verifyToken } from "../auth";
import { requirePlan } from "../entitlements";
import type { AppEnv } from "../env";

export const relayRoutes = new Hono<AppEnv>();

type DeviceRow = {
  id: string;
  name: string;
  pairing_token_hash: string;
  last_seen_at: string | null;
  created_at: string;
};

function toHex(bytes: Uint8Array): string {
  let hex = "";
  for (const byte of bytes) hex += byte.toString(16).padStart(2, "0");
  return hex;
}

function toBase64Url(bytes: Uint8Array): string {
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "");
}

async function sha256Hex(text: string): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text) as unknown as ArrayBuffer);
  return toHex(new Uint8Array(digest));
}

/** Length-safe constant-time string comparison via XOR accumulation. */
export function constantTimeEquals(a: string, b: string): boolean {
  let diff = a.length ^ b.length;
  const max = Math.max(a.length, b.length);
  for (let i = 0; i < max; i++) {
    diff |= (a.charCodeAt(i % a.length) || 0) ^ (b.charCodeAt(i % b.length) || 0);
  }
  return diff === 0;
}

async function verifyPairingToken(token: string, storedHashHex: string): Promise<boolean> {
  return constantTimeEquals(await sha256Hex(token), storedHashHex);
}

// ── REST pairing ──

relayRoutes.use("/devices", authMiddleware);
relayRoutes.use("/devices/*", authMiddleware);

relayRoutes.get("/devices", async (c) => {
  const { results } = await c.env.DB.prepare(
    "SELECT id, name, last_seen_at, created_at FROM devices WHERE user_id = ? ORDER BY created_at DESC",
  )
    .bind(c.get("userId"))
    .all<{ id: string; name: string; last_seen_at: string | null; created_at: string }>();
  return c.json(
    results.map((row) => ({ id: row.id, name: row.name, lastSeenAt: row.last_seen_at, createdAt: row.created_at })),
  );
});

relayRoutes.post("/devices", async (c) => {
  const body = await c.req.json().catch(() => null);
  const name = body && typeof body === "object" ? (body as Record<string, unknown>).name : undefined;
  if (typeof name !== "string" || name.trim().length === 0) {
    return c.json({ error: "name is required" }, 400);
  }

  const id = crypto.randomUUID();
  const tokenBytes = crypto.getRandomValues(new Uint8Array(32));
  const pairingToken = toBase64Url(tokenBytes);
  const now = new Date().toISOString();
  await c.env.DB.prepare(
    "INSERT INTO devices (id, user_id, name, pairing_token_hash, created_at) VALUES (?, ?, ?, ?, ?)",
  )
    .bind(id, c.get("userId"), name.trim(), await sha256Hex(pairingToken), now)
    .run();

  // pairingToken is returned exactly once; the DB only keeps its hash.
  return c.json({ id, name: name.trim(), pairingToken, createdAt: now }, 201);
});

relayRoutes.delete("/devices/:id", async (c) => {
  // Scoped to the session user: missing and foreign ids are both 404.
  const result = await c.env.DB.prepare("DELETE FROM devices WHERE id = ? AND user_id = ?")
    .bind(c.req.param("id"), c.get("userId"))
    .run();
  if (result.meta.changes === 0) return c.json({ error: "device not found" }, 404);
  return c.body(null, 204);
});

// ── WebSocket upgrade (routed into the RelayRoom DO for the device) ──

relayRoutes.get("/relay/:deviceId", async (c) => {
  const deviceId = c.req.param("deviceId");
  const url = new URL(c.req.url);
  const role = url.searchParams.get("role") === "device" ? "device" : "ide";

  if (role === "device") {
    const token = url.searchParams.get("token") ?? "";
    const row = await c.env.DB.prepare("SELECT pairing_token_hash FROM devices WHERE id = ?")
      .bind(deviceId)
      .first<Pick<DeviceRow, "pairing_token_hash">>();
    if (!row || !(await verifyPairingToken(token, row.pairing_token_hash))) {
      return c.json({ error: "invalid pairing token" }, 401);
    }
    await c.env.DB.prepare("UPDATE devices SET last_seen_at = ? WHERE id = ?")
      .bind(new Date().toISOString(), deviceId)
      .run();
  } else {
    // Browsers cannot set Authorization headers on the WebSocket constructor,
    // so the ide upgrade also accepts `token=<session token>` in the query
    // string. verifyToken returns null on any failure, keeping the compare
    // constant-time for both sources.
    const header = c.req.header("Authorization") ?? "";
    const match = /^Bearer\s+(.+)$/.exec(header);
    const queryToken = url.searchParams.get("token");
    const userId = match
      ? await verifyToken(c.env.JWT_SECRET, match[1])
      : queryToken
        ? await verifyToken(c.env.JWT_SECRET, queryToken)
        : null;
    if (!userId) return c.json({ error: "unauthorized" }, 401);
    const gate = await requirePlan(c.env.DB, userId, "solo");
    if (gate) return c.json(gate.body, gate.status);
    const owned = await c.env.DB.prepare("SELECT id FROM devices WHERE id = ? AND user_id = ?")
      .bind(deviceId, userId)
      .first<{ id: string }>();
    if (!owned) return c.json({ error: "device not found" }, 404); // Foreign and missing look identical.
  }

  // Forward the upgrade into the per-device Durable Object.
  const namespace = c.env.RELAY;
  return namespace.get(namespace.idFromName(deviceId)).fetch(c.req.raw);
});
