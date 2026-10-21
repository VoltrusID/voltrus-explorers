import { useState, useCallback, useEffect, useRef, useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  ArrowLeftRight,
  Check,
  Cpu,
  FileText,
  FolderTree,
  Network,
  Play,
  Plug,
  Radar,
  Square,
  TrendingUp,
} from "lucide-react";
import {
  ConnectionForm,
  DataTable,
  StatusBar,
  Toolbar,
  LogPanel,
  ValueEditor,
  useProtocol,
  Badge,
  Button,
  Dialog,
  EmptyState,
  Field,
  IconButton,
  Panel,
  Section,
  Segmented,
  Tabs,
  ProAboutRow,
  ProBadge,
  useProStatus,
} from "@mactools/ui";
import type { LogEntry } from "@mactools/ui";
import appIcon from "../src-tauri/icons/128x128.png";
import { DiscoveryPanel } from "./DiscoveryPanel";
import { IdentityPanel } from "./IdentityPanel";
import { CipBrowser } from "./CipBrowser";
import { CipTree } from "./CipTree";
import { PacketLog } from "./PacketLog";
import { EdsPanel } from "./EdsPanel";
import { IoPump } from "./IoPump";
import TrendPanel, { type TrendSeries } from "./TrendPanel";
import { buildEdsMap, loadEds, type Eds } from "./eds";

function cipTypeOf(v: any): string {
  if (v && typeof v === "object") {
    for (const k of ["Bool", "Sint", "Int", "Dint", "Real", "String", "Bytes"]) if (k in v) return k.toLowerCase();
  }
  return "—";
}

function formatCipValue(v: any, hex = false): string {
  if (v == null) return "—";
  if (typeof v === "object") {
    if ("Bool" in v) return v.Bool ? "On" : "Off";
    if ("String" in v) return String(v.String);
    if ("Real" in v) {
      const n = Number(v.Real);
      if (!Number.isFinite(n)) return "—";
      return Number.isInteger(n) ? String(n) : String(Number(n.toFixed(4)));
    }
    if ("Dint" in v) return hex ? "0x" + (v.Dint >>> 0).toString(16).toUpperCase() : String(v.Dint);
    if ("Int" in v) return hex ? "0x" + (v.Int & 0xFFFF).toString(16).toUpperCase() : String(v.Int);
    if ("Sint" in v) return hex ? "0x" + (v.Sint & 0xFF).toString(16).toUpperCase() : String(v.Sint);
    if ("Bytes" in v) return (v.Bytes as number[]).map((b) => b.toString(16).padStart(2, "0")).join(" ");
  }
  return String(v);
}

/** Extract a numeric value from a serialized CipValue ({Real:..},{Dint:..},…). */
function cipToNum(v: any): number {
  if (v == null || typeof v !== "object") return NaN;
  if ("Real" in v) return Number(v.Real);
  if ("Dint" in v) return v.Dint;
  if ("Int" in v) return v.Int;
  if ("Sint" in v) return v.Sint;
  if ("Bool" in v) return v.Bool ? 1 : 0;
  return NaN;
}

const CIP_EDITOR_TYPES = [
  { value: "bool", label: "BOOL" },
  { value: "sint", label: "SINT" },
  { value: "int", label: "INT" },
  { value: "dint", label: "DINT" },
  { value: "real", label: "REAL" },
  { value: "string", label: "STRING" },
];

/** Map any CIP type to one of the editor's supported CIP types. */
function cipTypeForEditor(t: string): string {
  const d = t.toLowerCase();
  if (["bool", "sint", "int", "dint", "real", "string"].includes(d)) return d;
  if (d === "lreal") return "real";
  if (d === "lint") return "dint";
  if (d === "usint") return "sint";
  if (d === "uint" || d === "udint" || d === "ulint" || d === "dword") return "dint";
  return "dint";
}

/** Build a { dataType, CipValue } from a CIP type name + raw string value. */
function editorToCip(dataType: string, value: string): { dataType: string; value: any } {
  const dt = dataType.toLowerCase();
  if (dt === "bool") return { dataType: "bool", value: { Bool: value === "true" || value === "1" } };
  if (dt === "string") return { dataType: "string", value: { String: value } };
  if (dt === "sint") return { dataType: "sint", value: { Sint: Math.round(Number(value)) } };
  if (dt === "int") return { dataType: "int", value: { Int: Math.round(Number(value)) } };
  if (dt === "real") return { dataType: "real", value: { Real: Number(value) } };
  return { dataType: "dint", value: { Dint: Math.round(Number(value)) } };
}

interface EnipConfig { host: string; port: number; }
interface CipTag { name: string; data_type: string; element_count: number; }
interface ServerLogEvent { timestamp: number; service: string; tag_name: string; message: string; }

interface SimPoint {
  key: string;
  name: string;
  data_type: string;
  element_count: number;
  base: number;
  variance: number;
  noise: number;
  period: number;
  min?: number;
  max?: number;
  decimals?: number;
  unit?: string;
  isBool?: boolean;
  writable?: boolean;
  source?: "wave" | "drive";
}

