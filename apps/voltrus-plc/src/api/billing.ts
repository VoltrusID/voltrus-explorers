// Billing/entitlement client for the paywall UI (GET /billing/plan and the
// Polar customer-portal link). Shares the fetch/error model of cloud-auth.ts.
import { cloudRequest, CloudApiError } from "./cloud-auth";

export type Plan = "free" | "solo" | "team";

export interface PlanInfo {
  plan: Plan;
  /** Free-tier caps; null = unlimited (paid plans). */
  limits: {
    maxProjects: number | null;
    maxFilesPerProject: number | null;
  };
  /** Polar checkout URLs; null = the plan has no purchasable link configured. */
  checkout: {
    solo: string | null;
    team: string | null;
  };
}

export function getPlan(token: string): Promise<PlanInfo> {
  return cloudRequest<PlanInfo>("/api/billing/plan", {}, token);
}

/** Opens the Polar customer portal for the signed-in user. Throws CloudApiError on failure. */
export async function openBillingPortal(token: string): Promise<void> {
  const { url } = await cloudRequest<{ url: string }>("/api/billing/portal", {}, token);
  window.open(url, "_blank", "noopener");
}

export { CloudApiError };
