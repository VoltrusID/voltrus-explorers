import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  Badge,
  Button,
  Callout,
  ConnectionForm,
  Dialog,
  EmptyState,
  Field,
  LogPanel,
  ProAboutRow,
  ProBadge,
  Section,
  Segmented,
  SheetSelect,
  StatusBar,
  StatusDot,
  Tabs,
  Toolbar,
  noteDeviceError,
  noteDeviceSuccess,
  useProStatus,
  withUtm,
} from "@mactools/ui";
import type { LogEntry } from "@mactools/ui";
import { Cpu, Play, Plus, RotateCcw, Square, Trash2 } from "lucide-react";
import appIcon from "../src-tauri/icons/128x128.png";
import FanucView from "./FanucView";
import MtcView from "./MtcView";
import { FocasSimPanel, MtcSimPanel } from "./SimPanels";
import TrendPanel from "./TrendPanel";
import { useTrend } from "./useTrend";
import { fanucTrendRows } from "./fanuc";
import { applyObservations, flattenItems, liveIdSet, numericValue, trendLabels, type Item } from "./mtc";
import {
  FREE_LIVE_ITEMS,
  loadProfiles,
  needsPro,
  saveProfiles,
  type ConnProfile,
} from "./storage";
import type {
  ConnType,
  Devices,
  FanucPoll,
  MtcSimInfo,
  MtcStreamEvent,
  Observation,
  Streams,
  SysInfo,
} from "./types";

const TYPE_OPTIONS = [
  { value: "mtconnect", label: "MTConnect agent" },
  { value: "fanuc", label: "FANUC (FOCAS/Ethernet) Beta" },
];
const BETA_EXPLAINER =
  "Tested against the built-in simulator; please report your controller model to support@voltrus.id";

type FormValues = Record<string, string | number>;

const DEFAULTS: Record<ConnType, FormValues> = {
  mtconnect: { url: "http://localhost:5050", device: "", timeout: 10000, interval: 500 },
  fanuc: { host: "192.168.1.100", port: 8193, timeout: 5000, interval: 1000 },
};

const FIELDS = {
  mtconnect: [
    { name: "url", label: "Agent URL", type: "text" as const, default: "http://localhost:5050" },
    { name: "device", label: "Device (optional)", type: "text" as const, default: "", placeholder: "All devices" },
    { name: "timeout", label: "Timeout (ms)", type: "number" as const, default: 10000 },
    { name: "interval", label: "Stream interval (ms)", type: "number" as const, default: 500 },
  ],
  fanuc: [
    { name: "host", label: "Host", type: "text" as const, default: "192.168.1.100" },
    { name: "port", label: "Port", type: "number" as const, default: 8193 },
    { name: "timeout", label: "Timeout (ms)", type: "number" as const, default: 5000 },
    { name: "interval", label: "Poll interval (ms)", type: "number" as const, default: 1000 },
  ],
};

const MAX_LOG = 2000;

function profileOf(type: ConnType, v: FormValues, name = ""): ConnProfile {
  return {
    name,
    type,
    url: String(v.url ?? DEFAULTS.mtconnect.url),
    device: String(v.device ?? ""),
    host: String(v.host ?? DEFAULTS.fanuc.host),
    port: Number(v.port ?? 8193),
    timeout: Number(v.timeout ?? DEFAULTS[type].timeout),
  };
}

function valuesOf(p: ConnProfile, interval: string | number): FormValues {
  return p.type === "mtconnect"
    ? { url: p.url, device: p.device, timeout: p.timeout, interval }
    : { host: p.host, port: p.port, timeout: p.timeout, interval };
}

const sameProfile = (a: ConnProfile, b: ConnProfile) =>
  a.type === b.type && a.url === b.url && a.device === b.device && a.host === b.host && a.port === b.port && a.timeout === b.timeout;

const hint: React.CSSProperties = { fontSize: "var(--fs-xs)", color: "var(--text-muted)", lineHeight: 1.5 };

