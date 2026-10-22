import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type MutableRefObject,
  type ReactNode,
} from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Check, Cloud, Copy, LogOut, RefreshCw, Share2, Sparkles, UserRound, Users } from "lucide-react";
import { Badge, Button, Callout, Dialog, EmptyState, Field, IconButton, Section, Tabs } from "./primitives";
import {
  acceptServerCopy,
  applyOps,
  buildRequest,
  emptyMeta,
  entitlementSignature,
  isDirty,
  localDeletions,
  snapshotOf,
  teamFor,
  uniqueName,
  type ConnectStorage,
  type LocalProfile,
  type ProfileData,
  type ServerProfile,
  type SyncMeta,
  type SyncOp,
} from "./connectSync";

export type { ConnectStorage, LocalProfile, ProfileData, ServerProfile } from "./connectSync";

// ── Wire types (mirror crates/mt-connect) ──

export interface ConnectInfo {
  app_slug: string;
  /** App Store build: subscriptions are bought through StoreKit. */
  store: boolean;
  /** Account page for builds outside the App Store; null in store builds. */
  console_url: string | null;
  /** Console settings page that deletes the account (confirmed by an emailed code). */
  delete_account_url: string;
}

export interface Entitlement {
  active: boolean;
  source: "console" | "app_store" | "app_store_sandbox" | "none";
  expires_at: number | null;
}

export interface ConnectTeam {
  id: string;
  name: string;
  role: "owner" | "member" | null;
}

export interface ConnectMe {
  user: { id: string; email: string; email_verified: boolean };
  entitlement: Entitlement;
  teams: ConnectTeam[];
  app_account_token: string | null;
}

interface TeamDetail extends ConnectTeam {
  members: { user_id: string; email: string; role: string }[];
  invites: { id: string; email: string; expires_at: number | null }[];
}

interface RemoteSite {
  id: string;
  name: string;
  team_id: string | null;
  login_server: string;
  dashboard_url: string | null;
  subnets: string[];
  notes: string | null;
}

interface RemoteJoin {
  site_id: string;
  name: string;
  login_server: string;
  needs_enrolment_key: boolean;
  tailscale_command: string;
  steps: string[];
  limits: string[];
}

interface StoreProduct {
  id: string;
  title: string;
  description: string;
  price: string | null;
}

type SubscribeOutcome = { status: "linked"; me: ConnectMe } | { status: "pending" } | { status: "cancelled" };

/** What an app gives Connect so its saved profiles can sync. */
export interface ConnectAppHooks {
  storage: ConnectStorage;
  /** Call `notify` after the app's profiles change by any route; returns the unsubscribe. */
  subscribe?: (notify: () => void) => () => void;
  /** The app's current connection as a new profile, or null when there is nothing to save. */
  captureCurrent?: () => { name: string; data: ProfileData } | null;
  /** Applies a profile to the app, e.g. fills the connection form or connects. */
  open?: (profile: LocalProfile) => void | Promise<void>;
}

const NOT_SIGNED_IN = "You are not signed in to Voltrus.";
const AUTO_SYNC_DELAY_MS = 4000;

interface SyncSummary {
  at: number;
  uploaded: number;
  downloaded: number;
  removed: number;
}

interface ConnectContextValue {
  /** False when the host build has no Connect commands (plain browser, tests). */
  available: boolean;
  /** App Store builds: the Connect subscriptions StoreKit offers; null until loaded, empty when none exist. */
  storeProducts: StoreProduct[] | null;
  /**
   * Whether a paid plan can be bought from this build. Everything that mentions a subscription is hidden
   * while this is false, unless the account already has one.
   */
  plansOffered: boolean;
  info: ConnectInfo | null;
  signedIn: boolean;
  me: ConnectMe | null;
  /** The user is in the browser and has not come back yet. */
  waitingForBrowser: boolean;
  error: string | null;
  syncing: boolean;
  syncSummary: SyncSummary | null;
  syncError: string | null;
  conflicts: Record<string, ServerProfile | null>;
  failures: { key: string; message: string }[];
  pendingDeletes: { key: string; id: string; name: string }[];
  meta: SyncMeta;
  /** Something needs the user: a conflict or a deletion to confirm. */
  attention: boolean;
  dialogOpen: boolean;
  openDialog: () => void;
  closeDialog: () => void;
  signIn: () => Promise<void>;
  cancelSignIn: () => void;
  signOut: () => Promise<void>;
  refreshMe: () => Promise<ConnectMe | null>;
  setMe: (me: ConnectMe) => void;
  syncNow: (decision?: "remove" | "keep") => Promise<void>;
  setTeam: (key: string, teamId: string | null) => void;
  keepMine: (key: string) => Promise<void>;
  useServerCopy: (key: string) => void;
  saveCurrent: (name: string) => string | null;
  register: (hooks: MutableRefObject<ConnectAppHooks>) => () => void;
  hooks: () => ConnectAppHooks | null;
}

const ConnectContext = createContext<ConnectContextValue | null>(null);

export function useConnect(): ConnectContextValue {
  const ctx = useContext(ConnectContext);
  if (!ctx) throw new Error("useConnect must be used inside <ConnectProvider>");
  return ctx;
}

