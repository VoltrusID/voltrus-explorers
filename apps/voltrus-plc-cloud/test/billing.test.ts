import { describe, expect, it, vi, afterEach } from "vitest";
import { env, SELF } from "cloudflare:test";
import { issueToken } from "../src/auth";
import type { AppEnv } from "../src/env";
import { billingRoutes } from "../src/routes/billing";
import { effectivePlan, getPlan, setPlanForTest } from "../src/entitlements";

const JWT_SECRET = "test-secret-change-me";
const WEBHOOK_SECRET = "whsec_dGVzdC1vbmx5LW5vdC1hLXJlYWwtc2VjcmV0"; // gitleaks:allow (dummy test secret)
const PRODUCT_SOLO = "prod_solo_00000000";
const PRODUCT_TEAM = "prod_team_00000000";

// billing routes only touch DB + Polar vars, so the Relay DO binding is omitted here.
function billingEnv(overrides: Partial<AppEnv["Bindings"]> = {}): AppEnv["Bindings"] {
  return {
    DB: env.DB,
    FILES: env.FILES,
    JWT_SECRET,
    CORS_ORIGINS: "http://localhost:1440",
    POLAR_WEBHOOK_SECRET: WEBHOOK_SECRET,
    POLAR_PRODUCT_SOLO: PRODUCT_SOLO,
    POLAR_PRODUCT_TEAM: PRODUCT_TEAM,
    ...overrides,
  } as unknown as AppEnv["Bindings"];
}

let userSeq = 0;
async function insertUser(): Promise<{ id: string; email: string }> {
  const id = crypto.randomUUID();
  const email = `polar-${++userSeq}-${crypto.randomUUID()}@example.com`;
  await env.DB.prepare("INSERT INTO users (id, email, password_hash, salt, created_at) VALUES (?, ?, 'test-hash', 'test-salt', ?)")
    .bind(id, email, new Date().toISOString())
    .run();
  return { id, email };
}

async function getEntitlement(userId: string): Promise<{ plan: string; status: string; period_end: string | null; polar_subscription_id: string | null } | null> {
  return env.DB.prepare("SELECT plan, status, period_end, polar_subscription_id FROM entitlements WHERE user_id = ?")
    .bind(userId)
    .first();
}

// Recompute signatures independently of the implementation under test.
async function hmacSha256Base64(secret: string, content: string): Promise<string> {
  const key = await crypto.subtle.importKey("raw", new TextEncoder().encode(secret), { name: "HMAC", hash: "SHA-256" }, false, [
    "sign",
  ]);
  const mac = await crypto.subtle.sign("HMAC", key, new TextEncoder().encode(content));
  let binary = "";
  for (const byte of new Uint8Array(mac)) binary += String.fromCharCode(byte);
  return btoa(binary);
}

function subscriptionEvent(type: string, email: string, data: Record<string, unknown> = {}): string {
  return JSON.stringify({
    type,
    data: {
      id: `sub_${crypto.randomUUID().slice(0, 8)}`,
      status: "active",
      current_period_end: "2027-01-01T00:00:00.000Z",
      product: { id: PRODUCT_SOLO },
      customer: { email },
      ...data,
    },
  });
}

async function postWebhook(body: string, headers: Record<string, string>): Promise<Response> {
  return billingRoutes.request("/webhooks/polar", { method: "POST", headers, body }, billingEnv());
}

async function legacySigned(body: string, secret = WEBHOOK_SECRET, timestampOffsetSeconds = 0): Promise<Record<string, string>> {
  const t = Math.floor(Date.now() / 1000) + timestampOffsetSeconds;
  const signature = await hmacSha256Base64(secret, `${t}.${body}`);
  return { "polar-signature": `t=${t},v1=${signature}` };
}

async function standardSigned(body: string, secret = WEBHOOK_SECRET, timestampOffsetSeconds = 0): Promise<Record<string, string>> {
  const t = Math.floor(Date.now() / 1000) + timestampOffsetSeconds;
  const signature = await hmacSha256Base64(secret, `msg_${t}.${t}.${body}`);
  return { "webhook-id": `msg_${t}`, "webhook-timestamp": `${t}`, "webhook-signature": `v1,${signature}` };
}

