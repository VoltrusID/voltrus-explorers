// Entitlement plans and free-tier limits. The DB is the source of truth; Polar
// webhooks (src/routes/billing.ts) keep it in sync, routes only read through here.
import type { AppEnv } from "./env";

export type Plan = "free" | "solo" | "team";

export const FREE_MAX_PROJECTS = 1;
export const FREE_MAX_FILES_PER_PROJECT = 25;

type DB = AppEnv["Bindings"]["DB"];

export type PlanGate = {
  status: 402;
  body: { error: "plan_required"; requiredPlan: "solo" | "team" };
};

const PLAN_RANK: Record<Plan, number> = { free: 0, solo: 1, team: 2 };

/** Raw stored plan — missing row means free. Ignores cancellation grace windows. */
export async function getPlan(db: DB, userId: string): Promise<Plan> {
  const row = await db
    .prepare("SELECT plan FROM entitlements WHERE user_id = ?")
    .bind(userId)
    .first<{ plan: string }>();
  return row && (row.plan === "solo" || row.plan === "team") ? row.plan : "free";
}

/**
 * Plan a request should be gated by: a team_seats row grants 'team' outright;
 * a canceled (or otherwise lapsed) entitlement keeps its plan only until
 * period_end passes. Rows the webhooks never wrote stay free.
 */
export async function effectivePlan(db: DB, userId: string, now: Date = new Date()): Promise<Plan> {
  const seat = await db
    .prepare("SELECT 1 AS member FROM team_seats WHERE user_id = ?")
    .bind(userId)
    .first<{ member: number }>();
  if (seat) return "team";

  const row = await db
    .prepare("SELECT plan, status, period_end FROM entitlements WHERE user_id = ?")
    .bind(userId)
    .first<{ plan: string; status: string; period_end: string | null }>();
  if (!row) return "free";

  const plan = row.plan === "solo" || row.plan === "team" ? row.plan : "free";
  if (plan === "free") return "free";
  if (row.status === "active") return plan;
  // Grace window: the plan survives until period_end, then reverts to free.
  return row.period_end && new Date(row.period_end) > now ? plan : "free";
}

/** Generic paid-plan gate: null means the caller may proceed. */
export async function requirePlan(db: DB, userId: string, required: "solo" | "team"): Promise<PlanGate | null> {
  const current = await effectivePlan(db, userId);
  if (PLAN_RANK[current] >= PLAN_RANK[required]) return null;
  return { status: 402, body: { error: "plan_required", requiredPlan: required } };
}

/**
 * Test/dev helper: writes an active entitlement directly, bypassing Polar
 * webhooks. Passing 'free' removes the row so the user falls back to default
 * free-tier behavior.
 */
export async function setPlanForTest(db: DB, userId: string, plan: Plan): Promise<void> {
  if (plan === "free") {
    await db.prepare("DELETE FROM entitlements WHERE user_id = ?").bind(userId).run();
    return;
  }
  const now = new Date().toISOString();
  await db
    .prepare(
      `INSERT INTO entitlements (user_id, plan, status, period_end, polar_subscription_id, updated_at)
       VALUES (?, ?, 'active', NULL, NULL, ?)
       ON CONFLICT(user_id) DO UPDATE SET plan = excluded.plan, status = 'active', updated_at = excluded.updated_at`,
    )
    .bind(userId, plan, now)
    .run();
}
