import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Download, RefreshCw, X } from "lucide-react";
import { Button, Callout } from "./primitives";

/** Mirrors `mt_update::UpdateInfo`. */
export interface UpdateInfo {
  version: string;
  current_version: string;
  notes: string | null;
  date: string | null;
  /** `install`: replaced in place. `download`: this copy cannot replace itself, the DMG opens in the browser. */
  mode: "install" | "download";
  download_url: string | null;
  /** Downloaded; installed when the app quits. */
  pending_on_quit: boolean;
}

type Phase = "idle" | "downloading" | "installing";

interface UpdateContextValue {
  /** The build updates itself (direct-download DMG with the `updater` feature). False in App Store and source builds. */
  supported: boolean;
  info: UpdateInfo | null;
  /** Banner visible. */
  shown: boolean;
  phase: Phase;
  progress: number | null;
  error: string | null;
  /** Manual check (About dialog): shows the banner when there is an update; returns the result or throws. */
  check: () => Promise<UpdateInfo | null>;
  install: (when: "now" | "quit") => Promise<void>;
  openDownload: () => void;
  later: () => void;
}

const UpdateContext = createContext<UpdateContextValue | null>(null);

/** Snoozed by "Later": the launch check does not show this version again for a day. */
const LATER_KEY = "mt-update-later";
const LATER_MS = 24 * 3600 * 1000;
/** First check a few seconds after launch (the app's own startup first), then every six hours while it runs. */
const FIRST_CHECK_MS = 5000;
const RECHECK_MS = 6 * 3600 * 1000;

function snoozed(version: string): boolean {
  try {
    const raw = localStorage.getItem(LATER_KEY);
    if (!raw) return false;
    const { v, at } = JSON.parse(raw) as { v: string; at: number };
    return v === version && Date.now() - at < LATER_MS;
  } catch {
    return false;
  }
}

export interface UpdateProviderProps {
  appName: string;
  children: ReactNode;
}

/**
 * In-app updates of the direct-download builds (`mt-update` on the Rust side). Checks quietly on launch and every
 * few hours; being offline or a server error shows nothing. Render `<UpdateBanner busy={...} />` inside the app and
 * `<UpdateAboutRow />` in its About dialog. In builds without the updater (App Store, source) both render nothing.
 */