interface UdtMember {
  name: string;
  data_type: string;
  value: number;
}

interface UdtDef {
  tag: string;
  template_id: number;
  template_name: string;
  members: UdtMember[];
}

interface DeviceTemplate {
  name: string;
  description: string;
  points: SimPoint[];
  udts?: UdtDef[];
}

const TEMPLATES: Record<string, DeviceTemplate> = {
  motor: {
    name: "MotorDrive-1 (ABB ACS880)",
    description: "ABB ACS880 VFD with speed, current, torque, temperature",
    points: [
      { key: "speed", name: "Program:Main.Motor1.Speed", data_type: "Real", element_count: 1, base: 1450, variance: 12, noise: 3, period: 15000, max: 1800, decimals: 1, unit: "RPM", source: "drive" },
      { key: "current", name: "Program:Main.Motor1.Current", data_type: "Real", element_count: 1, base: 12.4, variance: 0.8, noise: 0.2, period: 12000, min: 8, max: 18, decimals: 1, unit: "A" },
      { key: "torque", name: "Program:Main.Motor1.Torque", data_type: "Real", element_count: 1, base: 45.8, variance: 2.5, noise: 0.8, period: 14000, min: 30, max: 60, decimals: 1, unit: "Nm" },
      { key: "temp", name: "Program:Main.Motor1.Temp", data_type: "Real", element_count: 1, base: 67.2, variance: 1.5, noise: 0.3, period: 60000, min: 50, max: 90, decimals: 1, unit: "\u00b0C" },
      { key: "power", name: "Program:Main.Motor1.Power", data_type: "Real", element_count: 1, base: 7.5, variance: 0.4, noise: 0.1, period: 13000, min: 5, max: 10, decimals: 2, unit: "kW" },
      { key: "runtime", name: "Program:Main.Motor1.Runtime", data_type: "Dint", element_count: 1, base: 15234, variance: 0, noise: 0, period: 3600000, decimals: 0, unit: "h" },
      { key: "freq", name: "Program:Main.Motor1.Frequency", data_type: "Real", element_count: 1, base: 50.0, variance: 0.02, noise: 0.01, period: 10000, max: 63.0, decimals: 2, unit: "Hz", source: "drive" },
      { key: "running", name: "Program:Main.Motor1.Status.Running", data_type: "Bool", element_count: 1, base: 1, variance: 0, noise: 0, period: 100000, isBool: true },
      { key: "faulted", name: "Program:Main.Motor1.Status.Faulted", data_type: "Bool", element_count: 1, base: 0, variance: 0, noise: 0, period: 100000, isBool: true },
      { key: "overloaded", name: "Program:Main.Motor1.Status.Overloaded", data_type: "Bool", element_count: 1, base: 0, variance: 0, noise: 0, period: 100000, isBool: true },
      { key: "mode", name: "Program:Main.Motor1.Mode", data_type: "Dint", element_count: 1, base: 2, variance: 0, noise: 0, period: 100000, writable: true },
      { key: "speedRef", name: "Program:Main.Motor1.SpeedReference", data_type: "Real", element_count: 1, base: 1450, variance: 0, noise: 0, period: 100000, min: 0, max: 1800, decimals: 0, unit: "RPM", writable: true },
    ],
    udts: [
      {
        tag: "Program:Main.Motor1.DriveInfo",
        template_id: 1,
        template_name: "DriveInfo",
        members: [
          { name: "RatedSpeed", data_type: "Real", value: 1450 },
          { name: "RatedCurrent", data_type: "Real", value: 12.4 },
          { name: "RatedPower", data_type: "Real", value: 7.5 },
          { name: "Poles", data_type: "Dint", value: 4 },
        ],
      },
    ],
  },
  tank: {
    name: "TankFarm-1 (Storage Tank)",
    description: "Storage tank with level, temperature, pressure, pump, valves",
    points: [
      { key: "level", name: "Program:Main.Tank[1].Level", data_type: "Real", element_count: 1, base: 72.5, variance: 1.2, noise: 0.3, period: 45000, min: 10, max: 95, decimals: 1, unit: "%" },
      { key: "temp", name: "Program:Main.Tank[1].Temperature", data_type: "Real", element_count: 1, base: 23.4, variance: 0.3, noise: 0.1, period: 30000, min: 15, max: 35, decimals: 1, unit: "\u00b0C" },
      { key: "pressure", name: "Program:Main.Tank[1].Pressure", data_type: "Real", element_count: 1, base: 1.21, variance: 0.03, noise: 0.01, period: 20000, min: 0.8, max: 2.0, decimals: 2, unit: "bar" },
      { key: "flow", name: "Program:Main.Tank[1].Pump.FlowRate", data_type: "Real", element_count: 1, base: 45.2, variance: 1.5, noise: 0.5, period: 18000, min: 30, max: 60, decimals: 1, unit: "m\u00b3/h" },
      { key: "inlet", writable: true, name: "Program:Main.Tank[1].InletValve", data_type: "Bool", element_count: 1, base: 1, variance: 0, noise: 0, period: 100000, isBool: true },
      { key: "outlet", writable: true, name: "Program:Main.Tank[1].OutletValve", data_type: "Bool", element_count: 1, base: 0, variance: 0, noise: 0, period: 100000, isBool: true },
      { key: "pump", writable: true, name: "Program:Main.Tank[1].Pump.Status", data_type: "Bool", element_count: 1, base: 1, variance: 0, noise: 0, period: 100000, isBool: true },
      { key: "alarm", name: "Program:Main.Tank[1].Alarm.HighLevel", data_type: "Bool", element_count: 1, base: 0, variance: 0, noise: 0, period: 100000, isBool: true },
    ],
  },
};

