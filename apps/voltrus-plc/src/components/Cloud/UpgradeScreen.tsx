import { useEffect } from "react";
import { useCloudStore } from "../../store/cloud";

// Paywall overlay. Raised automatically by any 402 plan_required from the
// cloud API (project/file free-tier caps) or manually from AccountBadge.
// Never blocks the editor: "Not now" returns to local work.
const PLANS: Array<{
  id: "free" | "solo" | "team";
  name: string;
  price: string;
  blurb: string;
  features: string[];
}> = [
  {
    id: "free",
    name: "Free",
    price: "$0",
    blurb: "Try everything in the browser",
    features: ["1 cloud project", "25 files per project", "Full IEC 61131-3 editor", "Browser simulation"],
  },
  {
    id: "solo",
    name: "Solo",
    price: "$12/mo",
    blurb: "For the solo integrator",
    features: ["Unlimited cloud projects", "Unlimited files", "Device relay access", "AI assistants included"],
  },
  {
    id: "team",
    name: "Team",
    price: "$29/mo",
    blurb: "For panels shops and crews",
    features: ["Everything in Solo", "Team seats & shared projects", "Priority relay", "Priority support"],
  },
];

function planLabel(plan: string | null | undefined): string {
  return plan === "solo" || plan === "team" ? plan : "free";
}

export default function UpgradeScreen() {
  const planGate = useCloudStore((s) => s.planGate);
  const plan = useCloudStore((s) => s.plan);
  const refreshPlan = useCloudStore((s) => s.refreshPlan);
  const refreshProjects = useCloudStore((s) => s.refreshProjects);
  const openPortal = useCloudStore((s) => s.openPortal);
  const portalBusy = useCloudStore((s) => s.portalBusy);
  const token = useCloudStore((s) => s.token);
  const dismissUpgrade = useCloudStore((s) => s.dismissUpgrade);

  const open = planGate;

  // First open with no fetched plan → fetch. Also refetch when the user comes
  // back from a Polar checkout tab so a just-granted entitlement lands.
  useEffect(() => {
    if (open) void refreshPlan();
  }, [open, refreshPlan]);

  useEffect(() => {
    if (!open) return;
    const onVisible = () => {
      if (document.visibilityState === "visible") {
        void refreshPlan();
        void refreshProjects();
      }
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => document.removeEventListener("visibilitychange", onVisible);
  }, [open, refreshPlan, refreshProjects]);

  if (!open) return null;

  const current = planLabel(plan?.plan);

  const checkoutFor = (id: "solo" | "team"): string | null => plan?.checkout[id] ?? null;

  const upgrade = (id: "solo" | "team") => {
    const url = checkoutFor(id);
    if (url) window.open(url, "_blank", "noopener");
  };

  return (
    <div className="modal-overlay" onClick={dismissUpgrade}>
      <div
        className="upgrade-modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="upgrade-title"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="upgrade-head">
          <h3 id="upgrade-title">Cloud plans</h3>
          <span className={`plan-chip plan-chip-${current}`}>{current}</span>
        </div>
        <p className="upgrade-sub">
          {plan?.limits.maxProjects != null
            ? `Free plan: ${plan.limits.maxProjects} cloud project, ${plan.limits.maxFilesPerProject} files per project. Upgrade for unlimited cloud sync.`
            : "Pick the plan that fits how you ship projects."}
        </p>

        <div className="plan-cards">
          {PLANS.map((p) => {
            const isCurrent = p.id === current;
            const url = p.id === "free" ? null : checkoutFor(p.id);
            return (
              <div key={p.id} className={`plan-card ${p.id === "solo" ? "featured" : ""} ${isCurrent ? "current" : ""}`}>
                <div className="plan-card-head">
                  <strong>{p.name}</strong>
                  <span className="plan-price">{p.price}</span>
                </div>
                <div className="plan-blurb">{p.blurb}</div>
                <ul className="plan-features">
                  {p.features.map((f) => (
                    <li key={f}>{f}</li>
                  ))}
                </ul>
                {isCurrent ? (
                  <button className="toolbar-btn" disabled>
                    Current plan
                  </button>
                ) : url ? (
                  <button className="toolbar-btn primary" onClick={() => upgrade(p.id as "solo" | "team")}>
                    Upgrade to {p.name}
                  </button>
                ) : p.id === "free" ? (
                  <button className="toolbar-btn" onClick={dismissUpgrade}>
                    Keep using Free
                  </button>
                ) : (
                  <button className="toolbar-btn" disabled title="Checkout link not configured yet">
                    Coming soon
                  </button>
                )}
              </div>
            );
          })}
        </div>

        <div className="upgrade-actions">
          {token && (
            <button className="toolbar-btn" onClick={() => void openPortal()} disabled={portalBusy}>
              {portalBusy ? "Opening portal…" : "Manage billing"}
            </button>
          )}
          <button className="toolbar-btn" onClick={dismissUpgrade}>
            Not now
          </button>
        </div>
      </div>
    </div>
  );
}
