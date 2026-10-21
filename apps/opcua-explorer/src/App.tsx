import { useState, useCallback, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  Button,
  ConnectionForm,
  DataTable,
  Dialog,
  Field,
  IconButton,
  Section,
  Segmented,
  StatusBar,
  Tabs,
  Toolbar,
  LogPanel,
  ProAboutRow,
  ProBadge,
  ValueEditor,
  useProStatus,
} from "@mactools/ui";
import type { LogEntry } from "@mactools/ui";
import { ChartLine, Play, Server, Plug, Square } from "lucide-react";
import appIcon from "../src-tauri/icons/128x128.png";
import { DiscoveryPanel } from "./DiscoveryPanel";
import { BrowserPanel } from "./BrowserPanel";
import { SubscriptionPanel } from "./SubscriptionPanel";
import { CertManager } from "./CertManager";
import { NamespacePanel } from "./NamespacePanel";
import TrendPanel, { type TrendSeries } from "./TrendPanel";

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
  node_id: string;
  value: any;
  source_ts: number;
  server_ts: number;
  status: string;
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

function App() {
  const [mode, setMode] = useState<"server" | "client">("server");
  const [leftWidth, setLeftWidth] = useState(264);
  const [tableHeight, setTableHeight] = useState(300);
  const [dragSide, setDragSide] = useState<"left" | "table" | null>(null);
  const [mobileTab, setMobileTab] = useState<"connect" | "data" | "log">("connect");
  const [isMobile, setIsMobile] = useState(window.innerWidth < 768);
  const [log, setLog] = useState<LogEntry[]>([]);

  // Server state
  const [serverOn, setServerOn] = useState(false);
  const [serverPort, setServerPort] = useState(4840);
  const [serverNodes, setServerNodes] = useState<any[]>([]);
  const serverPoll = useRef<ReturnType<typeof setInterval> | null>(null);

  // Client state
  const [clientValues, setClientValues] = useState<any[]>([]);
  const [clientConnected, setClientConnected] = useState(false);
  const [clientLoading, setClientLoading] = useState(false);
  const [browsedNodes, setBrowsedNodes] = useState<OpcUaNode[]>([]);
  const clientPoll = useRef<ReturnType<typeof setInterval> | null>(null);
  const mainSubIdRef = useRef<number | null>(null);
  const unlistenMainRef = useRef<(() => void) | null>(null);

  // Template / Sim state
  const [simTemplate, setSimTemplate] = useState<string>("");
  const [simRunning, setSimRunning] = useState(false);
  const [simValues, setSimValues] = useState<number[]>([]);

  const [editingNode, setEditingNode] = useState<{ id: string; value: string; dataType: string } | null>(null);
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

  useEffect(() => {
    const onResize = () => setIsMobile(window.innerWidth < 768);
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  // Poll server nodes when server is on
  useEffect(() => {
    if (!serverOn) { setServerNodes([]); return; }
    const poll = setInterval(async () => {
      try {
        const nodes = await invoke<any[]>("server_read_nodes");
        setServerNodes(nodes ?? []);
      } catch { /* ignore */ }
    }, 500);
    serverPoll.current = poll;
    return () => clearInterval(poll);
  }, [serverOn]);

  // Fallback: poll client values when no live subscription is possible.
  const startClientPoll = useCallback((nodes: OpcUaNode[]) => {
    if (clientPoll.current) clearInterval(clientPoll.current);
    const all = nodes.length > 0 ? nodes : DEFAULT_NODES;
    const nodesToRead = pro ? all : all.slice(0, FREE_MONITORED_ITEMS);
    const doPoll = async () => {
      try {
        const vals = await invoke<any[]>("read_tags", { tags: nodesToRead });
        setClientValues(vals);
      } catch (e) { addLog("error", String(e)); }
    };
    doPoll();
    clientPoll.current = setInterval(doPoll, 1000);
  }, [addLog, pro]);

  // Live updates: subscription notifications drive the main table.
  // Registered once for the app's lifetime (not gated on connection) so there
  // is no race between the async listen() resolution and the first
  // notification burst after a subscription is created.
  const connectedRef = useRef(clientConnected);
  connectedRef.current = clientConnected;
  useEffect(() => {
    let active = true;
    listen<NotificationValue>("opcua://notification", (e) => {
      if (!active) return;
      if (!connectedRef.current) return; // ignore notifications while disconnected
      const n = e.payload;
      setClientValues((prev) => {
        if (!prev.some((row) => row.node_id === n.node_id)) {
          // Unknown node — add it so the row still appears.
          return [...prev, { node_id: n.node_id, name: n.node_id, data_type: "", value: n.value, timestamp: n.server_ts || n.source_ts || 0 }];
        }
        return prev.map((row) => row.node_id === n.node_id ? { ...row, value: n.value, timestamp: n.server_ts || n.source_ts || row.timestamp } : row);
      });
    }).then((fn) => { if (active) unlistenMainRef.current = fn; });
    return () => { active = false; unlistenMainRef.current?.(); unlistenMainRef.current = null; };
  }, []);

  // Cleanup on disconnect: drop live subscription + poll.
  useEffect(() => {
    if (clientConnected) return;
    if (clientPoll.current) { clearInterval(clientPoll.current); clientPoll.current = null; }
    if (mainSubIdRef.current !== null) {
      const id = mainSubIdRef.current;
      mainSubIdRef.current = null;
      invoke("delete_subscription", { subscriptionId: id }).catch(() => {});
    }
    setClientValues([]);
    setBrowsedNodes([]);
  }, [clientConnected]);

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
  const readValues = useCallback(async (nodes: OpcUaNode[]) => {
    try {
      const vals = await invoke<any[]>("read_tags", { tags: nodes });
      if (vals.length > 0) {
        setClientValues(vals.map((b: any, i: number) => ({
          node_id: b?.node_id ?? nodes[i]?.node_id ?? "",
          name: b?.name ?? nodes[i]?.name ?? "",
          data_type: nodes[i]?.data_type ?? "",
          value: b?.value ?? null,
          timestamp: b?.timestamp ?? 0,
        })));
      }
    } catch (e) {
      addLog("error", `Read failed: ${e}`);
    }
  }, [addLog]);

  const handleConnect = useCallback(async (values: Record<string, string | number>) => {
    const config: OpcUaConfig = {
      endpoint_url: String(values.endpointUrl),
      security_policy: String(values.securityPolicy || "None"),
      security_mode: String(values.securityMode || "None"),
      username: String(values.username || ""),
      password: String(values.password || ""),
      poll_interval_ms: Number(values.pollInterval || 1000),
    };
    setClientLoading(true);
    try {
      await invoke("connect", { config });
      setClientConnected(true);

      try {
        const nodes = (await invoke<OpcUaNode[]>("browse_nodes")) ?? [];
        setBrowsedNodes(nodes);
        setClientValues(nodes.map((n) => ({ node_id: n.node_id, name: n.name, data_type: n.data_type, value: null, timestamp: 0 })));
        addLog("info", `Discovered ${nodes.length} nodes via Browse`);
        if (nodes.length > 0) {
          // One Read of every browsed node. Free tier ends here (use "Read values" to refresh); with Pro the
          // live subscription below keeps the table current. Data-change notifications only fire on change,
          // so static nodes would stay blank without this snapshot either way.
          await readValues(nodes);
          const liveNodes = pro ? nodes : nodes.slice(0, FREE_MONITORED_ITEMS);
          {
            try {
              const cfg = {
                publishing_interval_ms: 1000,
                lifetime_count: 60,
                max_keep_alive_count: 10,
                max_notifications_per_publish: 0,
                priority: 0,
                publishing_enabled: true,
              };
              const subId = await invoke<number>("create_subscription", { cfg });
              mainSubIdRef.current = subId;
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
              const created = await invoke<number>("create_monitored_items", { subscriptionId: subId, items });
              addLog("info", `Live subscription ${subId} created for ${created} nodes`);
            } catch (e) {
              addLog("error", `Live subscription failed, falling back to polling: ${e}`);
              startClientPoll(liveNodes);
            }
          }
        }
      } catch (e) {
        addLog("error", `Browse failed: ${e}`);
        setBrowsedNodes([]);
        startClientPoll([]);
      }

      addLog("info", `Connected to ${config.endpoint_url}`);
    } catch (e) {
      addLog("error", `Connection failed: ${e}`);
      setClientConnected(false);
    } finally {
      setClientLoading(false);
    }
  }, [addLog, startClientPoll, readValues, pro]);

  const handleDisconnect = useCallback(async () => {
    setClientLoading(false);
    if (clientPoll.current) { clearInterval(clientPoll.current); clientPoll.current = null; }
    if (mainSubIdRef.current !== null) {
      const id = mainSubIdRef.current;
      mainSubIdRef.current = null;
      try { await invoke("delete_subscription", { subscriptionId: id }); } catch { /* ignore */ }
    }
    try { await invoke("disconnect"); } catch { /* ignore */ }
    setClientConnected(false);
    setClientValues([]);
    addLog("info", "Disconnected");
  }, [addLog]);

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
  const openEditor = useCallback((row: Record<string, unknown>) => {
    if (!requirePro("Writing values")) return;
    setEditingNode({ id: String(row.nodeId), value: String(row.value ?? ""), dataType: String(row.dataType ?? "Double") });
  }, [requirePro]);

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
      ? serverNodes.map((n: any) => ({
          nodeId: n.node_id ?? "",
          browseName: n.name ?? "",
          dataType: n.data_type ?? "",
          value: n.value !== null && n.value !== undefined ? String(n.value) : "-",
          _nodeId: n.node_id ?? "",
        }))
    : mode === "client" && clientValues.length > 0
      ? clientValues.map((v: any, i: number) => ({
          nodeId: v?.node_id ?? activeNodes[i]?.node_id ?? "",
          browseName: v?.name ?? activeNodes[i]?.name ?? "",
          dataType: v?.data_type ?? activeNodes[i]?.data_type ?? "",
          value: v?.value !== null && v?.value !== undefined ? String(v?.value) : "-",
          _nodeId: v?.node_id ?? activeNodes[i]?.node_id ?? "",
        }))
      : activeNodes.map((n) => ({
          nodeId: n.node_id,
          browseName: n.name,
          dataType: n.data_type,
          value: "\u2014",
          _nodeId: n.node_id,
        }));

  // Append a sample to every tracked series on each value change.
  const trendRows = tableData.map((r) => ({ nodeId: r._nodeId, name: r.browseName, v: Number(r.value) }));
  const trendRowsRef = useRef(trendRows);
  trendRowsRef.current = trendRows;
  const valueSig = trendRows.map((r) => r.nodeId + ":" + r.v).join("|");

  useEffect(() => {
    const active = trendSeriesRef.current;
    if (active.size === 0) return;
    const ts = Date.now();
    const byName = new Map(trendRowsRef.current.map((r) => [r.nodeId, r.v]));
    const cutoff = ts - trendWindow * 1000 - 5000;
    const next = new Map(active);
    let changed = false;
    for (const [nodeId, s] of next) {
      const v = byName.get(nodeId);
      if (v === undefined || Number.isNaN(v)) continue;
      const points = [...s.points, { t: ts, v }].filter((p) => p.t > cutoff).slice(-500);
      next.set(nodeId, { ...s, points });
      changed = true;
    }
    if (changed) setTrendSeries(next);
  }, [valueSig, trendWindow]);

  const toggleTrend = useCallback((nodeId: string, name: string) => {
    // Adding a series is Pro; removing one never is.
    if (!trendSeriesRef.current.has(nodeId) && !requirePro("Live trends")) return;
    setTrendSeries((prev) => {
      const next = new Map(prev);
      if (next.has(nodeId)) {
        next.delete(nodeId);
      } else {
        const v = trendRowsRef.current.find((r) => r.nodeId === nodeId)?.v ?? 0;
        const colors = ["#3b82f6", "#ef4444", "#22c55e", "#f59e0b", "#a855f7", "#ec4899", "#14b8a6", "#f97316"];
        next.set(nodeId, { name, color: colors[next.size % colors.length], points: [{ t: Date.now(), v: Number.isNaN(v) ? 0 : v }] });
      }
      return next;
    });
  }, [requirePro]);

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
                <select
                  id={id}
                  className="vt-select"
                  value={simTemplate}
                  onChange={(e) => { setSimTemplate(e.target.value); if (simRunning) handleSimToggle(); }}
                >
                  <option value="">Select template</option>
                  {Object.entries(PROFILES).map(([k, p]) => (
                    <option key={k} value={k}>{p.name}</option>
                  ))}
                </select>
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
          <Section
            title="Connection"
            actions={
              <Button size="sm" variant="ghost" disabled={clientConnected} onClick={() => setShowDiscovery(true)}>
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
              onDisconnect={handleDisconnect}
              onCancel={() => setClientLoading(false)}
              connected={clientConnected}
              loading={clientLoading}
            />
          </Section>

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

          {clientConnected && (
            <Section title="Session">
              <span className="vt-hint">
                Connected to {browsedNodes.length > 0 ? `${browsedNodes.length} browsed nodes` : `${DEFAULT_NODES.length} default nodes`}.{" "}
                {pro ? "Values update live. Click any row to write a value." : `Live updates for the first ${Math.min(FREE_MONITORED_ITEMS, browsedNodes.length || DEFAULT_NODES.length)} nodes (Pro for more). Use Read values to refresh the rest. Click any row to write a value (Pro).`}
              </span>
              <Button size="sm" disabled={browsedNodes.length === 0} onClick={() => readValues(browsedNodes)}>Read values</Button>
            </Section>
          )}
        </>
      )}
    </>
  );

  // ── Mobile tab bar ──
  const mobileTabBar = (
    <nav style={{ display: "flex", borderTop: "1px solid var(--border)", background: "var(--card)", flexShrink: 0, paddingBottom: "env(safe-area-inset-bottom, 0px)" }}>
      {([{ id: "connect" as const, label: mode === "server" ? "Server" : "Connect" }, { id: "data" as const, label: "Data" }, { id: "log" as const, label: "Log" }] as const).map((tab) => (
        <button key={tab.id} onClick={() => setMobileTab(tab.id)}
          style={{ flex: 1, padding: "10px 0 8px", fontSize: 11, fontWeight: 600, background: "none", border: "none",
            color: mobileTab === tab.id ? "var(--accent)" : "var(--text-muted)",
            borderTop: mobileTab === tab.id ? "2px solid var(--accent)" : "2px solid transparent", cursor: "pointer" }}
        >{tab.label}</button>
      ))}
    </nav>
  );

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100vh", background: "var(--bg)", color: "var(--text)" }}>
      <Toolbar title="OPC UA Explorer" icon={appIcon} connected={clientConnected || serverOn} onDisconnect={() => { if (clientConnected) handleDisconnect(); if (serverOn) handleServerToggle(); }} onInfo={() => setShowInfo(true)} />

      {isMobile ? (
        <div style={{ flex: 1, overflow: "hidden", display: "flex", flexDirection: "column" }}>
          <div style={{ flex: 1, overflow: "auto", padding: 16 }}>
            {mobileTab === "connect" && <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>{sidebarContent}</div>}
            {mobileTab === "data" && <DataTable columns={columns} data={tableData} onRowClick={openEditor} />}
            {mobileTab === "log" && <LogPanel entries={log} onExport={(f) => addLog("info", `Log exported to ${f}`)} />}
          </div>
          {mobileTabBar}
          <StatusBar connected={clientConnected || serverOn} latency={0} pollCount={0} error={null} />
        </div>
      ) : (
        <div style={{ display: "flex", flex: 1, overflow: "hidden" }}>
          <aside className="vt-sidebar" style={{ width: leftWidth, cursor: dragSide ? "col-resize" : undefined }}>{sidebarContent}</aside>
          <div onMouseDown={(e) => handleDragStart("left", e)}
            style={{ width: 4, cursor: "col-resize", background: dragSide === "left" ? "var(--accent)" : "transparent", flexShrink: 0, zIndex: 10, transition: "background 0.15s" }}
            onMouseEnter={(e) => { if (!dragSide) (e.target as HTMLElement).style.background = "var(--border)"; }}
            onMouseLeave={(e) => { if (!dragSide) (e.target as HTMLElement).style.background = "transparent"; }}
          />
          <main style={{ display: "flex", flex: 1, flexDirection: "column", overflow: "hidden", padding: 8 }}>
            <div style={{ height: tableHeight, minHeight: 80, overflow: "auto", flexShrink: 0 }}>
              <DataTable columns={columns} data={tableData} onRowClick={openEditor} renderCell={(key, row) => {
                if (key === "nodeId") {
                  return <span className="mono">{String(row[key] ?? "")}</span>;
                }
                if (key === "trend") {
                  const r = row as Record<string, string>;
                  const active = trendSeries.has(String(r._nodeId));
                  const name = String(r.browseName || r.nodeId);
                  return (
                    <IconButton
                      size="sm"
                      outline
                      pressed={active}
                      label={`${active ? "Remove" : "Add"} ${name} ${active ? "from" : "to"} trend`}
                      onClick={(e) => { e.stopPropagation(); toggleTrend(String(r._nodeId), name); }}
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

      {!isMobile && <StatusBar connected={clientConnected || serverOn} latency={0} pollCount={0} error={null} />}
      {editingNode && (
        <ValueEditor
          address={editingNode.id}
          currentValue={editingNode.value !== "—" ? editingNode.value : undefined}
          currentDataType={editingNode.dataType === "Boolean" ? "bool" : "float64"}
          onSubmit={async (value, dataType) => {
            const parsed = dataType === "bool" ? value === "true" || value === "1" : parseFloat(value);
            const val = dataType === "bool" ? parsed : isNaN(parsed as number) ? value : parsed;
            if (mode === "server") {
              await invoke("server_write_node", { nodeId: editingNode.id, value: val }).catch((e) => addLog("error", String(e)));
              addLog("tx", `Write ${val} to ${editingNode.id} (server)`);
            } else {
              try {
                await invoke("write_tag", { tag: { node_id: editingNode.id, name: "", data_type: editingNode.dataType }, value: { node_id: editingNode.id, name: "", value: val, timestamp: Date.now() } });
              } catch (e) { addLog("error", String(e)); }
              addLog("tx", `Write ${val} to ${editingNode.id}`);
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
            <p style={{ margin: "0 0 10px" }}>
              <strong style={{ color: "var(--text)" }}>Server mode</strong>: Run a local OPC UA server. Pro adds device templates that inject simulated values.
            </p>
            <p style={{ margin: "0 0 10px" }}>
              <strong style={{ color: "var(--text)" }}>Client mode</strong>: Connect to an OPC UA server, browse its address space and read node values. Pro adds live subscriptions, trends, writing values and certificate management.
            </p>
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
            setConnKey((k) => k + 1);
            addLog("info", `Loaded ${url} (${policy} / ${secMode}) into connection form`);
          }}
          onLog={addLog}
        />
      )}
      {showBrowser && <BrowserPanel onClose={() => setShowBrowser(false)} onLog={addLog} />}
      {showSubscriptions && <SubscriptionPanel onClose={() => setShowSubscriptions(false)} onLog={addLog} />}
      {showCerts && <CertManager onClose={() => setShowCerts(false)} onLog={addLog} />}
      {showNamespaces && <NamespacePanel onClose={() => setShowNamespaces(false)} onLog={addLog} />}
    </div>
  );
}

export default App;
