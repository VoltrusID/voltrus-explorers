import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow, currentMonitor, LogicalSize } from "@tauri-apps/api/window";
import {
  DataTable,
  StatusBar,
  Toolbar,
  LogPanel,
  ValueEditor,
  EmptyState,
  IconButton,
  Tabs,
  useProtocol,
} from "@mactools/ui";
import { ChartLine, Moon, Sun } from "lucide-react";
import appIcon from "../src-tauri/icons/128x128.png";
import TrafficPanel from "./TrafficPanel";
import TrendPanel from "./TrendPanel";
import type { ModbusConfig, ReadBlock } from "./types";
import { sidebarStyle } from "./constants";
import { typeStep } from "./helpers";
import { useStore } from "./store";

// Hooks
import { useSlave } from "./hooks/useSlave";
import { useConnectionProfiles } from "./hooks/useConnectionProfiles";
import { useTrend } from "./hooks/useTrend";
import { usePanelResize } from "./hooks/usePanelResize";

// Utilities
import { formatValue, displayAddr, extractRaw } from "./utils/format";

// Components
import { MasterSidebar } from "./components/MasterSidebar";
import { SlaveSidebar } from "./components/SlaveSidebar";
import { BlocksPanel } from "./components/BlocksPanel";
import { TemplatePanel } from "./components/TemplatePanel";
import { SlaveTable } from "./components/SlaveTable";
import { MobileTabBar } from "./components/MobileTabBar";
import { InfoDialog } from "./components/InfoDialog";
import { CustomTemplateDialog } from "./components/CustomTemplateDialog";

