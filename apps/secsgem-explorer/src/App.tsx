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
  useProStatus,
  withUtm,
} from "@mactools/ui";
import type { LogEntry } from "@mactools/ui";
import { Cpu, Play, Plus, RotateCcw, Square, Trash2 } from "lucide-react";
import appIcon from "../src-tauri/icons/128x128.png";
import AlarmsView, { type AlarmEvent } from "./AlarmsView";
import ConstantsView from "./ConstantsView";
import EventsView from "./EventsView";
import MessagesView from "./MessagesView";
import ProgramsView from "./ProgramsView";
import RemoteView, { DEFAULT_PARAMS, DEFAULT_RCMD, type RemoteResult } from "./RemoteView";
import SimDashboard from "./SimView";
import StatusView from "./StatusView";
import TerminalView, { type TerminalLine } from "./TerminalView";
import TrendPanel from "./TrendPanel";
import { useTrend } from "./useTrend";
import { useStatusVars } from "./useStatusVars";
import { eventRow, hostConnectOf, hostFormOf, pushCapped, sameHost, type EventRow } from "./gem";
import { HOST_DEFAULTS, loadProfiles, needsPro, saveProfiles, type ConnProfile } from "./storage";
import type { ConnInfo, ControlView, EstablishView, GemEvent, HostForm, LocalReport, MsgEntry, OnlineView, ParamIn } from "./types";
import { ackText } from "./gem";
import { hint } from "./ui";

const BETA_EXPLAINER =
  "Independent implementation, cross-checked against the open-source secsgem library, untested on real equipment or commercial hosts. Please report what you connect it to at support@voltrus.id";

const FIELDS = [
  {
    name: "mode",
    label: "HSMS mode",
    type: "select" as const,
    default: "active",
    options: [
      { value: "active", label: "Active (connect to the equipment)" },
      { value: "passive", label: "Passive (wait for the equipment)" },
    ],
  },
  { name: "host", label: "Address (equipment, or listen address when passive)", type: "text" as const, default: HOST_DEFAULTS.host },
  { name: "port", label: "Port", type: "number" as const, default: HOST_DEFAULTS.port },
  { name: "deviceId", label: "Device id", type: "number" as const, default: HOST_DEFAULTS.deviceId },
  { name: "t3", label: "T3 reply timeout (s)", type: "number" as const, default: HOST_DEFAULTS.t3 },
  { name: "t5", label: "T5 connect separation (s)", type: "number" as const, default: HOST_DEFAULTS.t5 },
  { name: "t6", label: "T6 control transaction (s)", type: "number" as const, default: HOST_DEFAULTS.t6 },
  { name: "t7", label: "T7 not selected (s)", type: "number" as const, default: HOST_DEFAULTS.t7 },
  { name: "t8", label: "T8 inter-character (s)", type: "number" as const, default: HOST_DEFAULTS.t8 },
  { name: "controlSvid", label: "Control state SVID (0 for none)", type: "number" as const, default: HOST_DEFAULTS.controlSvid },
];

type Tab = "status" | "constants" | "events" | "alarms" | "commands" | "programs" | "terminal" | "messages";
type SimTab = "dashboard" | "messages";

const MAX_LOG = 2000;
const MAX_MSGS = 4000;
const MAX_ROWS = 2000;

const profileOf = (f: HostForm, name = ""): ConnProfile => ({ ...f, name });
const toForm = (p: ConnProfile): HostForm => ({
  mode: p.mode, host: p.host, port: p.port, deviceId: p.deviceId, t3: p.t3, t5: p.t5, t6: p.t6, t7: p.t7, t8: p.t8, controlSvid: p.controlSvid,
});

