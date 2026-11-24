// Server-side AI proxy. Stores provider API keys encrypted in ai_keys, relays
// /claude and /ollama traffic to their upstreams, and rate-limits proxying per
// user with a 60s sliding-floor window in ai_usage.

import { Hono } from "hono";
import type { Context } from "hono";
import { authMiddleware } from "../auth";
import { requirePlan } from "../entitlements";
import { decryptKey, encryptKey, maskKey } from "../keys";
import type { AppEnv } from "../env";

export const PROVIDERS = ["claude", "ollama"] as const;
export type Provider = (typeof PROVIDERS)[number];

const ANTHROPIC_MESSAGES_URL = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION = "2023-06-01";
const RATE_LIMIT = 20;
const RATE_WINDOW_MS = 60_000;

/** Response headers relayed to the client; everything else (hop-by-hop, set-cookie) is dropped. */
const PASSTHROUGH_HEADERS = ["content-type", "content-length"] as const;

export type FetchLike = (input: string | URL | Request, init?: RequestInit) => Promise<Response>;

type StoredKey = { key_encrypted: string; key_iv: string; masked: string; updated_at: string };

function isPrivateIpv4(a: number, b: number, c: number, d: number): boolean {
  if (a === 0 || a === 10 || a === 127) return true;
  if (a === 192 && b === 168) return true;
  if (a === 172 && b >= 16 && b <= 31) return true;
  if (a === 169 && b === 254) return true; // link-local (cloud metadata: 169.254.169.254)
  if (a === 100 && b >= 64 && b <= 127) return true; // CGNAT 100.64.0.0/10
  return false;
}

function isPrivateHost(hostname: string): boolean {
  let host = hostname.toLowerCase();
  if (host.startsWith("[") && host.endsWith("]")) host = host.slice(1, -1);
  if (host === "localhost" || host.endsWith(".localhost") || host.endsWith(".internal")) return true;
  if (host === "::1" || host === "::" || host === "0.0.0.0") return true;
  // IPv4-mapped IPv6 (::ffff:127.0.0.1) — URL serializes it as hex, e.g. ::ffff:7f00:1.
  if (host.startsWith("::ffff:")) {
    const groups = host.slice(7).split(":");
    const hi = groups.length >= 2 ? parseInt(groups[groups.length - 2]!, 16) : 0;
    const lo = parseInt(groups[groups.length - 1]!, 16);
    if (!Number.isNaN(hi) && !Number.isNaN(lo)) {
      return isPrivateIpv4(hi >> 8, hi & 0xff, lo >> 8, lo & 0xff);
    }
    return false;
  }
  const parts = host.split(".").map(Number);
  if (parts.length === 4 && parts.every((n) => Number.isInteger(n) && n >= 0 && n <= 255)) {
    return isPrivateIpv4(parts[0]!, parts[1]!, parts[2]!, parts[3]!);
  }
  return false;
}

/** Chat URL for a validated endpoint, or null when the endpoint is missing/unsafe. */
export function ollamaChatUrl(endpoint: string): URL | null {
  const trimmed = endpoint.trim();
  let parsed: URL;
  try {
    parsed = new URL(trimmed);
  } catch {
    return null;
  }
  if (parsed.protocol !== "http:" && parsed.protocol !== "https:") return null;
  if (isPrivateHost(parsed.hostname)) return null;
  return new URL(trimmed.replace(/\/+$/, "") + "/api/chat");
}

/** Bumps the caller's 60s window counter and returns a 429 response once over the limit. */
async function rateLimit(c: Context<AppEnv>): Promise<Response | null> {
  const windowStart = new Date(Math.floor(Date.now() / RATE_WINDOW_MS) * RATE_WINDOW_MS).toISOString();
  const row = await c.env.DB.prepare(
    "INSERT INTO ai_usage (user_id, window_start, count) VALUES (?, ?, 1) " +
      "ON CONFLICT (user_id, window_start) DO UPDATE SET count = count + 1 RETURNING count",
  )
    .bind(c.get("userId"), windowStart)
    .first<{ count: number }>();
  if ((row?.count ?? 0) > RATE_LIMIT) return c.json({ error: "rate_limited" }, 429);
  return null;
}

async function loadKey(c: Context<AppEnv>, provider: Provider): Promise<StoredKey | null> {
  return c.env.DB.prepare("SELECT key_encrypted, key_iv, masked, updated_at FROM ai_keys WHERE user_id = ? AND provider = ?")
    .bind(c.get("userId"), provider)
    .first<StoredKey>();
}

/** Streams the upstream response through, relaying status and a small header allowlist. */
function relayResponse(upstream: Response): Response {
  const headers = new Headers();
  for (const name of PASSTHROUGH_HEADERS) {
    const value = upstream.headers.get(name);
    if (value !== null) headers.set(name, value);
  }
  return new Response(upstream.body, { status: upstream.status, headers });
}