function App() {
  const protocol = useProtocol<ModbusConfig, ReadBlock, unknown>("modbus");
  const {
    mode, setMode, leftWidth, rightWidth, tableHeight,
    dragSide, isMobile, setIsMobile,
    blocks, setBlocks, activeBlockTab, setActiveBlockTab,
    addLog, setLog, editingTag, setEditingTag,
    theme, setTheme,
    transport, tcpFormValues, formKey,
    serialPorts, setSerialPorts,
    rtuPort, rtuPortManual, rtuBaud, rtuDataBits, rtuStopBits,
    rtuParity, rtuUnitId, rtuTimeout, rtuPollInterval,
    useOneBased, displayFormat, endianness,
    slaveRunning,
    trendSeries, setTrendSeries, trendWindow, setTrendWindow,
    bottomTab, setBottomTab,
    slaveTransport,
    mobileTab, log, platform, setPlatform,
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
    if (!rtuPort) return;
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

  // ── Protocol error logging ──
  useEffect(() => { if (protocol.error) addLog("error", protocol.error); }, [protocol.error, addLog]);

  // ── Poll logging ──
  useEffect(() => {
    if (mode !== "master" || !protocol.connected || protocol.pollCount <= 0) return;
    const n = tableData.length;
    const first = tableData[0];
    const last = tableData[n - 1];
    if (!first || !last) return;
    const range = n === 1 ? `${first.name}=${first.value}` : `${first.name}…${last.name} (${n} regs)`;
    addLog("rx", `Poll #${protocol.pollCount}: ${range} (${protocol.latency}ms)`);
  }, [protocol.pollCount]); // eslint-disable-line react-hooks/exhaustive-deps

  // ── Load serial ports ──
  useEffect(() => {
    if ((mode === "master" && transport === "rtu") || (mode === "slave" && slaveTransport === "rtu")) {
      invoke<string[]>("list_serial_ports").then(setSerialPorts).catch(() => setSerialPorts([]));
    }
  }, [mode, transport, slaveTransport]);

  // ── Active block tab ──
  useEffect(() => {
    if (activeBlockTab >= blocks.length) setActiveBlockTab(Math.max(0, blocks.length - 1));
  }, [blocks.length, activeBlockTab]);

  // ── Connection handlers ──
  const disconnectDuringScanRef = useRef(false);

  const buildReadTags = useCallback(() =>
    blocks.flatMap((b) => {
      const step = typeStep(b.type);
      return Array.from({ length: b.quantity }, (_, i) => ({
        name: `${b.name}_${(b.address + i * step).toString().padStart(3, "0")}`,
        address: b.address + i * step,
        quantity: step,
        area: b.area,
        register_area: b.area,
        type: b.type,
        scale: 1.0,
      }));
    }), [blocks]);

  const handleConnect = useCallback(async (values: Record<string, string | number>) => {
    const config: ModbusConfig = {
      host: String(values.host), port: Number(values.port),
      unit_id: Number(values.unitId), timeout_ms: 5000,
      poll_interval_ms: Number(values.pollInterval || 1000), endianness,
    };
    addLog("info", `Connecting to ${config.host}:${config.port}…`);
    const ok = await protocol.connect(config);
    if (ok) {
      addLog("rx", "Connected");
      const readTags = buildReadTags();
      protocol.startPolling(readTags, config.poll_interval_ms);
      addLog("info", `Polling ${readTags.length} registers (${blocks.length} blocks) every ${config.poll_interval_ms}ms`);
    }
  }, [protocol, addLog, blocks, endianness, buildReadTags]);

  const handleDisconnect = useCallback(async () => {
    disconnectDuringScanRef.current = true;
    protocol.stopPolling();
    await protocol.disconnect();
    addLog("info", "Disconnected");
  }, [protocol, addLog]);

  const handleRtuConnect = useCallback(async () => {
    if (!rtuPort) { addLog("error", "No serial port selected"); return; }
    try {
      if (rtuPortManual) {
        addLog("info", `Connecting virtual RTU ${rtuPort}…`);
        await invoke("connect_rtu_virtual", { unitId: rtuUnitId });
      } else {
        addLog("info", `Connecting RTU ${rtuPort} @ ${rtuBaud} baud…`);
        await invoke("connect_rtu", {
          serialConfig: { port_name: rtuPort, baud_rate: rtuBaud, data_bits: rtuDataBits, stop_bits: rtuStopBits, parity: rtuParity },
          unitId: rtuUnitId, timeoutMs: rtuTimeout,
        });
      }
      addLog("rx", "RTU connected");
      protocol.setConnected(true);
      const readTags = buildReadTags();
      protocol.startPolling(readTags, rtuPollInterval);
      addLog("info", `Polling ${readTags.length} registers via RTU every ${rtuPollInterval}ms (timeout ${rtuTimeout}ms)`);
    } catch (e) { addLog("error", `RTU connect failed: ${String(e)}`); }
  }, [rtuPort, rtuPortManual, rtuBaud, rtuDataBits, rtuStopBits, rtuParity, rtuUnitId, rtuTimeout, rtuPollInterval, buildReadTags, protocol, addLog]);

  const handleScanBus = useCallback(async () => {
    if (!protocol.connected) { addLog("error", "Not connected. Connect to a serial port first."); return; }
    if (!rtuPort || rtuPortManual) { addLog("error", "Select a real serial port for bus scanning"); return; }
    const { setBusScanning, setBusScanResults, busScanAddr, busScanArea, busScanStart, busScanEnd } = useStore.getState();
    setBusScanning(true);
    setBusScanResults([]);
    addLog("info", `Scanning bus IDs ${busScanStart}–${busScanEnd} (reg ${busScanAddr}, ${busScanArea})…`);
    protocol.stopPolling();
    try {
      const results = await invoke<{ unit_id: number; value: number }[]>("scan_bus", {
        registerAddress: busScanAddr, registerArea: busScanArea, startId: busScanStart, endId: busScanEnd,
      });
      setBusScanResults(results);
      addLog("rx", `Bus scan complete: ${results.length} device(s) found`);
    } catch (e) { addLog("error", `Bus scan failed: ${String(e)}`); }
    finally {
      setBusScanning(false);
      if (protocol.connected && !disconnectDuringScanRef.current) {
        protocol.startPolling(buildReadTags(), rtuPollInterval);
      }
      disconnectDuringScanRef.current = false;
    }
  }, [protocol, buildReadTags, rtuPollInterval, rtuPort, rtuPortManual, addLog]);

  // ── CSV export/import ──
  const exportBlocksCsv = useCallback(() => {
    const header = "name,address,quantity,type,area";
    const rows = blocks.map((b) => `${b.name},${b.address},${b.quantity},${b.type},${b.area}`);
    const csv = [header, ...rows].join("\n");
    const blob = new Blob([csv], { type: "text/csv" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a"); a.href = url; a.download = "modbus-blocks.csv"; a.click();
    URL.revokeObjectURL(url);
    addLog("info", `Exported ${blocks.length} blocks to CSV`);
  }, [blocks, addLog]);

  const importBlocksCsv = useCallback(() => {
    const input = document.createElement("input"); input.type = "file"; input.accept = ".csv";
    input.onchange = async (e) => {
      const file = (e.target as HTMLInputElement).files?.[0]; if (!file) return;
      const text = await file.text();
      const lines = text.trim().split("\n");
      const imported: ReadBlock[] = [];
      for (let i = 1; i < lines.length; i++) {
        const parts = lines[i].split(",").map((s) => s.trim());
        if (parts.length >= 5) {
          const addr = Number(parts[1]); const qty = Number(parts[2]);
          if (!isNaN(addr) && !isNaN(qty) && qty > 0) {
            imported.push({ name: parts[0], address: addr, quantity: qty, type: parts[3], area: parts[4] });
          }
        }
      }
      if (imported.length > 0) { setBlocks(imported); addLog("info", `Imported ${imported.length} blocks from CSV`); }
    };
    input.click();
  }, [addLog]);

  // ── Table data ──
  const columns = [
    { key: "name", label: "Name", width: "140px" },
    { key: "address", label: "Addr", width: useOneBased ? "60px" : "50px", align: "right" as const },
    { key: "hex", label: "Hex", width: "70px" },
    { key: "type", label: "Type", width: "60px" },
    { key: "value", label: "Value" },
    { key: "trend", label: "Chart", width: "56px" },
  ];

  const tableData = blocks.flatMap((b, blockIdx) => {
    const step = typeStep(b.type);
    if (b.type === "string") {
      const flatStart = blocks.slice(0, blockIdx).reduce((sum, prev) => sum + prev.quantity * typeStep(prev.type), 0);
      const chars: string[] = [];
      for (let i = 0; i < b.quantity; i++) {
        const raw = extractRaw(protocol.values[flatStart + i]);
        if (!isNaN(raw)) { chars.push(String.fromCharCode((raw >> 8) & 0xFF)); chars.push(String.fromCharCode(raw & 0xFF)); }
      }
      const str = chars.join("").replace(/\0+$/, "");
      const allHex: string[] = [];
      for (let i = 0; i < b.quantity; i++) {
        const raw = extractRaw(protocol.values[flatStart + i]);
        allHex.push(isNaN(raw) ? "????" : ((raw | 0) & 0xFFFF).toString(16).toUpperCase().padStart(4, "0"));
      }
      return [{ name: b.name, address: displayAddr(b.address, b.area, useOneBased), rawAddress: b.address, hex: allHex.join(" "), type: "string", value: str || "—", _blockIdx: blockIdx }];
    }
    return Array.from({ length: b.quantity }, (_, i) => {
      const flatIndex = blocks.slice(0, blockIdx).reduce((sum, prev) => sum + prev.quantity * typeStep(prev.type), 0) + i * step;
      const raw = extractRaw(protocol.values[flatIndex]);
      const isCoil = b.area === "coil" || b.area === "discrete";
      return {
        name: `${b.name}_${(b.address + i * step).toString().padStart(3, "0")}`,
        address: displayAddr(b.address + i * step, b.area, useOneBased),
        rawAddress: b.address + i * step,
        hex: isNaN(raw) ? "---" : "0x" + ((raw | 0) & 0xFFFF).toString(16).toUpperCase().padStart(4, "0"),
        type: b.type, area: b.area,
        value: isNaN(raw) ? "—" : isCoil ? (raw !== 0 ? "ON" : "OFF") : formatValue(raw, b.type, displayFormat),
        _blockIdx: blockIdx,
      };
    });
  });

  const { toggleTrend } = useTrend(tableData);
  const activeBlockData = blocks.length > 1 ? tableData.filter((r) => r._blockIdx === activeBlockTab) : tableData;

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
      <DataTable columns={columns} data={activeBlockData} selectable onRowClick={(row) => {
        const r = row as Record<string, unknown>; const area = String(r.area ?? "holding"); const isCoil = area === "coil" || area === "discrete";
        setEditingTag({ name: String(r.name), address: Number(r.rawAddress ?? r.address), type: isCoil ? "bool" : String(r.type), area, currentValue: String(r.value) });
      }} renderCell={(key, row) => {
        if (key === "trend") {
          const name = String(row.name); const active = trendSeries.has(name);
          return (
            <IconButton size="sm" outline pressed={active} label={active ? `Remove ${name} from trend` : `Add ${name} to trend`} onClick={(e) => { e.stopPropagation(); toggleTrend(name); }}>
              <ChartLine size={13} strokeWidth={1.75} />
            </IconButton>
          );
        }
        if (key === "value") {
          const area = String((row as Record<string, unknown>).area ?? "holding"); const isCoil = area === "coil" || area === "discrete";
          if (isCoil) {
            const val = String(row[key] ?? ""); const isOn = val === "ON";
            return <span style={{ display: "inline-block", padding: "1px 8px", borderRadius: "var(--radius-sm)", fontSize: "var(--fs-xs)", fontWeight: 600, background: isOn ? "var(--success-soft)" : "var(--sunken)", color: isOn ? "var(--success)" : "var(--text-muted)", border: `1px solid ${isOn ? "var(--success)" : "var(--border)"}` }}>{val}</span>;
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

  // ── Sidebar content ──
  const sidebarContent = mode === "master" ? (
    <MasterSidebar
      protocol={protocol} masterFormRef={masterFormRef} formKey={formKey}
      onConnect={handleConnect} onDisconnect={handleDisconnect}
      onRtuConnect={handleRtuConnect} onScanBus={handleScanBus}
      onAutoDetect={handleAutoDetect} disconnectDuringScanRef={disconnectDuringScanRef}
      saveProfileAs={saveProfileAs} saveProfileChanges={saveProfileChanges}
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
        connected={mode === "master" ? protocol.connected : slaveRunning}
        onDisconnect={mode === "master" ? handleDisconnect : handleSlaveStop}
        onInfo={() => useStore.getState().setShowInfo(true)}
        actions={
          <IconButton
            label={theme === "dark" ? "Switch to light theme" : "Switch to dark theme"}
            size="sm"
            onClick={toggleTheme}
          >
            {theme === "dark" ? <Sun size={16} strokeWidth={1.75} /> : <Moon size={16} strokeWidth={1.75} />}
          </IconButton>
        }
      />

      {isMobile ? (
        /* ── Mobile layout ── */
        <div style={{ flex: 1, overflow: "hidden", display: "flex", flexDirection: "column" }}>
          <div style={{ flex: 1, overflow: "auto", padding: 16 }}>
            {mobileTab === "connect" && <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>{sidebarContent}</div>}
            {mobileTab === "data" && dataTable}
            {mobileTab === "blocks" && mode === "master" && <div style={{ display: "flex", flexDirection: "column", gap: 8 }}><BlocksPanel onExportCsv={exportBlocksCsv} onImportCsv={importBlocksCsv} /></div>}
            {mobileTab === "template" && mode === "slave" && <div style={{ display: "flex", flexDirection: "column", gap: 8 }}><TemplatePanel applyProfile={applyProfile} toggleSimulation={toggleSimulation} saveCustomTemplate={saveCustomTemplate} /></div>}
            {mobileTab === "log" && <LogPanel entries={log} onExport={(f) => addLog("info", `Log exported to ${f}`)} onClear={() => setLog([])} />}
          </div>
          <MobileTabBar />
          <StatusBar connected={mode === "master" ? protocol.connected : slaveRunning} latency={protocol.latency} pollCount={protocol.pollCount} error={protocol.error} />
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
              {bottomTab === "log" ? (
                <LogPanel entries={log} onExport={(f) => addLog("info", `Log exported to ${f}`)} onClear={() => setLog([])} />
              ) : bottomTab === "traffic" ? (
                <TrafficPanel />
              ) : (
                <TrendPanel series={Array.from(trendSeries.values())} onRemove={(name) => setTrendSeries((prev) => { const next = new Map(prev); next.delete(name); return next; })} windowSec={trendWindow} onWindowChange={setTrendWindow} theme={theme} />
              )}
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

      {!isMobile && <StatusBar connected={mode === "master" ? protocol.connected : slaveRunning} latency={protocol.latency} pollCount={protocol.pollCount} error={protocol.error} />}

      {editingTag && (
        <ValueEditor
          address={editingTag.name}
          currentValue={editingTag.currentValue}
          currentDataType={editingTag.type}
          onSubmit={async (value, dataType) => {
            addLog("tx", `Write ${value} (${dataType}) to ${editingTag.name}`);
            const numValue = dataType === "bool" ? (value === "true" ? 1 : 0) : Number(value);
            await protocol.writeTag(
              { name: editingTag.name, address: editingTag.address, quantity: 1, area: editingTag.area, register_area: editingTag.area, type: dataType === "bool" ? "uint16" : editingTag.type, scale: 1.0 },
              numValue,
            );
            await protocol.readTags(buildReadTags());
            setEditingTag(null);
          }}
          onCancel={() => setEditingTag(null)}
        />
      )}

      <InfoDialog toggleTheme={toggleTheme} />
      <CustomTemplateDialog onSave={saveCustomTemplate} />
    </div>
  );
}

export default App;