describe("POST /webhooks/polar signature verification", () => {
  it("accepts a valid legacy signature and upserts the entitlement", async () => {
    const user = await insertUser();
    const body = subscriptionEvent("subscription.paid", user.email, { id: "sub_paid_1" });
    const res = await postWebhook(body, await legacySigned(body));

    expect(res.status).toBe(200);
    expect(await res.json()).toEqual({ ok: true });
    const row = await getEntitlement(user.id);
    expect(row).toMatchObject({
      plan: "solo",
      status: "active",
      period_end: "2027-01-01T00:00:00.000Z",
      polar_subscription_id: "sub_paid_1",
    });
  });

  it("rejects a signature made with the wrong secret", async () => {
    const user = await insertUser();
    const body = subscriptionEvent("subscription.paid", user.email);
    const res = await postWebhook(body, await legacySigned(body, "whsec_wrong-secret"));

    expect(res.status).toBe(401);
    expect(await getEntitlement(user.id)).toBeNull();
  });

  it("rejects a tampered body signed with the real secret", async () => {
    const user = await insertUser();
    const signed = subscriptionEvent("subscription.paid", user.email);
    const tampered = subscriptionEvent("subscription.paid", user.email, { product: { id: PRODUCT_TEAM } });
    const res = await postWebhook(tampered, await legacySigned(signed));

    expect(res.status).toBe(401);
    expect(await getEntitlement(user.id)).toBeNull();
  });

  it("rejects stale timestamps (>5 minutes old) with 400", async () => {
    const user = await insertUser();
    const body = subscriptionEvent("subscription.paid", user.email);
    const res = await postWebhook(body, await legacySigned(body, WEBHOOK_SECRET, -301));

    expect(res.status).toBe(400);
    expect(await res.json()).toEqual({ error: "stale_timestamp" });
    expect(await getEntitlement(user.id)).toBeNull();
  });

  it("rejects missing signature headers", async () => {
    const user = await insertUser();
    const res = await postWebhook(subscriptionEvent("subscription.paid", user.email), {});

    expect(res.status).toBe(401);
    expect(await res.json()).toEqual({ error: "missing_signature" });
  });

  it("accepts Standard Webhooks signatures (post-2026-09 secrets)", async () => {
    const user = await insertUser();
    const body = subscriptionEvent("subscription.paid", user.email);
    const res = await postWebhook(body, await standardSigned(body));

    expect(res.status).toBe(200);
    expect((await getEntitlement(user.id))?.plan).toBe("solo");
  });
});

