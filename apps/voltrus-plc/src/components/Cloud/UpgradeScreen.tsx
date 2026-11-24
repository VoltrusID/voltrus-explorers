import { useEffect } from "react";
import { Button, Dialog } from "@mactools/ui";
import { Check } from "lucide-react";
import { useCloudStore } from "../../store/cloud";
import "./cloud.css";

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

  const current = plan?.plan === "solo" || plan?.plan === "team" ? plan.plan : "free";

  const upgrade = (id: "solo" | "team") => {
    const url = plan?.checkout[id] ?? null;
    if (url) window.open(url, "_blank", "noopener");
  };

  return (
    <Dialog
      open
      onClose={dismissUpgrade}
      title="Cloud plans"
      width={760}
      footer={
        <>
          {token && (
            <Button variant="ghost" onClick={() => void openPortal()} disabled={portalBusy}>
              {portalBusy ? "Opening portal…" : "Manage billing"}
            </Button>
          )}
          <Button onClick={dismissUpgrade}>Not now</Button>
        </>
      }
    >
      <div className="cld-row" style={{ padding: 0 }}>
        <span className={`plan-chip plan-chip-${current}`}>{current}</span>
        <p className="cld-upgrade-sub">
          {plan?.limits.maxProjects != null
            ? `Free plan: ${plan.limits.maxProjects} cloud project, ${plan.limits.maxFilesPerProject} files per project. Upgrade for unlimited cloud sync.`
            : "Pick the plan that fits how you ship projects."}
        </p>

        <div className="cld-plans">
          {PLANS.map((p) => {
            const isCurrent = p.id === current;
            const url = p.id === "free" ? null : plan?.checkout[p.id] ?? null;
            return (
              <div
                key={p.id}
                className={`cld-plan-card${p.id === "solo" ? " cld-plan-card--featured" : ""}${isCurrent ? " cld-plan-card--current" : ""}`}
              >
                <div className="cld-plan-card__head">
                  <span className="cld-plan-card__name">{p.name}</span>
                  <span className="cld-plan-card__price">{p.price}</span>
                </div>
                <div className="cld-plan-card__blurb">{p.blurb}</div>
                <ul className="cld-plan-card__features">
                  {p.features.map((f) => (
                    <li key={f}>
                      <Check size={12} strokeWidth={1.75} />
                      {f}
                    </li>
                  ))}
                </ul>
                <div className="cld-plan-card__spacer" />
                {isCurrent ? (
                  <Button block size="sm" disabled>
                    Current plan
                  </Button>
                ) : url ? (
                  <Button
                    block
                    size="sm"
                    variant={p.id === "solo" ? "primary" : "default"}
                    onClick={() => upgrade(p.id as "solo" | "team")}
                  >
                    Upgrade to {p.name}
                  </Button>
                ) : p.id === "free" ? (
                  <Button block size="sm" onClick={dismissUpgrade}>
                    Keep using Free
                  </Button>
                ) : (
                  <Button block size="sm" disabled title="Checkout link not configured yet">
                    Coming soon
                  </Button>
                )}
              </div>
            );
          })}
        </div>
      </div>
    </Dialog>
  );
}
