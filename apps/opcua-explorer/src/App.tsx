import { useState, useCallback, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  ConnectionForm,
  DataTable,
  Dialog,
  EmptyState,
  Field,
  IconButton,
  ListGroup,
  ListRow,
  PageTitle,
  PhoneTabBar,
  Section,
  SheetSelect,
  Segmented,
  StatusBar,
  Tabs,
  Toolbar,
  LogPanel,
  ProAboutRow,
  ProBadge,
  ValueEditor,
  saveTextFile,
  usePhoneShell,
  useProStatus,
} from "@mactools/ui";
import type { LogEntry, PhoneTabItem } from "@mactools/ui";
import { ChartLine, Download, FileDown, Layers, ListTree, Play, Plug, Plus, Radio, RefreshCw, ScrollText, Server, Square, Eye, X } from "lucide-react";
import appIcon from "../src-tauri/icons/128x128.png";
import { DiscoveryPanel } from "./DiscoveryPanel";
import { BrowserPanel } from "./BrowserPanel";
import { SubscriptionPanel } from "./SubscriptionPanel";
import { CertManager } from "./CertManager";
import { NamespacePanel } from "./NamespacePanel";
import TrendPanel, { type TrendSeries } from "./TrendPanel";
import { PhoneConnect, DEFAULT_CONN, type ConnForm } from "./phone/PhoneConnect";
import { normalizePolicy } from "./phone/policies";
import { PhoneBrowse, type BrowseNodeRef } from "./phone/PhoneBrowse";
import { WatchList, type WatchItem } from "./phone/WatchList";
import { ValueLog, samplesToCsv, valuesToCsv } from "./valueLog";

interface OpcUaConfig {
  endpoint_url: string;
  security_policy: string;
  security_mode: string;
  username: string;
  password: string;
  poll_interval_ms: number;
}

interface OpcUaNode {
  node_id: string;
  name: string;
  data_type: string;
}

interface NotificationValue {
  session: string;
  node_id: string;
  value: unknown;
  source_ts: number;
  server_ts: number;
  status: string;
}

/** One row of a session's value table. `read_tags` rows carry no data type. */
interface ClientValue {
  node_id: string;
  name: string;
  data_type?: string;
  value: unknown;
  timestamp: number;
  /** Status of the last notification (Good, Uncertain, Bad); unset for values that came from a plain read. */
  status?: string;
}

/** A connected OPC UA server. */
interface ClientSession {
  id: string;
  /** Endpoint without the scheme, made unique when several sessions share it. */
  label: string;
  endpoint: string;
  browsed: OpcUaNode[];
  values: ClientValue[];
}

interface SessionRuntime {
  subId: number | null;
  poll?: number;
}

/** A node of the built-in server, as returned by `server_read_nodes`. */
interface ServerNode {
  node_id: string;
  name: string;
  data_type: string;
  value: unknown;
  timestamp?: number;
}

interface SimRegisterDef {
  nodeId: string;
  name: string;
  dataType: string;
  base: number;
  variance: number;
  noise: number;
  period: number;
  min?: number;
  max?: number;
  isBool: boolean;
}

interface DeviceProfile {
  id: string;
  name: string;
  description: string;
  registers: SimRegisterDef[];
}

const REACTOR_PROFILE: DeviceProfile = {
  id: "reactor",
  name: "Process Plant (Reactor R-101)",
  description: "Chemical reactor: temperature, pressure, level, flow, pH",
  registers: [
    { nodeId: "ns=2;s=Reactor.Temperature", name: "Temperature", dataType: "Double", base: 185.4, variance: 1.2, noise: 0.3, period: 25000, min: 175, max: 195, isBool: false },
    { nodeId: "ns=2;s=Reactor.Pressure", name: "Pressure", dataType: "Double", base: 4.21, variance: 0.08, noise: 0.02, period: 20000, min: 3.5, max: 5.0, isBool: false },
    { nodeId: "ns=2;s=Reactor.Level", name: "Level", dataType: "Double", base: 72.5, variance: 1.5, noise: 0.5, period: 30000, min: 50, max: 90, isBool: false },
    { nodeId: "ns=2;s=Reactor.AgitatorSpeed", name: "AgitatorSpeed", dataType: "Double", base: 145, variance: 2, noise: 0.5, period: 15000, min: 130, max: 160, isBool: false },
    { nodeId: "ns=2;s=FeedFlow_Rate", name: "FeedFlow", dataType: "Double", base: 12.4, variance: 0.3, noise: 0.1, period: 18000, min: 10, max: 15, isBool: false },
    { nodeId: "ns=2;s=ProductFlow_Rate", name: "ProductFlow", dataType: "Double", base: 11.8, variance: 0.25, noise: 0.08, period: 20000, min: 9, max: 14, isBool: false },
    { nodeId: "ns=2;s=pH_Value", name: "pH", dataType: "Double", base: 7.12, variance: 0.05, noise: 0.02, period: 15000, min: 6.5, max: 7.8, isBool: false },
    { nodeId: "ns=2;s=Reactor.Status.Running", name: "Running", dataType: "Boolean", base: 1, variance: 0, noise: 0, period: 100000, isBool: true },
  ],
};

const BOILER_PROFILE: DeviceProfile = {
  id: "boiler",
  name: "Boiler System (B-201)",
  description: "Steam boiler: pressure, water level, fuel flow, efficiency",
  registers: [
    { nodeId: "ns=2;s=Boiler.SteamPressure", name: "SteamPressure", dataType: "Double", base: 10.5, variance: 0.15, noise: 0.05, period: 20000, min: 9, max: 12, isBool: false },
    { nodeId: "ns=2;s=Boiler.WaterLevel", name: "WaterLevel", dataType: "Double", base: 55.2, variance: 1.5, noise: 0.5, period: 25000, min: 40, max: 70, isBool: false },
    { nodeId: "ns=2;s=Boiler.FuelFlow", name: "FuelFlow", dataType: "Double", base: 2.4, variance: 0.08, noise: 0.02, period: 18000, min: 1.8, max: 3.0, isBool: false },
    { nodeId: "ns=2;s=Boiler.FeedWaterTemp", name: "FeedWaterTemp", dataType: "Double", base: 95.4, variance: 0.5, noise: 0.2, period: 30000, min: 85, max: 105, isBool: false },
    { nodeId: "ns=2;s=Boiler.FlueGasTemp", name: "FlueGasTemp", dataType: "Double", base: 285, variance: 3, noise: 1, period: 25000, min: 250, max: 320, isBool: false },
    { nodeId: "ns=2;s=Boiler.BurnerActive", name: "BurnerActive", dataType: "Boolean", base: 1, variance: 0, noise: 0, period: 100000, isBool: true },
    { nodeId: "ns=2;s=Boiler.Efficiency", name: "Efficiency", dataType: "Double", base: 87.2, variance: 0.5, noise: 0.1, period: 30000, min: 82, max: 92, isBool: false },
    { nodeId: "ns=2;s=Boiler.SafetyValve", name: "SafetyValve", dataType: "Boolean", base: 0, variance: 0, noise: 0, period: 100000, isBool: true },
  ],
};

const PROFILES: Record<string, DeviceProfile> = {
  reactor: REACTOR_PROFILE,
  boiler: BOILER_PROFILE,
};