function App() {
  const protocol = useProtocol<EnipConfig, CipTag, unknown>("enip");
  const { requirePro, status: proStatus } = useProStatus();
  const [mode, setMode] = useState<"scanner" | "adapter">("scanner");
  const [leftWidth, setLeftWidth] = useState(260);
  const [rightWidth, setRightWidth] = useState(220);
  const [tableHeight, setTableHeight] = useState(300);
  const [dragSide, setDragSide] = useState<"left" | "right" | "table" | null>(null);
  const [mobileTab, setMobileTab] = useState<"connect" | "data" | "log">("connect");
  const [isMobile, setIsMobile] = useState(window.innerWidth < 768);
  const [tags, setTags] = useState<CipTag[]>([]);
  const [log, setLog] = useState<LogEntry[]>([]);
  const [editingTag, setEditingTag] = useState<{ name: string; currentValue: string; type?: string } | null>(null);
  const [showInfo, setShowInfo] = useState(false);
  const [showDiscovery, setShowDiscovery] = useState(false);
  const [showIdentity, setShowIdentity] = useState(false);
  const [showCipBrowser, setShowCipBrowser] = useState(false);
  const [showPacketLog, setShowPacketLog] = useState(false);
  const [showEds, setShowEds] = useState(false);
  const [showIo, setShowIo] = useState(false);
  const [eds, setEds] = useState<Eds | null>(() => loadEds());
  const edsMap = useMemo(() => buildEdsMap(eds), [eds]);
  const [trendSeries, setTrendSeries] = useState<Map<string, TrendSeries>>(new Map());
  const [trendWindow, setTrendWindow] = useState(60);
  const [view, setView] = useState<"log" | "trend">("log");
  const [mainView, setMainView] = useState<"table" | "objects">("table");
  const trendSeriesRef = useRef(trendSeries);
  trendSeriesRef.current = trendSeries;
  const [hexMode, setHexMode] = useState(false);
  const [pollMs, setPollMs] = useState(1000);
  const [simTemplate, setSimTemplate] = useState<string | null>(null);

  // Adapter state
  const [adapterRunning, setAdapterRunning] = useState(false);
  const [adapterPort, setAdapterPort] = useState(44818);
  const [adapterTags, setAdapterTags] = useState<Array<[string, string, any]>>([]);
  const [adapterPollMs, setAdapterPollMs] = useState(1000);
  const unlistenRef = useRef<UnlistenFn | null>(null);

  const addLog = useCallback((direction: LogEntry["direction"], message: string) => {
    setLog((prev) => [...prev, { timestamp: Date.now(), direction, message }]);
  }, []);

  useEffect(() => {
    if (protocol.error) addLog("error", protocol.error);
  }, [protocol.error, addLog]);

  useEffect(() => {
    if (protocol.connected && tags.length === 0) {
      addLog("info", "Browsing tags…");
      invoke<CipTag[]>("browse_tags").then((b) => {
        if (b.length > 0) {
          setTags(b);
          addLog("info", `Browsed ${b.length} tags from PLC`);
          protocol.startPolling(b, pollMs);
          addLog("info", `Polling ${b.length} tags every ${pollMs}ms`);
        } else {
          addLog("info", "No tags found via Symbol class. Device may not be a Logix PLC. Try the CIP Object Browser.");
        }
      }).catch((e) => addLog("error", `Browse failed: ${String(e)}`));
    }
  }, [protocol.connected, tags.length, addLog, pollMs]);

  // Restart polling when the rate changes while connected.
  useEffect(() => {
    if (protocol.connected && tags.length > 0) {
      protocol.startPolling(tags, pollMs);
      addLog("info", `Poll rate set to ${pollMs}ms`);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pollMs]);

  const dragStartRef = useRef({ startX: 0, startY: 0, startWidth: 0, startHeight: 0 });
  const handleDragStart = useCallback((side: "left" | "right" | "table", e: React.MouseEvent) => {
    e.preventDefault();
    dragStartRef.current = { startX: e.clientX, startY: e.clientY, startWidth: side === "left" ? leftWidth : side === "right" ? rightWidth : 0, startHeight: side === "table" ? tableHeight : 0 };
    setDragSide(side);
  }, [leftWidth, rightWidth, tableHeight]);

  useEffect(() => {
    if (!dragSide) return;
    const onMove = (e: MouseEvent) => {
      if (dragSide === "left") { setLeftWidth(Math.max(180, Math.min(400, dragStartRef.current.startWidth + e.clientX - dragStartRef.current.startX))); }
      else if (dragSide === "right") { setRightWidth(Math.max(160, Math.min(400, dragStartRef.current.startWidth - (e.clientX - dragStartRef.current.startX)))); }
      else if (dragSide === "table") { setTableHeight(Math.max(80, dragStartRef.current.startHeight + e.clientY - dragStartRef.current.startY)); }
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


  // Adapter log listener + tag refresh
  useEffect(() => {
    if (mode !== "adapter" || !adapterRunning) return;

    let unlisten: UnlistenFn | null = null;
    listen<ServerLogEvent>("server-log", (event) => {
      const e = event.payload;
      addLog(e.service === "read" ? "rx" : e.service === "write" ? "tx" : "info", e.message);
    }).then((fn) => { unlisten = fn; });

    const interval = setInterval(async () => {
      try { setAdapterTags(await invoke<Array<[string, string, any]>>("server_read_tags")); } catch { /* */ }
    }, adapterPollMs);

    return () => { unlisten?.(); clearInterval(interval); };
  }, [mode, adapterRunning, adapterPollMs, addLog]);

  const handleConnect = useCallback(async (values: Record<string, string | number>) => {
    const config: EnipConfig = { host: String(values.host), port: Number(values.port) };
    addLog("info", `Connecting to ${config.host}:${config.port}\u2026`);
    const ok = await protocol.connect(config);
    if (ok) addLog("rx", "Connected");
  }, [protocol, addLog]);

  const handleDisconnect = useCallback(() => {
    protocol.disconnect();
    protocol.stopPolling();
    setTags([]);
    addLog("info", "Disconnected");
  }, [protocol, addLog]);

  const handleCancel = useCallback(() => { protocol.cancel(); addLog("info", "Connection cancelled"); }, [protocol, addLog]);

  const handleConnectFromDevice = useCallback(async (host: string, port: number) => {
    setShowDiscovery(false);
    const config: EnipConfig = { host, port };
    addLog("info", `Connecting to ${host}:${port}…`);
    const ok = await protocol.connect(config);
    if (ok) addLog("rx", "Connected");
  }, [protocol, addLog]);

  const handleBrowseTags = useCallback(async () => {
    try {
      addLog("info", "Browsing tags\u2026");
      const browsed = await invoke<CipTag[]>("browse_tags");
      setTags(browsed);
      addLog("info", `Found ${browsed.length} tags`);
    } catch (e) { addLog("error", `Browse failed: ${String(e)}`); }
  }, [addLog]);

  const handleAdapterStart = useCallback(async () => {
    if (!requirePro("PLC simulator")) return;
    try {
      const points = simTemplate && TEMPLATES[simTemplate]
        ? TEMPLATES[simTemplate].points.map((pt) => ({
            key: pt.key,
            name: pt.name,
            data_type: pt.data_type.toLowerCase(),
            element_count: pt.element_count,
            base: pt.base,
            variance: pt.variance,
            noise: pt.noise,
            period_ms: pt.period,
            min: pt.min,
            max: pt.max,
            decimals: pt.decimals,
            unit: pt.unit,
            writable: pt.writable ?? false,
            source: pt.source ?? "wave",
            is_bool: pt.isBool ?? false,
          }))
        : [
            { key: "tag1", name: "Program:Main.Tag1", data_type: "dint", element_count: 1, base: 0, variance: 0, noise: 0, period_ms: 10000, writable: false, is_bool: false },
            { key: "tag2", name: "Program:Main.Tag2", data_type: "real", element_count: 1, base: 0, variance: 0, noise: 0, period_ms: 10000, writable: true, is_bool: false },
            { key: "tag3", name: "Program:Main.Tag3", data_type: "bool", element_count: 1, base: 0, variance: 0, noise: 0, period_ms: 10000, writable: false, is_bool: true },
          ];

      const udts = (simTemplate && TEMPLATES[simTemplate]?.udts) ?? [];

      await invoke("sim_start", { points, udts, port: adapterPort });
      setAdapterRunning(true);
      addLog("info", `Simulator started on port ${adapterPort} (${points.length} tags)`);
    } catch (e) { addLog("error", `Simulator start failed: ${String(e)}`); }
  }, [adapterPort, simTemplate, addLog, requirePro]);

  const handleAdapterStop = useCallback(async () => {
    try {
      await invoke("sim_stop");
      setAdapterRunning(false);
      setAdapterTags([]);
      addLog("info", "Simulator stopped");
    } catch (e) { addLog("error", `Simulator stop failed: ${String(e)}`); }
  }, [addLog]);

  const columns = [
    { key: "name", label: "Tag Name", width: "200px" },
    { key: "type", label: "CIP Type", width: "100px" },
    { key: "elements", label: "Elements", width: "80px" },
    { key: "value", label: "Value" },
    { key: "trend", label: "", width: "54px" },
  ];

  const scannerTableData = !protocol.connected
    ? []
    : protocol.values.length > 0
      ? protocol.values.map((v: any, i: number) => ({ name: tags[i]?.name ?? `tag[${i}]`, type: tags[i]?.data_type ?? cipTypeOf(v), elements: tags[i]?.element_count ?? 1, value: formatCipValue(v, hexMode) }))
      : tags.length > 0 ? tags.map((t) => ({ name: t.name, type: t.data_type, elements: t.element_count, value: "—" })) : [];
  // Numeric tag values for the trend (PLC poll).
  const trendRows = useMemo(() => {
    if (protocol.connected && protocol.values.length > 0) {
      return tags.map((t, i) => ({ name: t.name, v: cipToNum(protocol.values[i]) }));
    }
    return [];
  }, [protocol.connected, protocol.values, tags]);

  const trendRowsRef = useRef(trendRows);
  trendRowsRef.current = trendRows;
  // Signature of current values; stable across renders unless a value changed
  // (avoids the infinite effect→setState→render loop the raw array would cause).
  const valueSig = trendRows.map((r) => r.name + " " + r.v).join("");

  // Append a sample to every tracked series on each new poll / value change.
  useEffect(() => {
    const active = trendSeriesRef.current;
    if (active.size === 0) return;
    const ts = Date.now();
    const byName = new Map(trendRowsRef.current.map((r) => [r.name, r.v]));
    const cutoff = ts - trendWindow * 1000 - 5000;
    const next = new Map(active);
    let changed = false;
    for (const [name, s] of next) {
      const v = byName.get(name);
      if (v === undefined || isNaN(v)) continue;
      const points = [...s.points, { t: ts, v }].filter((p) => p.t > cutoff).slice(-500);
      next.set(name, { ...s, points });
      changed = true;
    }
    if (changed) setTrendSeries(next);
  }, [valueSig, trendWindow]);

  // Adding a series is Pro; removing one never is.
  const toggleTrend = useCallback((name: string) => {
    if (!trendSeriesRef.current.has(name) && !requirePro("Trends")) return;
    setTrendSeries((prev) => {
      const next = new Map(prev);
      if (next.has(name)) {
        next.delete(name);
      } else {
        const v = trendRowsRef.current.find((r) => r.name === name)?.v ?? 0;
        const colors = ["#3b82f6", "#ef4444", "#22c55e", "#f59e0b", "#a855f7", "#ec4899", "#14b8a6", "#f97316"];
        next.set(name, { name, color: colors[next.size % colors.length], points: [{ t: Date.now(), v: isNaN(v) ? 0 : v }] });
      }
      return next;
    });
  }, [requirePro]);

  const adapterColumns = [
    { key: "name", label: "Tag Name", width: "200px" },
    { key: "type", label: "CIP Type", width: "80px" },
    { key: "value", label: "Value" },
  ];
  const adapterTableData = adapterTags.map(([name, type, val]) => ({
    name,
    type,
    value: formatCipValue(val, hexMode),
  }));

  // ── Sidebar content ──

  const sidebarContent = (
    <>
      {/* Mode toggle */}
      <Section>
        <Segmented
          block
          label="Mode"
          value={mode}
          onChange={setMode}
          options={[
            { value: "scanner", label: "Scanner" },
            { value: "adapter", label: <>Adapter <ProBadge /></>, title: adapterRunning ? "Simulator running" : undefined },
          ]}
        />
      </Section>

      {mode === "scanner" ? (
        <>
          <Section title="Connection">
            <ConnectionForm
              fields={[
                { name: "host", label: "IP Address", type: "text", default: "192.168.1.100" },
                { name: "port", label: "Port", type: "number", default: 44818 },
              ]}
              onSubmit={handleConnect}
              onDisconnect={handleDisconnect}
              onCancel={handleCancel}
              connected={protocol.connected}
              loading={protocol.loading}
            />
          </Section>

          <Section title="Tools">
            <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 6 }}>
              <Button variant="ghost" icon={<Radar size={14} />} onClick={() => setShowDiscovery(true)}>Discover</Button>
              <Button
                variant="ghost"
                icon={eds ? <Check size={14} /> : <FileText size={14} />}
                title={eds ? `${eds.fileName} loaded` : undefined}
                onClick={() => setShowEds(true)}
              >
                EDS
              </Button>
              {protocol.connected && (
                <>
                  <Button variant="ghost" icon={<Cpu size={14} />} onClick={() => setShowIdentity(true)}>Identity</Button>
                  <Button variant="ghost" icon={<FolderTree size={14} />} onClick={() => setShowCipBrowser(true)}>CIP Object</Button>
                  <Button variant="ghost" icon={<Network size={14} />} onClick={() => setShowPacketLog(true)}>Packets</Button>
                  <Button variant="ghost" icon={<ArrowLeftRight size={14} />} onClick={() => { if (requirePro("Implicit I/O")) setShowIo(true); }}>IO <ProBadge /></Button>
                </>
              )}
            </div>
          </Section>

          {protocol.connected && (
            <Section>
              <Field label="Poll interval (ms)">
                <input
                  className="vt-input"
                  type="number"
                  min={50}
                  step={50}
                  value={pollMs}
                  onChange={(e) => setPollMs(Math.max(50, Number(e.target.value)))}
                />
              </Field>
            </Section>
          )}

          <Section
            title="Tags"
            meta={tags.length}
            actions={protocol.connected ? <Button variant="ghost" size="sm" onClick={handleBrowseTags}>Browse</Button> : undefined}
          >
            {tags.length > 0 ? (
              <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
                {tags.map((t) => (
                  <div key={t.name} style={{ display: "flex", justifyContent: "space-between", alignItems: "center", padding: "4px 0", borderBottom: "1px solid var(--border)" }}>
                    <span className="mono" style={{ fontSize: 12 }}>{t.name}</span>
                    <Badge>{t.data_type}</Badge>
                  </div>
                ))}
              </div>
            ) : (
              <EmptyState
                compact
                title={protocol.connected ? "No tags yet" : "Not connected"}
                description={protocol.connected ? "Click Browse to list tags from the PLC." : "Connect to a PLC to browse its tags."}
              />
            )}
          </Section>

        </>
      ) : (
        <>
          <Section title="Simulator">
            <Field label="TCP port">
              <input className="vt-input" type="number" value={adapterPort} onChange={(e) => setAdapterPort(Number(e.target.value))} disabled={adapterRunning} />
            </Field>
            <Field label="Refresh interval (ms)">
              <input className="vt-input" type="number" value={adapterPollMs} onChange={(e) => setAdapterPollMs(Number(e.target.value))} min={100} step={100} />
            </Field>
          </Section>

          <Section title="Template">
            <select
              className="vt-select"
              value={simTemplate ?? ""}
              onChange={(e) => setSimTemplate(e.target.value || null)}
              disabled={adapterRunning}
            >
              <option value="">Default tags</option>
              {Object.entries(TEMPLATES).map(([k, t]) => (<option key={k} value={k}>{t.name}</option>))}
            </select>
            {simTemplate && TEMPLATES[simTemplate] && (
              <div style={{ fontSize: 11, color: "var(--text-muted)", lineHeight: 1.5 }}>
                {TEMPLATES[simTemplate].description}<br />{TEMPLATES[simTemplate].points.length} tags
              </div>
            )}
          </Section>

          <Section>
            {!adapterRunning ? (
              <Button variant="primary" block icon={<Play size={14} />} onClick={handleAdapterStart}>Start simulator</Button>
            ) : (
              <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
                <Button variant="danger" block icon={<Square size={14} />} onClick={handleAdapterStop}>Stop simulator</Button>
                <Button block onClick={() => invoke("open_device_panel").catch(() => {})}>Open control panel</Button>
              </div>
            )}
          </Section>

          {adapterRunning && (
            <Section title="Tags" meta={adapterTags.length}>
              <div style={{ fontSize: 12, color: "var(--text-dim)", lineHeight: 1.5 }}>
                External clients can connect on port {adapterPort} to read and write tags.
              </div>
            </Section>
          )}
        </>
      )}
    </>
  );


  // ── Data table ──
  const dataTable = mode === "scanner" ? (
    !protocol.connected ? (
      <EmptyState
        icon={<Plug size={20} />}
        title="Not connected"
        description="Connect to a PLC or start the simulator to browse tags."
      />
    ) : (
      <DataTable
        columns={columns}
        data={scannerTableData}
        emptyMessage="No tags found on this device"
        onRowClick={(row) => { if (!requirePro("Tag writes")) return; const t = String(row.type); setEditingTag({ name: String(row.name), type: t, currentValue: t.toLowerCase() === "bool" ? (/^(on|true|1)$/i.test(String(row.value)) ? "true" : "false") : String(row.value) }); }}
        renderCell={(key: string, row: Record<string, unknown>) => {
          if (key === "trend") {
            const name = String(row.name);
            const active = trendSeries.has(name);
            return (
              <IconButton
                size="sm"
                label={active ? `Remove ${name} from trend` : `Add ${name} to trend`}
                pressed={active}
                onClick={(e) => { e.stopPropagation(); toggleTrend(name); }}
              >
                <TrendingUp size={12} />
              </IconButton>
            );
          }
          return String(row[key] ?? "");
        }}
      />
    )
  ) : adapterRunning ? (
    <DataTable columns={adapterColumns} data={adapterTableData} onRowClick={(row) => { if (!requirePro("Tag writes")) return; const t = String(row.type); setEditingTag({ name: String(row.name), type: t, currentValue: t.toLowerCase() === "bool" ? (/^(on|true|1)$/i.test(String(row.value)) ? "true" : "false") : String(row.value) }); }} />
  ) : (
    <EmptyState
      icon={<Network size={20} />}
      title="Simulator not running"
      description="Configure the adapter and click Start simulator to serve tags."
    />
  );

  // ── Mobile tab bar ──
  const mobileTabBar = (
    <nav style={{ display: "flex", borderTop: "1px solid var(--border)", background: "var(--card)", flexShrink: 0, paddingBottom: "env(safe-area-inset-bottom, 0px)" }}>
      {([
        { id: "connect" as const, label: "Connect" },
        { id: "data" as const, label: "Data" },
        { id: "log" as const, label: "Log" },
      ] as const).map((tab) => (
        <button
          key={tab.id}
          onClick={() => setMobileTab(tab.id)}
          style={{
            flex: 1, padding: "10px 0", fontSize: 12, fontWeight: 500, cursor: "pointer", border: "none",
            borderBottom: mobileTab === tab.id ? "2px solid var(--accent)" : "2px solid transparent",
            color: mobileTab === tab.id ? "var(--accent)" : "var(--text-muted)",
            background: "transparent",
            transition: "color 0.15s, border-color 0.15s", textAlign: "center",
            display: "flex", alignItems: "center", justifyContent: "center", gap: 4,
          }}
        >
          {tab.label}
          {mode === "adapter" && adapterRunning && tab.id === "connect" && (
            <span style={{ width: 6, height: 6, borderRadius: "50%", background: "var(--accent)", display: "inline-block" }} />
          )}
        </button>
      ))}
    </nav>
  );

  const isConnected = mode === "scanner" ? protocol.connected : adapterRunning;

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100vh", background: "var(--bg)", color: "var(--text)" }}>
      <Toolbar title="EtherNet/IP Explorer" icon={appIcon} connected={isConnected} onDisconnect={mode === "scanner" ? handleDisconnect : handleAdapterStop} onInfo={() => setShowInfo(true)} />

      {isMobile ? (
        <div style={{ flex: 1, overflow: "hidden", display: "flex", flexDirection: "column" }}>
          <div style={{ flex: 1, overflow: "auto", padding: 16 }}>
            {mobileTab === "connect" && <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>{sidebarContent}</div>}
            {mobileTab === "data" && dataTable}
            {mobileTab === "log" && <LogPanel entries={log} onExport={(f) => addLog("info", `Log exported to ${f}`)} onClear={() => setLog([])} />}
          </div>
          {mobileTabBar}
          <StatusBar connected={isConnected} latency={protocol.latency} pollCount={protocol.pollCount} error={protocol.error} />
        </div>
      ) : (
        <div style={{ display: "flex", flex: 1, overflow: "hidden" }}>
          <aside
            className="vt-sidebar"
            style={{ width: leftWidth, flexShrink: 0, borderRight: "1px solid var(--border)", cursor: dragSide ? "col-resize" : undefined }}
          >
            {sidebarContent}
          </aside>
          <div
            onMouseDown={(e) => handleDragStart("left", e)}
            style={{ width: 4, cursor: "col-resize", background: dragSide === "left" ? "var(--accent)" : "transparent", flexShrink: 0, zIndex: 10, transition: "background 0.15s" }}
            onMouseEnter={(e) => { if (!dragSide) (e.target as HTMLElement).style.background = "var(--border)"; }}
            onMouseLeave={(e) => { if (!dragSide) (e.target as HTMLElement).style.background = "transparent"; }}
          />
          <main style={{ display: "flex", flex: 1, flexDirection: "column", overflow: "hidden", padding: 8, gap: 8 }}>
            <Panel style={{ height: tableHeight, minHeight: 80, flexShrink: 0 }}>
              <div className="vt-panel__header">
                <span className="vt-panel__title">{mode === "scanner" ? "Tags" : "Adapter tags"}</span>
                {protocol.connected && (
                  <Button
                    variant="ghost"
                    size="sm"
                    aria-pressed={mainView === "objects"}
                    onClick={() => setMainView((v) => (v === "objects" ? "table" : "objects"))}
                  >
                    Objects
                  </Button>
                )}
                <Button variant="ghost" size="sm" aria-pressed={hexMode} onClick={() => setHexMode((h) => !h)}>HEX</Button>
              </div>
              <div style={{ flex: 1, minHeight: 0, overflow: "auto", display: "flex", flexDirection: "column" }}>
                {mainView === "objects" && protocol.connected ? (
                  <div style={{ flex: 1, minHeight: 80, overflow: "auto" }}>
                    <CipTree eds={edsMap} onLog={addLog} />
                  </div>
                ) : dataTable}
              </div>
            </Panel>
            {mainView !== "objects" && (
              <div
                onMouseDown={(e) => handleDragStart("table", e)}
                style={{ height: 4, cursor: "row-resize", background: dragSide === "table" ? "var(--accent)" : "transparent", flexShrink: 0, zIndex: 10, transition: "background 0.15s" }}
                onMouseEnter={(e) => { if (!dragSide) (e.target as HTMLElement).style.background = "var(--border)"; }}
                onMouseLeave={(e) => { if (!dragSide) (e.target as HTMLElement).style.background = "transparent"; }}
              />
            )}
            <Panel style={{ flex: 1, minHeight: 0 }}>
              <Tabs
                label="Bottom view"
                value={view}
                onChange={setView}
                items={[
                  { value: "log", label: "Log" },
                  { value: "trend", label: "Trend", count: trendSeries.size > 0 ? trendSeries.size : undefined },
                ]}
              />
              {view === "trend" ? (
                <TrendPanel
                  series={Array.from(trendSeries.values())}
                  onRemove={(name) => setTrendSeries((prev) => { const n = new Map(prev); n.delete(name); return n; })}
                  windowSec={trendWindow}
                  onWindowChange={setTrendWindow}
                  theme={document.documentElement.getAttribute("data-theme") || "dark"}
                />
              ) : (
                <LogPanel entries={log} onExport={(f) => addLog("info", `Log exported to ${f}`)} onClear={() => setLog([])} />
              )}
            </Panel>
          </main>


        </div>
      )}

      {!isMobile && <StatusBar connected={isConnected} latency={protocol.latency} pollCount={protocol.pollCount} error={protocol.error} />}
      {editingTag && (
        <ValueEditor
          address={editingTag.name}
          currentValue={editingTag.currentValue}
          currentDataType={cipTypeForEditor(editingTag.type ?? "")}
          types={CIP_EDITOR_TYPES}
          onSubmit={async (value, dataType) => {
            const { dataType: cipType, value: cipVal } = editorToCip(dataType, value);
            if (mode === "adapter" && adapterRunning) {
              try {
                await invoke("server_write_tag", { name: editingTag.name, value: cipVal });
                addLog("tx", `Write ${value} (${dataType}) to ${editingTag.name}`);
              } catch (e) { addLog("error", `Write failed: ${String(e)}`); }
            } else if (mode === "scanner" && protocol.connected) {
              try {
                const tag = tags.find((t) => t.name === editingTag.name);
                await invoke("write_tag", {
                  tag: { name: editingTag.name, data_type: cipType, element_count: tag?.element_count ?? 1 },
                  value: cipVal,
                });
                addLog("tx", `Write ${value} (${dataType}) to ${editingTag.name}`);
              } catch (e) { addLog("error", `Write failed: ${String(e)}`); }
            }
            setEditingTag(null);
          }}
          onCancel={() => setEditingTag(null)}
        />
      )}
      {showDiscovery && <DiscoveryPanel onClose={() => setShowDiscovery(false)} onConnect={handleConnectFromDevice} />}
      {showIdentity && <IdentityPanel onClose={() => setShowIdentity(false)} />}
      {showCipBrowser && <CipBrowser onClose={() => setShowCipBrowser(false)} onLog={addLog} eds={edsMap} />}
      {showPacketLog && <PacketLog onClose={() => setShowPacketLog(false)} />}
      {showEds && (
        <EdsPanel
          eds={eds}
          onImport={(e) => { setEds(e); addLog("info", `EDS imported: ${e.fileName} (${e.params.length} params)`); }}
          onClear={() => { setEds(null); addLog("info", "EDS removed"); }}
          onClose={() => setShowEds(false)}
        />
      )}
      {showIo && <IoPump onClose={() => setShowIo(false)} onLog={addLog} />}
      {showInfo && (
        <Dialog
          open
          onClose={() => setShowInfo(false)}
          title="EtherNet/IP Explorer"
          width={360}
          footer={
            <>
              {/* No outbound source link in App Store builds: review treats links to builds that unlock paid
                  features elsewhere as steering away from in-app purchase. */}
              {proStatus && !proStatus.store && (
                <Button variant="ghost" onClick={() => invoke("open_url", { url: "https://github.com/VoltrusID/voltrus-explorers" })}>
                  Source on GitHub
                </Button>
              )}
              <Button onClick={() => setShowInfo(false)}>Close</Button>
            </>
          }
        >
          <div style={{ fontSize: 12, color: "var(--text-dim)", lineHeight: 1.6, display: "flex", flexDirection: "column", gap: 12 }}>
            <div>Browse and interact with CIP tags on Allen-Bradley PLCs.</div>
            <div>
              <strong style={{ color: "var(--text)" }}>Scanner</strong><br />
              Connect to a PLC to browse and read tags.
              <br /><br />
              <strong style={{ color: "var(--text)" }}>Adapter</strong><br />
              Run an EtherNet/IP server to simulate a PLC. Other clients can browse and read/write tags.
            </div>
            <ProAboutRow />
          </div>
        </Dialog>
      )}
    </div>
  );
}

export default App;