export function UpdateProvider({ appName, children }: UpdateProviderProps) {
  const [supported, setSupported] = useState(false);
  const [info, setInfo] = useState<UpdateInfo | null>(null);
  const [shown, setShown] = useState(false);
  const [phase, setPhase] = useState<Phase>("idle");
  const [progress, setProgress] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const phaseRef = useRef<Phase>("idle");
  phaseRef.current = phase;

  const check = useCallback(async () => {
    const next = await invoke<UpdateInfo | null>("update_check");
    setInfo(next);
    if (next) {
      setError(null);
      setShown(true);
    }
    return next;
  }, []);

  useEffect(() => {
    let alive = true;
    invoke<boolean>("update_supported")
      .then((ok) => alive && setSupported(ok))
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);

  useEffect(() => {
    if (!supported) return;
    const quiet = () => {
      if (phaseRef.current !== "idle") return;
      invoke<UpdateInfo | null>("update_check")
        .then((next) => {
          setInfo(next);
          if (next && (next.pending_on_quit || !snoozed(next.version))) setShown(true);
        })
        .catch(() => {}); // offline, server down: say nothing
    };
    const first = window.setTimeout(quiet, FIRST_CHECK_MS);
    const again = window.setInterval(quiet, RECHECK_MS);
    return () => {
      window.clearTimeout(first);
      window.clearInterval(again);
    };
  }, [supported]);

  useEffect(() => {
    if (!supported) return;
    const off = listen<{ downloaded: number; total: number | null }>("mt-update://progress", (e) => {
      const { downloaded, total } = e.payload;
      setProgress(total ? Math.min(1, downloaded / total) : null);
    });
    return () => {
      void off.then((f) => f());
    };
  }, [supported]);

  const install = useCallback(
    async (when: "now" | "quit") => {
      setError(null);
      setPhase("downloading");
      setProgress(0);
      try {
        await invoke("update_download");
        setPhase(when === "now" ? "installing" : "idle");
        await invoke("update_install", { when });
        // `now` relaunches the app and never gets here; `quit` is armed.
        setInfo((i) => (i ? { ...i, pending_on_quit: true } : i));
      } catch (e) {
        setError(String(e));
        setPhase("idle");
      } finally {
        setProgress(null);
      }
    },
    [],
  );

  const openDownload = useCallback(() => {
    invoke("update_open_download").catch((e) => setError(String(e)));
  }, []);

  const later = useCallback(() => {
    if (info) {
      try {
        localStorage.setItem(LATER_KEY, JSON.stringify({ v: info.version, at: Date.now() }));
      } catch {
        /* storage full or unavailable: only this session forgets */
      }
    }
    setShown(false);
    setError(null);
  }, [info]);

  const value = useMemo(
    () => ({ supported, info, shown, phase, progress, error, check, install, openDownload, later }),
    [supported, info, shown, phase, progress, error, check, install, openDownload, later],
  );

  return (
    <UpdateContext.Provider value={value}>
      <AppNameContext.Provider value={appName}>{children}</AppNameContext.Provider>
    </UpdateContext.Provider>
  );
}

const AppNameContext = createContext("The app");

export function useUpdate(): UpdateContextValue {
  const ctx = useContext(UpdateContext);
  if (!ctx) throw new Error("useUpdate must be used inside <UpdateProvider>");
  return ctx;
}

/** Release notes from the feed: one paragraph per line, list markers dropped. */
function Notes({ text }: { text: string }) {
  const lines = text
    .split(/\r?\n/)
    .map((l) => l.replace(/^\s*[-*•]\s+/, "").trim())
    .filter(Boolean);
  return (
    <ul style={{ margin: 0, paddingLeft: 16, display: "flex", flexDirection: "column", gap: 4, maxHeight: 160, overflowY: "auto" }}>
      {lines.map((l, i) => (
        <li key={i}>{l}</li>
      ))}
    </ul>
  );
}

export interface UpdateBannerProps {
  /**
   * A bus connection, recording or other session is open: the update is not installed now (that would cut it);
   * "Install when I quit" downloads it and installs it when the app quits.
   */
  busy?: boolean;
}

/** Non-blocking card in the bottom-right corner when an update is available. */
export function UpdateBanner({ busy = false }: UpdateBannerProps) {
  const ctx = useUpdate();
  const appName = useContext(AppNameContext);
  const [notesOpen, setNotesOpen] = useState(false);
  const { info, shown, phase, progress, error } = ctx;
  if (!ctx.supported || !info || !shown) return null;

  const working = phase !== "idle";
  const pct = progress == null ? null : Math.round(progress * 100);
  return (
    <div
      role="status"
      aria-live="polite"
      style={{
        position: "fixed",
        right: 16,
        bottom: 40,
        zIndex: 150,
        width: 340,
        maxWidth: "calc(100vw - 32px)",
        background: "var(--card)",
        border: "1px solid var(--border)",
        borderRadius: "var(--radius-lg)",
        boxShadow: "var(--shadow-pop)",
        padding: 14,
        display: "flex",
        flexDirection: "column",
        gap: 10,
        fontSize: "var(--fs-sm)",
        color: "var(--text-dim)",
        lineHeight: 1.45,
      }}
    >
      <div style={{ display: "flex", alignItems: "flex-start", gap: 8 }}>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ color: "var(--text)", fontWeight: 600 }}>
            {info.pending_on_quit ? `${appName} ${info.version} is ready` : `${appName} ${info.version} is available`}
          </div>
          <div style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)" }}>
            {info.pending_on_quit
              ? "It installs when you quit the app. The next launch is the new version."
              : `You have ${info.current_version}.`}
          </div>
        </div>
        {!working && (
          <button
            type="button"
            className="vt-icon-btn"
            aria-label="Close"
            onClick={ctx.later}
            style={{ background: "none", border: 0, padding: 2, color: "var(--text-muted)", cursor: "pointer" }}
          >
            <X size={14} strokeWidth={1.75} />
          </button>
        )}
      </div>

      {info.notes && !info.pending_on_quit && (
        <div>
          <button
            type="button"
            onClick={() => setNotesOpen((o) => !o)}
            aria-expanded={notesOpen}
            style={{ background: "none", border: 0, padding: 0, color: "var(--accent-text)", cursor: "pointer", fontSize: "var(--fs-sm)" }}
          >
            {notesOpen ? "Hide what's new" : "What's new"}
          </button>
          {notesOpen && (
            <div style={{ marginTop: 6 }}>
              <Notes text={info.notes} />
            </div>
          )}
        </div>
      )}

      {info.mode === "install" && busy && !info.pending_on_quit && !working && (
        <div style={{ fontSize: "var(--fs-xs)" }}>A bus session is open, so nothing is interrupted: the update installs when you quit.</div>
      )}
      {info.mode === "download" && (
        <div style={{ fontSize: "var(--fs-xs)" }}>
          Download the new version, quit {appName} and drag the new copy to Applications. Your settings and licence stay.
        </div>
      )}

      {working && (
        <div aria-label="Download progress" style={{ height: 4, borderRadius: 2, background: "var(--hover-overlay)", overflow: "hidden" }}>
          <div
            style={{
              height: "100%",
              width: pct == null ? "30%" : `${pct}%`,
              background: "var(--accent)",
              transition: "width 120ms var(--ease-out)",
            }}
          />
        </div>
      )}
      {error && (
        <Callout tone="danger">
          {error}
          {info.download_url && info.mode === "install" && (
            <>
              {" "}
              <button
                type="button"
                onClick={ctx.openDownload}
                style={{ background: "none", border: 0, padding: 0, color: "inherit", textDecoration: "underline", cursor: "pointer" }}
              >
                Download it instead
              </button>
            </>
          )}
        </Callout>
      )}

      <div style={{ display: "flex", justifyContent: "flex-end", gap: 8 }}>
        {info.pending_on_quit ? (
          <>
            <Button size="sm" variant="ghost" onClick={ctx.later}>
              OK
            </Button>
            {!busy && (
              <Button size="sm" variant="primary" icon={<RefreshCw size={14} strokeWidth={1.75} />} loading={working} onClick={() => void ctx.install("now")}>
                Restart now
              </Button>
            )}
          </>
        ) : info.mode === "download" ? (
          <>
            <Button size="sm" variant="ghost" onClick={ctx.later}>
              Later
            </Button>
            <Button size="sm" variant="primary" icon={<Download size={14} strokeWidth={1.75} />} onClick={ctx.openDownload}>
              Download
            </Button>
          </>
        ) : (
          <>
            <Button size="sm" variant="ghost" disabled={working} onClick={ctx.later}>
              Later
            </Button>
            <Button
              size="sm"
              variant="primary"
              icon={busy ? <Download size={14} strokeWidth={1.75} /> : <RefreshCw size={14} strokeWidth={1.75} />}
              loading={working}
              disabled={working}
              onClick={() => void ctx.install(busy ? "quit" : "now")}
            >
              {phase === "downloading"
                ? pct == null ? "Downloading" : `Downloading ${pct}%`
                : phase === "installing"
                  ? "Installing"
                  : busy
                    ? "Install when I quit"
                    : "Install and restart"}
            </Button>
          </>
        )}
      </div>
    </div>
  );
}

