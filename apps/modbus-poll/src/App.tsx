import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow, currentMonitor, LogicalSize } from "@tauri-apps/api/window";
import {
  DataTable,
  ConnectButton,
  StatusBar,
  Toolbar,
  LogPanel,
  ValueEditor,
  EmptyState,
  IconButton,
  Tabs,
  useProStatus,
  saveTextFile,
  openTextFile,
} from "@mactools/ui";
import { ChartLine, Moon, Sun } from "lucide-react";
import appIcon from "../src-tauri/icons/128x128.png";
import TrafficPanel from "./TrafficPanel";
import TrendPanel from "./TrendPanel";
import type { ModbusConfig } from "./types";
import { FREE_PROFILE_LIMIT, VIRTUAL_PORT, sidebarStyle } from "./constants";
import { isBitArea, isWritableArea, readPlanKey } from "./helpers";
import { blocksToCsv, parseBlocksCsv } from "./csv";
import { startTrafficFeed } from "./trafficStore";
import { useStore } from "./store";
import { fieldsOf } from "./sessions";
import { effectivePlan, mayOpenConnection, valueCapNotice } from "./limits";
import {
  IDLE, useConnRuntime, runtimeOf, startPolling, pausePolling, readNow, connectTcp, cancelConnect,
  disconnect, closeConnection, markConnected, writeTag, startCsvLog, stopCsvLog, connectionLabel,
} from "./connections";

// Hooks
import { useSlave } from "./hooks/useSlave";
import { useConnectionProfiles } from "./hooks/useConnectionProfiles";
import { useConnectSync } from "./hooks/useConnectSync";
import { useTrend } from "./hooks/useTrend";
import { usePanelResize } from "./hooks/usePanelResize";

// Utilities
import { buildTableRows, type TableRow } from "./utils/rows";

// Components
import { MasterSidebar } from "./components/MasterSidebar";
import { SlaveSidebar } from "./components/SlaveSidebar";
import { BlocksPanel } from "./components/BlocksPanel";
import { TemplatePanel } from "./components/TemplatePanel";
import { SlaveTable } from "./components/SlaveTable";
import { MobileTabBar } from "./components/MobileTabBar";
import { ConnectionTabs } from "./components/ConnectionTabs";
import { InfoDialog } from "./components/InfoDialog";
import { CustomTemplateDialog } from "./components/CustomTemplateDialog";

const NO_VALUES: unknown[] = [];

const NUMERIC_WRITE_TYPES = [
  { value: "uint16", label: "UInt16" },
  { value: "int16", label: "Int16" },
  { value: "int32", label: "Int32" },
  { value: "float32", label: "Float32" },
  { value: "float64", label: "Float64" },
];
const COIL_WRITE_TYPES = [{ value: "bool", label: "Bool" }];

