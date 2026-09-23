// Polar.sh billing: inbound webhooks (subscription -> entitlement sync) and the
// customer billing-portal link. Mount point: app.route("/api", billingRoutes).
import { Hono } from "hono";
import { authMiddleware } from "../auth";
import type { AppEnv, Env } from "../env";
import {
  effectivePlan,
  FREE_MAX_FILES_PER_PROJECT,
  FREE_MAX_PROJECTS,
  type Plan,
} from "../entitlements";

export const billingRoutes = new Hono<AppEnv>();

const POLAR_API = "https://api.polar.sh/v1";

// Polar rotates signing schemes: secrets minted before 2026-09-08 use the legacy
// `polar-signature` HMAC; newer secrets follow Standard Webhooks. We accept both.
const MAX_SIGNATURE_AGE_SECONDS = 5 * 60;

const encoder = new TextEncoder();

type WebhookVerification =
  | { ok: true }
  | { ok: false; reason: "missing_signature" | "invalid_signature" | "stale_timestamp" };

function timingSafeEqual(a: Uint8Array, b: Uint8Array): boolean {
  if (a.length !== b.length) return false;
  let diff = 0;
  for (let i = 0; i < a.length; i++) diff |= a[i]! ^ b[i]!;
  return diff === 0;
}

function toBase64(bytes: Uint8Array): string {
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
}