describe("POST /webhooks/polar event handling", () => {
  it("ignores unmapped products with 202 and no entitlement", async () => {
    const user = await insertUser();
    const body = subscriptionEvent("subscription.paid", user.email, { product: { id: "prod_something_else" } });
    const res = await postWebhook(body, await legacySigned(body));

    expect(res.status).toBe(202);
    expect(await getEntitlement(user.id)).toBeNull();
  });

  it("acknowledges unrecognized event types with 202 and never 500s", async () => {
    const user = await insertUser();
    const body = subscriptionEvent("order.paid", user.email);
    const res = await postWebhook(body, await legacySigned(body));

    expect(res.status).toBe(202);
    expect(await getEntitlement(user.id)).toBeNull();
  });

  it("ignores events for customers with no matching account", async () => {
    await insertUser();
    const body = subscriptionEvent("subscription.paid", `nobody-${crypto.randomUUID()}@example.com`);
    const res = await postWebhook(body, await legacySigned(body));

    expect(res.status).toBe(202);
  });

  it("canceled keeps the plan until a future period_end, honoring expiry in effectivePlan", async () => {
    const user = await insertUser();
    const paid = subscriptionEvent("subscription.paid", user.email);
    await postWebhook(paid, await legacySigned(paid));

    const canceled = subscriptionEvent("subscription.canceled", user.email, {
      status: "canceled",
      current_period_end: "2099-01-01T00:00:00.000Z",
    });
    const res = await postWebhook(canceled, await legacySigned(canceled));
    expect(res.status).toBe(200);

    expect(await effectivePlan(env.DB, user.id)).toBe("solo"); // inside the grace window
    const row = await getEntitlement(user.id);
    expect(row).toMatchObject({ plan: "solo", status: "canceled" });

    // Expiry: a canceled subscription whose window has passed gates as free.
    const expired = subscriptionEvent("subscription.canceled", user.email, {
      status: "canceled",
      current_period_end: "2020-01-01T00:00:00.000Z",
    });
    await postWebhook(expired, await legacySigned(expired));
    expect(await effectivePlan(env.DB, user.id)).toBe("free");
    expect(await getPlan(env.DB, user.id)).toBe("solo"); // raw row keeps the last plan
  });

  it("canceled with no period end anywhere cuts off now", async () => {
    const user = await insertUser();
    // Paid event that never carried a period end, so the row has no window to keep.
    const paid = subscriptionEvent("subscription.paid", user.email);
    const paidEvent = JSON.parse(paid) as { data: Record<string, unknown> };
    delete paidEvent.data.current_period_end;
    const paidBody = JSON.stringify(paidEvent);
    await postWebhook(paidBody, await legacySigned(paidBody));
    expect((await getEntitlement(user.id))?.period_end).toBeNull();

    const canceled = JSON.parse(subscriptionEvent("subscription.canceled", user.email, { status: "canceled" })) as {
      data: Record<string, unknown>;
    };
    delete canceled.data.current_period_end;
    const body = JSON.stringify(canceled);
    const res = await postWebhook(body, await legacySigned(body));
    expect(res.status).toBe(200);

    const row = await getEntitlement(user.id);
    expect(row).toMatchObject({ status: "canceled" });
    expect(typeof row?.period_end).toBe("string"); // defaulted to "now"
    expect(await effectivePlan(env.DB, user.id)).toBe("free");
  });

  it("subscription.updated syncs plan and status", async () => {
    const user = await insertUser();
    const paid = subscriptionEvent("subscription.paid", user.email);
    await postWebhook(paid, await legacySigned(paid));

    const upgraded = subscriptionEvent("subscription.updated", user.email, { product: { id: PRODUCT_TEAM } });
    await postWebhook(upgraded, await legacySigned(upgraded));
    expect(await getEntitlement(user.id)).toMatchObject({ plan: "team", status: "active" });
    expect(await effectivePlan(env.DB, user.id)).toBe("team");

    // An updated event with an unmapped product syncs status while keeping the plan.
    const canceled = subscriptionEvent("subscription.updated", user.email, {
      status: "canceled",
      product: { id: "prod_something_else" },
    });
    await postWebhook(canceled, await legacySigned(canceled));
    expect(await getEntitlement(user.id)).toMatchObject({ plan: "team", status: "canceled" });
  });

  it("subscription.updated with paused status does not stay active forever", async () => {
    const user = await insertUser();
    const paid = subscriptionEvent("subscription.paid", user.email);
    await postWebhook(paid, await legacySigned(paid));
    expect(await getEntitlement(user.id)).toMatchObject({ status: "active" });

    // Polar pauses billing without a canceled event; benefits are revoked now.
    // The row becomes canceled; the period_end carried over from the last
    // payment bounds any remaining grace — here it is already lapsed.
    const paused = subscriptionEvent("subscription.updated", user.email, {
      status: "paused",
      product: { id: PRODUCT_TEAM },
      current_period_end: new Date(Date.now() - 1000).toISOString(),
    });
    await postWebhook(paused, await legacySigned(paused));
    expect(await getEntitlement(user.id)).toMatchObject({ status: "canceled" });
    expect(await effectivePlan(env.DB, user.id)).toBe("free");
  });

  it("team seats grant team regardless of entitlements rows", async () => {
    const user = await insertUser();
    const subscriptionId = `sub_${crypto.randomUUID().slice(0, 8)}`;
    await env.DB
      .prepare("INSERT INTO team_seats (team_subscription_id, user_id, invited_by, created_at) VALUES (?, ?, NULL, ?)")
      .bind(subscriptionId, user.id, new Date().toISOString())
      .run();

    expect(await getPlan(env.DB, user.id)).toBe("free"); // no entitlements row
    expect(await effectivePlan(env.DB, user.id)).toBe("team");

    await env.DB.prepare("DELETE FROM team_seats WHERE user_id = ?").bind(user.id).run();
    expect(await effectivePlan(env.DB, user.id)).toBe("free");
  });
});

