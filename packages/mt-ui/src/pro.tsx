import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Check, Sparkles } from "lucide-react";
import { Badge, Button, Callout, Dialog } from "./primitives";

/** Mirrors `mt_store::StoreStatus`. */
export interface StoreStatus {
  /** Pro is sold in this build (App Store build). */
  store: boolean;
  pro: boolean;
  reason: "build" | "grandfathered" | "purchased" | "none";
  price: string | null;
}

interface ProContextValue {
  /** `null` until the first status arrives; Pro features stay locked meanwhile. */
  status: StoreStatus | null;
  pro: boolean;
  /** True when Pro is unlocked; otherwise opens the upsell for `feature` and returns false. */
  requirePro: (feature: string) => boolean;
  openUpsell: (feature?: string) => void;
}

const ProContext = createContext<ProContextValue | null>(null);

export function useProStatus(): ProContextValue {
  const ctx = useContext(ProContext);
  if (!ctx) throw new Error("useProStatus must be used inside <ProProvider>");
  return ctx;
}

export interface ProProviderProps {
  appName: string;
  /** What Pro unlocks, one line each, shown in the upsell. */
  features: string[];
  children: ReactNode;
}

export function ProProvider({ appName, features, children }: ProProviderProps) {
  const [status, setStatus] = useState<StoreStatus | null>(null);
  const [upsell, setUpsell] = useState<{ feature?: string } | null>(null);
  const [busy, setBusy] = useState<"purchase" | "restore" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  useEffect(() => {
    invoke<StoreStatus>("store_status")
      .then(setStatus)
      .catch((e) => {
        // Store build whose StoreKit query failed: stay on the free tier; purchase/restore still work.
        setStatus({ store: true, pro: false, reason: "none", price: null });
        setError(String(e));
      });
  }, []);
  const pro = status?.pro ?? false;

  const openUpsell = useCallback((feature?: string) => {
    setError(null);
    setNotice(null);
    setUpsell({ feature });
  }, []);

  const requirePro = useCallback(
    (feature: string) => {
      if (pro) return true;
      openUpsell(feature);
      return false;
    },
    [pro, openUpsell],
  );

  const run = async (kind: "purchase" | "restore") => {
    setBusy(kind);
    setError(null);
    setNotice(null);
    try {
      const next = await invoke<StoreStatus>(kind === "purchase" ? "store_purchase" : "store_restore");
      setStatus(next);
      if (next.pro) setUpsell(null);
      else if (kind === "restore") setNotice("No previous Pro purchase was found for this Apple ID.");
      else setNotice("The purchase did not complete. If it needs approval (Ask to Buy), Pro unlocks once it is approved.");
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  const value = useMemo(() => ({ status, pro, requirePro, openUpsell }), [status, pro, requirePro, openUpsell]);

  return (
    <ProContext.Provider value={value}>
      {children}
      <Dialog
        open={upsell !== null}
        onClose={() => busy === null && setUpsell(null)}
        title={`${appName} Pro`}
        width={420}
        footer={
          <>
            <Button variant="ghost" loading={busy === "restore"} disabled={busy !== null} onClick={() => run("restore")}>
              Restore purchase
            </Button>
            <Button
              variant="primary"
              icon={<Sparkles size={14} strokeWidth={1.75} />}
              loading={busy === "purchase"}
              disabled={busy !== null}
              onClick={() => run("purchase")}
            >
              {status?.price ? `Unlock Pro · ${status.price}` : "Unlock Pro"}
            </Button>
          </>
        }
      >
        <div style={{ display: "flex", flexDirection: "column", gap: 12, fontSize: "var(--fs-sm)", color: "var(--text-dim)", lineHeight: 1.5 }}>
          {upsell?.feature && (
            <p style={{ margin: 0, color: "var(--text)" }}>
              <strong>{upsell.feature}</strong> is part of Pro.
            </p>
          )}
          <p style={{ margin: 0 }}>A one-time purchase. No subscription, no account.</p>
          <ul style={{ listStyle: "none", margin: 0, padding: 0, display: "flex", flexDirection: "column", gap: 6 }}>
            {features.map((f) => (
              <li key={f} style={{ display: "flex", gap: 8, alignItems: "flex-start" }}>
                <Check size={14} strokeWidth={2} style={{ color: "var(--accent)", flexShrink: 0, marginTop: 3 }} />
                <span>{f}</span>
              </li>
            ))}
          </ul>
          {notice && <Callout tone="info">{notice}</Callout>}
          {error && <Callout tone="danger">{error}</Callout>}
        </div>
      </Dialog>
    </ProContext.Provider>
  );
}

/** Small "PRO" marker next to a locked control; renders nothing once Pro is unlocked. */
export function ProBadge() {
  const { pro } = useProStatus();
  if (pro) return null;
  return (
    <Badge tone="accent" aria-label="Pro feature">
      PRO
    </Badge>
  );
}

/** Pro row for an About dialog: state plus purchase/restore entry point. Hidden in source builds. */
export function ProAboutRow() {
  const { status, openUpsell } = useProStatus();
  if (!status?.store) return null;
  const label =
    status.reason === "purchased" ? "Pro unlocked. Thank you!" :
    status.reason === "grandfathered" ? "Pro included with your original purchase." :
    "Free version";
  return (
    <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: 12 }}>
      <span style={{ fontSize: "var(--fs-sm)", color: "var(--text-dim)" }}>{label}</span>
      {!status.pro && (
        <Button size="sm" variant="primary" icon={<Sparkles size={14} strokeWidth={1.75} />} onClick={() => openUpsell()}>
          Unlock Pro
        </Button>
      )}
    </div>
  );
}
