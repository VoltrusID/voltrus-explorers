import { useState, useCallback, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  Badge,
  Button,
  ConnectionForm,
  DataTable,
  Dialog,
  EmptyState,
  Field,
  IconButton,
  LogPanel,
  ProAboutRow,
  ProBadge,
  Section,
  Segmented,
  StatusBar,
  StatusDot,
  Tabs,
  Toolbar,
  ValueEditor,
  useProStatus,
  useProtocol,
} from "@mactools/ui";
import type { LogEntry } from "@mactools/ui";
import {
  Clock,
  Pause,
  Play,
  Plus,
  RotateCcw,
  Server,
  Snowflake,
  Square,
  Trash2,
  X,
} from "lucide-react";
import appIcon from "../src-tauri/icons/128x128.png";
import { BUILTIN_TEMPLATES, loadCustomTemplates, saveCustomTemplates, type DeviceTemplate } from "./templates";
import TrendPanel from "./TrendPanel";
import { useTrend } from "./useTrend";

interface Dnp3Config {
  id: string;
  host: string;
  port: number;
  master_address: number;
  outstation_address: number;
}

interface Dnp3Point {
  name: string;
  group: number;
  variation: number;
  index: number;
}

interface OutstationLogEvent {
  timestamp: number;
  function_code: number;
  message: string;
}

interface OutstationEvent {
  class: number;
  group: number;
  index: number;
  value: number;
  timestamp: number;
  sequence: number;
}

// ── Connection profiles ──
interface ConnProfile {
  name: string;
  host: string;
  port: number;
  masterAddress: number;
  outstationAddress: number;
}

const CONN_PROFILES_KEY = "dnp3-explorer-conn-profiles";

function loadConnProfiles(): ConnProfile[] {
  try { const raw = localStorage.getItem(CONN_PROFILES_KEY); return raw ? JSON.parse(raw) : []; } catch { return []; }
}

function saveConnProfiles(profiles: ConnProfile[]) {
  localStorage.setItem(CONN_PROFILES_KEY, JSON.stringify(profiles));
}

// ── Style constants ──

const eventBoxStyle: React.CSSProperties = {
  background: "var(--sunken)",
  border: "1px solid var(--border)",
  borderRadius: "var(--radius)",
  padding: "6px 8px",
  maxHeight: 180,
  overflowY: "auto",
  fontFamily: "var(--font-mono)",
  fontSize: "var(--fs-xs)",
  lineHeight: 1.6,
};

const liveValuesStyle: React.CSSProperties = {
  display: "flex",
  justifyContent: "space-between",
  gap: 8,
  padding: "3px 0",
  borderBottom: "1px solid var(--border)",
  fontSize: "var(--fs-xs)",
};

// ── Master point list ──
const FREE_POINT_LIMIT = 10;
const FREE_PROFILE_LIMIT = 1;
const POINTS_KEY = "dnp3-explorer-points";
const POINT_TYPES: { value: string; label: string }[] = [
  { value: "30:4", label: "Analog input (float32)" },
  { value: "30:1", label: "Analog input (int32)" },
  { value: "1:2", label: "Binary input" },
  { value: "10:2", label: "Binary output status" },
  { value: "20:1", label: "Counter (uint32)" },
  { value: "32:1", label: "Analog output status (int32)" },
];
const DEFAULT_POINTS: Dnp3Point[] = [
  { name: "Voltage", group: 30, variation: 4, index: 0 },
  { name: "Current", group: 30, variation: 4, index: 1 },
  { name: "Frequency", group: 30, variation: 4, index: 2 },
  { name: "BreakerStatus", group: 1, variation: 2, index: 0 },
  { name: "Power", group: 30, variation: 4, index: 3 },
];

function loadPoints(): Dnp3Point[] {
  try {
    const raw = localStorage.getItem(POINTS_KEY);
    if (raw) {
      const parsed = JSON.parse(raw);
      if (Array.isArray(parsed)) return parsed;
    }
  } catch { /* fall through to defaults */ }
  return DEFAULT_POINTS;
}

function savePoints(points: Dnp3Point[]) {
  localStorage.setItem(POINTS_KEY, JSON.stringify(points));
}

