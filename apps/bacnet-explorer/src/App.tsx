import { useState, useCallback, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Bell, ExternalLink, LineChart, ListTree, Play, Plug, Radar, ScrollText, Square } from "lucide-react";
import appIcon from "../src-tauri/icons/128x128.png";
import {
  Button,
  ConnectionForm,
  DataTable,
  Dialog,
  EmptyState,
  Field,
  Section,
  StatusBar,
  Tabs,
  Toolbar,
  ProBadge,
  ProAboutRow,
  useProStatus,
  LogPanel,
  ValueEditor,
  useProtocol,
} from "@mactools/ui";
import type { LogEntry } from "@mactools/ui";
import type { BacnetConfig, BacnetObject, BacnetValue, BacnetDiscoveredDevice, SimPoint, SimPointDef, SimInfo, DeviceTemplate } from "./types";
import { bacnetValueToNumber } from "./types";
import { WRITE_KIND_TYPES, parseWriteValue, writeKindFor, type WriteKind } from "./writeValue";
import { ObjectBrowser } from "./components/ObjectBrowser";
import { DevicePanel } from "./components/DevicePanel";
import { MobileTabBar, type PhoneTab, type PhoneTabItem } from "./components/MobileTabBar";
import { AlarmPanel } from "./components/AlarmPanel";
import TrendPanel from "./components/TrendPanel";
import { trendKey, useBacnetTrend } from "./hooks/useBacnetTrend";
import { TrendLogDialog } from "./components/TrendLogDialog";
import { ScheduleDialog } from "./components/ScheduleDialog";

const TEMPLATES: Record<string, DeviceTemplate> = {
  ahu: {
    name: "AHU-1 (Air Handling Unit)",
    description: "Typical rooftop AHU with supply/return temp, fan, damper, filter",
    points: [
      { key: "supply_temp", label: "SupplyTemp", object_type: "analogInput", object_instance: 1, property: "presentValue", base: 22.5, variance: 0.4, noise: 0.1, period: 30000, min: 18, max: 28, decimals: 1, unit: "°C" },
      { key: "return_temp", label: "ReturnTemp", object_type: "analogInput", object_instance: 2, property: "presentValue", base: 24.1, variance: 0.3, noise: 0.1, period: 35000, min: 20, max: 28, decimals: 1, unit: "°C" },
      { key: "outdoor_temp", label: "OutdoorTemp", object_type: "analogInput", object_instance: 3, property: "presentValue", base: 18.3, variance: 0.2, noise: 0.05, period: 60000, min: 10, max: 25, decimals: 1, unit: "°C" },
      { key: "supply_pressure", label: "SupplyPressure", object_type: "analogInput", object_instance: 4, property: "presentValue", base: 245, variance: 8, noise: 2, period: 20000, min: 200, max: 300, decimals: 0, unit: "Pa" },
      { key: "fan_speed", label: "FanSpeed", object_type: "analogInput", object_instance: 5, property: "presentValue", base: 1450, variance: 15, noise: 5, period: 25000, min: 1400, max: 1500, decimals: 0, unit: "RPM" },
      { key: "fan_status", label: "FanStatus", object_type: "binaryInput", object_instance: 1, property: "presentValue", base: 1, variance: 0, noise: 0, period: 100000, isBool: true },
      { key: "filter_status", label: "FilterOK", object_type: "binaryInput", object_instance: 2, property: "presentValue", base: 1, variance: 0, noise: 0, period: 100000, isBool: true },
      { key: "setpoint", label: "TempSetpoint", object_type: "analogValue", object_instance: 1, property: "presentValue", base: 23.0, variance: 0, noise: 0, period: 100000, decimals: 1, unit: "°C" },
      { key: "damper_pos", label: "DamperPosition", object_type: "analogValue", object_instance: 2, property: "presentValue", base: 45, variance: 5, noise: 1, period: 40000, min: 0, max: 100, decimals: 1, unit: "%" },
    ],
  },
  chiller: {
    name: "Chiller-1 (Centrifugal)",
    description: "Centrifugal chiller with chilled/condenser water, compressor",
    points: [
      { key: "chw_supply", label: "CHW_Supply", object_type: "analogInput", object_instance: 1, property: "presentValue", base: 7.2, variance: 0.3, noise: 0.1, period: 25000, min: 5, max: 10, decimals: 1, unit: "°C" },
      { key: "chw_return", label: "CHW_Return", object_type: "analogInput", object_instance: 2, property: "presentValue", base: 12.8, variance: 0.4, noise: 0.1, period: 30000, min: 10, max: 16, decimals: 1, unit: "°C" },
      { key: "cw_supply", label: "CW_Supply", object_type: "analogInput", object_instance: 3, property: "presentValue", base: 32.1, variance: 0.5, noise: 0.2, period: 35000, min: 28, max: 36, decimals: 1, unit: "°C" },
      { key: "cw_return", label: "CW_Return", object_type: "analogInput", object_instance: 4, property: "presentValue", base: 37.4, variance: 0.5, noise: 0.2, period: 35000, min: 34, max: 42, decimals: 1, unit: "°C" },
      { key: "compressor_kw", label: "CompressorkW", object_type: "analogInput", object_instance: 5, property: "presentValue", base: 185, variance: 8, noise: 3, period: 20000, min: 150, max: 220, decimals: 1, unit: "kW" },
      { key: "compressor_status", label: "CompressorRun", object_type: "binaryInput", object_instance: 1, property: "presentValue", base: 1, variance: 0, noise: 0, period: 100000, isBool: true },
      { key: "alarm", label: "Alarm", object_type: "binaryInput", object_instance: 2, property: "presentValue", base: 0, variance: 0, noise: 0, period: 100000, isBool: true },
      { key: "cop", label: "COP", object_type: "analogValue", object_instance: 1, property: "presentValue", base: 4.2, variance: 0.15, noise: 0.05, period: 30000, min: 3.5, max: 5.0, decimals: 2 },
      { key: "chw_setpoint", label: "CHW_Setpoint", object_type: "analogValue", object_instance: 2, property: "presentValue", base: 7.0, variance: 0, noise: 0, period: 100000, decimals: 1, unit: "°C" },
    ],
  },
  vav: {
    name: "VAV-101 (Variable Air Volume)",
    description: "Zone-level VAV box with temperature, airflow, occupancy",
    points: [
      { key: "zone_temp", label: "ZoneTemp", object_type: "analogInput", object_instance: 1, property: "presentValue", base: 23.4, variance: 0.3, noise: 0.1, period: 25000, min: 20, max: 28, decimals: 1, unit: "°C" },
      { key: "airflow", label: "Airflow", object_type: "analogInput", object_instance: 2, property: "presentValue", base: 320, variance: 15, noise: 5, period: 20000, min: 100, max: 600, decimals: 0, unit: "CFM" },
      { key: "discharge_temp", label: "DischargeAirTemp", object_type: "analogInput", object_instance: 3, property: "presentValue", base: 14.2, variance: 0.5, noise: 0.2, period: 30000, min: 10, max: 20, decimals: 1, unit: "°C" },
      { key: "occupancy", label: "Occupancy", object_type: "binaryInput", object_instance: 1, property: "presentValue", base: 1, variance: 0, noise: 0, period: 100000, isBool: true },
      { key: "setpoint", label: "TempSetpoint", object_type: "analogValue", object_instance: 1, property: "presentValue", base: 24.0, variance: 0, noise: 0, period: 100000, decimals: 1, unit: "°C" },
      { key: "min_airflow", label: "MinAirflow", object_type: "analogValue", object_instance: 2, property: "presentValue", base: 150, variance: 0, noise: 0, period: 100000, decimals: 0, unit: "CFM" },
      { key: "damper", label: "DamperPosition", object_type: "analogOutput", object_instance: 1, property: "presentValue", base: 45, variance: 5, noise: 2, period: 20000, min: 0, max: 100, decimals: 1, unit: "%" },
    ],
  },
};