/** Registers the app's profile storage with Connect for as long as the calling component is mounted. */
export function useConnectApp(hooks: ConnectAppHooks) {
  const { register } = useConnect();
  const ref = useRef(hooks);
  ref.current = hooks;
  useEffect(() => register(ref), [register]);
}

const metaKey = (slug: string) => `voltrus-connect:${slug}`;

function loadMeta(slug: string): SyncMeta {
  try {
    const raw = localStorage.getItem(metaKey(slug));
    return raw ? { ...emptyMeta(), ...JSON.parse(raw) } : emptyMeta();
  } catch {
    return emptyMeta();
  }
}

export function ConnectProvider({ children }: { children: ReactNode }) {
  const [available, setAvailable] = useState(false);
  const [info, setInfo] = useState<ConnectInfo | null>(null);
  const [signedIn, setSignedIn] = useState(false);
  const [me, setMe] = useState<ConnectMe | null>(null);
  const [waitingForBrowser, setWaiting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [syncing, setSyncing] = useState(false);
  const [syncSummary, setSyncSummary] = useState<SyncSummary | null>(null);
  const [syncError, setSyncError] = useState<string | null>(null);
  const [conflicts, setConflicts] = useState<Record<string, ServerProfile | null>>({});
  const [failures, setFailures] = useState<{ key: string; message: string }[]>([]);
  const [pendingDeletes, setPendingDeletes] = useState<{ key: string; id: string; name: string }[]>([]);
  const [meta, setMetaState] = useState<SyncMeta>(emptyMeta);
  const [registered, setRegistered] = useState(0);
  const [storeProducts, setStoreProducts] = useState<StoreProduct[] | null>(null);

  const hooksRef = useRef<MutableRefObject<ConnectAppHooks> | null>(null);
  const metaRef = useRef(meta);
  const syncingRef = useRef(false);
  const applyingRef = useRef(false);
  const slugRef = useRef<string | null>(null);
  const timerRef = useRef<number | undefined>(undefined);

  const setMeta = useCallback((next: SyncMeta) => {
    metaRef.current = next;
    setMetaState(next);
    if (slugRef.current) localStorage.setItem(metaKey(slugRef.current), JSON.stringify(next));
  }, []);

  const hooks = useCallback(() => hooksRef.current?.current ?? null, []);

  const register = useCallback((ref: MutableRefObject<ConnectAppHooks>) => {
    hooksRef.current = ref;
    setRegistered((n) => n + 1);
    return () => {
      if (hooksRef.current === ref) hooksRef.current = null;
      setRegistered((n) => n + 1);
    };
  }, []);

  const handleError = useCallback((e: unknown, set: (m: string) => void) => {
    const message = String(e);
    if (message.includes(NOT_SIGNED_IN)) {
      setSignedIn(false);
      setMe(null);
      return;
    }
    set(message);
  }, []);

  const refreshMe = useCallback(async () => {
    try {
      const next = await invoke<ConnectMe>("connect_me");
      setMe(next);
      return next;
    } catch (e) {
      handleError(e, setError);
      return null;
    }
  }, [handleError]);

  // Startup: is Connect compiled in, and is there a stored session?
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const i = await invoke<ConnectInfo>("connect_info");
        if (cancelled) return;
        slugRef.current = i.app_slug;
        setMetaState((metaRef.current = loadMeta(i.app_slug)));
        setInfo(i);
        setAvailable(true);
        if (await invoke<boolean>("connect_signed_in")) {
          setSignedIn(true);
          void refreshMe();
        }
      } catch {
        // No Connect commands in this host (browser preview): the entry point stays hidden.
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [refreshMe]);

  const syncNow = useCallback(
    async (decision?: "remove" | "keep") => {
      const app = hooks();
      if (!app || syncingRef.current) return;
      syncingRef.current = true;
      setSyncing(true);
      setSyncError(null);
      try {
        const fresh = await invoke<ConnectMe>("connect_me");
        setMe(fresh);
        const signature = entitlementSignature(fresh);
        const locals = app.storage.list();
        let meta = metaRef.current;
        const gone = localDeletions(locals, meta);
        let deleted: string[] = [];
        let full = meta.signature !== signature;
        if (gone.length > 0) {
          if (!decision) {
            // Never delete from the account on a guess: ask first.
            setPendingDeletes(gone);
            return;
          }
          if (decision === "remove") {
            deleted = gone.map((g) => g.id);
          } else {
            const links = { ...meta.links };
            for (const g of gone) delete links[g.key];
            meta = { ...meta, links };
            full = true;
          }
        }
        setPendingDeletes([]);
        const request = buildRequest(locals, meta, deleted, full);
        const result = await invoke<{ ops: SyncOp[]; cursor: number | null }>("connect_sync", { request });
        applyingRef.current = true;
        const applied = applyOps(result.ops, request.locals, app.storage, meta, result.cursor, signature, Date.now());
        setMeta(applied.meta);
        setConflicts(applied.conflicts);
        setFailures(applied.failures);
        setSyncSummary({ at: Date.now(), ...applied.counts });
      } catch (e) {
        handleError(e, setSyncError);
      } finally {
        applyingRef.current = false;
        syncingRef.current = false;
        setSyncing(false);
      }
    },
    [hooks, handleError, setMeta],
  );

  // Browser round trip finished: the Rust side emits the result of the redirect.
  useEffect(() => {
    let off: (() => void) | undefined;
    let stopped = false;
    listen<{ ok: boolean; error: string | null }>("connect://auth", (event) => {
      setWaiting(false);
      if (!event.payload.ok) {
        setError(event.payload.error ?? "Sign-in failed.");
        return;
      }
      setError(null);
      setSignedIn(true);
      setDialogOpen(true);
      void (async () => {
        const next = await refreshMe();
        // App Store builds: a subscription bought before signing in links to this account now.
        if (next && !next.entitlement.active && (await invoke<ConnectInfo>("connect_info")).store) {
          try {
            const linked = await invoke<{ linked: number; me: ConnectMe }>("connect_link_purchases");
            setMe(linked.me);
          } catch {
            // Nothing to link, or the store is unavailable: the account tab explains the state.
          }
        }
        void syncNow();
      })();
    })
      .then((un) => (stopped ? un() : (off = un)))
      .catch(() => {});
    return () => {
      stopped = true;
      off?.();
    };
  }, [refreshMe, syncNow]);

  // Sync when the app first registers its storage after sign-in, and shortly after local changes.
  useEffect(() => {
    if (signedIn && registered > 0 && hooksRef.current) void syncNow();
  }, [signedIn, registered, syncNow]);

  useEffect(() => {
    const app = hooks();
    if (!signedIn || !app?.subscribe) return;
    const unsubscribe = app.subscribe(() => {
      if (applyingRef.current) return;
      clearTimeout(timerRef.current);
      timerRef.current = window.setTimeout(() => void syncNow(), AUTO_SYNC_DELAY_MS);
    });
    return () => {
      unsubscribe();
      clearTimeout(timerRef.current);
    };
  }, [signedIn, registered, hooks, syncNow]);

  const signIn = useCallback(async () => {
    setError(null);
    try {
      await invoke("connect_sign_in");
      setWaiting(true);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const signOut = useCallback(async () => {
    try {
      await invoke("connect_sign_out");
    } catch (e) {
      setError(String(e));
      return;
    }
    setSignedIn(false);
    setMe(null);
    setConflicts({});
    setFailures([]);
    setPendingDeletes([]);
    setSyncSummary(null);
    // Local profiles stay; only the link to the account goes, so a later sign-in (maybe as someone else)
    // uploads them fresh instead of treating them as deleted.
    setMeta(emptyMeta());
  }, [setMeta]);

  const setTeam = useCallback(
    (key: string, teamId: string | null) => {
      setMeta({ ...metaRef.current, teams: { ...metaRef.current.teams, [key]: teamId } });
      void syncNow();
    },
    [setMeta, syncNow],
  );

  const keepMine = useCallback(
    async (key: string) => {
      const app = hooks();
      const server = conflicts[key];
      const local = app?.storage.list().find((p) => p.key === key);
      const link = metaRef.current.links[key];
      if (!app || !local || !link) return;
      if (!server) {
        // The server will not show its copy: upload ours as a new profile.
        const links = { ...metaRef.current.links };
        delete links[key];
        setMeta({ ...metaRef.current, links });
        setConflicts(({ [key]: _gone, ...rest }) => rest);
        void syncNow();
        return;
      }
      try {
        const team = teamFor(metaRef.current, key);
        const out = await invoke<{ status: "saved"; profile: ServerProfile } | { status: "conflict"; server: ServerProfile | null }>(
          "connect_profile_save",
          { draft: { id: link.id, name: local.name, team_id: team, data: local.data, base_version: server.version } },
        );
        if (out.status === "saved") {
          const links = {
            ...metaRef.current.links,
            [key]: {
              id: out.profile.id,
              version: out.profile.version,
              canEdit: out.profile.can_edit,
              name: local.name,
              snapshot: snapshotOf(local.name, local.data, out.profile.team_id),
              teamId: out.profile.team_id,
            },
          };
          const teams = { ...metaRef.current.teams };
          delete teams[key];
          setMeta({ ...metaRef.current, links, teams });
          setConflicts(({ [key]: _gone, ...rest }) => rest);
        } else {
          setConflicts((prev) => ({ ...prev, [key]: out.server }));
        }
      } catch (e) {
        handleError(e, setSyncError);
      }
    },
    [conflicts, hooks, handleError, setMeta, syncNow],
  );

  const useServerCopy = useCallback(
    (key: string) => {
      const app = hooks();
      const server = conflicts[key];
      if (!app || !server) return;
      applyingRef.current = true;
      try {
        setMeta(acceptServerCopy(key, server, app.storage, metaRef.current));
      } finally {
        applyingRef.current = false;
      }
      setConflicts(({ [key]: _gone, ...rest }) => rest);
    },
    [conflicts, hooks, setMeta],
  );

  const saveCurrent = useCallback(
    (name: string) => {
      const app = hooks();
      const current = app?.captureCurrent?.();
      if (!app || !current) return "There is no open connection to save.";
      const taken = new Set(app.storage.list().map((p) => p.name));
      const wanted = name.trim() || current.name;
      if (taken.has(wanted)) return `A profile named "${wanted}" already exists.`;
      app.storage.upsert({ name: uniqueName(wanted, taken), data: current.data });
      void syncNow();
      return null;
    },
    [hooks, syncNow],
  );

  const cancelSignIn = useCallback(() => setWaiting(false), []);
  const openDialog = useCallback(() => {
    setDialogOpen(true);
    if (signedIn) void syncNow();
  }, [signedIn, syncNow]);
  const closeDialog = useCallback(() => setDialogOpen(false), []);

  // App Store review safety: until StoreKit actually returns Connect subscriptions (they have no price
  // yet), the app shows no subscribe button and never mentions a paid plan.
  useEffect(() => {
    if (!info?.store) return;
    let stopped = false;
    invoke<StoreProduct[]>("connect_store_products")
      .then((p) => !stopped && setStoreProducts(p))
      .catch(() => !stopped && setStoreProducts([]));
    return () => {
      stopped = true;
    };
  }, [info?.store]);
  const plansOffered = me?.entitlement.active === true || (storeProducts?.length ?? 0) > 0;

  const attention = pendingDeletes.length > 0 || Object.keys(conflicts).length > 0;

  const value = useMemo<ConnectContextValue>(
    () => ({
      available,
      storeProducts,
      plansOffered,
      info,
      signedIn,
      me,
      waitingForBrowser,
      error,
      syncing,
      syncSummary,
      syncError,
      conflicts,
      failures,
      pendingDeletes,
      meta,
      attention,
      dialogOpen,
      openDialog,
      closeDialog,
      signIn,
      cancelSignIn,
      signOut,
      refreshMe,
      setMe,
      syncNow,
      setTeam,
      keepMine,
      useServerCopy,
      saveCurrent,
      register,
      hooks,
    }),
    [
      available, storeProducts, plansOffered, info, signedIn, me, waitingForBrowser, error, syncing, syncSummary, syncError, conflicts, failures,
      pendingDeletes, meta, attention, dialogOpen, openDialog, closeDialog, signIn, cancelSignIn, signOut, refreshMe,
      syncNow, setTeam, keepMine, useServerCopy, saveCurrent, register, hooks,
    ],
  );

  return (
    <ConnectContext.Provider value={value}>
      {children}
      {available && <ConnectDialog />}
    </ConnectContext.Provider>
  );
}

/** Toolbar entry point. Renders nothing where Connect is not built in. */
export function ConnectButton() {
  const { available, signedIn, attention, openDialog } = useConnect();
  if (!available) return null;
  return (
    <IconButton
      label={signedIn ? (attention ? "Voltrus Connect: needs attention" : "Voltrus Connect: signed in") : "Voltrus Connect"}
      size="sm"
      onClick={openDialog}
      style={{ position: "relative" }}
    >
      {signedIn ? <Cloud size={16} strokeWidth={1.75} /> : <UserRound size={16} strokeWidth={1.75} />}
      {signedIn && (
        <span
          aria-hidden
          style={{
            position: "absolute",
            top: 3,
            right: 3,
            width: 7,
            height: 7,
            borderRadius: "50%",
            background: attention ? "var(--warn, #d97706)" : "var(--success, #16a34a)",
          }}
        />
      )}
    </IconButton>
  );
}

type Tab = "account" | "profiles" | "teams" | "remote";

const dim = { fontSize: "var(--fs-sm)", color: "var(--text-dim)", lineHeight: 1.5 } as const;
const col = { display: "flex", flexDirection: "column", gap: 12 } as const;
const row = { display: "flex", alignItems: "center", gap: 8, flexWrap: "wrap" } as const;

function ConnectDialog() {
  const c = useConnect();
  const [tab, setTab] = useState<Tab>("account");
  const tabs = useMemo(
    () =>
      c.signedIn
        ? [
            { value: "account" as const, label: "Account" },
            { value: "profiles" as const, label: "Profiles", count: c.hooks()?.storage.list().length },
            { value: "teams" as const, label: "Teams" },
            { value: "remote" as const, label: "Remote" },
          ]
        : [{ value: "account" as const, label: "Account" }],
    // The profile count is read when the dialog opens or sync settles.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [c.signedIn, c.syncSummary, c.dialogOpen],
  );
  const active = tabs.some((t) => t.value === tab) ? tab : "account";
  return (
    <Dialog open={c.dialogOpen} onClose={c.closeDialog} title="Voltrus Connect" width={560}>
      <div style={col}>
        {tabs.length > 1 && <Tabs label="Connect sections" value={active} items={tabs} onChange={setTab} />}
        {active === "account" && <AccountTab />}
        {active === "profiles" && <ProfilesTab />}
        {active === "teams" && <TeamsTab />}
        {active === "remote" && <RemoteTab />}
      </div>
    </Dialog>
  );
}

function useAction() {
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const run = useCallback(async <T,>(name: string, fn: () => Promise<T>): Promise<T | undefined> => {
    setBusy(name);
    setError(null);
    setNotice(null);
    try {
      return await fn();
    } catch (e) {
      setError(String(e));
      return undefined;
    } finally {
      setBusy(null);
    }
  }, []);
  return { busy, error, notice, setNotice, run };
}

function openLink(url: string) {
  void invoke("connect_open_url", { url }).catch(() => {});
}

function Messages({ error, notice }: { error: string | null; notice: string | null }) {
  return (
    <>
      {notice && <Callout tone="info">{notice}</Callout>}
      {error && <Callout tone="danger">{error}</Callout>}
    </>
  );
}

// ── Account ──

function entitlementLine(e: Entitlement, store: boolean, plansOffered: boolean): string {
  const via = e.source === "app_store" || e.source === "app_store_sandbox" ? "the App Store" : "your Voltrus account";
  if (!e.active) return plansOffered ? "No active Connect subscription." : "Your saved profiles sync across your devices.";
  const until = e.expires_at ? ` Renews or ends ${new Date(e.expires_at).toLocaleDateString()}.` : "";
  return `Connect is active through ${via}.${store && e.source === "console" ? " It is managed on your Voltrus account." : ""}${until}`;
}

function AccountTab() {
  const c = useConnect();
  if (!c.signedIn) {
    return (
      <div style={col}>
        <p style={{ ...dim, margin: 0 }}>
          Sign in with your Voltrus account to keep your saved profiles in sync across your devices.
          {c.plansOffered &&
            " Creating a team, sharing profiles and adding Voltrus Remote sites need an active Connect subscription; viewing a subscribed team's profiles and sites is free."}
        </p>
        {c.waitingForBrowser ? (
          <Callout tone="info">
            Finish signing in in your browser. This window updates when you come back.{" "}
            <Button size="sm" variant="ghost" onClick={c.cancelSignIn}>
              Cancel
            </Button>
          </Callout>
        ) : (
          <div style={row}>
            <Button variant="primary" icon={<UserRound size={14} strokeWidth={1.75} />} onClick={() => void c.signIn()}>
              Sign in with Voltrus
            </Button>
          </div>
        )}
        {c.error && <Callout tone="danger">{c.error}</Callout>}
      </div>
    );
  }
  const store = c.info?.store ?? false;
  const ent = c.me?.entitlement;
  return (
    <div style={col}>
      <Section
        title={c.me?.user.email ?? "Signed in"}
        actions={
          <Button size="sm" variant="ghost" icon={<LogOut size={14} strokeWidth={1.75} />} onClick={() => void c.signOut()}>
            Sign out
          </Button>
        }
      >
        <div style={{ ...row, marginBottom: 6 }}>
          {ent && <Badge tone={ent.active ? "success" : "neutral"}>{ent.active ? "Connect active" : "Free account"}</Badge>}
        </div>
        {ent && <p style={{ ...dim, margin: 0 }}>{entitlementLine(ent, store, c.plansOffered)}</p>}
        {!c.me && !c.error && <p style={{ ...dim, margin: 0 }}>Loading account…</p>}
      </Section>
      <SyncStatus />
      {ent && !ent.active && store && c.plansOffered && <SubscribePanel />}
      {ent && !ent.active && !store && c.info?.console_url && (
        <div style={row}>
          <Button size="sm" onClick={() => openLink(c.info!.console_url!)}>
            Open your Voltrus account
          </Button>
        </div>
      )}
      <Section title="Delete account">
        <p style={{ ...dim, margin: "0 0 8px" }}>
          Permanently deletes your Voltrus account and everything Connect stores for it. You confirm with a code we email you.
        </p>
        <Button size="sm" variant="danger" onClick={() => openLink(c.info?.delete_account_url ?? "https://console.voltrus.id/app/settings?delete=1")}>
          Delete account
        </Button>
      </Section>
      {c.error && <Callout tone="danger">{c.error}</Callout>}
    </div>
  );
}

function SyncStatus() {
  const c = useConnect();
  const s = c.syncSummary;
  return (
    <Section
      title="Sync"
      actions={
        <Button size="sm" icon={<RefreshCw size={14} strokeWidth={1.75} />} loading={c.syncing} onClick={() => void c.syncNow()}>
          Sync now
        </Button>
      }
    >
      <p style={{ ...dim, margin: 0 }}>
        {c.syncing
          ? "Syncing…"
          : s
            ? `Last synced ${new Date(s.at).toLocaleTimeString()}: ${s.uploaded} sent, ${s.downloaded} received${s.removed ? `, ${s.removed} removed` : ""}.`
            : "Not synced yet in this session."}
      </p>
      {c.syncError && <Callout tone="danger">{c.syncError}</Callout>}
    </Section>
  );
}

const APPLE_TERMS = "https://www.apple.com/legal/internet-services/itunes/dev/stdeula/";
const PRIVACY = "https://voltrus.id/privacy";

/** App Store builds only: StoreKit shows the localized price; this app never states or links to one. */
function SubscribePanel() {
  const c = useConnect();
  const { busy, error, notice, setNotice, run } = useAction();
  const products = c.storeProducts;

  const subscribe = (id: string) =>
    run(`buy:${id}`, async () => {
      const out = await invoke<SubscribeOutcome>("connect_subscribe", { productId: id });
      if (out.status === "linked") {
        c.setMe(out.me);
        void c.syncNow();
      } else if (out.status === "pending") {
        setNotice("The purchase is waiting for approval. Connect turns on once it is approved.");
      }
    });

  const restore = () =>
    run("restore", async () => {
      const out = await invoke<{ linked: number; me: ConnectMe }>("connect_link_purchases");
      c.setMe(out.me);
      if (out.linked === 0) setNotice("No Connect subscription was found for this Apple ID.");
      else void c.syncNow();
    });

  return (
    <Section title="Connect subscription">
      <div style={col}>
        {(products ?? []).map((p) => (
            <div key={p.id} style={{ ...row, justifyContent: "space-between" }}>
              <span style={{ ...dim, color: "var(--text)" }}>
                {p.title}
                {p.description ? <span style={{ ...dim, display: "block" }}>{p.description}</span> : null}
              </span>
              <Button
                variant="primary"
                size="sm"
                icon={<Sparkles size={14} strokeWidth={1.75} />}
                loading={busy === `buy:${p.id}`}
                disabled={busy !== null}
                onClick={() => void subscribe(p.id)}
              >
                {p.price ? `Subscribe · ${p.price}` : "Subscribe"}
              </Button>
            </div>
        ))}
        <div style={row}>
          <Button size="sm" variant="ghost" loading={busy === "restore"} disabled={busy !== null} onClick={() => void restore()}>
            Restore purchases
          </Button>
        </div>
        <p style={{ ...dim, margin: 0, fontSize: "var(--fs-xs)" }}>
          Payment is charged to your Apple ID. The subscription renews automatically unless it is cancelled at least 24 hours before the
          end of the current period. Manage or cancel it in your Apple ID settings.
        </p>
        <div style={row}>
          <Button size="sm" variant="ghost" onClick={() => openLink(APPLE_TERMS)}>
            Terms of Use
          </Button>
          <Button size="sm" variant="ghost" onClick={() => openLink(PRIVACY)}>
            Privacy Policy
          </Button>
        </div>
        <Messages error={error} notice={notice} />
      </div>
    </Section>
  );
}

// ── Profiles ──

function ProfilesTab() {
  const c = useConnect();
  const app = c.hooks();
  const [name, setName] = useState("");
  const [saveError, setSaveError] = useState<string | null>(null);
  const profiles = app?.storage.list() ?? [];
  const teams = c.me?.teams ?? [];
  const canShare = c.me?.entitlement.active === true;
  const canCapture = Boolean(app?.captureCurrent);

  return (
    <div style={col}>
      {c.pendingDeletes.length > 0 && (
        <Callout tone="warn">
          <p style={{ margin: "0 0 8px" }}>
            {c.pendingDeletes.length === 1 ? `"${c.pendingDeletes[0].name}" was deleted on this device.` : `${c.pendingDeletes.length} profiles were deleted on this device.`}{" "}
            Remove {c.pendingDeletes.length === 1 ? "it" : "them"} from your account too?
          </p>
          <div style={row}>
            <Button size="sm" variant="danger" onClick={() => void c.syncNow("remove")}>
              Remove from account
            </Button>
            <Button size="sm" onClick={() => void c.syncNow("keep")}>
              Keep in account
            </Button>
          </div>
        </Callout>
      )}
      {canCapture && (
        <Field label="Save the current connection as a profile">
          {(id) => (
            <div style={row}>
              <input
                id={id}
                className="vt-input"
                style={{ flex: 1, minWidth: 140 }}
                placeholder="Profile name"
                value={name}
                onChange={(e) => setName(e.target.value)}
              />
              <Button
                onClick={() => {
                  const problem = c.saveCurrent(name);
                  setSaveError(problem);
                  if (!problem) setName("");
                }}
              >
                Save
              </Button>
            </div>
          )}
        </Field>
      )}
      {saveError && <Callout tone="danger">{saveError}</Callout>}
      {profiles.length === 0 ? (
        <EmptyState
          compact
          icon={<Cloud size={20} strokeWidth={1.5} />}
          title="No saved profiles"
          description="Profiles you save in this app appear here and sync to your other devices."
        />
      ) : (
        <ul style={{ listStyle: "none", margin: 0, padding: 0, display: "flex", flexDirection: "column", gap: 8 }}>
          {profiles.map((p) => {
            const link = c.meta.links[p.key];
            const conflict = p.key in c.conflicts;
            const failed = c.failures.find((f) => f.key === p.key);
            const dirty = isDirty(c.meta, p);
            const team = teamFor(c.meta, p.key);
            const teamName = teams.find((t) => t.id === team)?.name;
            return (
              <li key={p.key} style={{ border: "1px solid var(--border)", borderRadius: "var(--radius)", padding: 10, ...col, gap: 8 }}>
                <div style={{ ...row, justifyContent: "space-between" }}>
                  <strong style={{ fontSize: "var(--fs-sm)", wordBreak: "break-word" }}>{p.name}</strong>
                  <div style={row}>
                    {conflict ? (
                      <Badge tone="warn">Conflict</Badge>
                    ) : failed ? (
                      <Badge tone="danger">Not synced</Badge>
                    ) : dirty ? (
                      <Badge>Waiting to sync</Badge>
                    ) : (
                      <Badge tone="success">Synced</Badge>
                    )}
                    {link && !link.canEdit && <Badge>Read only</Badge>}
                    {teamName && <Badge tone="info">Shared with {teamName}</Badge>}
                  </div>
                </div>
                {failed && <span style={{ ...dim, color: "var(--danger, #dc2626)" }}>{failed.message}</span>}
                {conflict && (
                  <div style={col}>
                    <span style={dim}>
                      This profile was changed here and in your account. Keep this device's version or use the account's.
                    </span>
                    <div style={row}>
                      <Button size="sm" onClick={() => void c.keepMine(p.key)}>
                        Keep this device's version
                      </Button>
                      <Button size="sm" variant="ghost" disabled={!c.conflicts[p.key]} onClick={() => c.useServerCopy(p.key)}>
                        Use the account's version
                      </Button>
                    </div>
                  </div>
                )}
                <div style={row}>
                  {app?.open && (
                    <Button size="sm" onClick={() => void app.open!(p)}>
                      Open
                    </Button>
                  )}
                  <label style={{ ...row, gap: 6, fontSize: "var(--fs-sm)", color: "var(--text-dim)" }}>
                    <Share2 size={14} strokeWidth={1.75} aria-hidden />
                    <span>Share</span>
                    <select
                      className="vt-select vt-select--sm"
                      value={team ?? ""}
                      disabled={!canShare || teams.length === 0 || (link ? !link.canEdit : false)}
                      title={
                        !canShare
                          ? c.plansOffered
                            ? "Sharing needs an active Connect subscription."
                            : "Sharing is not available on your account yet."
                          : teams.length === 0
                            ? "Create or join a team in the Teams tab first."
                            : undefined
                      }
                      onChange={(e) => c.setTeam(p.key, e.target.value || null)}
                    >
                      <option value="">Only me</option>
                      {teams.map((t) => (
                        <option key={t.id} value={t.id}>
                          {t.name}
                        </option>
                      ))}
                    </select>
                  </label>
                </div>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}

// ── Teams ──

function TeamsTab() {
  const c = useConnect();
  const { busy, error, notice, setNotice, run } = useAction();
  const [teams, setTeams] = useState<TeamDetail[] | null>(null);
  const [newName, setNewName] = useState("");
  const [inviteEmail, setInviteEmail] = useState<Record<string, string>>({});
  const [token, setToken] = useState("");
  const active = c.me?.entitlement.active === true;

  const load = useCallback(
    () =>
      run("load", async () => {
        setTeams(await invoke<TeamDetail[]>("connect_teams"));
      }),
    [run],
  );
  useEffect(() => {
    void load();
  }, [load]);

  const changed = async () => {
    await load();
    const me = await c.refreshMe();
    if (me) void c.syncNow();
  };

  const inviteToken = (raw: string) => {
    const text = raw.trim();
    try {
      return new URL(text).searchParams.get("invite") ?? text;
    } catch {
      return text;
    }
  };

  return (
    <div style={col}>
      {!active && (
        <Callout tone="info">
          Sign in with your Voltrus account to keep your saved profiles in sync across your devices.
          {c.plansOffered &&
            " Creating a team, sharing profiles and adding Voltrus Remote sites need an active Connect subscription; viewing a subscribed team's profiles and sites is free."}
        </Callout>
      )}
      {teams === null ? (
        <p style={{ ...dim, margin: 0 }}>Loading…</p>
      ) : teams.length === 0 ? (
        <EmptyState compact icon={<Users size={20} strokeWidth={1.5} />} title="No teams" description="A team shares saved profiles between the people on it." />
      ) : (
        teams.map((t) => (
          <Section
            key={t.id}
            title={t.name}
            meta={t.role === "owner" ? "Owner" : "Member"}
            actions={
              t.role !== "owner" && (
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={busy !== null}
                  onClick={() =>
                    void run("leave", async () => {
                      await invoke("connect_team_remove_member", { teamId: t.id, userId: null });
                      await changed();
                    })
                  }
                >
                  Leave
                </Button>
              )
            }
          >
            {t.members.length > 0 && <p style={{ ...dim, margin: "0 0 6px" }}>{t.members.map((m) => m.email).join(", ")}</p>}
            {t.invites.length > 0 && <p style={{ ...dim, margin: "0 0 6px" }}>Invited: {t.invites.map((i) => i.email).join(", ")}</p>}
            {t.role === "owner" && active && (
              <div style={row}>
                <input
                  className="vt-input"
                  style={{ flex: 1, minWidth: 160 }}
                  type="email"
                  placeholder="Invite by email"
                  value={inviteEmail[t.id] ?? ""}
                  onChange={(e) => setInviteEmail({ ...inviteEmail, [t.id]: e.target.value })}
                />
                <Button
                  disabled={busy !== null || !(inviteEmail[t.id] ?? "").includes("@")}
                  onClick={() =>
                    void run("invite", async () => {
                      await invoke("connect_team_invite", { teamId: t.id, email: inviteEmail[t.id] });
                      setInviteEmail({ ...inviteEmail, [t.id]: "" });
                      setNotice("Invite sent. It is valid for 7 days.");
                      await load();
                    })
                  }
                >
                  Invite
                </Button>
              </div>
            )}
          </Section>
        ))
      )}
      {active && (
        <Field label="New team">
          {(id) => (
            <div style={row}>
              <input id={id} className="vt-input" style={{ flex: 1, minWidth: 160 }} placeholder="Team name" value={newName} onChange={(e) => setNewName(e.target.value)} />
              <Button
                disabled={busy !== null || newName.trim().length === 0}
                onClick={() =>
                  void run("create", async () => {
                    await invoke("connect_team_create", { name: newName });
                    setNewName("");
                    await changed();
                  })
                }
              >
                Create
              </Button>
            </div>
          )}
        </Field>
      )}
      <Field label="Join a team" hint="Paste the invite link from your email.">
        {(id) => (
          <div style={row}>
            <input id={id} className="vt-input" style={{ flex: 1, minWidth: 160 }} placeholder="Invite link or code" value={token} onChange={(e) => setToken(e.target.value)} />
            <Button
              disabled={busy !== null || token.trim().length === 0}
              onClick={() =>
                void run("accept", async () => {
                  const name = await invoke<string>("connect_invite_accept", { token: inviteToken(token) });
                  setToken("");
                  setNotice(`Joined ${name}.`);
                  await changed();
                })
              }
            >
              Join
            </Button>
          </div>
        )}
      </Field>
      <Messages error={error} notice={notice} />
    </div>
  );
}

// ── Remote ──

function RemoteTab() {
  const c = useConnect();
  const { busy, error, run } = useAction();
  const [sites, setSites] = useState<RemoteSite[] | null>(null);
  const [join, setJoin] = useState<RemoteJoin | null>(null);
  const [copied, setCopied] = useState(false);
  const active = c.me?.entitlement.active === true;

  // A free account can still view the sites of teams whose owner subscribes; the server answers 402 when there are none.
  const [locked, setLocked] = useState(false);
  useEffect(() => {
    void run("load", async () => {
      try {
        setSites(await invoke<RemoteSite[]>("connect_remote_sites"));
        setLocked(false);
      } catch (e) {
        if (!active) setLocked(true);
        else throw e;
      }
    });
  }, [active, run]);

  if (!active && locked) {
    return (
      <Callout tone="info">
        {c.plansOffered
          ? "Adding Voltrus Remote sites needs an active Connect subscription. You can still see the sites of a team whose owner subscribes."
          : "Listing Voltrus Remote sites is not available on your account yet."}
      </Callout>
    );
  }

  const copy = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      // Clipboard blocked: the command is still selectable on screen.
    }
  };

  return (
    <div style={col}>
      <p style={{ ...dim, margin: 0 }}>
        Voltrus Remote runs on your own server. These are the sites you or your teams registered, with the steps to join one from this
        computer.
      </p>
      {sites === null ? (
        <p style={{ ...dim, margin: 0 }}>{busy ? "Loading…" : ""}</p>
      ) : sites.length === 0 ? (
        <EmptyState compact title="No Remote sites" description="Sites are added to your account or team on the web." />
      ) : (
        <ul style={{ listStyle: "none", margin: 0, padding: 0, display: "flex", flexDirection: "column", gap: 8 }}>
          {sites.map((s) => (
            <li key={s.id} style={{ border: "1px solid var(--border)", borderRadius: "var(--radius)", padding: 10, ...col, gap: 6 }}>
              <div style={{ ...row, justifyContent: "space-between" }}>
                <strong style={{ fontSize: "var(--fs-sm)" }}>{s.name}</strong>
                <Button
                  size="sm"
                  loading={busy === `join:${s.id}`}
                  disabled={busy !== null}
                  onClick={() => void run(`join:${s.id}`, async () => setJoin(await invoke<RemoteJoin>("connect_remote_join", { siteId: s.id })))}
                >
                  How to join
                </Button>
              </div>
              <span style={{ ...dim, fontFamily: "var(--font-mono)" }}>{s.login_server}</span>
              {s.subnets.length > 0 && <span style={dim}>Networks: {s.subnets.join(", ")}</span>}
              {s.notes && <span style={dim}>{s.notes}</span>}
            </li>
          ))}
        </ul>
      )}
      {join && (
        <Section title={`Join ${join.name}`}>
          <div style={col}>
            <ol style={{ ...dim, margin: 0, paddingLeft: 18, display: "flex", flexDirection: "column", gap: 4 }}>
              {join.steps.map((step, i) => (
                <li key={i}>{step}</li>
              ))}
            </ol>
            <div style={{ ...row, alignItems: "flex-start" }}>
              <code
                style={{
                  flex: 1,
                  minWidth: 0,
                  fontFamily: "var(--font-mono)",
                  fontSize: "var(--fs-xs)",
                  background: "var(--card)",
                  border: "1px solid var(--border)",
                  borderRadius: "var(--radius)",
                  padding: 8,
                  wordBreak: "break-all",
                  userSelect: "text",
                }}
              >
                {join.tailscale_command}
              </code>
              <Button size="sm" icon={copied ? <Check size={14} /> : <Copy size={14} strokeWidth={1.75} />} onClick={() => void copy(join.tailscale_command)}>
                {copied ? "Copied" : "Copy"}
              </Button>
            </div>
            {join.limits.length > 0 && (
              <ul style={{ ...dim, margin: 0, paddingLeft: 18 }}>
                {join.limits.map((l, i) => (
                  <li key={i}>{l}</li>
                ))}
              </ul>
            )}
          </div>
        </Section>
      )}
      {error && <Callout tone="danger">{error}</Callout>}
    </div>
  );
}
