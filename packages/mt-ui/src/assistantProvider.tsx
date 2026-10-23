import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Button, Callout, Dialog } from "./primitives";
import { useConnect } from "./connect";
import {
  CONSENT_VERSION,
  consentKey,
  isFailure,
  type AssistantFailure,
  type AssistantQuota,
  type AssistantStatus,
  type AssistantTask,
} from "./assistantLogic";

/**
 * How the Assistant entry should look for this build and account:
 *   hidden   the feature is off for this account (or the build has no Connect): show nothing
 *   signin   signed out while the Assistant is open to everyone: offer Voltrus Connect
 *   upgrade  signed in without a Connect subscription: offer Voltrus Connect
 *   ready    can be used
 */
export type AssistantAccess = "hidden" | "signin" | "upgrade" | "ready";

const TASK_NAME: Record<AssistantTask, string> = {
  lookup: "register_map",
  manual: "manual_blocks",
  diagnose: "diagnose",
  decode: "decode",
  ask: "ask",
};

interface AssistantContextValue {
  access: AssistantAccess;
  quota: AssistantQuota | null;
  consented: boolean;
  grantConsent: () => void;
  /** Runs a task and resolves with the console's `result`; rejects with an AssistantFailure. Updates the quota from the answer. */
  run: <T>(task: AssistantTask, payload: Record<string, unknown>) => Promise<T>;
  /** Opens the Voltrus Connect dialog (sign in, or subscribe where the build offers it). */
  openConnect: () => void;
  /** Mounted entry points call this so nothing is fetched in apps that never show the Assistant. */
  watch: () => () => void;
}

const AssistantContext = createContext<AssistantContextValue | null>(null);

export function useAssistant(): AssistantContextValue {
  const ctx = useContext(AssistantContext);
  if (!ctx) throw new Error("useAssistant must be used inside <AssistantProvider>");
  return ctx;
}

/** Access state for entry points (tab bar row, toolbar button); also keeps the status loaded while mounted. */
export function useAssistantAccess(): AssistantAccess {
  const { access, watch } = useAssistant();
  useEffect(() => watch(), [watch]);
  return access;
}

function asFailure(e: unknown): AssistantFailure {
  if (isFailure(e)) return e;
  return { status: 0, code: "error", message: String(e) };
}