function App() {
  const { requirePro, pro, status: proStatus } = useProStatus();
  const [log, setLog] = useState<LogEntry[]>([]);
  const [showInfo, setShowInfo] = useState(false);
  const [bottomTab, setBottomTab] = useState<"log" | "trend">("log");

  const [mode, setMode] = useState<"client" | "simulator">("client");
  const [connType, setConnType] = useState<ConnType>("mtconnect");
  const [formValues, setFormValues] = useState<FormValues>(DEFAULTS.mtconnect);
  const [formKey, setFormKey] = useState(0);
  const [connected, setConnected] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [pollCount, setPollCount] = useState(0);
  const [latency, setLatency] = useState(0);

  // Profiles
  const [profiles, setProfiles] = useState<ConnProfile[]>(loadProfiles);
  const [activeProfile, setActiveProfile] = useState<string | null>(null);
  const [profileName, setProfileName] = useState("");

  // MTConnect session
  const [devices, setDevices] = useState<Devices | null>(null);
  const [obs, setObs] = useState<Map<string, Observation>>(new Map());
  const items = useMemo<Item[]>(() => (devices ? flattenItems(devices) : []), [devices]);
  const liveIds = useMemo(() => (pro ? null : liveIdSet(items, FREE_LIVE_ITEMS)), [pro, items]);
  const liveRef = useRef<Set<string> | null>(null);
  liveRef.current = liveIds;

  // FANUC session
  const [info, setInfo] = useState<SysInfo | null>(null);
  const [poll, setPoll] = useState<FanucPoll | null>(null);

  // Simulators
  const [simKind, setSimKind] = useState<ConnType>("mtconnect");
  const [mtcTemplate, setMtcTemplate] = useState("Vmc3Axis");
  const [mtcPort, setMtcPort] = useState(5050);
  const [shdrOn, setShdrOn] = useState(true);
  const [shdrPort, setShdrPort] = useState(7878);
  const [mtcSim, setMtcSim] = useState<MtcSimInfo | null>(null);
  const [cncProfile, setCncProfile] = useState("series31i_b");
  const [cncPort, setCncPort] = useState(8193);
  const [cncSimPort, setCncSimPort] = useState<number | null>(null);

  const addLog = useCallback((direction: LogEntry["direction"], message: string) => {
    setLog((prev) => [...prev.slice(-(MAX_LOG - 1)), { timestamp: Date.now(), direction, message }]);
  }, []);

  // ── Events from Rust ──
  const noteEvent = useRef({ disconnect: () => {} });
  useEffect(() => {
    const subs = [
      listen<{ direction: LogEntry["direction"]; message: string }>("cnc-log", (e) => addLog(e.payload.direction, e.payload.message)),
      listen<MtcStreamEvent>("mtc-stream", (e) => {
        const p = e.payload;
        if (p.event === "error") {
          setError(p.message ?? "Stream failed");
          noteDeviceError();
          noteEvent.current.disconnect();
          return;
        }
        const incoming = p.observations ?? [];
        setObs((prev) => applyObservations(prev, incoming, (id) => liveRef.current == null || liveRef.current.has(id), p.event !== "observations"));
        setPollCount((c) => c + 1);
        setError(null);
        if (incoming.length > 0) noteDeviceSuccess();
      }),
      listen<Devices>("mtc-devices", (e) => setDevices(e.payload)),
    ];
    return () => { subs.forEach((s) => s.then((un) => un())); };
  }, [addLog]);

  // ── FANUC poll loop ──
  const pollMs = Math.max(250, Number(formValues.interval) || 1000);
  const pollMsRef = useRef(pollMs);
  pollMsRef.current = pollMs;
  useEffect(() => {
    if (!connected || connType !== "fanuc") return;
    let alive = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const tick = async () => {
      const started = performance.now();
      try {
        const p = await invoke<FanucPoll>("fanuc_poll");
        if (!alive) return;
        setPoll(p);
        setLatency(Math.round(performance.now() - started));
        setPollCount((c) => c + 1);
        setError(null);
        noteDeviceSuccess();
      } catch (e) {
        if (!alive) return;
        setError(String(e));
        noteDeviceError();
      }
      if (alive) timer = setTimeout(tick, pollMsRef.current);
    };
    void tick();
    return () => { alive = false; if (timer) clearTimeout(timer); };
  }, [connected, connType]);

  // ── Connect / disconnect ──
  const disconnect = useCallback(async () => {
    try { await invoke(connType === "mtconnect" ? "mtc_disconnect" : "fanuc_disconnect"); } catch { /* not connected */ }
    setConnected(false);
    setDevices(null);
    setObs(new Map());
    setInfo(null);
    setPoll(null);
    setPollCount(0);
    setLatency(0);
  }, [connType]);
  noteEvent.current.disconnect = () => { void disconnect(); };

  const connectWith = useCallback(async (type: ConnType, v: FormValues) => {
    setLoading(true);
    setError(null);
    setPollCount(0);
    try {
      if (type === "mtconnect") {
        const d = await invoke<Devices>("mtc_connect", {
          config: { baseUrl: String(v.url).trim(), device: String(v.device ?? "").trim() || null, timeoutMs: Number(v.timeout) || 10000 },
        });
        setDevices(d);
        setObs(new Map());
        setConnected(true);
        await invoke("mtc_stream_start", { intervalMs: Number(v.interval) || 500 });
      } else {
        const i = await invoke<SysInfo>("fanuc_connect", {
          config: {
            host: String(v.host).trim(),
            port: Number(v.port) || 8193,
            connect_timeout_ms: Number(v.timeout) || 5000,
            request_timeout_ms: Number(v.timeout) || 5000,
          },
        });
        setInfo(i);
        setPoll(null);
        setConnected(true);
      }
    } catch (e) {
      setError(String(e));
      noteDeviceError();
      setConnected(false);
    } finally {
      setLoading(false);
    }
  }, []);

  const chooseType = useCallback((t: ConnType) => {
    setConnType(t);
    setFormValues(DEFAULTS[t]);
    setFormKey((k) => k + 1);
    setActiveProfile(null);
  }, []);

  // ── Profiles ──
  const saveProfile = useCallback((name: string) => {
    const n = name.trim();
    if (!n) return;
    if (needsPro(profiles, n, pro) && !requirePro("More than one saved connection profile")) return;
    const next = [...profiles.filter((p) => p.name !== n), profileOf(connType, formValues, n)];
    setProfiles(next);
    saveProfiles(next);
    setActiveProfile(n);
    setProfileName("");
  }, [profiles, pro, requirePro, connType, formValues]);

  const loadProfile = useCallback((name: string) => {
    const p = profiles.find((x) => x.name === name);
    if (!p) return;
    setConnType(p.type);
    setFormValues(valuesOf(p, formValues.interval ?? DEFAULTS[p.type].interval));
    setActiveProfile(p.name);
    setFormKey((k) => k + 1);
  }, [profiles, formValues.interval]);

  const deleteProfile = useCallback((name: string) => {
    const next = profiles.filter((p) => p.name !== name);
    setProfiles(next);
    saveProfiles(next);
    if (activeProfile === name) setActiveProfile(null);
  }, [profiles, activeProfile]);

  const profileDirty = useMemo(() => {
    const saved = profiles.find((p) => p.name === activeProfile);
    return !!saved && !sameProfile(saved, profileOf(connType, formValues));
  }, [profiles, activeProfile, connType, formValues]);

  // ── Simulators ──
  const startMtcSim = useCallback(async (): Promise<MtcSimInfo | null> => {
    try {
      const i = await invoke<MtcSimInfo>("mtc_sim_start", {
        config: { template: mtcTemplate, httpPort: mtcPort, shdrPort: shdrOn ? shdrPort : null },
      });
      setMtcSim(i);
      return i;
    } catch (e) {
      setError(String(e));
      return null;
    }
  }, [mtcTemplate, mtcPort, shdrOn, shdrPort]);

  const stopMtcSim = useCallback(async () => {
    await invoke("mtc_sim_stop");
    setMtcSim(null);
  }, []);

  const startCncSim = useCallback(async (): Promise<number | null> => {
    try {
      const port = await invoke<number>("focas_sim_start", { config: { profile: cncProfile, port: cncPort } });
      setCncSimPort(port);
      return port;
    } catch (e) {
      setError(String(e));
      return null;
    }
  }, [cncProfile, cncPort]);

  const stopCncSim = useCallback(async () => {
    await invoke("focas_sim_stop");
    setCncSimPort(null);
  }, []);

  const connectToSim = useCallback(async (kind: ConnType) => {
    if (connected) await disconnect();
    setMode("client");
    setConnType(kind);
    const v: FormValues =
      kind === "mtconnect"
        ? { ...DEFAULTS.mtconnect, url: `http://localhost:${mtcSim?.httpPort ?? mtcPort}` }
        : { ...DEFAULTS.fanuc, host: "127.0.0.1", port: cncSimPort ?? cncPort };
    setFormValues(v);
    setFormKey((k) => k + 1);
    setActiveProfile(null);
    await connectWith(kind, v);
  }, [connected, disconnect, mtcSim, mtcPort, cncSimPort, cncPort, connectWith]);

  /** Welcome state: start the simulator for the chosen type (default settings) and connect to it. */
  const trySimulator = useCallback(async () => {
    if (!requirePro("Built-in simulators")) return;
    if (connType === "mtconnect") {
      const i = mtcSim ?? (await startMtcSim());
      if (!i) return;
      const v = { ...DEFAULTS.mtconnect, url: i.baseUrl };
      setFormValues(v);
      setFormKey((k) => k + 1);
      await connectWith("mtconnect", v);
    } else {
      const port = cncSimPort ?? (await startCncSim());
      if (port == null) return;
      const v = { ...DEFAULTS.fanuc, host: "127.0.0.1", port };
      setFormValues(v);
      setFormKey((k) => k + 1);
      await connectWith("fanuc", v);
    }
  }, [requirePro, connType, mtcSim, startMtcSim, cncSimPort, startCncSim, connectWith]);

  const refreshMtc = useCallback(async () => {
    try {
      const s = await invoke<Streams>("mtc_current", { path: null });
      const all = s.devices.flatMap((d) => d.components.flatMap((c) => c.observations));
      setObs((prev) => applyObservations(prev, all, () => true, true));
      noteDeviceSuccess();
    } catch (e) {
      setError(String(e));
      noteDeviceError();
    }
  }, []);

  // ── Trend ──
  const labels = useMemo(() => trendLabels(items.filter((i) => i.item.category === "SAMPLE")), [items]);
  const mtcTrendRows = useMemo(
    () =>
      items.flatMap((i) => {
        const v = numericValue(obs.get(i.id));
        return v == null ? [] : [{ name: labels.get(i.id) ?? i.name, value: v }];
      }),
    [items, obs, labels],
  );
  const trendRows = connType === "mtconnect" ? mtcTrendRows : fanucTrendRows(poll);
  const { trendSeries, setTrendSeries, trendWindow, setTrendWindow, toggleTrend } = useTrend(trendRows);
  useEffect(() => { setTrendSeries(new Map()); }, [connType, connected, setTrendSeries]);
  const trendingIds = useMemo(
    () => new Set(items.filter((i) => trendSeries.has(labels.get(i.id) ?? i.name)).map((i) => i.id)),
    [items, trendSeries, labels],
  );

  const openPrivacy = useCallback(async () => {
    try { await invoke("open_url", { url: await withUtm("https://voltrus.id/privacy/", "privacy") }); } catch (e) { addLog("error", String(e)); }
  }, [addLog]);

  // ── Sidebar ──
  const clientSidebar = (
    <>
      <Section title="Connection type">
        <div title={connType === "fanuc" ? BETA_EXPLAINER : undefined}>
          <SheetSelect
            aria-label="Connection type"
            sheetTitle="Connection type"
            value={connType}
            options={TYPE_OPTIONS}
            onChange={(v) => chooseType(v as ConnType)}
            disabled={connected}
          />
        </div>
        {connType === "fanuc" && (
          <Callout tone="warn" title={BETA_EXPLAINER}>
            <strong>Beta.</strong> {BETA_EXPLAINER}.
          </Callout>
        )}
      </Section>

      <Section title="Profiles">
        <SheetSelect
          aria-label="Connection profile"
          sheetTitle="Connection profile"
          value={activeProfile ?? ""}
          options={[{ value: "", label: "New profile" }, ...profiles.map((p) => ({ value: p.name, label: p.name }))]}
          onChange={(v) => { if (v) loadProfile(v); else setActiveProfile(null); }}
          disabled={connected}
        />
        {activeProfile ? (
          <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
            <Badge tone={profileDirty ? "warn" : "success"}>{profileDirty ? "Modified" : "Saved"}</Badge>
            <span style={{ flex: 1 }} />
            {profileDirty && <Button size="sm" onClick={() => saveProfile(activeProfile)}>Save</Button>}
            {profileDirty && <Button size="sm" icon={<RotateCcw size={14} strokeWidth={1.75} />} onClick={() => loadProfile(activeProfile)}>Revert</Button>}
            <Button size="sm" variant="danger" icon={<Trash2 size={14} strokeWidth={1.75} />} onClick={() => deleteProfile(activeProfile)}>Delete</Button>
          </div>
        ) : (
          <div style={{ display: "flex", gap: 6 }}>
            <input className="vt-input" aria-label="Profile name" placeholder="Profile name" value={profileName}
              onChange={(e) => setProfileName(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter") saveProfile(profileName); }} />
            <Button size="sm" icon={<Plus size={14} strokeWidth={1.75} />} disabled={!profileName.trim()} onClick={() => saveProfile(profileName)}>Save</Button>
          </div>
        )}
        {!pro && <span style={hint}>One saved profile. More <ProBadge /></span>}
      </Section>

      <Section title="Connection">
        <ConnectionForm
          key={formKey}
          fields={FIELDS[connType]}
          initialValues={formValues}
          onChange={setFormValues}
          onSubmit={(v) => { void connectWith(connType, v); }}
          onDisconnect={() => { void disconnect(); }}
          connected={connected}
          loading={loading}
        />
      </Section>
    </>
  );

  const simSidebar = (
    <>
      <Section title="Simulator">
        <Segmented
          label="Simulator type"
          block
          value={simKind}
          onChange={setSimKind}
          options={[{ value: "mtconnect", label: "MTConnect" }, { value: "fanuc", label: "FANUC" }]}
        />
      </Section>
      {simKind === "mtconnect" ? (
        <Section title="Agent simulator" meta={mtcSim ? <StatusDot tone="success" label="Running" /> : undefined}>
          <Field label="Machine">
            <SheetSelect aria-label="Machine template" sheetTitle="Machine template" value={mtcTemplate} disabled={!!mtcSim} onChange={setMtcTemplate}
              options={[{ value: "Vmc3Axis", label: "3-axis VMC" }, { value: "Lathe2Axis", label: "2-axis lathe" }]} />
          </Field>
          <Field label="HTTP port">
            <input className="vt-input vt-input--mono" type="number" value={mtcPort} disabled={!!mtcSim} onChange={(e) => setMtcPort(Number(e.target.value))} />
          </Field>
          <label style={{ display: "flex", gap: 6, alignItems: "center", fontSize: "var(--fs-sm)" }}>
            <input type="checkbox" checked={shdrOn} disabled={!!mtcSim} onChange={(e) => setShdrOn(e.target.checked)} /> SHDR adapter
          </label>
          {shdrOn && (
            <Field label="SHDR port">
              <input className="vt-input vt-input--mono" type="number" value={shdrPort} disabled={!!mtcSim} onChange={(e) => setShdrPort(Number(e.target.value))} />
            </Field>
          )}
          {mtcSim ? (
            <Button variant="danger" block icon={<Square size={14} strokeWidth={1.75} />} onClick={() => { void stopMtcSim(); }}>Stop agent</Button>
          ) : (
            <Button variant="primary" block icon={<Play size={14} strokeWidth={1.75} />} onClick={() => { void startMtcSim(); }}>Start agent</Button>
          )}
          <span style={hint}>Listens on all interfaces, so other machines can read it. A cppagent can take the SHDR port as an adapter; save /probe as its devices file.</span>
        </Section>
      ) : (
        <Section title="CNC simulator" meta={cncSimPort != null ? <StatusDot tone="success" label="Running" /> : undefined}>
          <Field label="Controller">
            <SheetSelect aria-label="Controller profile" sheetTitle="Controller profile" value={cncProfile} disabled={cncSimPort != null} onChange={setCncProfile}
              options={[{ value: "series31i_b", label: "31i-B" }, { value: "series0i_f", label: "0i-F" }]} />
          </Field>
          <Field label="TCP port">
            <input className="vt-input vt-input--mono" type="number" value={cncPort} disabled={cncSimPort != null} onChange={(e) => setCncPort(Number(e.target.value))} />
          </Field>
          {cncSimPort != null ? (
            <Button variant="danger" block icon={<Square size={14} strokeWidth={1.75} />} onClick={() => { void stopCncSim(); }}>Stop CNC</Button>
          ) : (
            <Button variant="primary" block icon={<Play size={14} strokeWidth={1.75} />} onClick={() => { void startCncSim(); }}>Start CNC</Button>
          )}
          <span style={hint}>Listens on all interfaces. The 0i-F profile only changes the identity strings.</span>
        </Section>
      )}
    </>
  );

  // ── Main area ──
  const welcome = (
    <div style={{ flex: 1, display: "flex", alignItems: "center", justifyContent: "center" }}>
      <EmptyState
        icon={<Cpu size={20} strokeWidth={1.75} />}
        title={connType === "mtconnect" ? "Connect to an MTConnect agent" : "Connect to a FANUC CNC"}
        description={
          connType === "mtconnect"
            ? "Enter the agent URL in the sidebar and connect, or try the built-in simulated machine. https://demo.mtconnect.org is a public demo agent."
            : "Enter the controller address (FOCAS/Ethernet, port 8193) and connect, or try the built-in simulated CNC."
        }
        action={<Button variant="primary" icon={<Play size={14} strokeWidth={1.75} />} onClick={() => { void trySimulator(); }}>Try the built-in simulator <ProBadge /></Button>}
      />
    </div>
  );

  const clientMain = !connected ? welcome : connType === "mtconnect" && devices ? (
    <MtcView
      devices={devices}
      items={items}
      obs={obs}
      liveIds={liveIds}
      trending={trendingIds}
      onToggleTrend={(id) => toggleTrend(labels.get(id) ?? id)}
      onRefresh={() => { void refreshMtc(); }}
      requirePro={requirePro}
      addLog={addLog}
    />
  ) : connType === "fanuc" && info ? (
    <FanucView info={info} poll={poll} trending={new Set(trendSeries.keys())} onToggleTrend={toggleTrend} requirePro={requirePro} addLog={addLog} />
  ) : null;

  const simMain =
    simKind === "mtconnect" ? (
      mtcSim ? <MtcSimPanel baseUrl={mtcSim.baseUrl} onConnect={() => { void connectToSim("mtconnect"); }} /> : (
        <EmptyState icon={<Cpu size={20} strokeWidth={1.75} />} title="Agent simulator not running" description="Pick a machine and start the agent. It serves /probe, /current, /sample and /assets and, optionally, an SHDR adapter."
          action={<Button variant="primary" icon={<Play size={14} strokeWidth={1.75} />} onClick={() => { void startMtcSim(); }}>Start agent</Button>} />
      )
    ) : cncSimPort != null ? (
      <FocasSimPanel onConnect={() => { void connectToSim("fanuc"); }} />
    ) : (
      <EmptyState icon={<Cpu size={20} strokeWidth={1.75} />} title="CNC simulator not running" description="Starts a FANUC-style CNC that answers FOCAS/Ethernet requests on the chosen port."
        action={<Button variant="primary" icon={<Play size={14} strokeWidth={1.75} />} onClick={() => { void startCncSim(); }}>Start CNC</Button>} />
    );

  const simRunning = simKind === "mtconnect" ? !!mtcSim : cncSimPort != null;
  const stopSim = () => { void (simKind === "mtconnect" ? stopMtcSim() : stopCncSim()); };

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100vh", background: "var(--bg)", color: "var(--text)" }}>
      <Toolbar
        title="CNC Explorer"
        icon={appIcon}
        connected={mode === "client" ? connected : simRunning}
        onDisconnect={mode === "client" ? () => { void disconnect(); } : stopSim}
        onInfo={() => setShowInfo(true)}
      />
      <div style={{ display: "flex", flex: 1, overflow: "hidden" }}>
        <aside className="vt-sidebar" style={{ width: 280, flexShrink: 0, borderRight: "1px solid var(--border)" }}>
          <Section>
            <Segmented
              label="Mode"
              block
              value={mode}
              onChange={(v) => { if (v === "simulator" && !requirePro("Built-in simulators")) return; setMode(v); }}
              options={[
                { value: "client", label: "Client" },
                {
                  value: "simulator",
                  label: (
                    <>
                      Simulator <ProBadge />
                      {(mtcSim || cncSimPort != null) && <StatusDot tone="success" label="Simulator running" />}
                    </>
                  ),
                },
              ]}
            />
          </Section>
          {mode === "client" ? clientSidebar : simSidebar}
        </aside>
        <main style={{ display: "flex", flex: 1, flexDirection: "column", overflow: "hidden", padding: 8, minWidth: 0, gap: 8 }}>
          <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
            {mode === "client" ? clientMain : simMain}
          </div>
          <div style={{ height: 240, flexShrink: 0, display: "flex", flexDirection: "column", gap: 6 }}>
            <Tabs
              label="Bottom panel"
              value={bottomTab}
              onChange={setBottomTab}
              items={[
                { value: "log", label: "Comm log" },
                { value: "trend", label: "Trend", count: trendSeries.size > 0 ? trendSeries.size : undefined },
              ]}
            />
            {bottomTab === "log" ? (
              <LogPanel entries={log} onExport={(f) => addLog("info", `Log exported to ${f}`)} onClear={() => setLog([])} />
            ) : (
              <TrendPanel
                series={Array.from(trendSeries.values())}
                onRemove={(name) => setTrendSeries((prev) => { const next = new Map(prev); next.delete(name); return next; })}
                windowSec={trendWindow}
                onWindowChange={setTrendWindow}
                theme="dark"
              />
            )}
          </div>
        </main>
      </div>
      <StatusBar connected={mode === "client" ? connected : simRunning} latency={latency} pollCount={pollCount} error={error} />

      <Dialog open={showInfo} onClose={() => setShowInfo(false)} title="About CNC Explorer" width={420} footer={(
        <>
          {/* No outbound source link in App Store builds: review treats links to builds that unlock paid
              features elsewhere as steering away from in-app purchase. */}
          {proStatus && !proStatus.store && (
            <Button variant="ghost" onClick={() => invoke("open_url", { url: "https://github.com/VoltrusID/voltrus-explorers" })}>
              Source on GitHub
            </Button>
          )}
          <Button variant="ghost" onClick={openPrivacy}>Privacy policy</Button>
          <Button block={!proStatus || proStatus.store} onClick={() => setShowInfo(false)}>Close</Button>
        </>
      )}>
        <div style={{ fontSize: "var(--fs-sm)", color: "var(--text-dim)", lineHeight: 1.6 }}>
          <p style={{ margin: "0 0 12px" }}>Monitor CNC machines over MTConnect and FANUC FOCAS/Ethernet.</p>
          <p style={{ margin: "0 0 4px", fontWeight: 600, color: "var(--text)" }}>MTConnect agent</p>
          <p style={{ margin: "0 0 12px" }}>Device tree from /probe, live observations from the sample stream, conditions, trends of numeric samples and assets with their XML.</p>
          <p style={{ margin: "0 0 4px", fontWeight: 600, color: "var(--text)" }}>FANUC CNC (Beta)</p>
          <p style={{ margin: "0 0 12px" }}>Identity, status, axis positions, feed, spindle and servo load, program, alarms, macro variables, parameters, diagnosis and PMC. {BETA_EXPLAINER}.</p>
          <div style={{ borderTop: "1px solid var(--border)", paddingTop: 12, marginBottom: 12 }}>
            <ProAboutRow />
          </div>
          <p style={{ margin: "0 0 8px", fontSize: "var(--fs-xs)", color: "var(--text-muted)" }}>
            Independent: CNC Explorer is not affiliated with, endorsed by or supported by FANUC CORPORATION. FANUC and FOCAS are trademarks of FANUC CORPORATION and are used only to name the controllers and protocol this app works with.
          </p>
          <p style={{ margin: "0 0 12px", fontSize: "var(--fs-xs)", color: "var(--text-muted)" }}>
            MTConnect&reg; is a registered trademark of AMT - The Association For Manufacturing Technology. CNC Explorer is an independent implementation of the MTConnect client protocol, not affiliated with AMT.
          </p>
          <span style={{ color: "var(--text-muted)" }}>Part of the </span>
          <strong style={{ color: "var(--accent-text)" }}>Voltrus</strong>
          <span style={{ color: "var(--text-muted)" }}> suite by </span>
          <strong style={{ color: "var(--text)" }}>JoyoDigitama</strong>
        </div>
      </Dialog>
    </div>
  );
}

export default App;