describe("free-tier limits on project and file routes", () => {
  const jsonHeaders = (token: string) => ({ Authorization: `Bearer ${token}`, "Content-Type": "application/json" });

  async function createProject(token: string, name: string): Promise<Response> {
    return SELF.fetch("https://example.com/api/projects", { method: "POST", headers: jsonHeaders(token), body: JSON.stringify({ name }) });
  }

  async function putFile(token: string, projectId: string, path: string): Promise<Response> {
    return SELF.fetch(`https://example.com/api/projects/${projectId}/files/${path}`, {
      method: "PUT",
      headers: jsonHeaders(token),
      body: JSON.stringify({ content: "x" }),
    });
  }

  it("returns 402 plan_required at the free project limit and passes once solo", async () => {
    const user = await insertUser();
    const token = await issueToken(JWT_SECRET, user.id);

    const first = await createProject(token, "First");
    expect(first.status).toBe(201);
    const second = await createProject(token, "Second");
    expect(second.status).toBe(402);
    expect(await second.json()).toEqual({ error: "plan_required", requiredPlan: "solo" });

    await setPlanForTest(env.DB, user.id, "solo");
    const third = await createProject(token, "Third");
    expect(third.status).toBe(201);
  });

  it("returns 402 plan_required on the 26th file for free users, with overwrites exempt", async () => {
    const user = await insertUser();
    const token = await issueToken(JWT_SECRET, user.id);

    const project = await createProject(token, "Files");
    expect(project.status).toBe(201);
    const { id } = (await project.json()) as { id: string };

    for (let i = 1; i <= 25; i++) {
      const put = await putFile(token, id, `file-${i}.txt`);
      expect(put.status).toBe(204);
    }
    const twentySixth = await putFile(token, id, "file-26.txt");
    expect(twentySixth.status).toBe(402);
    expect(await twentySixth.json()).toEqual({ error: "plan_required", requiredPlan: "solo" });

    // Overwriting an existing file must never count against the quota.
    const overwrite = await putFile(token, id, "file-1.txt");
    expect(overwrite.status).toBe(204);

    await setPlanForTest(env.DB, user.id, "solo");
    const unblocked = await putFile(token, id, "file-26.txt");
    expect(unblocked.status).toBe(204);
  });
});