/** Goes inside <ConnectProvider>. */
export function AssistantProvider({ children }: { children: ReactNode }) {
  const connect = useConnect();
  const { available, signedIn, info } = connect;
  const slug = info?.app_slug ?? null;
  const [watchers, setWatchers] = useState(0);
  const [open, setOpen] = useState<boolean | null>(null);
  const [status, setStatus] = useState<AssistantStatus | null>(null);
  const [quota, setQuota] = useState<AssistantQuota | null>(null);
  const [consented, setConsented] = useState(false);
  const entitled = connect.me?.entitlement.active;

  const watch = useCallback(() => {
    setWatchers((n) => n + 1);
    return () => setWatchers((n) => n - 1);
  }, []);

  // Is the Assistant open to everyone? Needs no account, so signed-out users can be told about it.
  useEffect(() => {
    if (!available || watchers === 0 || signedIn || open !== null) return;
    let alive = true;
    invoke<{ available: boolean }>("connect_ai_availability")
      .then((r) => alive && setOpen(r.available))
      .catch(() => alive && setOpen(false));
    return () => {
      alive = false;
    };
  }, [available, watchers, signedIn, open]);

  // The account's own view: flag (allowlisted accounts see it before everyone), entitlement, quota.
  useEffect(() => {
    if (!available || watchers === 0) return;
    if (!signedIn) {
      setStatus(null);
      return;
    }
    let alive = true;
    invoke<AssistantStatus>("connect_ai_status")
      .then((r) => {
        if (!alive) return;
        setStatus(r);
        if (r.quota) setQuota(r.quota);
      })
      .catch(() => alive && setStatus({ enabled: false }));
    return () => {
      alive = false;
    };
    // The entitlement can change while the app runs (a purchase): ask again then.
  }, [available, watchers > 0, signedIn, entitled]);

  useEffect(() => {
    if (!slug) return;
    try {
      setConsented(localStorage.getItem(consentKey(slug)) === String(CONSENT_VERSION));
    } catch {
      setConsented(false);
    }
  }, [slug]);

  const grantConsent = useCallback(() => {
    setConsented(true);
    if (!slug) return;
    try {
      localStorage.setItem(consentKey(slug), String(CONSENT_VERSION));
    } catch {
      // Private mode: consent holds for this session only.
    }
  }, [slug]);

  const quotaRef = useRef(setQuota);
  quotaRef.current = setQuota;
  const run = useCallback(async <T,>(task: AssistantTask, payload: Record<string, unknown>): Promise<T> => {
    try {
      const out = await invoke<{ result: T; quota?: AssistantQuota }>("connect_ai_run", { task: TASK_NAME[task], payload });
      if (out.quota) quotaRef.current(out.quota);
      return out.result;
    } catch (e) {
      const f = asFailure(e);
      if (f.quota) quotaRef.current(f.quota);
      throw f;
    }
  }, []);

  const access: AssistantAccess = !available
    ? "hidden"
    : !signedIn
      ? open
        ? "signin"
        : "hidden"
      : !status?.enabled
        ? "hidden"
        : entitled === false || status.entitled === false
          ? "upgrade"
          : "ready";

  const value = useMemo<AssistantContextValue>(
    () => ({ access, quota, consented, grantConsent, run, openConnect: connect.openDialog, watch }),
    [access, quota, consented, grantConsent, run, connect.openDialog, watch],
  );
  return <AssistantContext.Provider value={value}>{children}</AssistantContext.Provider>;
}

function openLink(url: string) {
  void invoke("connect_open_url", { url }).catch(() => {});
}

/** Apple 5.1.2(i): who gets the data and what is sent, asked before first use. */
export function ConsentSheet({ open, onAgree, onClose }: { open: boolean; onAgree: () => void; onClose: () => void }) {
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title="Use the Assistant?"
      width={460}
      footer={
        <>
          <Button onClick={onClose}>Not now</Button>
          <Button variant="primary" onClick={onAgree}>
            Agree and continue
          </Button>
        </>
      }
    >
      <div style={{ display: "flex", flexDirection: "column", gap: 10, fontSize: "var(--fs-sm)", lineHeight: 1.5 }}>
        <p style={{ margin: 0 }}>
          The Assistant uses an AI model run by <strong>Cloudflare Workers AI</strong>. Each time you ask something, the app sends
          your request through Voltrus to Cloudflare so the model can answer.
        </p>
        <div>
          <strong>What is sent, only when you ask:</strong>
          <ul style={{ margin: "4px 0 0", paddingLeft: 18, listStyle: "disc" }}>
            <li>your question, or the manual text or photo you give it</li>
            <li>for questions about your session: the connection settings and the last few log lines</li>
          </ul>
        </div>
        <p style={{ margin: 0 }}>
          IP addresses, host names, e-mail addresses, links and anything that looks like a password or key are removed before
          sending. Voltrus does not keep what you send or the answers; it only counts your requests for the monthly limit. Cloudflare
          does not use it to train models.
        </p>
        <p style={{ margin: 0 }}>The Assistant only reads and suggests. It never writes to a device, and nothing is added until you confirm it.</p>
        <Button variant="ghost" size="sm" style={{ alignSelf: "flex-start" }} onClick={() => openLink("https://voltrus.id/privacy/")}>
          Read the privacy policy
        </Button>
        <Callout tone="neutral">You can stop any time by not using the Assistant. It is never run in the background.</Callout>
      </div>
    </Dialog>
  );
}
