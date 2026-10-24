import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Check, ExternalLink, KeyRound, Sparkles } from "lucide-react";
import { Badge, Button, Callout, Dialog, Field } from "./primitives";

/** Mirrors `mt_store::StoreStatus`. */
export interface StoreStatus {
  /** Pro is sold in this build (App Store build). */
  store: boolean;
  pro: boolean;
  reason: "build" | "grandfathered" | "purchased" | "key" | "none";
  price: string | null;
  /** Pro is unlocked by entering a licence key (`store_activate`) instead of a store purchase. */
  activation?: boolean;
  /** Where to buy a key, for builds with `activation`. */
  buy_url?: string | null;
}

interface ProContextValue {
  /** `null` until the first status arrives; Pro features stay locked meanwhile. */
  status: StoreStatus | null;
  pro: boolean;
  /** True when Pro is unlocked; otherwise opens the upsell for `feature` and returns false. */
  requirePro: (feature: string) => boolean;
  openUpsell: (feature?: string) => void;
  /** Replaces the status after a command that returned a new one (for example removing a licence key). */
  applyStatus: (status: StoreStatus) => void;
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
  const [busy, setBusy] = useState<"purchase" | "restore" | "activate" | null>(null);
  const [keyInput, setKeyInput] = useState("");
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

  const activate = async () => {
    setBusy("activate");
    setError(null);
    setNotice(null);
    try {
      const next = await invoke<StoreStatus>("store_activate", { key: keyInput });
      setStatus(next);
      if (next.pro) {
        setKeyInput("");
        setUpsell(null);
      } else setNotice("The key was accepted but Pro did not unlock. Restart the app and try again.");
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  const buy = () => {
    if (status?.buy_url) void invoke("connect_open_url", { url: status.buy_url }).catch((e) => setError(String(e)));
  };

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

  const value = useMemo(
    () => ({ status, pro, requirePro, openUpsell, applyStatus: setStatus }),
    [status, pro, requirePro, openUpsell],
  );

  return (
    <ProContext.Provider value={value}>
      {children}
      <Dialog
        open={upsell !== null}
        onClose={() => busy === null && setUpsell(null)}
        title={`${appName} Pro`}
        width={420}
        footer={
          status?.activation ? (
            <>
              <Button variant="ghost" icon={<ExternalLink size={14} strokeWidth={1.75} />} disabled={busy !== null} onClick={buy}>
                {status.price ? `Buy Pro · ${status.price}` : "Buy Pro"}
              </Button>
              <Button
                variant="primary"
                icon={<KeyRound size={14} strokeWidth={1.75} />}
                loading={busy === "activate"}
                disabled={busy !== null || keyInput.trim() === ""}
                onClick={activate}
              >
                Activate
              </Button>
            </>
          ) : (
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
          )
        }
      >
        <div style={{ display: "flex", flexDirection: "column", gap: 12, fontSize: "var(--fs-sm)", color: "var(--text-dim)", lineHeight: 1.5 }}>
          {upsell?.feature && (
            <p style={{ margin: 0, color: "var(--text)" }}>
              <strong>{upsell.feature}</strong> is part of Pro.
            </p>
          )}
          <p style={{ margin: 0 }}>
            {status?.activation
              ? "A one-time purchase, no subscription. Buy a licence key, enter it below and Pro unlocks on this PC. Activation needs the internet once; after that the app works offline."
              : "A one-time purchase. No subscription, no account."}
          </p>
          <ul style={{ listStyle: "none", margin: 0, padding: 0, display: "flex", flexDirection: "column", gap: 6 }}>
            {features.map((f) => (
              <li key={f} style={{ display: "flex", gap: 8, alignItems: "flex-start" }}>
                <Check size={14} strokeWidth={2} style={{ color: "var(--accent)", flexShrink: 0, marginTop: 3 }} />
                <span>{f}</span>
              </li>
            ))}
          </ul>
          {status?.activation && (
            <Field label="Licence key" hint="Looks like VLT-XXXXX-XXXXX-XXXXX. It is on your Licenses page at console.voltrus.id after you buy.">
              {(id) => (
                <input
                  id={id}
                  className="vt-input vt-input--mono"
                  placeholder="VLT-XXXXX-XXXXX-XXXXX"
                  value={keyInput}
                  autoComplete="off"
                  spellCheck={false}
                  disabled={busy !== null}
                  onChange={(e) => setKeyInput(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" && keyInput.trim() !== "" && busy === null) void activate();
                  }}
                />
              )}
            </Field>
          )}
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

/** Pro row for an About dialog: state plus purchase/restore/activate entry point. Hidden in source builds. */
export function ProAboutRow() {
  const { status, openUpsell, applyStatus } = useProStatus();
  const [error, setError] = useState<string | null>(null);
  if (!status?.store) return null;
  const label =
    status.reason === "purchased" ? "Pro unlocked. Thank you!" :
    status.reason === "key" ? "Pro unlocked with your licence key. Thank you!" :
    status.reason === "grandfathered" ? "Pro included with your original purchase." :
    "Free version";
  const removeKey = () => {
    setError(null);
    invoke<StoreStatus>("store_deactivate").then(applyStatus).catch((e) => setError(String(e)));
  };
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: 12 }}>
        <span style={{ fontSize: "var(--fs-sm)", color: "var(--text-dim)" }}>{label}</span>
        {!status.pro && (
          <Button size="sm" variant="primary" icon={status.activation ? <KeyRound size={14} strokeWidth={1.75} /> : <Sparkles size={14} strokeWidth={1.75} />} onClick={() => openUpsell()}>
            {status.activation ? "Enter key" : "Unlock Pro"}
          </Button>
        )}
        {status.reason === "key" && (
          <Button size="sm" variant="ghost" onClick={removeKey}>
            Remove key
          </Button>
        )}
      </div>
      {error && <Callout tone="danger">{error}</Callout>}
    </div>
  );
}