function fromBase64(value: string): Uint8Array {
  const binary = atob(value);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

async function hmacSha256Base64(key: Uint8Array, content: string): Promise<string> {
  const cryptoKey = await crypto.subtle.importKey("raw", key as BufferSource, { name: "HMAC", hash: "SHA-256" }, false, [
    "sign",
  ]);
  const mac = await crypto.subtle.sign("HMAC", cryptoKey, encoder.encode(content) as BufferSource);
  return toBase64(new Uint8Array(mac));
}

/** `polar-signature: t=<unix-seconds>,v1=<base64 hmac>` over `<t>.<body>`. */
function parseLegacySignature(header: string): { timestamp: number; signature: string } | null {
  let timestamp: number | null = null;
  let signature: string | null = null;
  for (const part of header.split(",")) {
    const eq = part.indexOf("=");
    if (eq <= 0) continue;
    const key = part.slice(0, eq).trim();
    const value = part.slice(eq + 1).trim();
    if (key === "t") timestamp = Number(value);
    if (key === "v1") signature = value;
  }
  if (timestamp === null || !Number.isFinite(timestamp) || !signature) return null;
  return { timestamp, signature };
}

/** Standard Webhooks `webhook-signature: v1,<base64>,…` — space-separated versions. */
function parseStandardSignatures(header: string): string[] {
  const signatures: string[] = [];
  for (const part of header.split(" ")) {
    const comma = part.indexOf(",");
    if (comma > 0 && part.slice(0, comma) === "v1") signatures.push(part.slice(comma + 1));
  }
  return signatures;
}

async function verifyLegacySignature(
  rawBody: string,
  header: string,
  secret: string,
  now: Date,
): Promise<WebhookVerification> {
  const parsed = parseLegacySignature(header);
  if (!parsed) return { ok: false, reason: "invalid_signature" };

  const ageSeconds = Math.abs(now.getTime() / 1000 - parsed.timestamp);
  if (ageSeconds > MAX_SIGNATURE_AGE_SECONDS) return { ok: false, reason: "stale_timestamp" };

  // Legacy secrets are keyed by the UTF-8 bytes of the full whsec_ string.
  const expected = await hmacSha256Base64(encoder.encode(secret), `${parsed.timestamp}.${rawBody}`);
  const matches = timingSafeEqual(encoder.encode(expected), encoder.encode(parsed.signature));
  return matches ? { ok: true } : { ok: false, reason: "invalid_signature" };
}

async function verifyStandardWebhookSignature(
  rawBody: string,
  headers: Headers,
  header: string,
  secret: string,
  now: Date,
): Promise<WebhookVerification> {
  const messageId = headers.get("webhook-id");
  const timestampHeader = headers.get("webhook-timestamp");
  const signatures = parseStandardSignatures(header);
  if (!messageId || !timestampHeader || signatures.length === 0) return { ok: false, reason: "invalid_signature" };

  const timestamp = Number(timestampHeader);
  if (!Number.isFinite(timestamp)) return { ok: false, reason: "invalid_signature" };
  const ageSeconds = Math.abs(now.getTime() / 1000 - timestamp);
  if (ageSeconds > MAX_SIGNATURE_AGE_SECONDS) return { ok: false, reason: "stale_timestamp" };

  // New secrets are base64 after the whsec_ prefix; Polar also still honors the
  // legacy literal-string key (their SDKs try both), so do the same.
  const keys: Uint8Array[] = [];
  if (secret.startsWith("whsec_")) keys.push(fromBase64(secret.slice("whsec_".length)));
  keys.push(encoder.encode(secret));

  const content = `${messageId}.${timestamp}.${rawBody}`;
  for (const key of keys) {
    const expected = await hmacSha256Base64(key, content);
    for (const signature of signatures) {
      if (timingSafeEqual(encoder.encode(expected), encoder.encode(signature))) return { ok: true };
    }
  }
  return { ok: false, reason: "invalid_signature" };
}

async function verifyWebhook(rawBody: string, headers: Headers, secret: string, now: Date): Promise<WebhookVerification> {
  const legacy = headers.get("polar-signature");
  if (legacy) return verifyLegacySignature(rawBody, legacy, secret, now);

  const standard = headers.get("webhook-signature");
  if (standard) return verifyStandardWebhookSignature(rawBody, headers, standard, secret, now);

  return { ok: false, reason: "missing_signature" };
}

type PolarSubscriptionData = {
  id?: unknown;
  status?: unknown;
  current_period_end?: unknown;
  product?: { id?: unknown } | null;
  product_id?: unknown;
  customer?: { email?: unknown } | null;
};

type PolarWebhookEvent = { type?: unknown; data?: PolarSubscriptionData | null };

function mappedPlan(env: Env, data: PolarSubscriptionData): Plan | null {
  const productId = typeof data.product?.id === "string" ? data.product.id : data.product_id;
  if (typeof productId !== "string") return null;
  if (env.POLAR_PRODUCT_SOLO && productId === env.POLAR_PRODUCT_SOLO) return "solo";
  if (env.POLAR_PRODUCT_TEAM && productId === env.POLAR_PRODUCT_TEAM) return "team";
  return null;
}

type EntitlementRow = { userId: string; plan: Plan; status: string; periodEnd: string | null; subscriptionId: string | null; updatedAt: string };

async function upsertEntitlement(db: Env["DB"], row: EntitlementRow): Promise<void> {
  await db
    .prepare(
      `INSERT INTO entitlements (user_id, plan, status, period_end, polar_subscription_id, updated_at)
       VALUES (?, ?, ?, ?, ?, ?)
       ON CONFLICT(user_id) DO UPDATE SET
         plan = excluded.plan,
         status = excluded.status,
         period_end = excluded.period_end,
         polar_subscription_id = excluded.polar_subscription_id,
         updated_at = excluded.updated_at`,
    )
    .bind(row.userId, row.plan, row.status, row.periodEnd, row.subscriptionId, row.updatedAt)
    .run();
}

billingRoutes.post("/webhooks/polar", async (c) => {
  // Raw body first: the signature covers these exact bytes, before any parsing.
  const rawBody = await c.req.text();
  if (!c.env.POLAR_WEBHOOK_SECRET) return c.json({ error: "webhook_secret_not_configured" }, 501);

  const verification = await verifyWebhook(rawBody, c.req.raw.headers, c.env.POLAR_WEBHOOK_SECRET, new Date());
  if (!verification.ok) {
    if (verification.reason === "stale_timestamp") return c.json({ error: verification.reason }, 400);
    return c.json({ error: verification.reason }, 401);
  }

  let event: PolarWebhookEvent;
  try {
    event = JSON.parse(rawBody) as PolarWebhookEvent;
  } catch {
    return c.json({ error: "invalid_payload" }, 400);
  }

  if (typeof event.type !== "string" || !event.type.startsWith("subscription.")) {
    return c.json({ ok: true }, 202); // recognized Polar event outside our scope — never 500
  }
  const data = event.data ?? {};

  // Entitlements attach to app accounts by email: Polar only knows the checkout email.
  const email = typeof data.customer?.email === "string" ? data.customer.email.trim().toLowerCase() : null;
  if (!email) return c.json({ ok: true }, 202);
  const user = await c.env.DB.prepare("SELECT id FROM users WHERE email = ?").bind(email).first<{ id: string }>();
  if (!user) return c.json({ ok: true }, 202); // checkout under an email that never registered here

  const existing = await c.env.DB.prepare("SELECT plan, period_end, polar_subscription_id FROM entitlements WHERE user_id = ?")
    .bind(user.id)
    .first<{ plan: string; period_end: string | null; polar_subscription_id: string | null }>();
  const storedPlan: Plan | null =
    existing && (existing.plan === "solo" || existing.plan === "team") ? existing.plan : null;

  const subscriptionId = typeof data.id === "string" ? data.id : (existing?.polar_subscription_id ?? null);
  const payloadPeriodEnd = typeof data.current_period_end === "string" ? data.current_period_end : null;
  const nowIso = new Date().toISOString();

  switch (event.type) {
    case "subscription.paid":
    case "subscription.active": {
      const plan = mappedPlan(c.env, data);
      if (!plan) return c.json({ ok: true }, 202); // unmapped product — nothing to grant
      await upsertEntitlement(c.env.DB, {
        userId: user.id,
        plan,
        status: "active",
        periodEnd: payloadPeriodEnd ?? existing?.period_end ?? null,
        subscriptionId,
        updatedAt: nowIso,
      });
      return c.json({ ok: true });
    }
    case "subscription.canceled": {
      // Keep the last known plan until period_end; with no window at all, cut off now.
      const plan = storedPlan ?? mappedPlan(c.env, data);
      if (!plan) return c.json({ ok: true }, 202);
      await upsertEntitlement(c.env.DB, {
        userId: user.id,
        plan,
        status: "canceled",
        periodEnd: payloadPeriodEnd ?? existing?.period_end ?? nowIso,
        subscriptionId,
        updatedAt: nowIso,
      });
      return c.json({ ok: true });
    }
    case "subscription.updated": {
      const plan = mappedPlan(c.env, data) ?? storedPlan;
      if (!plan) return c.json({ ok: true }, 202);
      // Only genuinely-active subscriptions persist as active; paused/past_due/
      // unpaid get "canceled" so the period_end grace bounds access instead of
      // unlocking features forever without billing.
      await upsertEntitlement(c.env.DB, {
        userId: user.id,
        plan,
        status: data.status === "active" ? "active" : "canceled",
        periodEnd: payloadPeriodEnd ?? existing?.period_end ?? null,
        subscriptionId,
        updatedAt: nowIso,
      });
      return c.json({ ok: true });
    }
    default:
      return c.json({ ok: true }, 202); // subscription event we don't act on
  }
});

// Customer portal link. Requires a Polar org access token with customer_sessions:write.
billingRoutes.get("/billing/portal", authMiddleware, async (c) => {
  if (!c.env.POLAR_ACCESS_TOKEN) return c.json({ error: "not_configured" }, 501);

  const user = await c.env.DB.prepare("SELECT email FROM users WHERE id = ?").bind(c.get("userId")).first<{ email: string }>();
  if (!user) return c.json({ error: "user not found" }, 404);

  // Same API base + Bearer style as the desktop worker (apps/voltrus-plc/cloudflare-worker/worker.ts).
  const polarHeaders: Record<string, string> = {
    Authorization: `Bearer ${c.env.POLAR_ACCESS_TOKEN}`,
    "Content-Type": "application/json",
  };

  const customersUrl = new URL(`${POLAR_API}/customers`);
  customersUrl.searchParams.set("email", user.email);
  const customersRes = await fetch(customersUrl, { headers: polarHeaders });
  if (!customersRes.ok) return c.json({ error: "polar_error" }, 502);
  const customersBody = (await customersRes.json()) as { items?: Array<{ id?: unknown }> };
  let customerId: string | null = null;
  for (const row of customersBody.items ?? []) {
    if (typeof row.id === "string") {
      customerId = row.id;
      break;
    }
  }
  if (!customerId) return c.json({ error: "no_polar_customer" }, 404);

  const sessionRes = await fetch(`${POLAR_API}/customer-sessions`, {
    method: "POST",
    headers: polarHeaders,
    body: JSON.stringify({ customer_id: customerId }),
  });
  if (!sessionRes.ok) return c.json({ error: "polar_error" }, 502);
  const session = (await sessionRes.json()) as { customer_portal_url?: unknown };
  if (typeof session.customer_portal_url !== "string") return c.json({ error: "polar_error" }, 502);
  return c.json({ url: session.customer_portal_url });
});

// Plan, free-tier limits, and checkout links for the paywall UI. effectivePlan
// is grace-aware: a canceled subscription keeps its plan until period_end.
// Null limits mean unlimited; null checkout URLs mean the plan has no
// purchasable link configured (the UI hides that Upgrade button).
billingRoutes.get("/billing/plan", authMiddleware, async (c) => {
  const plan = await effectivePlan(c.env.DB, c.get("userId"));
  return c.json({
    plan,
    limits: {
      maxProjects: plan === "free" ? FREE_MAX_PROJECTS : null,
      maxFilesPerProject: plan === "free" ? FREE_MAX_FILES_PER_PROJECT : null,
    },
    checkout: {
      solo: c.env.POLAR_CHECKOUT_SOLO || null,
      team: c.env.POLAR_CHECKOUT_TEAM || null,
    },
  });
});