function App() {
  const { requirePro, openUpsell, pro } = useProStatus();
  // Async handlers run after the render that created them; they read Pro status from here.
  const proRef = useRef(pro);
  proRef.current = pro;
  const activeId = useStore((s) => s.activeSessionId);
  const sessionCount = useStore((s) => s.sessions.length);
  const rt = useConnRuntime((s) => s.byId[activeId] ?? IDLE);
  const connProfiles = useStore((s) => s.connProfiles);
  const {
    mode, setMode, leftWidth, rightWidth, tableHeight,
    dragSide, isMobile, setIsMobile,
    blocks, setBlocks, activeBlockTab, setActiveBlockTab,
    addLog, setLog, editingTag, setEditingTag,
    theme, setTheme,
    transport, formKey,
    setSerialPorts,
    rtuPort, rtuBaud, rtuDataBits, rtuStopBits,
    rtuParity, rtuUnitId, rtuTimeout, rtuPollInterval,
    useOneBased, displayFormat, endianness,
    slaveRunning,
    trendSeries, setTrendSeries, trendWindow, setTrendWindow,
    bottomTab, setBottomTab,
    slaveTransport, busScanning,
    mobileTab, log, setPlatform,
  } = useStore();

  // ── Window init ──
  useEffect(() => {
    (async () => {
      const win = getCurrentWindow();
      const monitor = await currentMonitor();
      if (monitor) {
        const scaleFactor = monitor.scaleFactor;
        const screenW = monitor.size.width / scaleFactor;
        const screenH = monitor.size.height / scaleFactor;
        const menuBar = 28;
        const targetWidth = Math.round((screenW * 2) / 3);
        const targetHeight = Math.round(screenH - menuBar);
        await win.setSize(new LogicalSize(targetWidth, targetHeight));
        await win.center();
      }
    })();
  }, []);

  // ── Traffic history: recorded for the whole session, whichever panel is showing ──
  useEffect(() => startTrafficFeed(), []);

  // ── Hooks ──
  const {
    applyProfile, toggleSimulation, saveCustomTemplate,
    handleSlaveStart, handleSlaveStop, handleSlaveRegWrite,
  } = useSlave();
  const masterFormRef = useRef<Record<string, string | number> | null>({
    host: "127.0.0.1",
    port: 502,
    unitId: 1,
    pollInterval: 1000,
  });
  const {
    saveProfileAs, saveProfileChanges, loadProfile, deleteProfile,
  } = useConnectionProfiles(masterFormRef);
  useConnectSync(loadProfile);
  const { handleDragStart } = usePanelResize();

  // ── Platform (serial ports are desktop-only) ──
  useEffect(() => {
    invoke<string>("platform").then(setPlatform).catch(() => {});
  }, []);

  // ── Theme ──
  useEffect(() => {
    document.documentElement.setAttribute("data-theme", theme);
    localStorage.setItem("modbus-poll-theme", theme);
  }, [theme]);
  if (typeof document !== "undefined" && !document.documentElement.hasAttribute("data-theme")) {
    document.documentElement.setAttribute("data-theme", theme);
  }
  const toggleTheme = useCallback(() => setTheme(theme === "dark" ? "light" : "dark"), [theme]);

  // ── Auto-detect RTU ──
  const [autoDetecting, setAutoDetecting] = useState(false);
  const handleAutoDetect = useCallback(async () => {
    if (!rtuPort || rtuPort === VIRTUAL_PORT) return;
    setAutoDetecting(true);
    addLog("info", `Auto-detecting RTU on ${rtuPort}...`);
    try {
      const result = await invoke<{ baud_rate: number; parity: string; stop_bits: number; data_bits: number } | null>("auto_detect_rtu", { portName: rtuPort, unitId: rtuUnitId });
      if (result) {
        useStore.getState().setRtuBaud(result.baud_rate);
        useStore.getState().setRtuParity(result.parity);
        useStore.getState().setRtuStopBits(result.stop_bits);
        useStore.getState().setRtuDataBits(result.data_bits);
        addLog("rx", `Detected: ${result.baud_rate} baud, ${result.parity} parity, ${result.stop_bits} stop`);
      } else { addLog("error", "Auto-detect failed: no valid config found"); }
    } catch (e) { addLog("error", `Auto-detect error: ${String(e)}`); }
    setAutoDetecting(false);
  }, [rtuPort, rtuUnitId, addLog]);

  // ── Mobile detection ──
  useEffect(() => {
    const onResize = () => setIsMobile(window.innerWidth < 768);
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  // ── Load serial ports ──
  useEffect(() => {
    if ((mode === "master" && transport === "rtu") || (mode === "slave" && slaveTransport === "rtu")) {
      invoke<string[]>("list_serial_ports").then(setSerialPorts).catch((e) => {
        setSerialPorts([]);
        addLog("error", `Could not list serial ports: ${String(e)}`);
      });
    }
  }, [mode, transport, slaveTransport]);

  // ── Active block tab ──
  useEffect(() => {
    if (activeBlockTab >= blocks.length) setActiveBlockTab(Math.max(0, blocks.length - 1));
  }, [blocks.length, activeBlockTab]);

  // ── Polling ──
  // One tag per displayed value; `read_tags` returns one value per tag, so rows index values by tag.
  // Without Pro the plan stops at the first FREE_VALUES values; the rows past it show "Pro".
  const plan = useMemo(() => effectivePlan(blocks, pro), [blocks, pro]);
  const planKey = useMemo(() => readPlanKey(plan), [plan]);

  /** (Re)start connection `id`'s poll on its current blocks. */
  const beginPolling = useCallback((id: string, intervalMs: number) => {
    const blocksNow = fieldsOf(useStore.getState(), id)?.blocks ?? [];
    startPolling(id, blocksNow, intervalMs, proRef.current);
  }, []);

  // Editing, adding, removing or reordering blocks while connected re-targets the running poll.
  useEffect(() => {
    if (rt.pollIntervalMs === null || busScanning || rt.polledKey === planKey) return;
    beginPolling(activeId, rt.pollIntervalMs);
  }, [planKey, rt.polledKey, rt.pollIntervalMs, busScanning, activeId, beginPolling]);

  // ── Table data ──
  const liveValues = rt.connected && rt.polledKey === planKey && !rt.stale ? rt.values : NO_VALUES;
  const tableData = useMemo(
    () => buildTableRows(blocks, plan, liveValues, { useOneBased, displayFormat }),
    [blocks, plan, liveValues, useOneBased, displayFormat],
  );

  // ── Connection handlers ──
  // Each handler works on the connection that was active when it started: the user may switch
  // tabs while a connect or a scan is still running.
  const logFor = useStore.getState().addLogFor;

  /** "Polling N values (M blocks) every …", with the free cap spelled out when it applies. */
  const pollingSummary = (id: string, via: string, extra: string) => {
    const blocksNow = fieldsOf(useStore.getState(), id)?.blocks ?? [];
    const count = effectivePlan(blocksNow, proRef.current).tags.length;
    const cap = valueCapNotice(blocksNow, proRef.current);
    return `Polling ${count} values${via} (${blocksNow.length} blocks) every ${extra}${cap ? `. ${cap}` : ""}`;
  };

  const handleConnect = useCallback(async (values: Record<string, string | number>) => {
    const id = useStore.getState().activeSessionId;
    const config: ModbusConfig = {
      host: String(values.host), port: Number(values.port),
      unit_id: Number(values.unitId), timeout_ms: 5000,
      poll_interval_ms: Number(values.pollInterval || 1000), endianness: useStore.getState().endianness,
    };
    logFor(id, "info", `Connecting to ${config.host}:${config.port}…`);
    const ok = await connectTcp(id, config);
    if (ok) {
      logFor(id, "rx", "Connected");
      beginPolling(id, config.poll_interval_ms);
      logFor(id, "info", pollingSummary(id, "", `${config.poll_interval_ms}ms`));
    }
  }, [beginPolling]); // eslint-disable-line react-hooks/exhaustive-deps

  const handleCancelConnect = useCallback(async () => {
    const id = useStore.getState().activeSessionId;
    try {
      await cancelConnect(id);
    } catch (e) { logFor(id, "error", `Cancel failed: ${String(e)}`); }
    logFor(id, "info", "Connection cancelled");
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const handleDisconnect = useCallback(async () => {
    const id = useStore.getState().activeSessionId;
    await stopCsvLogAndReport(id);
    await disconnect(id);
    logFor(id, "info", "Disconnected");
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const handleRtuConnect = useCallback(async () => {
    const state = useStore.getState();
    const id = state.activeSessionId;
    const { rtuPort, rtuBaud, rtuDataBits, rtuStopBits, rtuParity, rtuUnitId, rtuTimeout, rtuPollInterval, endianness } = state;
    if (!rtuPort) { logFor(id, "error", "No serial port selected"); return; }
    try {
      if (rtuPort === VIRTUAL_PORT) {
        logFor(id, "info", "Connecting to the in-app virtual slave…");
        await invoke("connect_rtu_virtual", { connId: id, unitId: rtuUnitId, endianness });
      } else {
        if (!(rtuBaud > 0)) { logFor(id, "error", "Enter a baud rate"); return; }
        logFor(id, "info", `Connecting RTU ${rtuPort} @ ${rtuBaud} baud…`);
        await invoke("connect_rtu", {
          connId: id,
          serialConfig: { port_name: rtuPort, baud_rate: rtuBaud, data_bits: rtuDataBits, stop_bits: rtuStopBits, parity: rtuParity },
          unitId: rtuUnitId, endianness, timeoutMs: rtuTimeout,
        });
      }
      logFor(id, "rx", "RTU connected");
      markConnected(id, true);
      beginPolling(id, rtuPollInterval);
      logFor(id, "info", pollingSummary(id, " via RTU", `${rtuPollInterval}ms (timeout ${rtuTimeout}ms)`));
    } catch (e) { logFor(id, "error", `RTU connect failed: ${String(e)}`); }
  }, [beginPolling]); // eslint-disable-line react-hooks/exhaustive-deps

  const handleScanBus = useCallback(async () => {
    const state = useStore.getState();
    const id = state.activeSessionId;
    if (!runtimeOf(id).connected) { logFor(id, "error", "Not connected. Connect to a serial port first."); return; }
    const { busScanAddr, busScanArea, busScanStart, busScanEnd } = state;
    const scanState = (patch: { busScanning?: boolean; busScanResults?: { unit_id: number; value: number }[] }) =>
      useStore.getState().patchSession(id, () => patch);
    scanState({ busScanning: true, busScanResults: [] });
    logFor(id, "info", `Scanning bus IDs ${busScanStart}–${busScanEnd} (reg ${busScanAddr}, ${busScanArea})…`);
    pausePolling(id);
    try {
      const results = await invoke<{ unit_id: number; value: number }[]>("scan_bus", {
        connId: id, registerAddress: busScanAddr, registerArea: busScanArea, startId: busScanStart, endId: busScanEnd,
      });
      scanState({ busScanResults: results });
      logFor(id, "rx", `Bus scan finished: ${results.length} device(s) found`);
    } catch (e) {
      // Disconnecting during a scan closes the connection under it; that is not a scan failure.
      if (runtimeOf(id).pollIntervalMs === null) logFor(id, "info", "Bus scan aborted: disconnected");
      else logFor(id, "error", `Bus scan failed: ${String(e)}`);
    } finally {
      scanState({ busScanning: false });
      // The scan (also when stopped) leaves the connection up and back on its unit ID; resume
      // polling unless the user disconnected meanwhile.
      const interval = runtimeOf(id).pollIntervalMs;
      if (interval !== null) beginPolling(id, interval);
    }
  }, [beginPolling]); // eslint-disable-line react-hooks/exhaustive-deps

  const handleSelectScanUnit = useCallback(async (unitId: number) => {
    const id = useStore.getState().activeSessionId;
    try {
      await invoke("set_unit_id", { connId: id, unitId });
      useStore.getState().setRtuUnitId(unitId);
      logFor(id, "info", `Now polling unit ID ${unitId}`);
    } catch (e) { logFor(id, "error", `Could not switch to unit ID ${unitId}: ${String(e)}`); }
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // ── CSV logging of polled values (Pro) ──
  /** Stop the connection's CSV log, if one runs, and say where it went. */
  const stopCsvLogAndReport = async (id: string) => {
    const stopped = await stopCsvLog(id);
    if (stopped) logFor(id, "info", `Stopped logging: ${stopped.rows} values written to ${stopped.path}`);
  };

  const handleStartCsvLog = useCallback(async () => {
    if (!requirePro("Logging polled values to CSV")) return;
    const id = useStore.getState().activeSessionId;
    try {
      const path = await startCsvLog(id, connectionLabel(id));
      if (path) logFor(id, "info", `Logging every polled value to ${path}`);
    } catch (e) { logFor(id, "error", `Could not start logging: ${String(e)}`); }
  }, [requirePro]); // eslint-disable-line react-hooks/exhaustive-deps

  const handleStopCsvLog = useCallback(() => stopCsvLogAndReport(useStore.getState().activeSessionId), []); // eslint-disable-line react-hooks/exhaustive-deps

  // ── Several connections (Pro) ──
  const handleAddConnection = useCallback(() => {
    if (!mayOpenConnection(useStore.getState().sessions.length, proRef.current)) {
      openUpsell("Several connections at once");
      return;
    }
    useStore.getState().addSession();
  }, [openUpsell]);

  const handleCloseConnection = useCallback(async (id: string) => {
    await closeConnection(id);
    useStore.getState().closeSession(id);
  }, []);

  // The connect form keeps its values in a ref; point it at the connection that came to the front.
  useEffect(() => {
    masterFormRef.current = { ...useStore.getState().tcpFormValues };
    setWriteError(null);
  }, [activeId]);

  // ── CSV export/import ──
  const exportBlocksCsv = useCallback(async () => {
    if (!requirePro("CSV export")) return;
    try {
      const path = await saveTextFile("modbus-blocks.csv", blocksToCsv(blocks));
      if (path) addLog("info", `Exported ${blocks.length} blocks to ${path}`);
    } catch (e) { addLog("error", `CSV export failed: ${String(e)}`); }
  }, [blocks, addLog, requirePro]);

  const importBlocksCsv = useCallback(async () => {
    if (!requirePro("CSV import")) return;
    try {
      const file = await openTextFile();
      if (!file) return;
      const { blocks: imported, errors } = parseBlocksCsv(file.text);
      errors.forEach((message) => addLog("error", `${file.path}: ${message}`));
      if (imported.length === 0) { addLog("error", `No valid blocks in ${file.path}`); return; }
      setBlocks(imported);
      addLog("info", `Imported ${imported.length} blocks from ${file.path}${errors.length > 0 ? ` (${errors.length} line(s) skipped)` : ""}`);
    } catch (e) { addLog("error", `CSV import failed: ${String(e)}`); }
  }, [addLog, requirePro]);

  // ── Table columns & trend ──
  const columns = [
    { key: "name", label: "Name", width: "140px" },
    { key: "address", label: "Addr", width: useOneBased ? "60px" : "50px", align: "right" as const },
    { key: "hex", label: "Hex", width: "70px" },
    { key: "type", label: "Type", width: "60px" },
    { key: "value", label: "Value" },
    { key: "trend", label: "Chart", width: "56px" },
  ];

  const { toggleTrend } = useTrend(tableData);
  // Adding a trend series is Pro; removing one never is.
  const toggleTrendPro = (name: string) => {
    if (!trendSeries.has(name) && !requirePro("Live trends")) return;
    toggleTrend(name);
  };
  const activeBlockData = blocks.length > 1 ? tableData.filter((r) => r._blockIdx === activeBlockTab) : tableData;

  // ── Write dialog ──
  const [writeError, setWriteError] = useState<string | null>(null);
  const closeEditor = useCallback(() => { setWriteError(null); setEditingTag(null); }, []);

  const openEditor = (row: TableRow) => {
    if (!requirePro("Writing registers and coils")) return;
    if (!isWritableArea(row.area)) {
      addLog("info", `${row.name} is in the ${row.area} area, which is read-only`);
      return;
    }
    const bit = isBitArea(row.area);
    setWriteError(null);
    setEditingTag({
      name: row.name,
      address: row.rawAddress,
      type: bit ? "bool" : row.type,
      area: row.area,
      // The polled number, not its display format ("ON", "0x00FF", "0000000011111111"…).
      currentValue: bit ? (row.rawValue ? "true" : "false") : row.rawValue === null ? "" : String(row.rawValue),
    });
  };

  const handleWrite = async (value: string, dataType: string) => {
    if (!editingTag) return;
    const isBool = dataType === "bool";
    const numValue = isBool ? (value === "true" ? 1 : 0) : Number(value.trim());
    if (!isBool && (value.trim() === "" || !Number.isFinite(numValue))) {
      setWriteError(`"${value}" is not a number`);
      return;
    }
    const id = activeId;
    logFor(id, "tx", `Write ${value} (${dataType}) to ${editingTag.name}`);
    const result = await writeTag(
      id,
      { name: editingTag.name, address: editingTag.address, quantity: 1, area: editingTag.area, register_area: editingTag.area, type: isBool ? "uint16" : dataType, scale: 1.0 },
      numValue,
    );
    if (!result.ok) {
      setWriteError(result.error ?? "Write failed");
      return;
    }
    logFor(id, "rx", `Wrote ${editingTag.name}`);
    useStore.getState().patchSession(id, () => ({ editingTag: null }));
    setWriteError(null);
    await readNow(id);
  };

  // ── Data table section ──
  const dataTable = mode === "master" ? (
    <div style={{ display: "flex", flexDirection: "column", flex: 1, overflow: "hidden" }}>
      {blocks.length > 1 && (
        <div style={{ flexShrink: 0, background: "var(--card)" }}>
          <Tabs
            label="Read blocks"
            value={String(activeBlockTab)}
            items={blocks.map((b, i) => ({ value: String(i), label: b.name, count: b.address }))}
            onChange={(v) => setActiveBlockTab(Number(v))}
          />
        </div>
      )}
      <DataTable columns={columns} data={activeBlockData} selectable onRowClick={(row) => openEditor(row as unknown as TableRow)} renderCell={(key, row) => {
        const r = row as unknown as TableRow;
        if (key === "trend") {
          const name = r.name; const active = trendSeries.has(name);
          return (
            <IconButton size="sm" outline pressed={active} label={active ? `Remove ${name} from trend` : `Add ${name} to trend`} onClick={(e) => { e.stopPropagation(); toggleTrendPro(name); }}>
              <ChartLine size={13} strokeWidth={1.75} />
            </IconButton>
          );
        }
        if (key === "value") {
          if (r.error) {
            const code = /\(0x([0-9A-Fa-f]{2})\)/.exec(r.error)?.[1];
            return <span title={r.error} style={{ color: "var(--danger)", fontWeight: 600 }}>{code ? `EXC 0x${code}` : "ERR"}</span>;
          }
          if (isBitArea(r.area) && r.rawValue !== null) {
            const isOn = r.value === "ON";
            return <span style={{ display: "inline-block", padding: "1px 8px", borderRadius: "var(--radius-sm)", fontSize: "var(--fs-xs)", fontWeight: 600, background: isOn ? "var(--success-soft)" : "var(--sunken)", color: isOn ? "var(--success)" : "var(--text-muted)", border: `1px solid ${isOn ? "var(--success)" : "var(--border)"}` }}>{r.value}</span>;
          }
        }
        return String(row[key] ?? "");
      }} />
    </div>
  ) : slaveRunning ? (
    <SlaveTable handleSlaveRegWrite={handleSlaveRegWrite} />
  ) : (
    <div style={{ flex: 1, display: "flex", alignItems: "center", justifyContent: "center" }}>
      <EmptyState compact title="Slave not running" description="Configure the slave server and press Start server to simulate registers." />
    </div>
  );

  const logPanel = (
    <LogPanel entries={log} exportPrefix="modbus-log" onExport={(path) => addLog("info", `Log exported to ${path}`)} onClear={() => setLog([])} />
  );
  const trendPanel = (
    <TrendPanel series={Array.from(trendSeries.values())} onRemove={(name) => setTrendSeries((prev) => { const next = new Map(prev); next.delete(name); return next; })} windowSec={trendWindow} onWindowChange={setTrendWindow} theme={theme} />
  );

  // ── Sidebar content ──
  const sidebarContent = mode === "master" ? (
    <MasterSidebar
      protocol={rt} masterFormRef={masterFormRef} formKey={formKey}
      onConnect={handleConnect} onCancelConnect={handleCancelConnect} onDisconnect={handleDisconnect}
      onRtuConnect={handleRtuConnect} onScanBus={handleScanBus} onSelectScanUnit={handleSelectScanUnit}
      onAutoDetect={handleAutoDetect} autoDetecting={autoDetecting}
      csvLog={{ path: rt.csvLogPath, rows: rt.csvLogRows }} onStartCsvLog={handleStartCsvLog} onStopCsvLog={handleStopCsvLog}
      saveProfileAs={(name) => {
        const isNew = !connProfiles.some((p) => p.name === name.trim());
        if (isNew && connProfiles.length >= FREE_PROFILE_LIMIT && !requirePro("More than one saved connection profile")) return;
        saveProfileAs(name);
      }}
      saveProfileChanges={saveProfileChanges}
      loadProfile={loadProfile} deleteProfile={deleteProfile}
    />
  ) : (
    <SlaveSidebar onSlaveStart={handleSlaveStart} onSlaveStop={handleSlaveStop} />
  );

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100vh", background: "var(--bg)", color: "var(--text)" }}>
      <Toolbar
        title="Modbus Poll"
        icon={appIcon}
        connected={mode === "master" ? rt.connected : slaveRunning}
        onDisconnect={mode === "master" ? handleDisconnect : handleSlaveStop}
        onInfo={() => useStore.getState().setShowInfo(true)}
        actions={
          <>
            <ConnectButton />
            <IconButton
              label={theme === "dark" ? "Switch to light theme" : "Switch to dark theme"}
              size="sm"
              onClick={toggleTheme}
            >
              {theme === "dark" ? <Sun size={16} strokeWidth={1.75} /> : <Moon size={16} strokeWidth={1.75} />}
            </IconButton>
          </>
        }
      />

      {mode === "master" && (
        <ConnectionTabs sessionCount={sessionCount} onAdd={handleAddConnection} onClose={handleCloseConnection} />
      )}

      {isMobile ? (
        /* ── Mobile layout ── */
        <div style={{ flex: 1, overflow: "hidden", display: "flex", flexDirection: "column" }}>
          <div style={{ flex: 1, overflow: "auto", padding: 16 }}>
            {mobileTab === "connect" && <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>{sidebarContent}</div>}
            {mobileTab === "data" && dataTable}
            {mobileTab === "blocks" && mode === "master" && <div style={{ display: "flex", flexDirection: "column", gap: 8 }}><BlocksPanel onExportCsv={exportBlocksCsv} onImportCsv={importBlocksCsv} /></div>}
            {mobileTab === "template" && mode === "slave" && <div style={{ display: "flex", flexDirection: "column", gap: 8 }}><TemplatePanel applyProfile={applyProfile} toggleSimulation={toggleSimulation} saveCustomTemplate={saveCustomTemplate} /></div>}
            {mobileTab === "log" && logPanel}
            {mobileTab === "traffic" && <div style={{ display: "flex", flexDirection: "column", height: "100%", minHeight: 280 }}><TrafficPanel /></div>}
            {mobileTab === "trend" && <div style={{ display: "flex", flexDirection: "column", height: "100%", minHeight: 280 }}>{trendPanel}</div>}
          </div>
          <MobileTabBar />
          <StatusBar connected={mode === "master" ? rt.connected : slaveRunning} latency={rt.latency} pollCount={rt.pollCount} error={rt.error} />
        </div>
      ) : (
        /* ── Desktop layout ── */
        <div style={{ display: "flex", flex: 1, overflow: "hidden" }}>
          <aside style={{ ...sidebarStyle, width: leftWidth, cursor: dragSide ? "col-resize" : undefined }}>
            {sidebarContent}
          </aside>
          {/* Left drag handle */}
          <div
            onMouseDown={(e) => handleDragStart("left", e)}
            style={{ width: 4, cursor: "col-resize", background: dragSide === "left" ? "var(--accent)" : "transparent", flexShrink: 0, zIndex: 10, transition: "background 0.15s" }}
            onMouseEnter={(e) => { if (!dragSide) (e.target as HTMLElement).style.background = "var(--border)"; }}
            onMouseLeave={(e) => { if (!dragSide) (e.target as HTMLElement).style.background = "transparent"; }}
          />
          <main style={{ display: "flex", flex: 1, flexDirection: "column", overflow: "hidden", padding: 8 }}>
            <div style={{ height: tableHeight, minHeight: 80, display: "flex", flexDirection: "column", flexShrink: 0 }}>
              {dataTable}
            </div>
            {/* Horizontal drag handle */}
            <div
              onMouseDown={(e) => handleDragStart("table", e)}
              style={{ height: 4, cursor: "row-resize", background: dragSide === "table" ? "var(--accent)" : "transparent", flexShrink: 0, zIndex: 10, transition: "background 0.15s" }}
              onMouseEnter={(e) => { if (!dragSide) (e.target as HTMLElement).style.background = "var(--border)"; }}
              onMouseLeave={(e) => { if (!dragSide) (e.target as HTMLElement).style.background = "transparent"; }}
            />
            {/* Bottom panel tabs */}
            <Tabs
              label="Bottom panel"
              value={bottomTab}
              items={[
                { value: "log", label: "Comm log" },
                { value: "traffic", label: "Traffic" },
                { value: "trend", label: "Trend", ...(trendSeries.size > 0 ? { count: trendSeries.size } : {}) },
              ]}
              onChange={setBottomTab}
            />
            <div style={{ display: "flex", flexDirection: "column", flex: 1, minHeight: 0 }}>
              {bottomTab === "log" ? logPanel : bottomTab === "traffic" ? <TrafficPanel /> : trendPanel}
            </div>
          </main>

          {/* Right drag handle + Right panel */}
          <div
            onMouseDown={(e) => handleDragStart("right", e)}
            style={{ width: 4, cursor: "col-resize", background: dragSide === "right" ? "var(--accent)" : "transparent", flexShrink: 0, zIndex: 10, transition: "background 0.15s" }}
            onMouseEnter={(e) => { if (!dragSide) (e.target as HTMLElement).style.background = "var(--border)"; }}
            onMouseLeave={(e) => { if (!dragSide) (e.target as HTMLElement).style.background = "transparent"; }}
          />
          <aside style={{
            width: rightWidth, flexShrink: 0, borderLeft: "1px solid var(--border)", background: "var(--card)",
            padding: 16, overflowY: "auto", display: "flex", flexDirection: "column", gap: 8, cursor: dragSide ? "col-resize" : undefined,
          }}>
            {mode === "master"
              ? <BlocksPanel onExportCsv={exportBlocksCsv} onImportCsv={importBlocksCsv} />
              : <TemplatePanel applyProfile={applyProfile} toggleSimulation={toggleSimulation} saveCustomTemplate={saveCustomTemplate} />
            }
          </aside>
        </div>
      )}

      {!isMobile && <StatusBar connected={mode === "master" ? rt.connected : slaveRunning} latency={rt.latency} pollCount={rt.pollCount} error={rt.error} />}

      {editingTag && (
        <ValueEditor
          address={editingTag.name}
          currentValue={editingTag.currentValue}
          currentDataType={editingTag.type}
          types={editingTag.type === "bool" ? COIL_WRITE_TYPES : NUMERIC_WRITE_TYPES}
          error={writeError}
          onSubmit={handleWrite}
          onCancel={closeEditor}
        />
      )}

      <InfoDialog toggleTheme={toggleTheme} />
      <CustomTemplateDialog onSave={saveCustomTemplate} />
    </div>
  );
}

export default App;