export function createAiRoutes(upstreamFetch: FetchLike = fetch): Hono<AppEnv> {
  const ai = new Hono<AppEnv>();

  ai.use("*", authMiddleware);

  ai.put("/keys", async (c) => {
    const body = await c.req.json().catch(() => null);
    if (!body || typeof body !== "object") return c.json({ error: "provider and key are required" }, 400);
    const { provider, key } = body as Record<string, unknown>;
    if (typeof provider !== "string" || !PROVIDERS.includes(provider as Provider)) {
      return c.json({ error: "unknown provider" }, 400);
    }
    if (typeof key !== "string" || key.trim().length === 0) return c.json({ error: "key is required" }, 400);
    const plaintext = key.trim();
    const masked = maskKey(plaintext);
    const { encrypted, iv } = await encryptKey(c.env.JWT_SECRET, plaintext);
    await c.env.DB.prepare(
      "INSERT INTO ai_keys (user_id, provider, key_encrypted, key_iv, masked, updated_at) VALUES (?, ?, ?, ?, ?, ?) " +
        "ON CONFLICT (user_id, provider) DO UPDATE SET key_encrypted = excluded.key_encrypted, key_iv = excluded.key_iv, " +
        "masked = excluded.masked, updated_at = excluded.updated_at",
    )
      .bind(c.get("userId"), provider, encrypted, iv, masked, new Date().toISOString())
      .run();
    return c.json({ provider, masked });
  });

  ai.get("/keys", async (c) => {
    const { results } = await c.env.DB.prepare("SELECT provider, masked, updated_at FROM ai_keys WHERE user_id = ? ORDER BY provider")
      .bind(c.get("userId"))
      .all<{ provider: string; masked: string; updated_at: string }>();
    return c.json(results.map((r) => ({ provider: r.provider, masked: r.masked, updatedAt: r.updated_at })));
  });

  ai.delete("/keys/:provider", async (c) => {
    const provider = c.req.param("provider");
    if (!PROVIDERS.includes(provider as Provider)) return c.json({ error: "no_key" }, 404);
    const result = await c.env.DB.prepare("DELETE FROM ai_keys WHERE user_id = ? AND provider = ?")
      .bind(c.get("userId"), provider)
      .run();
    if (!result.meta.changes) return c.json({ error: "no_key" }, 404);
    return c.json({ ok: true });
  });

  ai.post("/claude", async (c) => {
    const gate = await requirePlan(c.env.DB, c.get("userId"), "solo");
    if (gate) return c.json(gate.body, gate.status);
    const stored = await loadKey(c, "claude");
    if (!stored) return c.json({ error: "no_key" }, 404);
    const apiKey = await decryptKey(c.env.JWT_SECRET, stored.key_encrypted, stored.key_iv);
    const limited = await rateLimit(c);
    if (limited) return limited;
    const upstream = await upstreamFetch(ANTHROPIC_MESSAGES_URL, {
      method: "POST",
      headers: {
        "content-type": c.req.header("content-type") ?? "application/json",
        "x-api-key": apiKey,
        "anthropic-version": ANTHROPIC_VERSION,
      },
      body: await c.req.arrayBuffer(),
    });
    return relayResponse(upstream);
  });

  ai.post("/ollama", async (c) => {
    const gate = await requirePlan(c.env.DB, c.get("userId"), "solo");
    if (gate) return c.json(gate.body, gate.status);
    const body = await c.req.json().catch(() => null);
    if (!body || typeof body !== "object" || Array.isArray(body)) {
      return c.json({ error: "JSON object body is required" }, 400);
    }
    const { endpoint, ...chat } = body as Record<string, unknown>;
    if (typeof endpoint !== "string") return c.json({ error: "endpoint is required" }, 400);
    const chatUrl = ollamaChatUrl(endpoint);
    if (!chatUrl) return c.json({ error: "endpoint not allowed" }, 400);
    const stored = await loadKey(c, "ollama");
    const limited = await rateLimit(c);
    if (limited) return limited;
    const headers: Record<string, string> = { "content-type": "application/json" };
    if (stored) {
      headers.authorization = `Bearer ${await decryptKey(c.env.JWT_SECRET, stored.key_encrypted, stored.key_iv)}`;
    }
    const upstream = await upstreamFetch(chatUrl.toString(), {
      method: "POST",
      headers,
      body: JSON.stringify(chat),
    });
    return relayResponse(upstream);
  });

  return ai;
}

/** Mounted by the integrator: `app.route("/api/ai", aiRoutes)`. */
export const aiRoutes = createAiRoutes();