// BACnet command/output object types are writable; inputs are sensors (read-only).
const SIM_WRITABLE_TYPES: Record<string, true> = {
  analogOutput: true,
  analogValue: true,
  binaryOutput: true,
  binaryValue: true,
  multiStateOutput: true,
  multiStateValue: true,
};

function templateToSimPoints(t: DeviceTemplate): SimPointDef[] {
  return t.points.map((p) => ({
    key: p.key,
    label: p.label,
    object_type: p.object_type,
    object_instance: p.object_instance,
    base: p.base,
    variance: p.variance,
    noise: p.noise,
    period: p.period,
    min: p.min,
    max: p.max,
    decimals: p.decimals ?? 0,
    unit: p.unit,
    is_bool: p.isBool ?? false,
    writable: SIM_WRITABLE_TYPES[p.object_type] === true,
  }));
}

type EditTarget = { kind: "sim"; point: SimPoint } | { kind: "device"; tag: BacnetObject };

function editTargetKind(t: EditTarget): WriteKind {
  if (t.kind === "sim") return t.point.isBool ? "bool" : writeKindFor(t.point.object_type);
  return writeKindFor(t.tag.object_type);
}

// ── Shared styles ──

function App() {
  const protocol = useProtocol<BacnetConfig, BacnetObject, unknown>("bacnet");
  const [log, setLog] = useState<LogEntry[]>([]);
  const addLog = useCallback((direction: LogEntry["direction"], message: string) => {
    setLog((prev) => [...prev, { timestamp: Date.now(), direction, message }]);
  }, []);
  const trend = useBacnetTrend(protocol.connected, (message) => addLog("error", message));
  const trendKeys = new Set(trend.series.map((s) => s.key));
  const { requirePro, status: proStatus } = useProStatus();
  // Adding a series is Pro; removing one never is.
  const toggleTrend = (obj: { type: string; instance: number }, name: string) => {
    if (!trendKeys.has(trendKey(obj.type, obj.instance)) && !requirePro("Live trends")) return;
    trend.toggle(obj, name);
  };

  // Trend log and schedule viewers are Pro.
  const openTrendLog = (obj: { type: "trendLog" | "trendLogMultiple"; instance: number }) => {
    if (!requirePro("Trend log viewer")) return;
    setTrendLogObject(obj);
  };
  const openSchedule = (instance: number) => {
    if (!requirePro("Schedule viewer")) return;
    setScheduleObject(instance);
  };

  // ── State ──
  const [objects] = useState<BacnetObject[]>([
    { name: "SupplyTemp", object_type: "analogInput", object_instance: 1, property: "presentValue" },
    { name: "ReturnTemp", object_type: "analogInput", object_instance: 2, property: "presentValue" },
    { name: "FanStatus", object_type: "binaryInput", object_instance: 1, property: "presentValue" },
    { name: "Setpoint", object_type: "analogValue", object_instance: 1, property: "presentValue" },
    { name: "DamperCmd", object_type: "analogOutput", object_instance: 1, property: "presentValue" },
  ]);
  const [editingObject, setEditingObject] = useState<string | null>(null);
  const [showInfo, setShowInfo] = useState(false);
  const [discoveredDevices, setDiscoveredDevices] = useState<BacnetDiscoveredDevice[]>([]);
  const [showDiscovery, setShowDiscovery] = useState(false);
  const [discovering, setDiscovering] = useState(false);
  const [connectingDevice, setConnectingDevice] = useState<number | null>(null);
  const [connectedDevice, setConnectedDevice] = useState<{ ip: string; port: number; device_id: number } | null>(null);

  // View mode: "browse" (object browser w/ trend) or "alarms"
  const [viewMode, setViewMode] = useState<"browse" | "alarms">("browse");

  // Dialogs
  const [trendLogObject, setTrendLogObject] = useState<{ type: "trendLog" | "trendLogMultiple"; instance: number } | null>(null);
  const [scheduleObject, setScheduleObject] = useState<number | null>(null);

  // Resizable panels
  const [leftWidth, setLeftWidth] = useState(264);
  const [tableHeight, setTableHeight] = useState(300);
  // Touch controls are taller (and the drag handle is mouse-only), so the touch default leaves the chart room.
  const [trendHeight, setTrendHeight] = useState(window.matchMedia("(pointer: coarse)").matches ? 340 : 220);
  const [dragSide, setDragSide] = useState<"left" | "table" | "trend" | null>(null);

  // Phone layout (< 768px): bottom tabs. Wider touch screens (iPad) keep the two-pane layout with touch-size controls.
  const [phoneTab, setPhoneTab] = useState<PhoneTab>("devices");
  const [viewportWidth, setViewportWidth] = useState(window.innerWidth);
  const isMobile = viewportWidth < 768;
  const isTouch = useRef(window.matchMedia("(pointer: coarse)").matches).current;
  const [alarmsOpened, setAlarmsOpened] = useState(false);
  const [platform, setPlatform] = useState("");
  // iOS has a single window: the simulator control panel opens as a full-screen sheet instead.
  const inlinePanel = platform === "ios" || isMobile;
  const [showSimPanel, setShowSimPanel] = useState(false);
  const [discoveryError, setDiscoveryError] = useState<string | null>(null);

  // Simulation (real BACnet device simulator on the backend)
  const [simTemplate, setSimTemplate] = useState<string | null>(null);
  const [simRunning, setSimRunning] = useState(false);
  const [simValues, setSimValues] = useState<Record<string, string>>({});
  const [simDeviceId, setSimDeviceId] = useState(100);
  const [simPort, setSimPort] = useState(47808);
  const [simBoundPort, setSimBoundPort] = useState<number | null>(null);

  // ── Callbacks ──
  const handleConnect = useCallback(
    async (values: Record<string, string | number>): Promise<boolean> => {
      const config: BacnetConfig = { id: `${values.ip}:${values.deviceId}`, ip: String(values.ip), port: Number(values.port), device_id: Number(values.deviceId) };
      addLog("info", `Connecting to ${config.ip}:${config.port} (device ${config.device_id})…`);
      const ok = await protocol.connect(config);
      if (ok) {
        addLog("info", "Connected. Use Browse Objects to explore the device.");
        setViewMode("browse");
        setConnectedDevice({ ip: config.ip, port: config.port, device_id: config.device_id });
        if (isMobile) setPhoneTab("objects");
      } else {
        addLog("error", protocol.error ? `Connection failed: ${protocol.error}` : "Connection failed (no BACnet response within 3s).");
      }
      return ok;
    },
    [protocol, addLog, isMobile],
  );

  const handleDisconnect = useCallback(() => {
    protocol.disconnect();
    protocol.stopPolling();
    setViewMode("browse");
    setConnectedDevice(null);
    setAlarmsOpened(false);
    setPhoneTab("devices");
    addLog("info", "Disconnected");
  }, [protocol, addLog]);

  const handleDiscover = useCallback(async () => {
    addLog("info", "Broadcasting Who-Is…");
    setDiscovering(true);
    setDiscoveredDevices([]);
    setDiscoveryError(null);
    setShowDiscovery(true);
    try {
      const devices = await invoke<BacnetDiscoveredDevice[]>("discover", { timeoutSecs: 5 });
      setDiscoveredDevices(devices);
      addLog("info", `Discovered ${devices.length} device(s)`);
    } catch (e) {
      setDiscoveryError(String(e));
      addLog("error", `Discovery failed: ${e}`);
    } finally {
      setDiscovering(false);
    }
  }, [addLog]);

  // ── Panel resize handlers ──
  const dragStartRef = useRef({ startX: 0, startY: 0, startWidth: 0, startHeight: 0 });

  const handleDragStart = useCallback((side: "left" | "table" | "trend", e: React.MouseEvent) => {
    e.preventDefault();
    dragStartRef.current = {
      startX: e.clientX,
      startY: e.clientY,
      startWidth: side === "left" ? leftWidth : 0,
      startHeight: side === "table" ? tableHeight : side === "trend" ? trendHeight : 0,
    };
    setDragSide(side);
  }, [leftWidth, tableHeight, trendHeight]);

  useEffect(() => {
    if (!dragSide) return;
    const onMove = (e: MouseEvent) => {
      if (dragSide === "left") {
        const dx = e.clientX - dragStartRef.current.startX;
        setLeftWidth(Math.max(180, Math.min(400, dragStartRef.current.startWidth + dx)));
      } else if (dragSide === "table") {
        const dy = e.clientY - dragStartRef.current.startY;
        setTableHeight(Math.max(80, dragStartRef.current.startHeight + dy));
      } else if (dragSide === "trend") {
        // Drag up (-dy) grows the trend panel.
        const dy = e.clientY - dragStartRef.current.startY;
        setTrendHeight(Math.max(120, Math.min(560, dragStartRef.current.startHeight - dy)));
      }
    };
    const onUp = () => setDragSide(null);
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    return () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
  }, [dragSide]);

  // ── Viewport width (phone vs. wider layouts) and host platform ──
  useEffect(() => {
    const onResize = () => setViewportWidth(window.innerWidth);
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  useEffect(() => {
    invoke<string>("platform").then(setPlatform).catch(() => {});
  }, []);

  // ── Error logging ──
  useEffect(() => {
    if (protocol.error) addLog("error", protocol.error);
  }, [protocol.error, addLog]);

  // ── Simulation: receive live values from the backend device ──
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    listen<{
      port: number;
      points: { key: string; display: string }[];
    }>("sim-tick", (e) => {
      const vals: Record<string, string> = {};
      for (const p of e.payload.points) vals[p.key] = p.display;
      setSimValues(vals);
      setSimBoundPort(e.payload.port);
    }).then((f) => { unlisten = f; });
    return () => { if (unlisten) unlisten(); };
  }, []);

  const markSimStopped = useCallback(() => {
    setSimRunning(false);
    setSimValues({});
    setSimBoundPort(null);
    setShowSimPanel(false);
    addLog("info", "Simulation stopped");
  }, [addLog]);

  // The simulator can be stopped from the control-panel window as well.
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    listen("sim-stopped", () => markSimStopped()).then((f) => { unlisten = f; });
    return () => { if (unlisten) unlisten(); };
  }, [markSimStopped]);

  const openDevicePanel = useCallback(async () => {
    if (inlinePanel) {
      setShowSimPanel(true);
      return;
    }
    try {
      await invoke("open_device_panel");
    } catch (e) {
      addLog("error", `Failed to open control panel: ${e}`);
    }
  }, [addLog, inlinePanel]);

  const handleSimStart = useCallback(async () => {
    if (!simTemplate || !requirePro("Device simulator")) return;
    try {
      const points = templateToSimPoints(TEMPLATES[simTemplate]);
      const info = await invoke<SimInfo>("sim_start", {
        points,
        deviceId: simDeviceId,
        port: simPort,
      });
      setSimBoundPort(info.port);
      setSimRunning(true);
      addLog("info", `Simulation started: ${TEMPLATES[simTemplate].name} (device ${info.device_id}, UDP ${info.port})`);
      if (inlinePanel) setShowSimPanel(true);
      else await invoke("open_device_panel");
    } catch (e) {
      addLog("error", `Simulation start failed: ${e}`);
    }
  }, [simTemplate, simDeviceId, simPort, addLog, requirePro, inlinePanel]);

  // sim_stop emits "sim-stopped" (also when the control-panel window stops the
  // simulator), which resets this window's state in the listener above.
  const handleSimStop = useCallback(async () => {
    try {
      await invoke("sim_stop");
    } catch (e) {
      addLog("error", `Simulation stop failed: ${e}`);
      try {
        if (!(await invoke<boolean>("sim_status"))) markSimStopped();
      } catch (statusError) {
        addLog("error", `Could not read simulator status: ${statusError}`);
      }
    }
  }, [addLog, markSimStopped]);

  // The polling table shows the simulator's template points while it runs, or the
  // placeholder tag list. Resolve a clicked row to something that can be written.
  const resolveEditTarget = useCallback(
    (name: string): EditTarget | null => {
      if (simRunning && simTemplate) {
        const point = TEMPLATES[simTemplate].points.find((p) => p.label === name);
        return point ? { kind: "sim", point } : null;
      }
      const tag = objects.find((o) => o.name === name);
      return tag ? { kind: "device", tag } : null;
    },
    [simRunning, simTemplate, objects],
  );

  const openRowEditor = useCallback(
    (name: string) => {
      const target = resolveEditTarget(name);
      if (!target) {
        addLog("error", `Cannot write "${name}": row does not map to a known object`);
        return;
      }
      if (target.kind === "sim" && SIM_WRITABLE_TYPES[target.point.object_type] !== true) {
        addLog("error", `Cannot write "${name}": ${target.point.object_type} is read-only`);
        return;
      }
      if (!requirePro("Writing properties")) return;
      setEditingObject(name);
    },
    [resolveEditTarget, addLog, requirePro],
  );

  const handleTableWrite = useCallback(
    async (name: string, input: string) => {
      const target = resolveEditTarget(name);
      setEditingObject(null);
      if (!target) {
        addLog("error", `Write to "${name}" dropped: row does not map to a known object`);
        return;
      }
      let value: BacnetValue;
      try {
        value = parseWriteValue(editTargetKind(target), input);
      } catch (e) {
        addLog("error", `Write to ${name} rejected: ${e instanceof Error ? e.message : String(e)}`);
        return;
      }
      addLog("tx", `Write ${input} to ${name}`);
      try {
        if (target.kind === "sim") {
          if ("Enum" in value) {
            await invoke("sim_set_bool", { key: target.point.key, active: value.Enum === 1 });
          } else {
            await invoke("sim_set_value", { key: target.point.key, value: bacnetValueToNumber(value) });
          }
        } else {
          await invoke("write_tag", { tag: target.tag, value });
          protocol.readTags(objects);
        }
      } catch (e) {
        addLog("error", `Write to ${name} failed: ${e}`);
      }
    },
    [resolveEditTarget, addLog, protocol, objects],
  );

  // ── Table config (polling mode) ──
  const columns = [
    { key: "name", label: "Name", width: "120px" },
    { key: "objectType", label: "Object Type", width: "120px" },
    { key: "instance", label: "Instance", width: "80px" },
    { key: "property", label: "Property", width: "100px" },
    { key: "value", label: "Value" },
  ];

  const tableData = simRunning && simTemplate
    ? TEMPLATES[simTemplate].points.map((p) => ({
        name: p.label,
        objectType: p.object_type,
        instance: p.object_instance,
        property: p.property,
        value: simValues[p.key] ?? "—",
      }))
    : protocol.values.length > 0
      ? protocol.values.map((v: any, i: number) => ({
          name: v?.name ?? objects[i]?.name ?? "",
          objectType: v?.object_type ?? objects[i]?.object_type ?? "",
          instance: v?.object_instance ?? objects[i]?.object_instance ?? i,
          property: v?.property ?? objects[i]?.property ?? "",
          value: v?.formatted ?? "-",
        }))
      : objects.map((o) => ({
          name: o.name,
          objectType: o.object_type,
          instance: o.object_instance,
          property: o.property,
          value: "—",
        }));

  // ── Sidebar content (shared between desktop and mobile) ──
  const sidebarContent = (
    <>
      <Section title="Connection">
        <ConnectionForm
          fields={[
            { name: "ip", label: "IP address", type: "text", default: "192.168.1.10" },
            { name: "port", label: "Port", type: "number", default: 47808 },
            { name: "deviceId", label: "Device ID", type: "number", default: 100 },
          ]}
          onSubmit={handleConnect}
          onDisconnect={handleDisconnect}
          connected={protocol.connected}
          loading={protocol.loading}
          initialValues={
            connectedDevice
              ? { ip: connectedDevice.ip, port: connectedDevice.port, deviceId: connectedDevice.device_id }
              : undefined
          }
        />
        {protocol.connected && !isMobile && (
          <Tabs
            label="View"
            value={viewMode}
            onChange={(v) => { if (v === "alarms" && !requirePro("Alarms and events")) return; setViewMode(v); }}
            items={[
              { value: "browse" as const, label: "Browse" },
              { value: "alarms" as const, label: <>Alarms <ProBadge /></> },
            ]}
          />
        )}
      </Section>
      <Section title="Devices" meta={discoveredDevices.length > 0 ? `${discoveredDevices.length} found` : undefined}>
        <Button block icon={<Radar size={14} strokeWidth={1.75} />} loading={discovering} onClick={handleDiscover}>
          Discover devices
        </Button>
      </Section>
      <Section title="Simulation" meta={<ProBadge />}>
        <span className="vt-hint">
          Runs a virtual BACnet device on this {inlinePanel ? "device" : "machine"} so you can test browsing, reads and writes without hardware.
        </span>
        <Field
          label="Template"
          hint={simTemplate ? TEMPLATES[simTemplate].description : undefined}
        >
          <select
            className="vt-select"
            value={simTemplate ?? ""}
            onChange={(e) => { setSimTemplate(e.target.value || null); setSimRunning(false); setSimValues({}); }}
          >
            <option value="">Select template&#x2026;</option>
            {Object.entries(TEMPLATES).map(([id, t]) => (
              <option key={id} value={id}>{t.name}</option>
            ))}
          </select>
        </Field>
        {simTemplate && (
          <>
            <div className="vt-field-row">
              <Field label="Device ID">
                <input
                  className="vt-input"
                  type="number"
                  value={simDeviceId}
                  disabled={simRunning}
                  onChange={(e) => setSimDeviceId(Number(e.target.value))}
                />
              </Field>
              <Field label="UDP port">
                <input
                  className="vt-input"
                  type="number"
                  value={simPort}
                  disabled={simRunning}
                  onChange={(e) => setSimPort(Number(e.target.value))}
                />
              </Field>
            </div>
            {!simRunning ? (
              <Button variant="primary" block icon={<Play size={14} strokeWidth={1.75} />} onClick={handleSimStart}>
                Start simulation
              </Button>
            ) : (
              <>
                {inlinePanel && !protocol.connected && simBoundPort != null && (
                  <Button
                    variant="primary"
                    block
                    icon={<Plug size={14} strokeWidth={1.75} />}
                    onClick={() => handleConnect({ ip: "127.0.0.1", port: simBoundPort, deviceId: simDeviceId })}
                  >
                    Connect to simulator
                  </Button>
                )}
                <Button block icon={<ExternalLink size={14} strokeWidth={1.75} />} onClick={openDevicePanel}>
                  Open control panel
                </Button>
                <Button variant="danger" block icon={<Square size={14} strokeWidth={1.75} />} onClick={handleSimStop}>
                  Stop simulation
                </Button>
              </>
            )}
            {simRunning && simBoundPort != null && (
              <span className="vt-hint">
                Listening on UDP <span className="mono">:{simBoundPort}</span>. Discover from YABE with Who-Is.
              </span>
            )}
          </>
        )}
      </Section>
    </>
  );

  // ── Main content area ──
  const mainContent = viewMode === "alarms" && protocol.connected ? (
    <AlarmPanel connected={protocol.connected} compact={isTouch && viewportWidth < 900} />
  ) : viewMode === "browse" && protocol.connected ? (
    <>
      <div style={{ flex: 1, minHeight: 0, overflow: "hidden" }}>
        <ObjectBrowser
          onObjectSelect={(obj) => addLog("info", `Selected ${obj.object_type}:${obj.object_instance}`)}
          trendKeys={trendKeys}
          onToggleTrend={toggleTrend}
          onOpenTrendLog={openTrendLog}
          onOpenSchedule={openSchedule}
          treeWidth={isTouch ? 208 : undefined}
        />
      </div>
      {/* Horizontal drag handle — resize the trend panel (mouse only) */}
      <div
        hidden={isTouch}
        onMouseDown={(e) => handleDragStart("trend", e)}
        style={{
          height: 4,
          cursor: "row-resize",
          background: dragSide === "trend" ? "var(--accent)" : "transparent",
          flexShrink: 0,
          zIndex: 10,
          transition: "background 0.15s",
        }}
        onMouseEnter={(e) => { if (!dragSide) (e.target as HTMLElement).style.background = "var(--border)"; }}
        onMouseLeave={(e) => { if (!dragSide) (e.target as HTMLElement).style.background = "transparent"; }}
      />
      <div style={{ height: trendHeight, flexShrink: 0, display: "flex", flexDirection: "column", overflow: "hidden" }}>
        {trend.hasSeries ? (
          <TrendPanel
            series={trend.series.map((s) => ({ id: s.key, name: s.name, color: s.color, source: s.source, points: s.points }))}
            onRemove={trend.remove}
            windowSec={trend.windowSec}
            onWindowChange={trend.setWindowSec}
            pollMs={trend.pollMs}
            onPollMsChange={trend.setPollMs}
            onSourceChange={trend.setSource}
            notices={Object.fromEntries(trend.errors)}
            theme="dark"
          />
        ) : (
          <EmptyState
            compact
            title="No trend series"
            description="Click the chart icon on an object to add it to the trend."
          />
        )}
      </div>
    </>
  ) : (
    <>
      <div style={{ height: tableHeight, minHeight: 80, display: "flex", flexDirection: "column", overflow: "auto", flexShrink: 0 }}>
        <DataTable
          columns={columns}
          data={tableData}
          selectable
          onRowClick={(row) => openRowEditor(String(row.name))}
        />
      </div>
      <div
        hidden={isTouch}
        onMouseDown={(e) => handleDragStart("table", e)}
        style={{
          height: 4,
          cursor: "row-resize",
          background: dragSide === "table" ? "var(--accent)" : "transparent",
          flexShrink: 0,
          zIndex: 10,
          transition: "background 0.15s",
        }}
        onMouseEnter={(e) => { if (!dragSide) (e.target as HTMLElement).style.background = "var(--border)"; }}
        onMouseLeave={(e) => { if (!dragSide) (e.target as HTMLElement).style.background = "transparent"; }}
      />
      <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
        <LogPanel
          entries={log}
          onExport={(f) => addLog("info", `Log exported to ${f}`)}
        />
      </div>
    </>
  );

  // ── Phone pages (bottom tabs) ──
  const phoneTabs: PhoneTabItem[] = [
    { id: "devices", label: "Devices", icon: <Radar size={20} strokeWidth={1.75} /> },
    { id: "objects", label: "Objects", icon: <ListTree size={20} strokeWidth={1.75} /> },
    { id: "trend", label: "Trend", icon: <LineChart size={20} strokeWidth={1.75} />, badge: trend.series.length },
    { id: "alarms", label: "Alarms", icon: <Bell size={20} strokeWidth={1.75} /> },
    { id: "log", label: "Log", icon: <ScrollText size={20} strokeWidth={1.75} /> },
  ];

  const selectPhoneTab = (tab: PhoneTab) => {
    if (tab === "alarms") {
      if (!requirePro("Alarms and events")) return;
      setAlarmsOpened(true);
    }
    setPhoneTab(tab);
  };

  const needsConnection = (what: string) => (
    <EmptyState
      icon={<Radar size={20} strokeWidth={1.75} />}
      title="Not connected"
      description={`Connect to a BACnet device to ${what}.`}
      action={
        <Button variant="primary" onClick={() => setPhoneTab("devices")}>
          Go to Devices
        </Button>
      }
    />
  );

  // Every page stays mounted (hidden when inactive) so a browsed object list, loaded alarms and the trend survive tab switches.
  const phonePage = (tab: PhoneTab, children: React.ReactNode, className = "bx-page") => (
    <div className={className} style={phoneTab === tab ? undefined : { display: "none" }}>
      {children}
    </div>
  );

  const phoneLayout = (
    <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
      {phonePage("devices", sidebarContent, "bx-page vt-sidebar")}
      {phonePage(
        "objects",
        protocol.connected ? (
          <div style={{ flex: 1, minHeight: 0, padding: 12, display: "flex", flexDirection: "column" }}>
            <ObjectBrowser
              layout="stacked"
              onObjectSelect={(obj) => addLog("info", `Selected ${obj.object_type}:${obj.object_instance}`)}
              trendKeys={trendKeys}
              onToggleTrend={toggleTrend}
              onOpenTrendLog={openTrendLog}
              onOpenSchedule={openSchedule}
            />
          </div>
        ) : (
          needsConnection("browse its objects")
        ),
      )}
      {phonePage(
        "trend",
        !protocol.connected ? (
          needsConnection("trend its points")
        ) : trend.hasSeries ? (
          <TrendPanel
            series={trend.series.map((s) => ({ id: s.key, name: s.name, color: s.color, source: s.source, points: s.points }))}
            onRemove={trend.remove}
            windowSec={trend.windowSec}
            onWindowChange={trend.setWindowSec}
            pollMs={trend.pollMs}
            onPollMsChange={trend.setPollMs}
            onSourceChange={trend.setSource}
            notices={Object.fromEntries(trend.errors)}
            theme="dark"
          />
        ) : (
          <EmptyState
            icon={<LineChart size={20} strokeWidth={1.75} />}
            title="No trend series"
            description="On the Objects tab, tap the chart icon next to an object to add it here."
          />
        ),
      )}
      {phonePage(
        "alarms",
        protocol.connected && alarmsOpened ? (
          <div style={{ flex: 1, minHeight: 0, padding: 12, display: "flex", flexDirection: "column" }}>
            <AlarmPanel connected={protocol.connected} compact />
          </div>
        ) : (
          needsConnection("read its alarms")
        ),
      )}
      {phonePage(
        "log",
        <div style={{ flex: 1, minHeight: 0, padding: 8, display: "flex", flexDirection: "column" }}>
          <LogPanel entries={log} onExport={(f) => addLog("info", `Log exported to ${f}`)} />
        </div>,
      )}
      <StatusBar connected={protocol.connected} error={protocol.error} device={connectedDevice} />
      <MobileTabBar tabs={phoneTabs} active={phoneTab} onChange={selectPhoneTab} />
    </div>
  );

  return (
    <div className={isMobile ? "bx-root bx-phone" : "bx-root"} style={{ display: "flex", flexDirection: "column", height: "100vh", background: "var(--bg)", color: "var(--text)" }}>
      <Toolbar
        title="BACnet Explorer"
        icon={appIcon}
        context={protocol.connected && connectedDevice ? `${connectedDevice.ip}:${connectedDevice.port}` : undefined}
        connected={protocol.connected}
        onDisconnect={handleDisconnect}
        onInfo={() => setShowInfo(true)}
      />

      {isMobile ? (
        phoneLayout
      ) : (
        /* ── Desktop layout ── */
        <div style={{ display: "flex", flex: 1, overflow: "hidden" }}>
          <aside
            className="vt-sidebar"
            style={{ width: isTouch ? (viewportWidth < 900 ? 248 : 296) : leftWidth, flexShrink: 0, borderRight: "1px solid var(--border)", cursor: dragSide ? "col-resize" : undefined }}
          >
            {sidebarContent}
          </aside>
          {/* Left drag handle (mouse only) */}
          <div
            hidden={isTouch}
            onMouseDown={(e) => handleDragStart("left", e)}
            style={{
              width: 4,
              cursor: "col-resize",
              background: dragSide === "left" ? "var(--accent)" : "transparent",
              flexShrink: 0,
              zIndex: 10,
              transition: "background 0.15s",
            }}
            onMouseEnter={(e) => { if (!dragSide) (e.target as HTMLElement).style.background = "var(--border)"; }}
            onMouseLeave={(e) => { if (!dragSide) (e.target as HTMLElement).style.background = "transparent"; }}
          />
          <main style={{ display: "flex", flex: 1, flexDirection: "column", overflow: "hidden", padding: 8, gap: 4 }}>
            {mainContent}
          </main>
        </div>
      )}

      {!isMobile && (
        <StatusBar
          connected={protocol.connected}
          error={protocol.error}
          device={connectedDevice}
        />
      )}

      {/* Value editor modal */}
      {editingObject && (() => {
        const target = resolveEditTarget(editingObject);
        if (!target) return null;
        const kind = editTargetKind(target);
        return (
          <ValueEditor
            address={editingObject}
            currentValue={kind === "bool" ? (target.kind === "sim" && simValues[target.point.key] === "Active" ? "true" : "false") : undefined}
            currentDataType={kind}
            types={WRITE_KIND_TYPES[kind]}
            onSubmit={(value) => handleTableWrite(editingObject, value)}
            onCancel={() => setEditingObject(null)}
          />
        );
      })()}

      {/* Info modal */}
      <Dialog open={showInfo} onClose={() => setShowInfo(false)} title="BACnet Explorer" width={360}
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
          <p style={{ margin: "0 0 10px" }}>Discover, monitor, and control BACnet/IP devices.</p>
          <p style={{ margin: "0 0 10px" }}>
            Browse objects, read and write properties, subscribe to COV notifications, view trend logs, and manage
            schedules across your building automation network.
          </p>
          <p style={{ margin: "0 0 12px", color: "var(--text-muted)" }}>
            Part of the <strong style={{ color: "var(--text)" }}>Voltrus</strong> suite by JoyoDigitama
          </p>
          <ProAboutRow />
        </div>
      </Dialog>

      {/* Discovery dialog */}
      <Dialog
        open={showDiscovery}
        onClose={() => setShowDiscovery(false)}
        title={discovering ? "Discovered devices (scanning…)" : "Discovered devices"}
        width={480}
      >
        {discovering && discoveredDevices.length === 0 && (
          <div style={{ display: "flex", alignItems: "center", justifyContent: "center", gap: 8, padding: "24px 0", fontSize: "var(--fs-sm)", color: "var(--text-muted)" }}>
            <span className="vt-spinner" aria-hidden="true" />
            Broadcasting Who-Is…
          </div>
        )}
        {!discovering && discoveryError && (
          <div className="vt-callout vt-callout--danger" role="alert" style={{ marginBottom: 12 }}>
            {discoveryError}
          </div>
        )}
        {!discovering && discoveredDevices.length === 0 && (
          <EmptyState
            icon={<Radar size={20} strokeWidth={1.75} />}
            title="No devices found"
            description={
              platform === "ios"
                ? "Scanning sends a network broadcast, and iOS only allows that for apps that hold Apple's multicast networking permission. If this build does not have it yet, scanning finds nothing even when devices are online. Connect by typing the controller's IP address on the Devices tab instead, and make sure Local Network is on for BACnet Explorer in Settings > Privacy & Security > Local Network."
                : "No BACnet/IP devices answered the Who-Is broadcast. Check that devices are powered, on the same subnet, and not blocking UDP 47808, then scan again."
            }
            action={
              <Button icon={<Radar size={14} strokeWidth={1.75} />} onClick={handleDiscover}>
                Scan again
              </Button>
            }
          />
        )}
        {discoveredDevices.length > 0 && isMobile && (
          <div style={{ overflowY: "auto", margin: "0 -18px", maxHeight: "52vh" }}>
            {discoveredDevices.map((d) => (
              <div
                key={d.device_instance}
                style={{ display: "flex", alignItems: "center", gap: 12, padding: "10px 18px", borderTop: "1px solid var(--border)" }}
              >
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div style={{ color: "var(--text)", fontWeight: 600, fontVariantNumeric: "tabular-nums" }}>Device {d.device_instance}</div>
                  <div className="mono" style={{ fontSize: "var(--fs-sm)" }}>{d.ip}:{d.port}</div>
                  <div style={{ fontSize: "var(--fs-sm)", color: "var(--text-muted)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                    {d.vendor_name ?? "—"}
                  </div>
                </div>
                <Button
                  variant="primary"
                  loading={connectingDevice === d.device_instance}
                  onClick={async () => {
                    setConnectingDevice(d.device_instance);
                    const ok = await handleConnect({ ip: d.ip, port: d.port, deviceId: d.device_instance });
                    setConnectingDevice(null);
                    if (ok) setShowDiscovery(false);
                  }}
                >
                  {connectingDevice === d.device_instance ? "Connecting…" : "Connect"}
                </Button>
              </div>
            ))}
          </div>
        )}
        {discoveredDevices.length > 0 && !isMobile && (
          <div style={{ overflowY: "auto", margin: "0 -18px", padding: "0 18px", maxHeight: "52vh" }}>
            <table className="vt-table">
              <thead>
                <tr>
                  <th style={{ textAlign: "left" }}>Device</th>
                  <th style={{ textAlign: "left" }}>Address</th>
                  <th style={{ textAlign: "left" }}>Vendor</th>
                  <th style={{ width: 90 }} aria-label="Actions" />
                </tr>
              </thead>
              <tbody>
                {discoveredDevices.map((d) => (
                  <tr key={d.device_instance}>
                    <td style={{ fontVariantNumeric: "tabular-nums" }}>{d.device_instance}</td>
                    <td className="is-mono">{d.ip}:{d.port}</td>
                    <td style={{ color: "var(--text-dim)" }}>{d.vendor_name ?? "—"}</td>
                    <td style={{ textAlign: "right" }}>
                      <Button
                        size="sm"
                        variant="primary"
                        loading={connectingDevice === d.device_instance}
                        onClick={async () => {
                          setConnectingDevice(d.device_instance);
                          const ok = await handleConnect({ ip: d.ip, port: d.port, deviceId: d.device_instance });
                          setConnectingDevice(null);
                          if (ok) setShowDiscovery(false);
                        }}
                      >
                        {connectingDevice === d.device_instance ? "Connecting…" : "Connect"}
                      </Button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </Dialog>

      {/* Simulator control panel as a full-screen sheet (iOS has no second window) */}
      {showSimPanel && simRunning && (
        <div className="bx-sheet">
          <DevicePanel onClose={() => setShowSimPanel(false)} />
        </div>
      )}

      {/* Trend Log dialog */}
      {trendLogObject !== null && (
        <TrendLogDialog
          objectType={trendLogObject.type}
          objectInstance={trendLogObject.instance}
          onClose={() => setTrendLogObject(null)}
        />
      )}

      {/* Schedule dialog */}
      {scheduleObject !== null && (
        <ScheduleDialog
          objectInstance={scheduleObject}
          onClose={() => setScheduleObject(null)}
        />
      )}
    </div>
  );
}

export default App;