describe("GET /billing/portal", () => {
  it("returns 501 not_configured without POLAR_ACCESS_TOKEN", async () => {
    const user = await insertUser();
    const token = await issueToken(JWT_SECRET, user.id);
    const res = await billingRoutes.request(
      "/billing/portal",
      { headers: { Authorization: `Bearer ${token}` } },
      billingEnv({ POLAR_ACCESS_TOKEN: undefined }),
    );

    expect(res.status).toBe(501);
    expect(await res.json()).toEqual({ error: "not_configured" });
  });

  describe("with POLAR_ACCESS_TOKEN configured", () => {
    // cloudflare:test does not export fetchMock in this setup, so the Polar API
    // is stubbed at the global level — no network leaves the test worker.
    afterEach(() => {
      vi.unstubAllGlobals();
    });

    function stubPolarApi(knownCustomerEmail: string | null): void {
      const handler = async (input: RequestInfo | URL): Promise<Response> => {
        const url = input instanceof Request ? input.url : String(input);
        if (url.startsWith("https://api.polar.sh/v1/customers")) {
          const requested = new URL(url).searchParams.get("email");
          const known = requested !== null && requested === knownCustomerEmail;
          return new Response(JSON.stringify({ items: known ? [{ id: "cust_123" }] : [] }), {
            status: 200,
            headers: { "content-type": "application/json" },
          });
        }
        if (url === "https://api.polar.sh/v1/customer-sessions") {
          return new Response(JSON.stringify({ customer_portal_url: "https://polar.sh/portal/session" }), {
            status: 201,
            headers: { "content-type": "application/json" },
          });
        }
        return new Response(`unexpected fetch: ${url}`, { status: 500 });
      };
      vi.stubGlobal("fetch", vi.fn(handler));
    }

    it("exchanges the account email for a Polar customer portal url", async () => {
      const user = await insertUser();
      stubPolarApi(user.email);
      const token = await issueToken(JWT_SECRET, user.id);

      const res = await billingRoutes.request(
        "/billing/portal",
        { headers: { Authorization: `Bearer ${token}` } },
        billingEnv({ POLAR_ACCESS_TOKEN: "polar_oat_test" }),
      );

      expect(res.status).toBe(200);
      expect(await res.json()).toEqual({ url: "https://polar.sh/portal/session" });
    });

    it("returns 404 no_polar_customer when the account has no Polar customer", async () => {
      const user = await insertUser();
      stubPolarApi(null);
      const token = await issueToken(JWT_SECRET, user.id);

      const res = await billingRoutes.request(
        "/billing/portal",
        { headers: { Authorization: `Bearer ${token}` } },
        billingEnv({ POLAR_ACCESS_TOKEN: "polar_oat_test" }),
      );

      expect(res.status).toBe(404);
      expect(await res.json()).toEqual({ error: "no_polar_customer" });
    });
  });
});

describe("GET /billing/plan", () => {
  it("returns free limits and null checkout links when none are configured", async () => {
    const user = await insertUser();
    const token = await issueToken(JWT_SECRET, user.id);

    const res = await billingRoutes.request(
      "/billing/plan",
      { headers: { Authorization: `Bearer ${token}` } },
      billingEnv(),
    );

    expect(res.status).toBe(200);
    expect(await res.json()).toEqual({
      plan: "free",
      limits: { maxProjects: 1, maxFilesPerProject: 25 },
      checkout: { solo: null, team: null },
    });
  });

  it("reports paid plans without free limits and surfaces configured checkout URLs", async () => {
    const user = await insertUser();
    await setPlanForTest(env.DB, user.id, "solo");
    const token = await issueToken(JWT_SECRET, user.id);

    const res = await billingRoutes.request(
      "/billing/plan",
      { headers: { Authorization: `Bearer ${token}` } },
      billingEnv({ POLAR_CHECKOUT_SOLO: "https://polar.sh/checkout/solo" }),
    );

    expect(res.status).toBe(200);
    expect(await res.json()).toEqual({
      plan: "solo",
      limits: { maxProjects: null, maxFilesPerProject: null },
      checkout: { solo: "https://polar.sh/checkout/solo", team: null },
    });
  });

  it("keeps a canceled plan during the period_end grace window", async () => {
    const user = await insertUser();
    const future = new Date(Date.now() + 7 * 24 * 3600 * 1000).toISOString();
    await env.DB
      .prepare(
        `INSERT INTO entitlements (user_id, plan, status, period_end, polar_subscription_id, updated_at)
         VALUES (?, 'team', 'canceled', ?, 'sub_1', ?)`,
      )
      .bind(user.id, future, new Date().toISOString())
      .run();
    const token = await issueToken(JWT_SECRET, user.id);

    const res = await billingRoutes.request(
      "/billing/plan",
      { headers: { Authorization: `Bearer ${token}` } },
      billingEnv(),
    );

    expect(res.status).toBe(200);
    const body = (await res.json()) as { plan: string };
    expect(body.plan).toBe("team");
  });

  it("requires auth", async () => {
    const res = await billingRoutes.request("/billing/plan", {}, billingEnv());
    expect(res.status).toBe(401);
  });
});