function App() {
  const { requirePro, pro, status: proStatus } = useProStatus();
  const [log, setLog] = useState<LogEntry[]>([]);
  const [msgs, setMsgs] = useState<MsgEntry[]>([]);
  const [showInfo, setShowInfo] = useState(false);
  const [bottomTab, setBottomTab] = useState<"log" | "trend">("log");
  const [mode, setMode] = useState<"client" | "simulator">("client");
  const [tab, setTab] = useState<Tab>("status");
  const [simTab, setSimTab] = useState<SimTab>("dashboard");

  // Connection
  const [form, setForm] = useState<HostForm>(HOST_DEFAULTS);
  const [formKey, setFormKey] = useState(0);
  const [connected, setConnected] = useState(false);
  const [listening, setListening] = useState<string | null>(null);
  const [peer, setPeer] = useState("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [commOk, setCommOk] = useState(false);
  const [identity, setIdentity] = useState("");
  const [control, setControl] = useState<ControlView | null>(null);

  // Profiles
  const [profiles, setProfiles] = useState<ConnProfile[]>(loadProfiles);
  const [activeProfile, setActiveProfile] = useState<string | null>(null);
  const [profileName, setProfileName] = useState("");

  // Session data
  const [reports, setReports] = useState<LocalReport[]>([]);
  const [eventRows, setEventRows] = useState<EventRow[]>([]);
  const [alarmEvents, setAlarmEvents] = useState<AlarmEvent[]>([]);
  const [terminal, setTerminal] = useState<TerminalLine[]>([]);
  const [rcmd, setRcmd] = useState(DEFAULT_RCMD);
  const [rcmdParams, setRcmdParams] = useState<ParamIn[]>(DEFAULT_PARAMS);
  const [rcmdResults, setRcmdResults] = useState<RemoteResult[]>([]);

  // Simulator
  const [simPort, setSimPort] = useState<number | null>(null);
  const [simCfg, setSimCfg] = useState({ mode: "passive", host: "127.0.0.1", port: 5000, deviceId: 0, initiate: true });

  const addLog = useCallback((direction: LogEntry["direction"], message: string) => {
    setLog((prev) => pushCapped(prev, { timestamp: Date.now(), direction, message }, MAX_LOG));
  }, []);

  const sv = useStatusVars(connected && commOk, (m) => setError(m));

  // ── Events from Rust ──
  const reportsRef = useRef(reports);
  reportsRef.current = reports;
  const namesRef = useRef(new Map<number, string>());
  namesRef.current = new Map(sv.names.map((n) => [n.id, n.name]));
  const noteEvent = useRef({ reset: (_reason?: string) => {} });

  useEffect(() => {
    const subs = [
      listen<{ direction: LogEntry["direction"]; message: string }>("gem-log", (e) => addLog(e.payload.direction, e.payload.message)),
      listen<MsgEntry>("gem-msg", (e) => {
        const m = e.payload;
        setMsgs((prev) => pushCapped(prev, m, MAX_MSGS));
        addLog(m.dir, `${m.source === "sim" ? "[sim] " : ""}${m.data ? m.text : `${m.kind} ${m.text}`.trim()}`);
        if (m.source === "host" && m.dir === "rx" && m.kind.startsWith("S1F13")) setCommOk(true);
      }),
      listen<GemEvent>("gem-event", (e) => {
        const ev = e.payload;
        const now = Date.now();
        if (ev.kind === "event") {
          setEventRows((prev) => pushCapped(prev, eventRow(now, ev, reportsRef.current, namesRef.current), MAX_ROWS));
        } else if (ev.kind === "alarm") {
          setAlarmEvents((prev) => pushCapped(prev, { time: now, alid: ev.alid, set: ev.set, category: ev.category, text: ev.text }, MAX_ROWS));
        } else if (ev.kind === "terminal") {
          setTerminal((prev) => pushCapped(prev, { time: now, dir: "rx", tid: ev.tid, text: ev.text }, MAX_ROWS));
        } else if (ev.kind === "s9") {
          addLog("error", `Equipment reported an error: ${ev.text}`);
        } else if (ev.kind === "unhandled") {
          addLog("info", `Unhandled message from the equipment: ${ev.text}`);
        }
      }),
      listen<{ state: "selected" | "closed"; peer?: string; reason?: string }>("gem-conn", (e) => {
        if (e.payload.state === "selected") {
          setListening(null);
          setConnected(true);
          setPeer(e.payload.peer ?? "");
          setError(null);
        } else {
          noteEvent.current.reset(e.payload.reason);
        }
      }),
    ];
    return () => { subs.forEach((s) => s.then((un) => un())); };
  }, [addLog]);

  const reset = useCallback((reason?: string) => {
    setConnected(false);
    setListening(null);
    setCommOk(false);
    setIdentity("");
    setControl(null);
    setPeer("");
    setReports([]);
    if (reason) {
      setError(reason);
      noteDeviceError();
    }
  }, []);
  noteEvent.current.reset = reset;

  // ── Connect / disconnect ──
  const disconnect = useCallback(async () => {
    try { await invoke("gem_disconnect"); } catch { /* not connected */ }
    reset();
  }, [reset]);

  const establish = useCallback(async () => {
    try {
      const r = await invoke<EstablishView>("gem_establish");
      setCommOk(r.ack.ok);
      setIdentity(`${r.mdln} ${r.softrev}`.trim());
      setControl(r.control);
      if (!r.ack.ok) setError(`The equipment refused communication: ${ackText(r.ack)}`);
      else setError(null);
    } catch (e) {
      setError(String(e));
      noteDeviceError();
    }
  }, []);

  const connectWith = useCallback(async (f: HostForm) => {
    setLoading(true);
    setError(null);
    try {
      const info = await invoke<ConnInfo>("gem_connect", { config: hostConnectOf(f) });
      if (info.state === "listening") {
        setListening(info.peer);
      } else {
        setConnected(true);
        setPeer(info.peer);
        await establish();
      }
    } catch (e) {
      setError(String(e));
      noteDeviceError();
      reset();
    } finally {
      setLoading(false);
    }
  }, [establish, reset]);

  const applyOnline = (r: OnlineView) => {
    setControl(r.control);
    addLog(r.ack.ok ? "info" : "error", `Online/offline request: ${ackText(r.ack)}`);
    if (!r.ack.ok) setError(`The equipment answered ${ackText(r.ack)}`);
    else setError(null);
  };
  const request = async (cmd: "gem_online" | "gem_offline") => {
    try { applyOnline(await invoke<OnlineView>(cmd)); } catch (e) { setError(String(e)); }
  };
  const refreshControl = async () => {
    try { setControl(await invoke<ControlView | null>("gem_control")); } catch (e) { setError(String(e)); }
  };

  // ── Profiles ──
  const saveProfile = useCallback((name: string) => {
    const n = name.trim();
    if (!n) return;
    if (needsPro(profiles, n, pro) && !requirePro("More than one saved connection profile")) return;
    const next = [...profiles.filter((p) => p.name !== n), profileOf(form, n)];
    setProfiles(next);
    saveProfiles(next);
    setActiveProfile(n);
    setProfileName("");
  }, [profiles, pro, requirePro, form]);

  const loadProfile = useCallback((name: string) => {
    const p = profiles.find((x) => x.name === name);
    if (!p) return;
    setForm(toForm(p));
    setActiveProfile(p.name);
    setFormKey((k) => k + 1);
  }, [profiles]);

  const deleteProfile = useCallback((name: string) => {
    const next = profiles.filter((p) => p.name !== name);
    setProfiles(next);
    saveProfiles(next);
    if (activeProfile === name) setActiveProfile(null);
  }, [profiles, activeProfile]);

  const profileDirty = useMemo(() => {
    const saved = profiles.find((p) => p.name === activeProfile);
    return !!saved && !sameHost(toForm(saved), form);
  }, [profiles, activeProfile, form]);

  // ── Simulator ──
  const startSim = useCallback(async (): Promise<number | null> => {
    try {
      const port = await invoke<number>("sim_start", {
        config: {
          mode: simCfg.mode, host: simCfg.host, port: simCfg.port, deviceId: simCfg.deviceId,
          t3Ms: 45000, t5Ms: 10000, t6Ms: 5000, t7Ms: 10000, t8Ms: 5000, initiateCommunication: simCfg.initiate,
        },
      });
      setSimPort(port);
      return port;
    } catch (e) {
      setError(String(e));
      return null;
    }
  }, [simCfg]);

  const stopSim = useCallback(async () => {
    await invoke("sim_stop");
    setSimPort(null);
  }, []);

  const connectToSim = useCallback(async (port: number) => {
    if (connected || listening) await disconnect();
    const f: HostForm = { ...HOST_DEFAULTS, host: "127.0.0.1", port };
    setMode("client");
    setForm(f);
    setFormKey((k) => k + 1);
    setActiveProfile(null);
    await connectWith(f);
  }, [connected, listening, disconnect, connectWith]);

  const trySimulator = useCallback(async () => {
    if (!requirePro("The equipment simulator")) return;
    const port = simPort ?? (await startSim());
    if (port != null && port > 0) await connectToSim(port);
  }, [requirePro, simPort, startSim, connectToSim]);

  // Names load once communication is up, however it was established (our S1F13 or the equipment's).
  const loadNames = sv.loadNames;
  const noNames = sv.names.length === 0;
  useEffect(() => {
    if (connected && commOk && noNames) void loadNames();
  }, [connected, commOk, noNames, loadNames]);

  // ── Trend ──
  const labelOf = useCallback((id: number) => {
    const n = sv.names.find((x) => x.id === id);
    if (!n) return String(id);
    return sv.names.filter((x) => x.name === n.name).length > 1 ? `${n.name} [${id}]` : n.name;
  }, [sv.names]);
  const trendRows = useMemo(
    () =>
      sv.names.flatMap((n) => {
        const v = sv.values.get(n.id)?.number;
        return v == null ? [] : [{ name: labelOf(n.id), value: v }];
      }),
    [sv.names, sv.values, labelOf],
  );
  const { trendSeries, setTrendSeries, trendWindow, setTrendWindow, toggleTrend } = useTrend(trendRows);
  useEffect(() => { setTrendSeries(new Map()); }, [connected, setTrendSeries]);

  const openPrivacy = useCallback(async () => {
    try { await invoke("open_url", { url: await withUtm("https://voltrus.id/privacy/", "privacy") }); } catch (e) { addLog("error", String(e)); }
  }, [addLog]);

  const active = connected || listening != null;
  const hostMsgs = useMemo(() => msgs.filter((m) => m.source === "host"), [msgs]);
  const simMsgs = useMemo(() => msgs.filter((m) => m.source === "sim"), [msgs]);

  // ── Sidebar ──
  const clientSidebar = (
    <>
      <Section title="Profiles">
        <SheetSelect
          aria-label="Connection profile"
          sheetTitle="Connection profile"
          value={activeProfile ?? ""}
          options={[{ value: "", label: "New profile" }, ...profiles.map((p) => ({ value: p.name, label: p.name }))]}
          onChange={(v) => { if (v) loadProfile(v); else setActiveProfile(null); }}
          disabled={active}
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
          fields={FIELDS}
          initialValues={{ ...form }}
          onChange={(v) => setForm(hostFormOf(v, HOST_DEFAULTS))}
          onSubmit={(v) => { void connectWith(hostFormOf(v, HOST_DEFAULTS)); }}
          onDisconnect={() => { void disconnect(); }}
          connected={active}
          loading={loading}
        />
      </Section>
      <Section title="Beta">
        <Callout tone="warn">{BETA_EXPLAINER}.</Callout>
      </Section>
    </>
  );

  const simSidebar = (
    <>
      <Section title="Equipment simulator" meta={simPort != null ? <StatusDot tone="success" label="Running" /> : undefined}>
        <Segmented
          label="Simulator HSMS mode"
          block
          value={simCfg.mode as "passive" | "active"}
          onChange={(m) => setSimCfg({ ...simCfg, mode: m })}
          options={[{ value: "passive", label: "Passive" }, { value: "active", label: "Active" }]}
        />
        <Field label={simCfg.mode === "passive" ? "Listen address" : "Host address"}>
          <input className="vt-input vt-input--mono" value={simCfg.host} disabled={simPort != null} onChange={(e) => setSimCfg({ ...simCfg, host: e.target.value })} />
        </Field>
        <Field label="Port">
          <input className="vt-input vt-input--mono" type="number" value={simCfg.port} disabled={simPort != null} onChange={(e) => setSimCfg({ ...simCfg, port: Number(e.target.value) })} />
        </Field>
        <Field label="Device id">
          <input className="vt-input vt-input--mono" type="number" value={simCfg.deviceId} disabled={simPort != null} onChange={(e) => setSimCfg({ ...simCfg, deviceId: Number(e.target.value) })} />
        </Field>
        <label style={{ display: "flex", gap: 6, alignItems: "center", fontSize: "var(--fs-sm)" }}>
          <input type="checkbox" checked={simCfg.initiate} disabled={simPort != null} onChange={(e) => setSimCfg({ ...simCfg, initiate: e.target.checked })} /> Send S1F13 after Select
        </label>
        {simPort != null ? (
          <Button variant="danger" block icon={<Square size={14} strokeWidth={1.75} />} onClick={() => { void stopSim(); }}>Stop equipment</Button>
        ) : (
          <Button variant="primary" block icon={<Play size={14} strokeWidth={1.75} />} onClick={() => { void startSim(); }}>Start equipment</Button>
        )}
        <span style={hint}>A wafer chamber with made-up ids, ranges and alarms. Passive mode listens on the address above; the standard HSMS port is 5000.</span>
      </Section>
    </>
  );

  // ── Main area ──
  const header = (
    <div style={{ display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap" }}>
      <Badge tone={connected ? "success" : "warn"}>{connected ? `HSMS selected${peer ? ` (${peer})` : ""}` : "HSMS not connected"}</Badge>
      <Badge tone={commOk ? "success" : "neutral"}>{commOk ? "Communicating" : "Communication not established"}</Badge>
      {control && <Badge tone={control.online ? "success" : "warn"}>{control.name}</Badge>}
      {identity && <span className="mono" style={{ fontSize: "var(--fs-xs)", color: "var(--text-dim)" }}>{identity}</span>}
      <span style={{ flex: 1 }} />
      <Button size="sm" variant={commOk ? undefined : "primary"} onClick={() => { void establish(); }}>Establish communications</Button>
      <Button size="sm" disabled={!commOk} onClick={() => { void request("gem_online"); }}>Request online</Button>
      <Button size="sm" disabled={!commOk} onClick={() => { void request("gem_offline"); }}>Request offline</Button>
      <Button size="sm" variant="ghost" disabled={!commOk} onClick={() => { void refreshControl(); }}>Refresh state</Button>
    </div>
  );

  const welcome = (
    <div style={{ flex: 1, display: "flex", alignItems: "center", justifyContent: "center" }}>
      <EmptyState
        icon={<Cpu size={20} strokeWidth={1.75} />}
        title={listening ? `Waiting for the equipment on ${listening}` : "Connect to equipment as a GEM host"}
        description={
          listening
            ? "The equipment connects to this address as the HSMS active entity and selects the session."
            : "Enter the equipment address in the sidebar (HSMS, port 5000 by default) and connect, or try the built-in simulated equipment."
        }
        action={!listening && <Button variant="primary" icon={<Play size={14} strokeWidth={1.75} />} onClick={() => { void trySimulator(); }}>Try the built-in simulator <ProBadge /></Button>}
      />
    </div>
  );

  const clientMain = !connected ? welcome : (
    <div style={{ display: "flex", flexDirection: "column", flex: 1, minHeight: 0, gap: 8 }}>
      {header}
      <Tabs
        label="GEM views"
        value={tab}
        onChange={setTab}
        items={[
          { value: "status", label: "Status variables" },
          { value: "constants", label: "Constants" },
          { value: "events", label: "Events", count: eventRows.length > 0 ? eventRows.length : undefined },
          { value: "alarms", label: "Alarms", count: alarmEvents.length > 0 ? alarmEvents.length : undefined },
          { value: "commands", label: "Commands" },
          { value: "programs", label: "Programs" },
          { value: "terminal", label: "Terminal" },
          { value: "messages", label: "Messages", count: hostMsgs.length > 0 ? hostMsgs.length : undefined },
        ]}
      />
      {tab === "status" && (
        <StatusView
          sv={sv}
          pro={pro}
          requirePro={requirePro}
          trending={new Set(trendSeries.keys())}
          trendLabel={labelOf}
          onToggleTrend={toggleTrend}
        />
      )}
      {tab === "constants" && <ConstantsView requirePro={requirePro} addLog={addLog} />}
      {tab === "events" && (
        <EventsView
          pro={pro}
          requirePro={requirePro}
          addLog={addLog}
          names={sv.names}
          reports={reports}
          setReports={setReports}
          rows={eventRows}
          onClear={() => setEventRows([])}
        />
      )}
      {tab === "alarms" && <AlarmsView events={alarmEvents} onClear={() => setAlarmEvents([])} addLog={addLog} />}
      {tab === "commands" && <RemoteView rcmd={rcmd} setRcmd={setRcmd} params={rcmdParams} setParams={setRcmdParams} result={rcmdResults} setResult={setRcmdResults} pro={pro} requirePro={requirePro} addLog={addLog} />}
      {tab === "programs" && <ProgramsView />}
      {tab === "terminal" && (
        <TerminalView
          lines={terminal}
          onSent={(tid, text) => setTerminal((prev) => pushCapped(prev, { time: Date.now(), dir: "tx", tid, text }, MAX_ROWS))}
          onClear={() => setTerminal([])}
          addLog={addLog}
        />
      )}
      {tab === "messages" && (
        <MessagesView
          entries={hostMsgs}
          onClear={() => setMsgs((prev) => prev.filter((m) => m.source !== "host"))}
          requirePro={requirePro}
          addLog={addLog}
          canCompose
        />
      )}
    </div>
  );

  const simMain = simPort == null ? (
    <EmptyState
      icon={<Cpu size={20} strokeWidth={1.75} />}
      title="Equipment simulator not running"
      description="Starts a wafer processing chamber that speaks GEM over HSMS. Connect this app, or any other host, to it."
      action={<Button variant="primary" icon={<Play size={14} strokeWidth={1.75} />} onClick={() => { void startSim(); }}>Start equipment</Button>}
    />
  ) : (
    <div style={{ display: "flex", flexDirection: "column", flex: 1, minHeight: 0, gap: 8 }}>
      <Tabs label="Simulator views" value={simTab} onChange={setSimTab} items={[{ value: "dashboard", label: "Equipment" }, { value: "messages", label: "Messages", count: simMsgs.length > 0 ? simMsgs.length : undefined }]} />
      {simTab === "dashboard" ? (
        <SimDashboard addLog={addLog} onConnect={(p) => { void connectToSim(p); }} />
      ) : (
        <MessagesView entries={simMsgs} onClear={() => setMsgs((prev) => prev.filter((m) => m.source !== "sim"))} requirePro={requirePro} addLog={addLog} canCompose={false} />
      )}
    </div>
  );

  const barConnected = mode === "client" ? connected : simPort != null;

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100vh", background: "var(--bg)", color: "var(--text)" }}>
      <Toolbar
        title="SECS/GEM Explorer"
        icon={appIcon}
        connected={barConnected}
        actions={<Badge tone="warn" title={BETA_EXPLAINER}>Beta</Badge>}
        onDisconnect={mode === "client" ? () => { void disconnect(); } : () => { void stopSim(); }}
        onInfo={() => setShowInfo(true)}
      />
      <div style={{ display: "flex", flex: 1, overflow: "hidden" }}>
        <aside className="vt-sidebar" style={{ width: 290, flexShrink: 0, borderRight: "1px solid var(--border)" }}>
          <Section>
            <Segmented
              label="Mode"
              block
              value={mode}
              onChange={(v) => { if (v === "simulator" && !requirePro("The equipment simulator")) return; setMode(v); }}
              options={[
                { value: "client", label: "Host" },
                { value: "simulator", label: (<>Equipment <ProBadge />{simPort != null && <StatusDot tone="success" label="Simulator running" />}</>) },
              ]}
            />
          </Section>
          {mode === "client" ? clientSidebar : simSidebar}
        </aside>
        <main style={{ display: "flex", flex: 1, flexDirection: "column", overflow: "hidden", padding: 8, minWidth: 0, gap: 8 }}>
          {error && mode === "client" && <Callout tone="danger">{error}</Callout>}
          <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
            {mode === "client" ? clientMain : simMain}
          </div>
          <div style={{ height: 220, flexShrink: 0, display: "flex", flexDirection: "column", gap: 6 }}>
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
      <StatusBar connected={barConnected} latency={sv.latency} pollCount={sv.polls} error={mode === "client" ? error : null} />

      <Dialog open={showInfo} onClose={() => setShowInfo(false)} title="About SECS/GEM Explorer" width={460} footer={(
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
          <p style={{ margin: "0 0 12px" }}>
            <Badge tone="warn">Beta</Badge> A GEM host and equipment simulator over HSMS. {BETA_EXPLAINER}.
          </p>
          <p style={{ margin: "0 0 4px", fontWeight: 600, color: "var(--text)" }}>Host</p>
          <p style={{ margin: "0 0 12px" }}>Establish communications, go online or offline, read status variables and equipment constants, define reports and watch event reports, alarms, remote commands, process programs and terminal messages. Every message is shown as SML.</p>
          <p style={{ margin: "0 0 4px", fontWeight: 600, color: "var(--text)" }}>Equipment simulator</p>
          <p style={{ margin: "0 0 12px" }}>A wafer chamber with a control state model, process state, constants with ranges and alarms, for trying a host without a tool.</p>
          <div style={{ borderTop: "1px solid var(--border)", paddingTop: 12, marginBottom: 12 }}>
            <ProAboutRow />
          </div>
          <p style={{ margin: "0 0 8px", fontSize: "var(--fs-xs)", color: "var(--text-muted)" }}>
            SECS/GEM, SECS-II and HSMS are the SEMI standards E5, E30 and E37. SECS/GEM Explorer is an independent implementation, not affiliated with, endorsed by or certified by SEMI, and it passing its own tests says nothing about SEMI compliance. SEMI is a trademark of SEMI and is used only to name the standards this app works with.
          </p>
          <p style={{ margin: "0 0 12px", fontSize: "var(--fs-xs)", color: "var(--text-muted)" }}>
            Message layouts follow the open-source secsgem library (bparzella/secsgem, LGPL-2.1-or-later), which this implementation was cross-checked against.
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