function App() {
  const protocol = useProtocol<Dnp3Config, Dnp3Point, unknown>("dnp3");
  const { requirePro, pro, status: proStatus } = useProStatus();
  const [points, setPoints] = useState<Dnp3Point[]>(loadPoints);
  const [newPoint, setNewPoint] = useState({ name: "", type: "30:4", index: "0" });
  useEffect(() => { savePoints(points); }, [points]);
  const [log, setLog] = useState<LogEntry[]>([]);
  const [editingPoint, setEditingPoint] = useState<string | null>(null);
  const [editingValue, setEditingValue] = useState<string>("");
  const [showInfo, setShowInfo] = useState(false);
  const [bottomTab, setBottomTab] = useState<"log" | "trend">("log");

  // Mode state
  const [mode, setMode] = useState<"master" | "outstation">("master");

  // Panel resize
  const [leftWidth, setLeftWidth] = useState(260);
  const [tableHeight, setTableHeight] = useState(300);
  const [dragSide, setDragSide] = useState<"left" | "table" | null>(null);

  // Mobile
  const [isMobile, setIsMobile] = useState(window.innerWidth < 768);
  const [mobileTab, setMobileTab] = useState<"connect" | "data" | "template" | "log" | "trend">("connect");

  // Connection profiles
  const [connProfiles, setConnProfiles] = useState<ConnProfile[]>(loadConnProfiles());
  const [activeConnProfile, setActiveConnProfile] = useState<string | null>(null);
  const [profileDirty, setProfileDirty] = useState(false);
  const [tcpFormValues, setTcpFormValues] = useState<Record<string, string | number>>({
    host: "192.168.1.100", port: 20000, masterAddress: 1, outstationAddress: 10, pollInterval: 1000,
  });
  const [formKey, setFormKey] = useState(0);

  // Outstation state
  const [osRunning, setOsRunning] = useState(false);
  const [osPort, setOsPort] = useState(20000);
  const [osMasterAddr, setOsMasterAddr] = useState(1);
  const [osOutstationAddr, setOsOutstationAddr] = useState(10);
  const [osAnalogCount, setOsAnalogCount] = useState(10);
  const [osBinaryCount, setOsBinaryCount] = useState(10);
  const [osCounterCount, setOsCounterCount] = useState(4);
  const [osAnalogValues, setOsAnalogValues] = useState<number[]>([]);
  const [osBinaryValues, setOsBinaryValues] = useState<boolean[]>([]);
  const [osCounterValues, setOsCounterValues] = useState<number[]>([]);
  const [osEvents, setOsEvents] = useState<OutstationEvent[]>([]);
  const [osPollMs, setOsPollMs] = useState(500);
  const unlistenRef = useRef<UnlistenFn | null>(null);

  // Simulation state
  const [activeTemplate, setActiveTemplate] = useState<DeviceTemplate | null>(null);
  const [simValues, setSimValues] = useState<number[]>([]);
  const [simRunning, setSimRunning] = useState(false);
  const [simSpeed, setSimSpeed] = useState(1000);
  const [customTemplates, setCustomTemplates] = useState<DeviceTemplate[]>(loadCustomTemplates());

  const addLog = useCallback((direction: LogEntry["direction"], message: string) => {
    setLog((prev) => [...prev, { timestamp: Date.now(), direction, message }]);
  }, []);

  // Drag handlers
  const dragStartRef = useRef({ startX: 0, startY: 0, startWidth: 0, startHeight: 0 });

  const handleDragStart = useCallback((side: "left" | "table", e: React.MouseEvent) => {
    e.preventDefault();
    dragStartRef.current = {
      startX: e.clientX, startY: e.clientY,
      startWidth: side === "left" ? leftWidth : 0,
      startHeight: side === "table" ? tableHeight : 0,
    };
    setDragSide(side);
  }, [leftWidth, tableHeight]);

  useEffect(() => {
    if (!dragSide) return;
    const onMove = (e: MouseEvent) => {
      if (dragSide === "left") {
        const dx = e.clientX - dragStartRef.current.startX;
        setLeftWidth(Math.max(180, Math.min(400, dragStartRef.current.startWidth + dx)));
      } else if (dragSide === "table") {
        const dy = e.clientY - dragStartRef.current.startY;
        setTableHeight(Math.max(80, dragStartRef.current.startHeight + dy));
      }
    };
    const onUp = () => setDragSide(null);
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    return () => { window.removeEventListener("mousemove", onMove); window.removeEventListener("mouseup", onUp); };
  }, [dragSide]);

  useEffect(() => {
    const onResize = () => setIsMobile(window.innerWidth < 768);
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  useEffect(() => {
    if (protocol.error) addLog("error", protocol.error);
  }, [protocol.error, addLog]);

  // Outstation log listener + point refresh
  const prevAnalogRef = useRef<number[]>([]);
  useEffect(() => {
    if (mode !== "outstation" || !osRunning) return;

    let unlisten: UnlistenFn | null = null;
    listen<OutstationLogEvent>("outstation-log", (event) => {
      addLog("rx", event.payload.message);
    }).then((fn) => { unlisten = fn; });

    const interval = setInterval(async () => {
      try {
        const [analogs, binaries, counters] = await invoke<[number[], boolean[], number[]]>("outstation_read_points");
        setOsAnalogValues(analogs);
        setOsBinaryValues(binaries);
        setOsCounterValues(counters);
        const events = await invoke<OutstationEvent[]>("outstation_read_events");
        setOsEvents(events);
      } catch { /* server may have stopped */ }
    }, osPollMs);

    return () => { unlisten?.(); clearInterval(interval); };
  }, [mode, osRunning, osPollMs, addLog]);

  // Poll simulation values from Rust
  useEffect(() => {
    if (!activeTemplate || !osRunning) return;
    const poll = setInterval(async () => {
      try {
        const vals = await invoke<number[]>("sim_read_values");
        if (vals.length > 0) setSimValues(vals);
      } catch { /* no profile loaded */ }
    }, 500);
    return () => clearInterval(poll);
  }, [activeTemplate, osRunning]);

  // Apply template to outstation
  const applyTemplate = useCallback(async (tmpl: DeviceTemplate) => {
    setActiveTemplate(tmpl);
    setSimRunning(false);

    // Ensure enough points
    const maxAnalogIdx = Math.max(...tmpl.points.filter(p => !p.isBool && p.group !== 20).map(p => p.index)) + 1;
    const maxBinaryIdx = Math.max(...tmpl.points.filter(p => p.isBool).map(p => p.index), 0) + 1;
    const maxCounterIdx = Math.max(...tmpl.points.filter(p => p.group === 20).map(p => p.index), 0) + 1;
    if (maxAnalogIdx > osAnalogCount) setOsAnalogCount(maxAnalogIdx);
    if (maxBinaryIdx > osBinaryCount) setOsBinaryCount(maxBinaryIdx);
    if (maxCounterIdx > osCounterCount) setOsCounterCount(maxCounterIdx);

    if (osRunning) {
      try {
        await invoke("sim_load_profile", { profile: tmpl });
        addLog("info", `Loaded template: ${tmpl.name}`);
      } catch (e) {
        addLog("error", `Load template failed: ${String(e)}`);
      }
    }
  }, [osRunning, osAnalogCount, osBinaryCount, osCounterCount, addLog]);

  // Toggle simulation
  const toggleSimulation = useCallback(async () => {
    if (!activeTemplate || !osRunning) return;
    if (simRunning) {
      try {
        await invoke("sim_stop");
        setSimRunning(false);
        addLog("info", "Simulation paused");
      } catch (e) {
        addLog("error", `Sim stop failed: ${String(e)}`);
      }
    } else {
      try {
        await invoke("sim_start", { profile: activeTemplate, intervalMs: simSpeed });
        setSimRunning(true);
        addLog("info", `Simulation started (${simSpeed}ms tick)`);
      } catch (e) {
        addLog("error", `Sim start failed: ${String(e)}`);
      }
    }
  }, [simRunning, activeTemplate, osRunning, simSpeed, addLog]);

  // ── Connection profile actions ──
  const buildCurrentProfile = useCallback((): ConnProfile => ({
    name: "",
    host: String(tcpFormValues.host ?? "192.168.1.100"),
    port: Number(tcpFormValues.port ?? 20000),
    masterAddress: Number(tcpFormValues.masterAddress ?? 1),
    outstationAddress: Number(tcpFormValues.outstationAddress ?? 10),
  }), [tcpFormValues]);

  const saveProfileAs = useCallback((name: string) => {
    if (!name.trim()) return;
    const profile = { ...buildCurrentProfile(), name: name.trim() };
    const isNew = !connProfiles.some((p) => p.name === profile.name);
    if (isNew && connProfiles.length >= FREE_PROFILE_LIMIT && !requirePro("More than one saved connection profile")) return;
    setConnProfiles((prev) => {
      const next = [...prev.filter((p) => p.name !== profile.name), profile];
      saveConnProfiles(next);
      return next;
    });
    setActiveConnProfile(profile.name);
    setProfileDirty(false);
  }, [buildCurrentProfile, connProfiles, requirePro]);

  const saveProfileChanges = useCallback(() => {
    if (!activeConnProfile) return;
    saveProfileAs(activeConnProfile);
  }, [activeConnProfile, saveProfileAs]);

  const loadProfile = useCallback((name: string) => {
    const p = connProfiles.find((c) => c.name === name);
    if (!p) return;
    const vals = { host: p.host, port: p.port, masterAddress: p.masterAddress, outstationAddress: p.outstationAddress, pollInterval: tcpFormValues.pollInterval ?? 1000 };
    setTcpFormValues(vals);
    setActiveConnProfile(p.name);
    setProfileDirty(false);
    setFormKey((k) => k + 1);
  }, [connProfiles]);

  const deleteProfile = useCallback((name: string) => {
    setConnProfiles((prev) => {
      const next = prev.filter((p) => p.name !== name);
      saveConnProfiles(next);
      return next;
    });
    if (activeConnProfile === name) { setActiveConnProfile(null); setProfileDirty(false); }
  }, [activeConnProfile]);

  // Detect dirty
  useEffect(() => {
    if (!activeConnProfile) { setProfileDirty(false); return; }
    const saved = connProfiles.find((p) => p.name === activeConnProfile);
    if (!saved) { setProfileDirty(false); return; }
    const current = buildCurrentProfile();
    setProfileDirty(
      saved.host !== current.host || saved.port !== current.port
      || saved.masterAddress !== current.masterAddress || saved.outstationAddress !== current.outstationAddress
    );
  }, [activeConnProfile, connProfiles, buildCurrentProfile, tcpFormValues]);

  const addPoint = useCallback(() => {
    const name = newPoint.name.trim();
    const index = Number(newPoint.index);
    if (!name || !Number.isInteger(index) || index < 0) return;
    if (points.length >= FREE_POINT_LIMIT && !requirePro("More than 10 points in the poll list")) return;
    const [group, variation] = newPoint.type.split(":").map(Number);
    setPoints((prev) => [...prev, { name, group, variation, index }]);
    setNewPoint((n) => ({ ...n, name: "", index: String(index + 1) }));
  }, [newPoint, points.length, requirePro]);

  const removePoint = useCallback((i: number) => {
    setPoints((prev) => prev.filter((_, j) => j !== i));
  }, []);

  // ── Handlers ──
  const handleConnect = useCallback(
    (values: Record<string, string | number>) => {
      const config: Dnp3Config = {
        id: String(values.host),
        host: String(values.host),
        port: Number(values.port),
        master_address: Number(values.masterAddress),
        outstation_address: Number(values.outstationAddress),
      };
      addLog("info", `Connecting to ${config.host}:${config.port}\u2026`);
      protocol.connect(config).then((ok) => {
        if (ok) {
          addLog("rx", "Connected");
          const interval = Number(values.pollInterval || 1000);
          protocol.startPolling(points, interval);
          addLog("info", `Polling ${points.length} points every ${interval}ms`);
        }
      });
    },
    [protocol, addLog, points],
  );

  const handleDisconnect = useCallback(() => {
    protocol.disconnect();
    protocol.stopPolling();
    addLog("info", "Disconnected");
  }, [protocol, addLog]);

  const handleEventScan = useCallback(async (classId: number) => {
    if (!protocol.connected) return;
    addLog("tx", `Class ${classId} event scan`);
    try {
      const vals = await invoke<unknown[]>("event_scan", { class: classId, tags: points });
      addLog("rx", `Class ${classId} scan returned ${vals.length} events`);
      protocol.readTags(points);
    } catch (e) {
      addLog("error", `Class ${classId} scan failed: ${String(e)}`);
    }
  }, [protocol, points, addLog]);

  const handleTimeSync = useCallback(async () => {
    if (!protocol.connected) return;
    addLog("tx", "Time sync (Group 50)");
    try {
      await invoke("time_sync");
      addLog("rx", "Time sync acknowledged");
    } catch (e) {
      addLog("error", `Time sync failed: ${String(e)}`);
    }
  }, [protocol.connected, addLog]);

  const handleRestart = useCallback(async (kind: "cold" | "warm") => {
    if (!protocol.connected) return;
    addLog("tx", `${kind === "cold" ? "Cold" : "Warm"} restart command`);
    try {
      await invoke(kind === "cold" ? "cold_restart" : "warm_restart");
      addLog("rx", `${kind === "cold" ? "Cold" : "Warm"} restart acknowledged`);
    } catch (e) {
      addLog("error", `${kind} restart failed: ${String(e)}`);
    }
  }, [protocol.connected, addLog]);

  const handleFreeze = useCallback(async () => {
    if (!protocol.connected) return;
    addLog("tx", "Counter freeze command");
    try {
      await invoke("freeze_counters");
      addLog("rx", "Freeze acknowledged");
    } catch (e) {
      addLog("error", `Freeze failed: ${String(e)}`);
    }
  }, [protocol.connected, addLog]);

  const handleOsStart = useCallback(async () => {
    try {
      await invoke("outstation_start", {
        config: {
          port: osPort,
          bind_addr: "0.0.0.0",
          master_address: osMasterAddr,
          outstation_address: osOutstationAddr,
          analog_count: osAnalogCount,
          binary_count: osBinaryCount,
          counter_count: osCounterCount,
        },
      });
      setOsRunning(true);
      addLog("info", `Outstation started on port ${osPort} (${osAnalogCount} analog, ${osBinaryCount} binary, ${osCounterCount} counter)`);
    } catch (e) {
      addLog("error", `Outstation start failed: ${String(e)}`);
    }
  }, [osPort, osMasterAddr, osOutstationAddr, osAnalogCount, osBinaryCount, osCounterCount, addLog]);

  const handleOsStop = useCallback(async () => {
    try {
      if (simRunning) { try { await invoke("sim_stop"); } catch { /* */ } setSimRunning(false); }
      await invoke("outstation_stop");
      setOsRunning(false);
      setOsAnalogValues([]);
      setOsBinaryValues([]);
      setOsCounterValues([]);
      setOsEvents([]);
      addLog("info", "Outstation stopped");
    } catch (e) {
      addLog("error", `Outstation stop failed: ${String(e)}`);
    }
  }, [simRunning, addLog]);

  // ── Table data ──
  const columns = [
    { key: "name", label: "Name", width: "120px" },
    { key: "group", label: "Group", width: "60px", align: "right" as const },
    { key: "variation", label: "Var", width: "60px", align: "right" as const },
    { key: "index", label: "Index", width: "60px", align: "right" as const },
    { key: "value", label: "Value", align: "right" as const },
    { key: "trend", label: "Trend", width: "56px", align: "center" as const },
  ];

  const masterTableData =
    mode === "outstation" && simRunning && activeTemplate
      ? activeTemplate.points.map((p) => ({
          name: p.label,
          group: p.group,
          variation: p.variation,
          index: p.index,
          value: p.isBool
            ? (simValues[activeTemplate.points.indexOf(p)] ?? p.base) >= 0.5 ? "Online" : "Offline"
            : (simValues[activeTemplate.points.indexOf(p)] ?? p.base).toFixed(p.decimals ?? 2) + (p.unit ? ` ${p.unit}` : ""),
        }))
      : protocol.values.length > 0
        ? protocol.values.map((v: any, i: number) => ({
            name: v?.point?.name ?? points[i]?.name ?? "",
            group: v?.point?.group ?? points[i]?.group ?? 0,
            variation: v?.point?.variation ?? points[i]?.variation ?? 0,
            index: v?.point?.index ?? points[i]?.index ?? 0,
            value: v?.value != null ? Number(v.value).toFixed(2) : "-",
          }))
        : points.map((p) => ({ name: p.name, group: p.group, variation: p.variation, index: p.index, value: "\u2014" }));

  const osColumns = [
    { key: "index", label: "Index", width: "60px", align: "right" as const },
    { key: "type", label: "Type", width: "80px" },
    { key: "value", label: "Value", align: "right" as const },
  ];

  const { trendSeries, setTrendSeries, trendWindow, setTrendWindow, toggleTrend } = useTrend(masterTableData);
  const osTableData = [
    ...osAnalogValues.map((v, i) => ({ index: i, type: "Analog", value: v.toFixed(3) })),
    ...osBinaryValues.map((v, i) => ({ index: i, type: "Binary", value: v ? "Online" : "Offline" })),
    ...osCounterValues.map((v, i) => ({ index: i, type: "Counter", value: v.toFixed(0) })),
  ];

  // ── Sidebar content ──
  const sidebarContent = (
    <>
      {/* Mode toggle */}
      <Section>
        <Segmented
          label="Mode"
          block
          value={mode}
          onChange={(v) => { if (v === "outstation" && !requirePro("Outstation simulator")) return; setMode(v); }}
          options={[
            { value: "master", label: "Master" },
            {
              value: "outstation",
              label: (
                <>
                  Outstation <ProBadge />
                  {osRunning && <StatusDot tone="success" label="Outstation running" />}
                </>
              ),
            },
          ]}
        />
      </Section>

      {mode === "master" ? (
        <>
          {/* Connection profiles */}
          <Section title="Profiles">
            <select
              className="vt-select"
              aria-label="Connection profile"
              value={activeConnProfile ?? ""}
              onChange={(e) => {
                if (e.target.value) loadProfile(e.target.value);
                else { setActiveConnProfile(null); setProfileDirty(false); }
              }}
              disabled={protocol.connected}
            >
              <option value="">New profile</option>
              {connProfiles.map((p) => <option key={p.name} value={p.name}>{p.name}</option>)}
            </select>

            {activeConnProfile && (
              <div style={{ display: "flex", alignItems: "center", gap: 6, padding: "4px 8px", borderRadius: "var(--radius-sm)", background: "var(--sunken)", border: "1px solid var(--border)" }}>
                <span style={{ flex: 1, fontSize: "var(--fs-xs)", fontWeight: 600, color: "var(--text)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                  {activeConnProfile}
                </span>
                <Badge tone={profileDirty ? "warn" : "success"}>{profileDirty ? "Modified" : "Saved"}</Badge>
              </div>
            )}

            {profileDirty && activeConnProfile ? (
              <div style={{ display: "flex", gap: 6 }}>
                <Button size="sm" block onClick={saveProfileChanges}>Save</Button>
                <Button size="sm" block icon={<RotateCcw size={14} strokeWidth={1.75} />} onClick={() => loadProfile(activeConnProfile)}>Revert</Button>
              </div>
            ) : activeConnProfile && !profileDirty ? (
              <div style={{ display: "flex", gap: 6 }}>
                <Button size="sm" block icon={<Plus size={14} strokeWidth={1.75} />} onClick={() => { setActiveConnProfile(null); }}>Save as new</Button>
                <Button size="sm" block variant="danger" icon={<Trash2 size={14} strokeWidth={1.75} />} onClick={() => deleteProfile(activeConnProfile)}>Delete</Button>
              </div>
            ) : null}
          </Section>

          <Section title="Connection">
            <ConnectionForm
              key={formKey}
              fields={[
                { name: "host", label: "IP address", type: "text", default: "192.168.1.100" },
                { name: "port", label: "Port", type: "number", default: 20000 },
                { name: "masterAddress", label: "Master address", type: "number", default: 1 },
                { name: "outstationAddress", label: "Outstation address", type: "number", default: 10 },
                { name: "pollInterval", label: "Poll interval (ms)", type: "number", default: 1000 },
              ]}
              initialValues={tcpFormValues}
              onSubmit={handleConnect}
              onChange={(vals) => setTcpFormValues(vals)}
              onDisconnect={handleDisconnect}
              connected={protocol.connected}
              loading={protocol.loading}
            />
          </Section>

          <Section
            title="Points"
            meta={pro ? <Badge tone="success">{points.length}</Badge> : <Badge tone={points.length >= FREE_POINT_LIMIT ? "warn" : "neutral"}>{points.length} of {FREE_POINT_LIMIT}</Badge>}
          >
            <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
              {points.map((p, i) => (
                <div key={`${p.name}-${i}`} style={{ display: "flex", alignItems: "center", gap: 6, fontSize: "var(--fs-xs)" }}>
                  <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", color: "var(--text)" }}>{p.name}</span>
                  <span className="mono" style={{ color: "var(--text-muted)" }}>g{p.group}v{p.variation} #{p.index}</span>
                  <IconButton size="sm" label={`Remove ${p.name}`} disabled={protocol.connected} onClick={() => removePoint(i)}>
                    <X size={14} strokeWidth={1.75} />
                  </IconButton>
                </div>
              ))}
              {points.length === 0 && <span style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)" }}>No points. Add one below.</span>}
            </div>
            <input className="vt-input" aria-label="Point name" placeholder="Point name" value={newPoint.name} disabled={protocol.connected}
              onChange={(e) => setNewPoint((n) => ({ ...n, name: e.target.value }))}
              onKeyDown={(e) => { if (e.key === "Enter") addPoint(); }} />
            <div style={{ display: "flex", gap: 6 }}>
              <select className="vt-select" aria-label="Point type" value={newPoint.type} disabled={protocol.connected}
                onChange={(e) => setNewPoint((n) => ({ ...n, type: e.target.value }))}>
                {POINT_TYPES.map((t) => <option key={t.value} value={t.value}>{t.label}</option>)}
              </select>
              <input className="vt-input vt-input--mono" style={{ width: 64, flexShrink: 0 }} type="number" min={0} aria-label="Point index" value={newPoint.index} disabled={protocol.connected}
                onChange={(e) => setNewPoint((n) => ({ ...n, index: e.target.value }))} />
            </div>
            <Button size="sm" block icon={<Plus size={14} strokeWidth={1.75} />} disabled={protocol.connected || !newPoint.name.trim()} onClick={addPoint}>Add point</Button>
            {protocol.connected && <span style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)" }}>Disconnect to edit the list.</span>}
          </Section>

          {protocol.connected && (
            <>
              <Section title="Class polling" meta={<ProBadge />}>
                <div style={{ display: "flex", gap: 6 }}>
                  {[1, 2, 3].map((c) => (
                    <Button key={c} size="sm" block onClick={() => requirePro("Class event scans") && handleEventScan(c)}>Class {c}</Button>
                  ))}
                </div>
              </Section>
              <Section title="Device controls" meta={<ProBadge />}>
                <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
                  <Button size="sm" block icon={<Clock size={14} strokeWidth={1.75} />} onClick={() => requirePro("Device controls") && handleTimeSync()}>Time sync</Button>
                  <div style={{ display: "flex", gap: 6 }}>
                    <Button size="sm" block icon={<RotateCcw size={14} strokeWidth={1.75} />} onClick={() => requirePro("Device controls") && handleRestart("cold")}>Cold restart</Button>
                    <Button size="sm" block icon={<RotateCcw size={14} strokeWidth={1.75} />} onClick={() => requirePro("Device controls") && handleRestart("warm")}>Warm restart</Button>
                  </div>
                  <Button size="sm" block icon={<Snowflake size={14} strokeWidth={1.75} />} onClick={() => requirePro("Device controls") && handleFreeze()}>Freeze counters</Button>
                </div>
              </Section>
            </>
          )}
        </>
      ) : (
        <>
          <Section title="Server">
            <Field label="TCP port">
              <input className="vt-input vt-input--mono" type="number" value={osPort} onChange={(e) => setOsPort(Number(e.target.value))} disabled={osRunning} />
            </Field>
            <div className="vt-field-row">
              <Field label="Master address">
                <input className="vt-input vt-input--mono" type="number" value={osMasterAddr} onChange={(e) => setOsMasterAddr(Number(e.target.value))} disabled={osRunning} />
              </Field>
              <Field label="Outstation address">
                <input className="vt-input vt-input--mono" type="number" value={osOutstationAddr} onChange={(e) => setOsOutstationAddr(Number(e.target.value))} disabled={osRunning} />
              </Field>
            </div>
            <div style={{ display: "grid", gridTemplateColumns: "repeat(3, minmax(0, 1fr))", gap: 8 }}>
              <Field label="Analog">
                <input className="vt-input vt-input--mono" type="number" value={osAnalogCount} onChange={(e) => setOsAnalogCount(Number(e.target.value))} disabled={osRunning} aria-label="Analog points" />
              </Field>
              <Field label="Binary">
                <input className="vt-input vt-input--mono" type="number" value={osBinaryCount} onChange={(e) => setOsBinaryCount(Number(e.target.value))} disabled={osRunning} aria-label="Binary points" />
              </Field>
              <Field label="Counters">
                <input className="vt-input vt-input--mono" type="number" value={osCounterCount} onChange={(e) => setOsCounterCount(Number(e.target.value))} disabled={osRunning} aria-label="Counter points" />
              </Field>
            </div>
            {!osRunning ? (
              <Button variant="primary" block icon={<Play size={14} strokeWidth={1.75} />} onClick={handleOsStart}>Start server</Button>
            ) : (
              <Button variant="danger" block icon={<Square size={14} strokeWidth={1.75} />} onClick={handleOsStop}>Stop server</Button>
            )}
            {osRunning && (
              <p style={{ margin: 0, fontSize: "var(--fs-xs)", color: "var(--text-muted)", lineHeight: 1.5 }}>
                {osAnalogValues.length + osBinaryValues.length + osCounterValues.length} points active. External masters can connect on port {osPort}.
              </p>
            )}
          </Section>
          {osRunning && (
            <Section title="Event buffer" meta="SOE">
              {osEvents.length === 0 ? (
                <EmptyState compact title="No events buffered" description="Events appear when a master reads data points or the simulation reports changes." />
              ) : (
                <div style={eventBoxStyle} role="log" aria-label="Sequence of events">
                  {osEvents.slice(-20).map((ev) => (
                    <div key={ev.sequence} style={{ padding: "1px 0", whiteSpace: "nowrap" }}>
                      <span style={{ color: "var(--accent-text)" }}>C{ev.class}</span>{" "}
                      <span style={{ color: "var(--text-muted)" }}>G{ev.group}V idx{ev.index}</span>{" "}
                      {ev.group === 1 ? (ev.value >= 0.5 ? "Online" : "Offline") : ev.value.toFixed(ev.group === 20 ? 0 : 3)}{" "}
                      <span style={{ color: "var(--text-muted)" }}>{new Date(ev.timestamp).toLocaleTimeString("en-US", { hour12: false })}</span>
                    </div>
                  ))}
                </div>
              )}
            </Section>
          )}
        </>
      )}
    </>
  );

  // ── Template/simulation section (sidebar in desktop, own mobile tab) ──
  const templateContent = (
    <Section title="Device template" meta={<ProBadge />}>
      <select
        className="vt-select"
        aria-label="Device template"
        value={activeTemplate?.id ?? ""}
        onChange={(e) => {
          const id = e.target.value;
          const all = [...BUILTIN_TEMPLATES, ...customTemplates];
          const t = all.find((t) => t.id === id);
          if (t && requirePro("Device templates and simulation")) applyTemplate(t);
        }}
      >
        <option value="">Select device</option>
        <optgroup label="Built-in">
          {BUILTIN_TEMPLATES.map((t) => <option key={t.id} value={t.id}>{t.name}</option>)}
        </optgroup>
        {customTemplates.length > 0 && (
          <optgroup label="Custom">
            {customTemplates.map((t) => <option key={t.id} value={t.id}>{t.name}</option>)}
          </optgroup>
        )}
      </select>

      {activeTemplate && (
        <p style={{ margin: 0, fontSize: "var(--fs-xs)", color: "var(--text-muted)", lineHeight: 1.5 }}>
          {activeTemplate.description}
          <br />{activeTemplate.points.length} data points
        </p>
      )}

      {activeTemplate && osRunning && (
        <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
          {simRunning ? (
            <Button size="sm" block icon={<Pause size={14} strokeWidth={1.75} />} onClick={toggleSimulation}>Pause</Button>
          ) : (
            <Button size="sm" block variant="primary" icon={<Play size={14} strokeWidth={1.75} />} onClick={toggleSimulation}>Simulate</Button>
          )}
          <select
            className="vt-select vt-select--sm"
            style={{ width: 72, flexShrink: 0 }}
            aria-label="Simulation tick interval"
            value={simSpeed}
            onChange={(e) => setSimSpeed(Number(e.target.value))}
          >
            <option value={500}>0.5s</option>
            <option value={1000}>1s</option>
            <option value={2000}>2s</option>
            <option value={5000}>5s</option>
          </select>
        </div>
      )}

      {activeTemplate && simValues.length > 0 && (
        <div>
          <div style={{ fontSize: "var(--fs-xs)", fontWeight: 600, color: "var(--text-dim)", marginBottom: 6 }}>Live values</div>
          {activeTemplate.points.map((p, i) => (
            <div key={i} style={liveValuesStyle}>
              <span style={{ color: "var(--text-dim)" }}>{p.label}</span>
              <span className="mono" style={{ color: "var(--text)" }}>
                {p.isBool
                  ? (simValues[i] ?? p.base) >= 0.5 ? "Online" : "Offline"
                  : (simValues[i] ?? p.base).toFixed(p.decimals ?? 2) + (p.unit ? ` ${p.unit}` : "")}
              </span>
            </div>
          ))}
        </div>
      )}
    </Section>
  );

  // ── Data table ──
  const monoCell = (v: unknown) => <span className="mono">{String(v ?? "")}</span>;
  const dataTable = mode === "master" ? (
    <DataTable
      columns={columns}
      data={masterTableData}
      onRowClick={(row) => {
        if (!requirePro("Controls (CROB and analog output)")) return;
        setEditingPoint(String(row.name));
        setEditingValue(row.value != null && String(row.value) !== "—" && String(row.value) !== "–" ? String(row.value) : "");
      }}
      renderCell={(key, row) => {
        if (key === "trend") {
          const name = String(row.name);
          const active = trendSeries.has(name);
          return (
            <IconButton
              size="sm"
              label={active ? `Remove ${name} from trend` : `Add ${name} to trend`}
              pressed={active}
              onClick={(e) => { e.stopPropagation(); if (!active && !requirePro("Live trends")) return; toggleTrend(name); }}
            >
              {active ? <X size={14} strokeWidth={1.75} /> : <Plus size={14} strokeWidth={1.75} />}
            </IconButton>
          );
        }
        if (key === "value") return monoCell(row[key]);
        return String(row[key] ?? "");
      }}
    />
  ) : osRunning ? (
    <DataTable
      columns={osColumns}
      data={osTableData}
      emptyMessage="Waiting for data points"
      renderCell={(key, row) => (key === "value" || key === "index" ? monoCell(row[key]) : String(row[key] ?? ""))}
    />
  ) : (
    <div style={{ flex: 1, display: "flex", alignItems: "center", justifyContent: "center" }}>
      <EmptyState
        icon={<Server size={20} strokeWidth={1.75} />}
        title="Outstation not running"
        description="Set the server options in the sidebar and start the outstation to view its points."
        action={(
          <Button variant="primary" icon={<Play size={14} strokeWidth={1.75} />} onClick={handleOsStart}>
            Start server
          </Button>
        )}
      />
    </div>
  );

  // ── Mobile tab bar ──
  const mobileTabBar = (
    <nav
      role="tablist"
      aria-label="Sections"
      style={{ display: "flex", background: "var(--surface)", borderTop: "1px solid var(--border)", flexShrink: 0, paddingBottom: "env(safe-area-inset-bottom, 0px)" }}
    >
      {([
        { id: "connect" as const, label: "Connect" },
        { id: "data" as const, label: "Data" },
        { id: "template" as const, label: "Template" },
        { id: "log" as const, label: "Log" },
        { id: "trend" as const, label: "Trend" },
      ]).map((tab) => (
        <button
          key={tab.id}
          role="tab"
          aria-selected={mobileTab === tab.id}
          onClick={() => setMobileTab(tab.id)}
          style={{
            flex: 1, padding: "10px 0", fontSize: "var(--fs-xs)", fontWeight: 500, cursor: "pointer",
            border: "none", borderBottom: mobileTab === tab.id ? "2px solid var(--accent)" : "2px solid transparent",
            color: mobileTab === tab.id ? "var(--text)" : "var(--text-muted)", background: "transparent",
            transition: "color 0.15s, border-color 0.15s",
            textAlign: "center", display: "flex", alignItems: "center", justifyContent: "center", gap: 4,
          }}
        >
          {tab.label}
          {mode === "outstation" && osRunning && tab.id === "connect" && (
            <StatusDot tone="success" label="Outstation running" />
          )}
        </button>
      ))}
    </nav>
  );

  // ── Drag handle component ──
  const dragHandle = (side: "left" | "table") => (
    <div
      onMouseDown={(e) => handleDragStart(side, e)}
      style={{
        width: side === "table" ? undefined : 4,
        height: side === "table" ? 4 : undefined,
        cursor: side === "table" ? "row-resize" : "col-resize",
        background: dragSide === side ? "var(--accent)" : "transparent",
        flexShrink: 0, zIndex: 10, transition: "background 0.15s",
      }}
      onMouseEnter={(e) => { if (!dragSide) (e.target as HTMLElement).style.background = "var(--border)"; }}
      onMouseLeave={(e) => { if (!dragSide) (e.target as HTMLElement).style.background = "transparent"; }}
    />
  );

  // ── Main render ──
  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100vh", background: "var(--bg)", color: "var(--text)" }}>
      <Toolbar
        title="DNP3 Explorer"
        icon={appIcon}
        connected={mode === "master" ? protocol.connected : osRunning}
        onDisconnect={mode === "master" ? handleDisconnect : handleOsStop}
        onInfo={() => setShowInfo(true)}
      />

      {isMobile ? (
        <div style={{ flex: 1, overflow: "hidden", display: "flex", flexDirection: "column" }}>
          <div style={{ flex: 1, overflow: "auto", padding: 16 }}>
            {mobileTab === "connect" && <div style={{ display: "flex", flexDirection: "column" }}>{sidebarContent}</div>}
            {mobileTab === "data" && dataTable}
            {mobileTab === "template" && templateContent}
            {mobileTab === "log" && <LogPanel entries={log} onExport={(f) => addLog("info", `Log exported to ${f}`)} onClear={() => setLog([])} />}
            {mobileTab === "trend" && <div style={{ display: "flex", flexDirection: "column", flex: 1, minHeight: 300 }}><TrendPanel series={Array.from(trendSeries.values())} onRemove={(name) => setTrendSeries((prev) => { const next = new Map(prev); next.delete(name); return next; })} windowSec={trendWindow} onWindowChange={setTrendWindow} theme="dark" /></div>}
          </div>
          {mobileTabBar}
          <StatusBar connected={mode === "master" ? protocol.connected : osRunning} latency={protocol.latency} pollCount={protocol.pollCount} error={protocol.error} />
        </div>
      ) : (
        <div style={{ display: "flex", flex: 1, overflow: "hidden" }}>
          <aside
            className="vt-sidebar"
            style={{ width: leftWidth, flexShrink: 0, borderRight: "1px solid var(--border)", cursor: dragSide ? "col-resize" : undefined }}
          >
            {sidebarContent}
            {templateContent}
          </aside>
          {dragHandle("left")}
          <main style={{ display: "flex", flex: 1, flexDirection: "column", overflow: "hidden", padding: 8, minWidth: 0 }}>
            <div style={{ height: tableHeight, minHeight: 80, overflow: "auto", flexShrink: 0, display: "flex", flexDirection: "column" }}>{dataTable}</div>
            {dragHandle("table")}
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
              <TrendPanel series={Array.from(trendSeries.values())} onRemove={(name) => setTrendSeries((prev) => { const next = new Map(prev); next.delete(name); return next; })} windowSec={trendWindow} onWindowChange={setTrendWindow} theme="dark" />
            )}
          </main>
        </div>
      )}

      {!isMobile && <StatusBar connected={mode === "master" ? protocol.connected : osRunning} latency={protocol.latency} pollCount={protocol.pollCount} error={protocol.error} />}

      {editingPoint && (() => {
        const matchPt = points.find((p) => p.name === editingPoint);
        return (
          <ValueEditor
            address={editingPoint}
            currentValue={editingValue || undefined}
            onSubmit={async (value, dataType) => {
              addLog("tx", `Write ${value} (${dataType}) to ${editingPoint}`);
              if (matchPt) {
                try {
                  const numeric = Number(value);
                  const isBinary = matchPt.group === 1 || matchPt.group === 10 || matchPt.group === 12 || dataType === "bool";
                  if (isBinary) {
                    await protocol.writeTag(matchPt, { point: matchPt, value: numeric });
                  } else {
                    await invoke("write_analog", { index: matchPt.index, value: numeric });
                  }
                  addLog("rx", `Write confirmed for ${editingPoint}`);
                } catch (e) {
                  addLog("error", `Write failed: ${String(e)}`);
                }
              }
              setEditingPoint(null);
            }}
            onCancel={() => setEditingPoint(null)}
          />
        );
      })()}

      <Dialog open={showInfo} onClose={() => setShowInfo(false)} title="About DNP3 Explorer" width={380} footer={(
        <>
          {/* No outbound source link in App Store builds: review treats links to builds that unlock paid
              features elsewhere as steering away from in-app purchase. */}
          {proStatus && !proStatus.store && (
            <Button variant="ghost" onClick={() => invoke("open_url", { url: "https://github.com/VoltrusID/voltrus-explorers" })}>
              Source on GitHub
            </Button>
          )}
          <Button block={!proStatus || proStatus.store} onClick={() => setShowInfo(false)}>Close</Button>
        </>
      )}>
        <div style={{ fontSize: "var(--fs-sm)", color: "var(--text-dim)", lineHeight: 1.6 }}>
          <p style={{ margin: "0 0 12px" }}>Connect to DNP3 outstations in SCADA networks.</p>
          <p style={{ margin: "0 0 4px", fontWeight: 600, color: "var(--text)" }}>Master mode</p>
          <p style={{ margin: "0 0 12px" }}>Connect to a DNP3 outstation to monitor an editable list of points. Pro: click rows to operate controls (CROB and analog output).</p>
          <p style={{ margin: "0 0 4px", fontWeight: 600, color: "var(--text)" }}>Outstation mode</p>
          <p style={{ margin: "0 0 12px" }}>Run a DNP3 outstation server to simulate a device. Load device templates, auto-simulate live data.</p>
          <div style={{ borderTop: "1px solid var(--border)", paddingTop: 12, marginBottom: 12 }}>
            <ProAboutRow />
          </div>
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