const DEFAULT_NODES: OpcUaNode[] = [
  ...REACTOR_PROFILE.registers.map(r => ({ node_id: r.nodeId, name: r.name, data_type: r.dataType })),
  ...BOILER_PROFILE.registers.map(r => ({ node_id: r.nodeId, name: r.name, data_type: r.dataType })),
];

// Free tier: live subscription (or fallback polling) covers this many nodes; the rest refresh via "Read values".
const FREE_MONITORED_ITEMS = 10;


type PhoneTab = "connect" | "browse" | "watch" | "trend" | "server" | "namespaces" | "log";
function App() {
  const [mode, setMode] = useState<"server" | "client">("server");
  const [leftWidth, setLeftWidth] = useState(264);
  const [tableHeight, setTableHeight] = useState(300);
  const [dragSide, setDragSide] = useState<"left" | "table" | null>(null);
  // Phone layout (< 768px): bottom tabs, the shared phone shell. Wider windows keep the two-pane layout.
  const [phoneTab, setPhoneTab] = useState<PhoneTab>("connect");
  const isMobile = usePhoneShell();
  // On a phone the page follows the system (iOS draws the status bar from it) and the chart reads the theme from <html>.
  const [systemTheme, setSystemTheme] = useState<"dark" | "light">(() => (window.matchMedia("(prefers-color-scheme: light)").matches ? "light" : "dark"));
  useEffect(() => {
    const system = window.matchMedia("(prefers-color-scheme: light)");
    const follow = () => setSystemTheme(system.matches ? "light" : "dark");
    system.addEventListener("change", follow);
    return () => system.removeEventListener("change", follow);
  }, []);
  useEffect(() => {
    if (!isMobile) return;
    document.documentElement.setAttribute("data-theme", systemTheme);
    return () => document.documentElement.removeAttribute("data-theme");
  }, [isMobile, systemTheme]);
  const [conn, setConn] = useState<ConnForm>(DEFAULT_CONN);
  const [lastError, setLastError] = useState<string | null>(null);
  // Last few numeric values per series key, for the sparklines of the phone lists.
  const [history, setHistory] = useState<Record<string, number[]>>({});
  const [log, setLog] = useState<LogEntry[]>([]);

  // Server state
  const [serverOn, setServerOn] = useState(false);
  const [serverPort, setServerPort] = useState(4840);
  const [serverNodes, setServerNodes] = useState<ServerNode[]>([]);
  const serverPoll = useRef<ReturnType<typeof setInterval> | null>(null);

  // Client state: any number of sessions (Pro; free is one). The table shows the active one.
  const [sessions, setSessions] = useState<ClientSession[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [clientLoading, setClientLoading] = useState(false);
  const sessionsRef = useRef(sessions);
  sessionsRef.current = sessions;
  // Per-session subscription id and fallback poll timer. They never render, so they stay out of state.
  const runtimeRef = useRef(new Map<string, SessionRuntime>());
  const nextSessionRef = useRef(1);
  const [valueLog] = useState(() => new ValueLog());
  const unlistenMainRef = useRef<(() => void) | null>(null);
  const [showExport, setShowExport] = useState(false);
  const activeSession = sessions.find((s) => s.id === activeId) ?? null;
  const clientConnected = activeSession !== null;
  const clientValues = activeSession?.values ?? [];
  const browsedNodes = activeSession?.browsed ?? [];

  // Template / Sim state
  const [simTemplate, setSimTemplate] = useState<string>("");
  const [simRunning, setSimRunning] = useState(false);
  const [simValues, setSimValues] = useState<number[]>([]);

  const [editingNode, setEditingNode] = useState<{ id: string; value: string; dataType: string; server: boolean } | null>(null);
  const [showInfo, setShowInfo] = useState(false);

  // Trend state
  const [trendSeries, setTrendSeries] = useState<Map<string, TrendSeries>>(new Map());
  const [trendWindow, setTrendWindow] = useState(60);
  const [view, setView] = useState<"log" | "trend">("log");
  const trendSeriesRef = useRef(trendSeries);
  trendSeriesRef.current = trendSeries;

  // Panels + connection form injection
  const [showDiscovery, setShowDiscovery] = useState(false);
  const [showBrowser, setShowBrowser] = useState(false);
  const [showSubscriptions, setShowSubscriptions] = useState(false);
  const [showCerts, setShowCerts] = useState(false);
  const [showNamespaces, setShowNamespaces] = useState(false);
  const [connKey, setConnKey] = useState(0);
  const [discovered, setDiscovered] = useState<Record<string, string | number>>({});

  const { requirePro, pro, status: proStatus } = useProStatus();

  const addLog = useCallback((direction: LogEntry["direction"], message: string) => {
    setLog((prev) => [...prev, { timestamp: Date.now(), direction, message }]);
    // The phone header shows the latest problem until something else goes right.
    setLastError((cur) => (direction === "error" ? message : direction === "info" || direction === "rx" ? null : cur));
  }, []);

  // Drag resize
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
      if (dragSide === "left") setLeftWidth(Math.max(180, Math.min(400, dragStartRef.current.startWidth + (e.clientX - dragStartRef.current.startX))));
      else setTableHeight(Math.max(80, dragStartRef.current.startHeight + (e.clientY - dragStartRef.current.startY)));
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", () => setDragSide(null));
    return () => { window.removeEventListener("mousemove", onMove); window.removeEventListener("mouseup", () => setDragSide(null)); };
  }, [dragSide]);

  // Poll server nodes when server is on
  useEffect(() => {
    if (!serverOn) { setServerNodes([]); return; }
    const poll = setInterval(async () => {
      try {
        const nodes = await invoke<ServerNode[]>("server_read_nodes");
        const rows = nodes ?? [];
        setServerNodes(rows);
        const now = Date.now();
        for (const n of rows) valueLog.record("server", "Server", n.node_id, n.name, n.value, n.timestamp || now);
      } catch { /* ignore */ }
    }, 500);
    serverPoll.current = poll;
    return () => clearInterval(poll);
  }, [serverOn, valueLog]);

  const updateSession = useCallback((id: string, fn: (s: ClientSession) => ClientSession) => {
    setSessions((prev) => prev.map((s) => (s.id === id ? fn(s) : s)));
  }, []);

  const recordValues = useCallback((session: { id: string; label: string }, rows: ClientValue[]) => {
    const now = Date.now();
    for (const r of rows) valueLog.record(session.id, session.label, r.node_id, r.name, r.value, r.timestamp || now);
  }, [valueLog]);

  // Fallback: poll a session's values when no live subscription is possible.
  const startClientPoll = useCallback((session: { id: string; label: string }, nodes: OpcUaNode[]) => {
    const runtime = runtimeRef.current.get(session.id);
    if (!runtime) return;
    clearInterval(runtime.poll);
    const all = nodes.length > 0 ? nodes : DEFAULT_NODES;
    const nodesToRead = pro ? all : all.slice(0, FREE_MONITORED_ITEMS);
    const doPoll = async () => {
      try {
        const vals = await invoke<ClientValue[]>("read_tags", { session: session.id, tags: nodesToRead });
        updateSession(session.id, (s) => ({ ...s, values: vals }));
        recordValues(session, vals);
      } catch (e) { addLog("error", String(e)); }
    };
    doPoll();
    runtime.poll = window.setInterval(doPoll, 1000);
  }, [addLog, pro, updateSession, recordValues]);

  // Live updates: subscription notifications drive the main table, each routed to the session it came from.
  // Registered once for the app's lifetime (not gated on connection) so there
  // is no race between the async listen() resolution and the first
  // notification burst after a subscription is created.
  useEffect(() => {
    let active = true;
    listen<NotificationValue>("opcua://notification", (e) => {
      if (!active) return;
      const n = e.payload;
      const session = sessionsRef.current.find((s) => s.id === n.session);
      if (!session) return; // notification for a session that is gone
      const ts = n.server_ts || n.source_ts || 0;
      const known = session.values.find((row) => row.node_id === n.node_id);
      valueLog.record(session.id, session.label, n.node_id, known?.name ?? n.node_id, n.value, ts || Date.now());
      updateSession(session.id, (s) => ({
        ...s,
        values: s.values.some((row) => row.node_id === n.node_id)
          ? s.values.map((row) => row.node_id === n.node_id ? { ...row, value: n.value, timestamp: ts || row.timestamp, status: n.status } : row)
          // Unknown node: add it so the row still appears.
          : [...s.values, { node_id: n.node_id, name: n.node_id, data_type: "", value: n.value, timestamp: ts, status: n.status }],
      }));
    }).then((fn) => { if (active) unlistenMainRef.current = fn; });
    return () => { active = false; unlistenMainRef.current?.(); unlistenMainRef.current = null; };
  }, [valueLog, updateSession]);

  // ── Handlers ──

  const handleServerToggle = useCallback(async () => {
    if (serverOn) {
      await invoke("server_stop").catch(() => {});
      if (simRunning) { await invoke("sim_stop").catch(() => {}); setSimRunning(false); }
      setServerOn(false);
      addLog("info", "Server stopped");
    } else {
      try {
        const url = await invoke<string>("server_start", { port: serverPort });
        setServerOn(true);
        addLog("info", `Server started: ${url}`);
      } catch (e) {
        addLog("error", `Server start failed: ${e}`);
      }
    }
  }, [serverOn, serverPort, simRunning, addLog]);

  // Read the current value of every node once (Read service).
  const readValues = useCallback(async (session: { id: string; label: string }, nodes: OpcUaNode[]) => {
    try {
      const vals = await invoke<(Partial<ClientValue> | null)[]>("read_tags", { session: session.id, tags: nodes });
      if (vals.length > 0) {
        const rows = vals.map((b, i) => ({
          node_id: b?.node_id ?? nodes[i]?.node_id ?? "",
          name: b?.name ?? nodes[i]?.name ?? "",
          data_type: nodes[i]?.data_type ?? "",
          value: b?.value ?? null,
          timestamp: b?.timestamp ?? 0,
        }));
        updateSession(session.id, (s) => ({ ...s, values: rows }));
        recordValues(session, rows);
      }
    } catch (e) {
      addLog("error", `Read failed: ${e}`);
    }
  }, [addLog, updateSession, recordValues]);

  const handleConnect = useCallback(async (values: Record<string, string | number>) => {
    const config: OpcUaConfig = {
      endpoint_url: String(values.endpointUrl),
      security_policy: String(values.securityPolicy || "None"),
      security_mode: String(values.securityMode || "None"),
      username: String(values.username || ""),
      password: String(values.password || ""),
      poll_interval_ms: Number(values.pollInterval || 1000),
    };
    const id = `s${nextSessionRef.current++}`;
    const base = config.endpoint_url.replace(/^opc\.tcp:\/\//, "");
    const taken = new Set(sessionsRef.current.map((s) => s.label));
    let label = base;
    for (let n = 2; taken.has(label); n++) label = `${base} (${n})`;
    const session = { id, label };

    setClientLoading(true);
    try {
      await invoke("connect", { session: id, config });
      const runtime: SessionRuntime = { subId: null };
      runtimeRef.current.set(id, runtime);
      setSessions((prev) => [...prev, { id, label, endpoint: config.endpoint_url, browsed: [], values: [] }]);
      setActiveId(id);
      setAdding(false);
      setPhoneTab("watch");

      try {
        const nodes = (await invoke<OpcUaNode[]>("browse_nodes", { session: id })) ?? [];
        updateSession(id, (s) => ({
          ...s,
          browsed: nodes,
          values: nodes.map((n) => ({ node_id: n.node_id, name: n.name, data_type: n.data_type, value: null, timestamp: 0 })),
        }));
        addLog("info", `Discovered ${nodes.length} nodes via Browse`);
        if (nodes.length > 0) {
          // One Read of every browsed node. Free tier ends here (use "Read values" to refresh); with Pro the
          // live subscription below keeps the table current. Data-change notifications only fire on change,
          // so static nodes would stay blank without this snapshot either way.
          await readValues(session, nodes);
          const liveNodes = pro ? nodes : nodes.slice(0, FREE_MONITORED_ITEMS);
          try {
            const cfg = {
              publishing_interval_ms: 1000,
              lifetime_count: 60,
              max_keep_alive_count: 10,
              max_notifications_per_publish: 0,
              priority: 0,
              publishing_enabled: true,
            };
            const subId = await invoke<number>("create_subscription", { session: id, cfg });
            runtime.subId = subId;
            const items = liveNodes.map((n) => ({
              node_id: n.node_id,
              attribute_id: 13,
              sampling_interval_ms: 1000,
              queue_size: 10,
              discard_oldest: true,
              monitoring_mode: "Reporting",
              trigger: "StatusValue",
              deadband_type: 0,
              deadband_value: 0,
            }));
            const created = await invoke<number>("create_monitored_items", { session: id, subscriptionId: subId, items });
            addLog("info", `Live subscription ${subId} created for ${created} nodes`);
          } catch (e) {
            addLog("error", `Live subscription failed, falling back to polling: ${e}`);
            startClientPoll(session, liveNodes);
          }
        }
      } catch (e) {
        addLog("error", `Browse failed: ${e}`);
        startClientPoll(session, []);
      }

      addLog("info", `Connected to ${config.endpoint_url}`);
    } catch (e) {
      addLog("error", `Connection failed: ${e}`);
    } finally {
      setClientLoading(false);
    }
  }, [addLog, startClientPoll, readValues, updateSession, pro]);

  const closeSession = useCallback(async (id: string) => {
    const runtime = runtimeRef.current.get(id);
    runtimeRef.current.delete(id);
    clearInterval(runtime?.poll);
    if (runtime?.subId != null) {
      await invoke("delete_subscription", { session: id, subscriptionId: runtime.subId }).catch(() => {});
    }
    await invoke("disconnect", { session: id }).catch(() => {});
    valueLog.forget(id);
    const label = sessionsRef.current.find((s) => s.id === id)?.label ?? id;
    setTrendSeries((prev) => new Map([...prev].filter(([key]) => !key.startsWith(`${id}|`))));
    setSessions((prev) => prev.filter((s) => s.id !== id));
    setActiveId((cur) => (cur === id ? sessionsRef.current.find((s) => s.id !== id)?.id ?? null : cur));
    addLog("info", `Disconnected ${label}`);
  }, [addLog, valueLog]);

  const handleSimToggle = useCallback(async () => {
    if (simRunning) {
      await invoke("sim_stop");
      setSimRunning(false);
      addLog("info", "Simulation stopped");
    } else {
      if (!requirePro("Device templates")) return;
      const profile = PROFILES[simTemplate];
      if (!profile) return;
      // Start the server first so the simulator has a subscription-aware
      // writer to push values into the address space.
      if (!serverOn) {
        try {
          await invoke("server_start", { port: serverPort });
          setServerOn(true);
          addLog("info", `Server auto-started on port ${serverPort}`);
        } catch (e) {
          addLog("error", `Server start failed: ${e}`);
          return;
        }
      }
      await invoke("sim_start", {
        profile: {
          id: profile.id,
          name: profile.name,
          description: profile.description,
          registers: profile.registers.map(r => ({
            nodeId: r.nodeId, name: r.name, dataType: r.dataType,
            base: r.base, variance: r.variance, noise: r.noise,
            period: r.period, min: r.min, max: r.max, isBool: r.isBool,
          })),
        },
        intervalMs: 1000,
      });
      setSimRunning(true);
      addLog("info", `Simulation started: ${profile.name}`);
    }
  }, [simRunning, simTemplate, serverOn, serverPort, addLog, requirePro]);

  // Writing a value is Pro.
  const openNodeEditor = useCallback((id: string, value: unknown, dataType: string, server: boolean) => {
    if (!requirePro("Writing values")) return;
    setEditingNode({ id, value: value === null || value === undefined || typeof value === "object" ? "" : String(value), dataType: dataType || "Double", server });
  }, [requirePro]);
  const openEditor = useCallback((row: Record<string, unknown>) => {
    openNodeEditor(String(row.nodeId), row.value ?? "", String(row.dataType ?? "Double"), mode === "server");
  }, [openNodeEditor, mode]);

  // ── Table ──

  const columns = [
    { key: "nodeId", label: "Node ID", width: "200px" },
    { key: "browseName", label: "Browse Name", width: "120px" },
    { key: "dataType", label: "Data Type", width: "100px" },
    { key: "value", label: "Value" },
    { key: "trend", label: "", width: "54px" },
  ];

  const activeNodes = mode === "client" && browsedNodes.length > 0 ? browsedNodes : DEFAULT_NODES;

  const tableData =
    mode === "server" && serverNodes.length > 0
      ? serverNodes.map((n) => ({
          nodeId: n.node_id ?? "",
          browseName: n.name ?? "",
          dataType: n.data_type ?? "",
          value: n.value !== null && n.value !== undefined ? String(n.value) : "-",
          _nodeId: n.node_id ?? "",
        }))
    : mode === "client" && clientValues.length > 0
      ? clientValues.map((v, i) => ({
          nodeId: v.node_id || activeNodes[i]?.node_id || "",
          browseName: v.name || activeNodes[i]?.name || "",
          dataType: v.data_type || activeNodes[i]?.data_type || "",
          value: v.value !== null && v.value !== undefined ? String(v.value) : "-",
          _nodeId: v.node_id || activeNodes[i]?.node_id || "",
        }))
      : activeNodes.map((n) => ({
          nodeId: n.node_id,
          browseName: n.name,
          dataType: n.data_type,
          value: "\u2014",
          _nodeId: n.node_id,
        }));

  // What the table shows: the built-in server or the active session. Keys trend series and labels exports.
  const scope = mode === "server" ? "server" : activeId ?? "";
  const scopeLabel = mode === "server" ? "Server" : activeSession?.label ?? "";

  // Trend samples come from the server table and from every session, so a series keeps filling while another
  // session is in view.
  const trendRows = [
    ...serverNodes.map((n) => ({ key: `server|${n.node_id}`, raw: n.value })),
    ...sessions.flatMap((s) => s.values.map((v) => ({ key: `${s.id}|${v.node_id}`, raw: v.value }))),
  ].map((r) => ({ key: r.key, v: r.raw === null || r.raw === undefined || r.raw === "" ? NaN : Number(r.raw) }));
  const trendRowsRef = useRef(trendRows);
  trendRowsRef.current = trendRows;
  const valueSig = trendRows.map((r) => r.key + ":" + r.v).join("|");

  // Append a sample to every tracked series on each value change.
  useEffect(() => {
    const active = trendSeriesRef.current;
    if (active.size === 0) return;
    const ts = Date.now();
    const byKey = new Map(trendRowsRef.current.map((r) => [r.key, r.v]));
    const cutoff = ts - trendWindow * 1000 - 5000;
    const next = new Map(active);
    let changed = false;
    for (const [key, s] of next) {
      const v = byKey.get(key);
      if (v === undefined || Number.isNaN(v)) continue;
      const points = [...s.points, { t: ts, v }].filter((p) => p.t > cutoff).slice(-500);
      next.set(key, { ...s, points });
      changed = true;
    }
    if (changed) setTrendSeries(next);
  }, [valueSig, trendWindow]);

  // Sparkline history of every value that changes (phone lists).
  useEffect(() => {
    if (!isMobile) return;
    setHistory((prev) => {
      let next = prev;
      for (const r of trendRowsRef.current) {
        if (Number.isNaN(r.v)) continue;
        const cur = prev[r.key];
        if (cur && cur[cur.length - 1] === r.v) continue;
        if (next === prev) next = { ...prev };
        next[r.key] = [...(cur ?? []), r.v].slice(-40);
      }
      return next;
    });
  }, [valueSig, isMobile]);

  const toggleTrend = useCallback((key: string, name: string, initial?: number) => {
    // Adding a series is Pro; removing one never is.
    if (!trendSeriesRef.current.has(key) && !requirePro("Live trends")) return;
    setTrendSeries((prev) => {
      const next = new Map(prev);
      if (next.has(key)) {
        next.delete(key);
      } else {
        const v = initial ?? trendRowsRef.current.find((r) => r.key === key)?.v ?? 0;
        const colors = ["#3b82f6", "#ef4444", "#22c55e", "#f59e0b", "#a855f7", "#ec4899", "#14b8a6", "#f97316"];
        next.set(key, { id: key, name, color: colors[next.size % colors.length], points: [{ t: Date.now(), v: Number.isNaN(v) ? 0 : v }] });
      }
      return next;
    });
  }, [requirePro]);

  // ── Sessions and CSV export ──

  const addSession = () => {
    if (sessions.length > 0 && !requirePro("Several sessions")) return;
    setAdding(true);
    setPhoneTab("connect");
  };

  const exportCsv = async (kind: "values" | "changes", from: "server" | "client" = mode) => {
    setShowExport(false);
    if (!requirePro("CSV export of monitored values")) return;
    const csv = kind === "changes"
      ? samplesToCsv(valueLog.all())
      : valuesToCsv(
          from === "server"
            ? serverNodes.map((n) => ({ session: "Server", nodeId: n.node_id, name: n.name, dataType: n.data_type, value: n.value, timestamp: n.timestamp ?? 0 }))
            : sessions.flatMap((s) => s.values.map((v) => ({
                session: s.label,
                nodeId: v.node_id,
                name: v.name,
                dataType: v.data_type ?? "",
                value: v.value,
                timestamp: v.timestamp,
              }))),
        );
    const stamp = new Date().toISOString().slice(0, 19).replace(/:/g, "-");
    try {
      const path = await saveTextFile(`opcua-${kind}-${stamp}.csv`, csv);
      if (path) addLog("info", `Exported ${kind === "changes" ? "recorded changes" : "current values"} to ${path}`);
    } catch (e) {
      addLog("error", `Export failed: ${e}`);
    }
  };

  const tableToolbar = (
    <div style={{ display: "flex", alignItems: "center", gap: 8, flexShrink: 0, background: "var(--surface)", borderBottom: "1px solid var(--border)", paddingRight: 8 }}>
      <div className="vt-tabs" role="tablist" aria-label="Sessions" style={{ flex: 1, minWidth: 0, borderBottom: "none" }}>
        {mode === "server" && <span className="vt-hint" style={{ alignSelf: "center", padding: "0 10px" }}>Built-in server</span>}
        {mode === "client" && sessions.length === 0 && (
          <span className="vt-hint" style={{ alignSelf: "center", padding: "0 10px" }}>No session. Connect to a server.</span>
        )}
        {mode === "client" && sessions.map((s) => (
          <div key={s.id} style={{ display: "flex", alignItems: "center", gap: 2 }}>
            <button
              role="tab"
              aria-selected={s.id === activeId}
              className="vt-tab"
              onClick={() => { setActiveId(s.id); setAdding(false); }}
              title={s.endpoint}
            >
              <span className="vt-dot vt-dot--success" aria-hidden="true" />
              {s.label}
            </button>
            <IconButton size="sm" label={`Disconnect ${s.label}`} onClick={() => closeSession(s.id)}>
              <X size={14} strokeWidth={1.75} />
            </IconButton>
          </div>
        ))}
        {mode === "client" && sessions.length > 0 && (
          <IconButton label="Add session (Pro for more than one)" onClick={addSession}>
            <Plus size={16} strokeWidth={1.75} />
          </IconButton>
        )}
      </div>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button size="sm" icon={<Download size={14} strokeWidth={1.75} />}>
            Export <ProBadge />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuItem icon={<FileDown size={14} strokeWidth={1.75} />} onSelect={() => exportCsv("values")}>
            Current values (CSV)
          </DropdownMenuItem>
          <DropdownMenuItem icon={<FileDown size={14} strokeWidth={1.75} />} disabled={valueLog.size === 0} onSelect={() => exportCsv("changes")}>
            {`Recorded changes, ${valueLog.size} (CSV)`}
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );

  // ── Sidebar ──

  const sidebarContent = (
    <>
      <Section title="Mode">
        <Segmented
          block
          label="Mode"
          value={mode}
          onChange={setMode}
          options={[
            { value: "server", label: "Server", icon: <Server size={14} strokeWidth={1.75} /> },
            { value: "client", label: "Client", icon: <Plug size={14} strokeWidth={1.75} /> },
          ]}
        />
      </Section>

      {mode === "server" ? (
        <>
          <Section title="Server">
            <Field label="Port">
              {(id) => (
                <input
                  id={id}
                  className="vt-input"
                  type="number"
                  value={serverPort}
                  onChange={(e) => setServerPort(Number(e.target.value))}
                  disabled={serverOn}
                />
              )}
            </Field>
            <Button
              block
              variant={serverOn ? "danger" : "primary"}
              icon={serverOn ? <Square size={14} strokeWidth={1.75} /> : <Play size={14} strokeWidth={1.75} />}
              onClick={handleServerToggle}
            >
              {serverOn ? "Stop server" : "Start server"}
            </Button>
          </Section>

          <Section title="Device template" meta={<ProBadge />}>
            <Field label="Template">
              {(id) => (
                <SheetSelect
                  id={id}
                  sheetTitle="Device template"
                  placeholder="Select template"
                  value={simTemplate}
                  options={Object.entries(PROFILES).map(([k, p]) => ({ value: k, label: p.name }))}
                  onChange={(v) => { setSimTemplate(v); if (simRunning) handleSimToggle(); }}
                />
              )}
            </Field>
            {simTemplate && (
              <span className="vt-hint">
                {PROFILES[simTemplate].description} {PROFILES[simTemplate].registers.length} points
              </span>
            )}
            <Button
              block
              variant={simRunning ? "danger" : "primary"}
              disabled={!simTemplate}
              icon={simRunning ? <Square size={14} strokeWidth={1.75} /> : <Play size={14} strokeWidth={1.75} />}
              onClick={handleSimToggle}
            >
              {simRunning ? "Stop simulation" : "Start simulation"}
            </Button>
            {!simTemplate && (
              <span className="vt-hint">Select a device template to start the simulation. The server starts automatically if needed.</span>
            )}
          </Section>

          {serverOn && (
            <Section title="Server info">
              <span className="vt-hint">
                OPC UA server active on port {serverPort}.{" "}
                {simRunning ? `Template injecting ${PROFILES[simTemplate]?.registers.length ?? 0} values.` : "No template loaded."}{" "}
                Click any row to write a value{pro ? "" : " (Pro)"}.
              </span>
            </Section>
          )}
        </>
      ) : (
        <>
          {(!clientConnected || adding) && (
            <Section
              title={adding ? "New session" : "Connection"}
              actions={
                <Button size="sm" variant="ghost" onClick={() => setShowDiscovery(true)}>
                  Discover
                </Button>
              }
            >
              <ConnectionForm
                key={connKey}
                initialValues={discovered}
                fields={[
                  { name: "endpointUrl", label: "Endpoint URL", type: "text", default: "opc.tcp://localhost:4840" },
                  { name: "securityPolicy", label: "Security policy", type: "select", default: "None", options: [
                    { value: "None", label: "None" },
                    { value: "Basic256Sha256", label: "Basic256Sha256" },
                    { value: "Basic128Rsa15", label: "Basic128Rsa15" },
                    { value: "Aes128Sha256RsaOaep", label: "Aes128_Sha256_RsaOaep" },
                  ] },
                  { name: "securityMode", label: "Security mode", type: "select", default: "None", options: [
                    { value: "None", label: "None" },
                    { value: "Sign", label: "Sign" },
                    { value: "SignAndEncrypt", label: "SignAndEncrypt" },
                  ] },
                  { name: "username", label: "Username", type: "text" },
                  { name: "password", label: "Password", type: "password" },
                ]}
                onSubmit={handleConnect}
                onCancel={() => setClientLoading(false)}
                loading={clientLoading}
              />
              {adding && (
                <Button block variant="ghost" onClick={() => { setAdding(false); setClientLoading(false); }}>
                  Cancel
                </Button>
              )}
            </Section>
          )}

          <Section title="Tools">
            <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 6 }}>
              <Button size="sm" onClick={() => { if (requirePro("Certificate management")) setShowCerts(true); }}>Certificates <ProBadge /></Button>
              <Button size="sm" onClick={() => setShowDiscovery(true)}>Discovery</Button>
              {clientConnected && <>
                <Button size="sm" onClick={() => setShowBrowser(true)}>Browse</Button>
                <Button size="sm" onClick={() => setShowNamespaces(true)}>Namespaces</Button>
                <Button size="sm" style={{ gridColumn: "span 2" }} onClick={() => { if (requirePro("Subscriptions")) setShowSubscriptions(true); }}>Subscriptions <ProBadge /></Button>
              </>}
            </div>
          </Section>

          {activeSession && !adding && (
            <Section title="Session">
              <span className="vt-hint mono" style={{ wordBreak: "break-all" }}>{activeSession.endpoint}</span>
              <span className="vt-hint">
                Connected to {browsedNodes.length > 0 ? `${browsedNodes.length} browsed nodes` : `${DEFAULT_NODES.length} default nodes`}.{" "}
                {pro ? "Values update live. Click any row to write a value." : `Live updates for the first ${Math.min(FREE_MONITORED_ITEMS, browsedNodes.length || DEFAULT_NODES.length)} nodes (Pro for more). Use Read values to refresh the rest. Click any row to write a value (Pro). Several sessions at once need Pro.`}
              </span>
              <Button size="sm" disabled={browsedNodes.length === 0} onClick={() => readValues(activeSession, browsedNodes)}>Read values</Button>
              <Button size="sm" variant="danger" onClick={() => closeSession(activeSession.id)}>Disconnect</Button>
            </Section>
          )}
        </>
      )}
    </>
  );

  // ── Phone: the built-in server ──
  const templateOptions = [
    { value: "", label: "None", hint: "Empty address space" },
    ...Object.entries(PROFILES).map(([k, p]) => ({ value: k, label: p.name, hint: `${p.registers.length} simulated points` })),
  ];
  const sidebarServer = (
    <>
      <Section title="Server">
        <Field label="Port">
          {(id) => <input id={id} className="vt-input vt-input--mono" type="number" inputMode="numeric" value={serverPort} onChange={(e) => setServerPort(Number(e.target.value))} disabled={serverOn} />}
        </Field>
        <Button block size="lg" variant={serverOn ? "danger" : "primary"} icon={serverOn ? <Square size={16} strokeWidth={2} /> : <Play size={16} strokeWidth={2} />} onClick={handleServerToggle}>
          {serverOn ? "Stop server" : "Start server"}
        </Button>
        <span className="vt-hint">
          {serverOn
            ? `Running. Other devices on this network connect to this device's address on port ${serverPort}.`
            : "Runs an OPC UA server on this device, so you can try a client against it."}
        </span>
      </Section>
      <Section title="Device template" meta={<ProBadge />}>
        <Field label="Template">
          <SheetSelect
            sheetTitle="Device template"
            value={simTemplate}
            options={templateOptions}
            onChange={(v) => { setSimTemplate(v); if (simRunning) handleSimToggle(); }}
          />
        </Field>
        <Button block variant={simRunning ? "danger" : "primary"} disabled={!simTemplate} icon={simRunning ? <Square size={16} strokeWidth={2} /> : <Play size={16} strokeWidth={2} />} onClick={handleSimToggle}>
          {simRunning ? "Stop simulation" : "Start simulation"}
        </Button>
        <span className="vt-hint">
          {simTemplate ? `${PROFILES[simTemplate]?.description}. ${PROFILES[simTemplate]?.registers.length} points.` : "Pick a device template to fill the server with changing values. The server starts by itself if it is not running."}
        </span>
      </Section>
    </>
  );

  // ── Phone pages (bottom tabs) ──
  const ICON = { size: 22, strokeWidth: 2 } as const;
  const phoneTabs: PhoneTabItem<PhoneTab>[] = [
    { id: "connect", label: "Connect", icon: <Plug {...ICON} /> },
    { id: "browse", label: "Browse", icon: <ListTree {...ICON} /> },
    { id: "watch", label: "Watch", icon: <Eye {...ICON} /> },
    { id: "trend", label: "Trend", icon: <ChartLine {...ICON} />, badge: trendSeries.size },
    { id: "server", label: "Server", icon: <Server {...ICON} /> },
    { id: "namespaces", label: "Namespaces", icon: <Layers {...ICON} /> },
    { id: "log", label: "Log", icon: <ScrollText {...ICON} /> },
  ];

  const watchItems: WatchItem[] = activeSession
    ? activeSession.values.map((v) => ({ key: `${activeSession.id}|${v.node_id}`, nodeId: v.node_id, name: v.name, dataType: v.data_type ?? "", value: v.value, status: v.status }))
    : [];
  const serverItems: WatchItem[] = serverNodes.map((n) => ({ key: `server|${n.node_id}`, nodeId: n.node_id, name: n.name, dataType: n.data_type, value: n.value }));

  const trendItem = (it: WatchItem) => {
    const name = it.name || it.nodeId;
    toggleTrend(it.key, it.key.startsWith("server|") || sessions.length < 2 ? name : `${activeSession?.label ?? ""}: ${name}`, Number.isNaN(Number(it.value)) ? undefined : Number(it.value));
  };

  // Watch a node from the browser: one more monitored item on the session's live subscription (or its polling fallback).
  const watchNode = async (n: BrowseNodeRef) => {
    const s = activeSession;
    if (!s || s.values.some((v) => v.node_id === n.nodeId)) return;
    const nodes: OpcUaNode[] = [...s.values.map((v) => ({ node_id: v.node_id, name: v.name, data_type: v.data_type ?? "" })), { node_id: n.nodeId, name: n.name, data_type: n.dataType }];
    updateSession(s.id, (x) => ({ ...x, values: [...x.values, { node_id: n.nodeId, name: n.name, data_type: n.dataType, value: n.value ?? null, timestamp: Date.now() }] }));
    const runtime = runtimeRef.current.get(s.id);
    if (runtime?.subId != null && (pro || s.values.length < FREE_MONITORED_ITEMS)) {
      const item = { node_id: n.nodeId, attribute_id: 13, sampling_interval_ms: 1000, queue_size: 10, discard_oldest: true, monitoring_mode: "Reporting", trigger: "StatusValue", deadband_type: 0, deadband_value: 0 };
      try {
        await invoke<number>("create_monitored_items", { session: s.id, subscriptionId: runtime.subId, items: [item] });
        addLog("info", `Watching ${n.name}`);
      } catch (e) { addLog("error", `Watch failed: ${e}`); }
    } else if (runtime?.poll !== undefined) {
      startClientPoll(s, nodes);
    } else {
      addLog("info", `${n.name} added to Watch. Free builds update ${FREE_MONITORED_ITEMS} items live, so use Read values to refresh this one.`);
    }
  };

  const trendNode = async (n: BrowseNodeRef) => {
    const key = `${activeId}|${n.nodeId}`;
    if (!trendSeries.has(key)) {
      if (!requirePro("Live trends")) return;
      await watchNode(n);
    }
    const num = Number(n.value);
    toggleTrend(key, sessions.length > 1 ? `${scopeLabel}: ${n.name}` : n.name, n.value === null || n.value === undefined || Number.isNaN(num) ? undefined : num);
  };

  const needsSession = (icon: React.ReactNode, what: string) => (
    <EmptyState
      icon={icon}
      title="Not connected"
      description={`Connect to an OPC UA server to ${what}.`}
      action={<Button variant="primary" onClick={() => setPhoneTab("connect")}>Go to Connect</Button>}
    />
  );

  // Every page stays mounted (hidden when inactive) so the browsed path, the watch list and the log survive tab switches.
  // `fill` pages scroll inside (lists, log); the others scroll as a whole.
  const phonePage = (tab: PhoneTab, children: React.ReactNode, fill = false) => (
    <div className={fill ? "vm-body vm-body--fill" : "vm-body"} style={phoneTab === tab ? undefined : { display: "none" }}>
      {children}
    </div>
  );

  const readAll = () => {
    if (!activeSession) return;
    void readValues(activeSession, activeSession.values.map((v) => ({ node_id: v.node_id, name: v.name, data_type: v.data_type ?? "" })));
  };

  const sessionSheet = sessions.length > 1 && (
    <SheetSelect
      sheetTitle="Session"
      aria-label="Session"
      value={activeId ?? ""}
      options={sessions.map((s) => ({ value: s.id, label: s.label, hint: s.endpoint }))}
      onChange={(id) => setActiveId(id)}
    />
  );

  const phoneLayout = (
    <>
      {phonePage(
        "connect",
        <PhoneConnect
          form={conn}
          onForm={setConn}
          sessions={sessions.map((s) => ({ id: s.id, label: s.label, endpoint: s.endpoint, nodes: s.values.length }))}
          activeId={activeId}
          adding={adding}
          loading={clientLoading}
          onConnect={() => handleConnect({
            endpointUrl: conn.endpointUrl.trim(),
            securityPolicy: conn.securityPolicy,
            securityMode: conn.securityMode,
            username: conn.auth === "user" ? conn.username : "",
            password: conn.auth === "user" ? conn.password : "",
          })}
          onCancel={() => { setAdding(false); setClientLoading(false); }}
          onActivate={(id) => { setActiveId(id); setAdding(false); }}
          onDisconnect={closeSession}
          onAdd={addSession}
          onDiscover={() => setShowDiscovery(true)}
          onCerts={() => { if (requirePro("Certificate management")) setShowCerts(true); }}
        />,
      )}
      {phonePage(
        "browse",
        activeSession ? (
          <PhoneBrowse
            key={activeSession.id}
            session={activeSession.id}
            watched={(id) => {
              const v = activeSession.values.find((x) => x.node_id === id);
              return v ? { value: v.value, dataType: v.data_type ?? "" } : undefined;
            }}
            trending={(id) => trendSeries.has(`${activeSession.id}|${id}`)}
            onWatch={watchNode}
            onTrend={trendNode}
            onWrite={(n) => openNodeEditor(n.nodeId, n.value, n.dataType, false)}
            onLog={addLog}
          />
        ) : (
          <>
            <PageTitle title="Browse" caption="Address space" />
            {needsSession(<ListTree size={22} strokeWidth={1.75} />, "browse its address space")}
          </>
        ),
        true,
      )}
      {phonePage(
        "watch",
        <>
          <PageTitle title="Watch" caption={activeSession ? `${watchItems.length} ${watchItems.length === 1 ? "item" : "items"}` : "Live values"} />
          {activeSession ? (
            <>
              <div className="oc-toolbar">
                {sessionSheet}
                <div style={{ flex: 1 }} />
                <IconButton outline label="Read values" onClick={readAll}>
                  <RefreshCw size={18} strokeWidth={1.75} />
                </IconButton>
                <IconButton outline label="Subscriptions" onClick={() => { if (requirePro("Subscriptions")) setShowSubscriptions(true); }}>
                  <Radio size={18} strokeWidth={1.75} />
                </IconButton>
                <IconButton outline label="Export CSV" onClick={() => setShowExport(true)}>
                  <Download size={18} strokeWidth={1.75} />
                </IconButton>
              </div>
              <div className="oc-scroll">
                {watchItems.length === 0 ? (
                  <EmptyState compact title="No values" description="Open a variable on the Browse tab and tap Watch to follow it here." />
                ) : (
                  <WatchList
                    items={watchItems}
                    history={history}
                    trending={trendSeries}
                    onOpen={(it) => openNodeEditor(it.nodeId, it.value, it.dataType, false)}
                    onToggleTrend={trendItem}
                  />
                )}
                <p className="oc-note" style={{ marginTop: 12 }}>
                  {pro ? "Values update live. Tap a row to write a value." : `Live updates for the first ${FREE_MONITORED_ITEMS} items. Tap a row to write a value (Pro). Several sessions at once need Pro.`}
                </p>
              </div>
            </>
          ) : (
            needsSession(<Eye size={22} strokeWidth={1.75} />, "watch its values")
          )}
        </>,
        true,
      )}
      {phonePage(
        "trend",
        <>
          <PageTitle title="Trend" caption={trendSeries.size > 0 ? `${trendSeries.size} live` : "Values over time"} />
          {phoneTab === "trend" && (
            <TrendPanel
              key={trendSeries.size > 0 ? "chart" : "empty"}
              series={Array.from(trendSeries.values())}
              onRemove={(nodeId) => setTrendSeries((prev) => { const n = new Map(prev); n.delete(nodeId); return n; })}
              windowSec={trendWindow}
              onWindowChange={setTrendWindow}
            />
          )}
        </>,
        true,
      )}
      {phonePage(
        "server",
        <>
          <PageTitle title="Server" caption={serverOn ? `Running on port ${serverPort}` : "Built-in OPC UA server"} />
          {sidebarServer}
          {serverOn && serverItems.length > 0 && (
            <Section title="Nodes" className="oc-bare">
              <WatchList
                items={serverItems}
                history={history}
                trending={trendSeries}
                onOpen={(it) => openNodeEditor(it.nodeId, it.value, it.dataType, true)}
                onToggleTrend={trendItem}
              />
            </Section>
          )}
        </>,
      )}
      {phonePage(
        "namespaces",
        <>
          {activeSession ? (
            phoneTab === "namespaces" && <NamespacePanel key={activeSession.id} session={activeSession.id} onClose={() => setPhoneTab("connect")} onLog={addLog} inline />
          ) : (
            <>
              <PageTitle title="Namespaces" caption="From the server" />
              {needsSession(<Layers size={22} strokeWidth={1.75} />, "read its namespaces")}
            </>
          )}
        </>,
      )}
      {phonePage(
        "log",
        <>
          <PageTitle title="Log" caption="Every request and reply" />
          <div className="vm-fill">
            <LogPanel entries={log} onExport={(f) => addLog("info", `Log exported to ${f}`)} />
          </div>
        </>,
        true,
      )}
      <PhoneTabBar tabs={phoneTabs} active={phoneTab} onChange={setPhoneTab} />
    </>
  );

  return (
    <div className={isMobile ? "vm-app" : undefined} style={isMobile ? undefined : { display: "flex", flexDirection: "column", height: "100vh", background: "var(--bg)", color: "var(--text)" }}>
      <Toolbar
        title="OPC UA Explorer"
        icon={appIcon}
        context={isMobile ? activeSession?.label ?? (serverOn ? `Port ${serverPort}` : undefined) : undefined}
        connected={sessions.length > 0 || serverOn}
        statusLabel={serverOn && sessions.length === 0 ? "Server running" : undefined}
        onDisconnect={() => { sessions.forEach((s) => closeSession(s.id)); if (serverOn) handleServerToggle(); }}
        onInfo={() => setShowInfo(true)}
        error={lastError}
      />

      {isMobile ? (
        phoneLayout
      ) : (
        <div style={{ display: "flex", flex: 1, overflow: "hidden" }}>
          <aside className="vt-sidebar" style={{ width: leftWidth, cursor: dragSide ? "col-resize" : undefined }}>{sidebarContent}</aside>
          <div onMouseDown={(e) => handleDragStart("left", e)}
            style={{ width: 4, cursor: "col-resize", background: dragSide === "left" ? "var(--accent)" : "transparent", flexShrink: 0, zIndex: 10, transition: "background 0.15s" }}
            onMouseEnter={(e) => { if (!dragSide) (e.target as HTMLElement).style.background = "var(--border)"; }}
            onMouseLeave={(e) => { if (!dragSide) (e.target as HTMLElement).style.background = "transparent"; }}
          />
          <main style={{ display: "flex", flex: 1, flexDirection: "column", overflow: "hidden", padding: 8 }}>
            {tableToolbar}
            <div style={{ height: tableHeight, minHeight: 80, overflow: "auto", flexShrink: 0 }}>
              <DataTable columns={columns} data={tableData} onRowClick={openEditor} renderCell={(key, row) => {
                if (key === "nodeId") {
                  return <span className="mono">{String(row[key] ?? "")}</span>;
                }
                if (key === "trend") {
                  const r = row as Record<string, string>;
                  const seriesKey = `${scope}|${r._nodeId}`;
                  const active = trendSeries.has(seriesKey);
                  const browseName = String(r.browseName || r.nodeId);
                  const name = mode === "client" && sessions.length > 1 ? `${scopeLabel}: ${browseName}` : browseName;
                  return (
                    <IconButton
                      size="sm"
                      outline
                      pressed={active}
                      label={`${active ? "Remove" : "Add"} ${name} ${active ? "from" : "to"} trend`}
                      onClick={(e) => { e.stopPropagation(); toggleTrend(seriesKey, name); }}
                    >
                      <ChartLine size={14} strokeWidth={1.75} />
                    </IconButton>
                  );
                }
                return String(row[key] ?? "");
              }} />
            </div>
            <div onMouseDown={(e) => handleDragStart("table", e)}
              style={{ height: 4, cursor: "row-resize", background: dragSide === "table" ? "var(--accent)" : "transparent", flexShrink: 0, zIndex: 10, transition: "background 0.15s" }}
              onMouseEnter={(e) => { if (!dragSide) (e.target as HTMLElement).style.background = "var(--border)"; }}
              onMouseLeave={(e) => { if (!dragSide) (e.target as HTMLElement).style.background = "transparent"; }}
            />
            <div style={{ display: "flex", flexDirection: "column", flex: 1, minHeight: 0 }}>
              <div style={{ padding: "4px 8px", borderBottom: "1px solid var(--border)", flexShrink: 0 }}>
                <Tabs
                  label="Main view"
                  value={view}
                  onChange={setView}
                  items={[
                    { value: "log", label: "Log" },
                    { value: "trend", label: "Trend", count: trendSeries.size > 0 ? trendSeries.size : undefined },
                  ]}
                />
              </div>
              {view === "trend" ? (
                <TrendPanel
                  series={Array.from(trendSeries.values())}
                  onRemove={(nodeId) => setTrendSeries((prev) => { const n = new Map(prev); n.delete(nodeId); return n; })}
                  windowSec={trendWindow}
                  onWindowChange={setTrendWindow}
                />
              ) : (
                <LogPanel entries={log} onExport={(f) => addLog("info", `Log exported to ${f}`)} />
              )}
            </div>
          </main>
        </div>
      )}

      {!isMobile && <StatusBar connected={sessions.length > 0 || serverOn} latency={0} pollCount={0} error={null} />}
      {isMobile && showExport && (
        <Dialog open onClose={() => setShowExport(false)} title="Export to CSV">
          <ListGroup>
            <ListRow icon={<FileDown size={18} strokeWidth={1.75} />} title="Current values" subtitle="One row per watched node" trailing={<ProBadge />} chevron onClick={() => exportCsv("values", "client")} />
            <ListRow
              icon={<FileDown size={18} strokeWidth={1.75} />}
              title="Recorded changes"
              subtitle={valueLog.size === 0 ? "Nothing recorded yet" : `${valueLog.size} samples since the app started`}
              trailing={<ProBadge />}
              chevron={valueLog.size > 0}
              onClick={valueLog.size > 0 ? () => exportCsv("changes", "client") : undefined}
            />
          </ListGroup>
        </Dialog>
      )}
      {editingNode && (
        <ValueEditor
          address={editingNode.id}
          currentValue={editingNode.value !== "—" ? editingNode.value : undefined}
          currentDataType={editingNode.dataType === "Boolean" ? "bool" : "float64"}
          onSubmit={async (value, dataType) => {
            const parsed = dataType === "bool" ? value === "true" || value === "1" : parseFloat(value);
            const val = dataType === "bool" ? parsed : isNaN(parsed as number) ? value : parsed;
            if (editingNode.server) {
              await invoke("server_write_node", { nodeId: editingNode.id, value: val }).catch((e) => addLog("error", String(e)));
              addLog("tx", `Write ${val} to ${editingNode.id} (server)`);
            } else {
              try {
                await invoke("write_tag", { session: activeId, tag: { node_id: editingNode.id, name: "", data_type: editingNode.dataType }, value: { node_id: editingNode.id, name: "", value: val, timestamp: Date.now() } });
              } catch (e) { addLog("error", String(e)); }
              addLog("tx", `Write ${val} to ${editingNode.id} (${scopeLabel})`);
            }
            setEditingNode(null);
          }}
          onCancel={() => setEditingNode(null)}
        />
      )}
      {showInfo && (
        <Dialog
          open
          onClose={() => setShowInfo(false)}
          title="OPC UA Explorer"
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
          <div style={{ fontSize: "var(--fs-sm)", color: "var(--text-dim)", lineHeight: 1.6 }}>
            {isMobile ? (
              <>
                <p style={{ margin: "0 0 10px" }}>
                  <strong style={{ color: "var(--text)" }}>Connect</strong>: Connect to an OPC UA server, browse its address space, watch values and trend them. Pro adds more than 10 live monitored items, several sessions at once, CSV export, trends, writing values and certificate management.
                </p>
                <p style={{ margin: "0 0 10px" }}>
                  <strong style={{ color: "var(--text)" }}>Server</strong> (More tab): Run an OPC UA server on this device. Pro adds device templates that fill it with simulated values.
                </p>
              </>
            ) : (
              <>
            <p style={{ margin: "0 0 10px" }}>
              <strong style={{ color: "var(--text)" }}>Server mode</strong>: Run a local OPC UA server. Pro adds device templates that inject simulated values.
            </p>
            <p style={{ margin: "0 0 10px" }}>
              <strong style={{ color: "var(--text)" }}>Client mode</strong>: Connect to an OPC UA server, browse its address space and read node values. Pro adds more than 10 live monitored items, several sessions at once, CSV export of monitored values, trends, writing values and certificate management.
            </p>
              </>
            )}
            <p style={{ margin: "0 0 12px", color: "var(--text-muted)" }}>
              Part of the <strong style={{ color: "var(--text)" }}>Voltrus</strong> suite by JoyoDigitama
            </p>
            <ProAboutRow />
          </div>
        </Dialog>
      )}
      {showDiscovery && (
        <DiscoveryPanel
          onClose={() => setShowDiscovery(false)}
          onSelect={(url, policy, secMode) => {
            setDiscovered({ endpointUrl: url, securityPolicy: policy, securityMode: secMode });
            setConn((c) => ({ ...c, endpointUrl: url, securityPolicy: normalizePolicy(policy), securityMode: secMode }));
            setConnKey((k) => k + 1);
            addLog("info", `Loaded ${url} (${policy} / ${secMode}) into connection form`);
          }}
          onLog={addLog}
        />
      )}
      {activeSession && showBrowser && <BrowserPanel key={activeSession.id} session={activeSession.id} onClose={() => setShowBrowser(false)} onLog={addLog} />}
      {activeSession && showSubscriptions && <SubscriptionPanel key={activeSession.id} session={activeSession.id} onClose={() => setShowSubscriptions(false)} onLog={addLog} />}
      {showCerts && <CertManager onClose={() => setShowCerts(false)} onLog={addLog} />}
      {activeSession && showNamespaces && <NamespacePanel key={activeSession.id} session={activeSession.id} onClose={() => setShowNamespaces(false)} onLog={addLog} />}
    </div>
  );
}

export default App;