/** "Check for updates" row for an About dialog. Renders nothing in builds without the updater. */
export function UpdateAboutRow() {
  const ctx = useUpdate();
  const appName = useContext(AppNameContext);
  const [state, setState] = useState<"idle" | "checking" | "current" | "found" | "failed">("idle");
  const [version, setVersion] = useState<string | null>(null);
  if (!ctx.supported) return null;

  const run = async () => {
    setState("checking");
    try {
      const next = await ctx.check();
      setVersion(next?.version ?? null);
      setState(next ? "found" : "current");
    } catch {
      setState("failed");
    }
  };

  const label =
    state === "checking" ? "Checking for updates…" :
    state === "current" ? `${appName} is up to date.` :
    state === "found" ? `Version ${version} is available.` :
    state === "failed" ? "Could not reach the update server. Check the internet connection and try again." :
    "Updates are checked when the app starts.";
  return (
    <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: 12 }}>
      <span style={{ fontSize: "var(--fs-sm)", color: state === "failed" ? "var(--danger)" : "var(--text-dim)" }}>{label}</span>
      <Button size="sm" variant="ghost" icon={<RefreshCw size={14} strokeWidth={1.75} />} loading={state === "checking"} disabled={state === "checking"} onClick={() => void run()}>
        Check for updates
      </Button>
    </div>
  );
}
