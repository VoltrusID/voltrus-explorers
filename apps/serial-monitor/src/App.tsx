import { useState, useCallback, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  DataTable,
  StatusBar,
  Toolbar,
  LogPanel,
  Badge,
  Segmented,
  ProBadge,
  useProStatus,
} from "@mactools/ui";
import type { LogEntry } from "@mactools/ui";
import appIcon from "../src-tauri/icons/128x128.png";

import type { PortInfo, SerialConfig, TrafficEntry, SidebarMode } from "./types";
import { TEMPLATES } from "./constants";
import { formatTimestamp } from "./components/ConnectionHealth";
import { MonitorSidebar } from "./components/MonitorSidebar";
import { VirtualPortSidebar } from "./components/VirtualPortSidebar";
import { SendComposer } from "./components/SendComposer";
import { InfoModal } from "./components/InfoModal";

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

function App() {
  const { requirePro } = useProStatus();

  // Connection state
  const [connected, setConnected] = useState(false);
  const [connecting, setConnecting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Port enumeration
  const [ports, setPorts] = useState<PortInfo[]>([]);

  // Connection form
  const [portName, setPortName] = useState("");
  const [baudRate, setBaudRate] = useState(115200);
  const [dataBits, setDataBits] = useState(8);
  const [stopBits, setStopBits] = useState(1);
  const [parity, setParity] = useState("none");
  const [timeoutMs, setTimeoutMs] = useState(1000);

  // Data display
  const [log, setLog] = useState<LogEntry[]>([]);
  const [trafficData, setTrafficData] = useState<TrafficEntry[]>([]);

  // Send controls
  const [sendData, setSendData] = useState("");
  const [sendFormat, setSendFormat] = useState<"hex" | "ascii">("ascii");

  // Connection health tracking
  const [lastDataTime, setLastDataTime] = useState<number | null>(null);

  // UI state
  const [showInfo, setShowInfo] = useState(false);
  const [leftWidth, setLeftWidth] = useState(264);
  const [tableHeight, setTableHeight] = useState(300);
  const [dragSide, setDragSide] = useState<"left" | "table" | null>(null);
  const [mobileTab, setMobileTab] = useState<"connect" | "data" | "log">("connect");
  const [isMobile, setIsMobile] = useState(window.innerWidth < 768);
  const [sidebarMode, setSidebarMode] = useState<SidebarMode>("monitor");

  // Virtual port state
  const [virtualPortPath, setVirtualPortPath] = useState<string | null>(null);
  const [virtualActive, setVirtualActive] = useState(false);
  const [virtualSendData, setVirtualSendData] = useState("");
  const [virtualSendFormat, setVirtualSendFormat] = useState<"hex" | "ascii">("hex");
  const [virtualTraffic, setVirtualTraffic] = useState<TrafficEntry[]>([]);

  // Simulation
  const [simTemplate, setSimTemplate] = useState<string | null>(null);
  const [simRunning, setSimRunning] = useState(false);

  // Refs for event listeners
  const unlistenRef = useRef<UnlistenFn[]>([]);
  const virtualUnlistenRef = useRef<UnlistenFn[]>([]);

  // -----------------------------------------------------------------------
  // Helpers
  // -----------------------------------------------------------------------

  const addLog = useCallback((direction: LogEntry["direction"], message: string) => {
    setLog((prev) => [...prev, { timestamp: Date.now(), direction, message }]);
  }, []);

  // -----------------------------------------------------------------------
  // Port scanning
  // -----------------------------------------------------------------------

  const refreshPorts = useCallback(async () => {
    try {
      const list = await invoke<PortInfo[]>("list_available_ports");
      setPorts(list);
      return list;
    } catch (e) {
      addLog("error", `Failed to list ports: ${e}`);
      return [];
    }
  }, [addLog]);

  useEffect(() => {
    refreshPorts().then((list) => {
      if (list.length > 0 && !portName) {
        setPortName(list[0].port_name);
      }
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // -----------------------------------------------------------------------
  // Mobile detection
  // -----------------------------------------------------------------------

  useEffect(() => {
    const onResize = () => setIsMobile(window.innerWidth < 768);
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  // -----------------------------------------------------------------------
  // Error effect
  // -----------------------------------------------------------------------

  useEffect(() => {
    if (error) {
      addLog("error", error);
      setError(null);
    }
  }, [error, addLog]);

  // -----------------------------------------------------------------------
  // Simulation effect
  // -----------------------------------------------------------------------

  useEffect(() => {
    if (!simRunning || !simTemplate) return;
    const tmpl = TEMPLATES[simTemplate];
    const timers: ReturnType<typeof setTimeout>[] = [];

    const cycle = () => {
      tmpl.frames.forEach((frame) => {
        const timer = setTimeout(() => {
          const ts = formatTimestamp(Date.now());
          setTrafficData((prev) => [
            ...prev.slice(-100),
            { direction: frame.direction, data: frame.hexData, timestamp: ts },
          ]);
          addLog(frame.direction === "TX" ? "tx" : "rx", `${frame.label}: ${frame.hexData}`);
        }, frame.intervalMs);
        timers.push(timer);
      });

      const maxInterval = Math.max(...tmpl.frames.map((f) => f.intervalMs));
      const nextCycle = setTimeout(cycle, maxInterval + 2000);
      timers.push(nextCycle);
    };

    addLog("info", `Simulation started: ${tmpl.name}`);
    cycle();

    return () => {
      timers.forEach(clearTimeout);
    };
  }, [simRunning, simTemplate, addLog]);

  // -----------------------------------------------------------------------
  // Panel resize handlers
  // -----------------------------------------------------------------------

  const dragStartRef = useRef({ startX: 0, startY: 0, startWidth: 0, startHeight: 0 });
  const handleDragStart = useCallback(
    (side: "left" | "table", e: React.MouseEvent) => {
      e.preventDefault();
      dragStartRef.current = {
        startX: e.clientX,
        startY: e.clientY,
        startWidth: side === "left" ? leftWidth : 0,
        startHeight: side === "table" ? tableHeight : 0,
      };
      setDragSide(side);
    },
    [leftWidth, tableHeight],
  );

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
    return () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
  }, [dragSide]);

  // -----------------------------------------------------------------------
  // Connection handlers
  // -----------------------------------------------------------------------

  const handleConnect = useCallback(async () => {
    if (connected) return;
    setConnecting(true);
    setError(null);

    try {
      const config: SerialConfig = {
        port_name: portName,
        baud_rate: baudRate,
        data_bits: dataBits,
        stop_bits: stopBits,
        parity,
        timeout_ms: timeoutMs,
      };
      await invoke("connect", { config });
      setConnected(true);
      setLastDataTime(null);
      addLog("info", `Opened ${config.port_name} @ ${config.baud_rate} baud (timeout: ${timeoutMs}ms)`);

      const unlistenData = await listen<{ hex: string; bytes: number[]; timestamp: number }>(
        "serial-data",
        (event) => {
          const ts = formatTimestamp(event.payload.timestamp);
          setTrafficData((prev) => [
            ...prev.slice(-200),
            { direction: "RX", data: event.payload.hex, timestamp: ts },
          ]);
          addLog("rx", event.payload.hex);
          setLastDataTime(Date.now());
        },
      );

      const unlistenDisconnect = await listen<{ reason: string }>(
        "serial-disconnect",
        (event) => {
          setConnected(false);
          setLastDataTime(null);
          addLog("error", `Disconnected: ${event.payload.reason}`);
          unlistenRef.current.forEach((fn) => fn());
          unlistenRef.current = [];
        },
      );

      unlistenRef.current = [unlistenData, unlistenDisconnect];
    } catch (e) {
      setError(String(e));
      addLog("error", `Connection failed: ${e}`);
    } finally {
      setConnecting(false);
    }
  }, [connected, portName, baudRate, dataBits, stopBits, parity, timeoutMs, addLog]);

  const handleDisconnect = useCallback(async () => {
    try {
      await invoke("disconnect");
    } catch (e) {
      addLog("error", `Disconnect error: ${e}`);
    }
    setConnected(false);
    setLastDataTime(null);
    unlistenRef.current.forEach((fn) => fn());
    unlistenRef.current = [];
    addLog("info", "Port closed");
  }, [addLog]);

  // -----------------------------------------------------------------------
  // Virtual port handlers
  // -----------------------------------------------------------------------

  const handleCreateVirtualPort = useCallback(async () => {
    try {
      const path = await invoke<string>("create_virtual_port");
      setVirtualPortPath(path);
      setVirtualActive(true);
      addLog("info", `Virtual port created: ${path}`);

      const updatedList = await refreshPorts();
      if (updatedList.length > 0 && !updatedList.find((p) => p.port_name === portName)) {
        setPortName(updatedList[0].port_name);
      }

      const unlistenData = await listen<{ hex: string; bytes: number[]; timestamp: number }>(
        "virtual-data",
        (event) => {
          const ts = formatTimestamp(event.payload.timestamp);
          setVirtualTraffic((prev) => [
            ...prev.slice(-200),
            { direction: "RX", data: event.payload.hex, timestamp: ts },
          ]);
          addLog("rx", `[VPort] ${event.payload.hex}`);
        },
      );

      const unlistenDisconnect = await listen<{ reason: string }>(
        "virtual-disconnect",
        (event) => {
          setVirtualActive(false);
          setVirtualPortPath(null);
          addLog("error", `Virtual port closed: ${event.payload.reason}`);
          virtualUnlistenRef.current.forEach((fn) => fn());
          virtualUnlistenRef.current = [];
        },
      );

      virtualUnlistenRef.current = [unlistenData, unlistenDisconnect];
    } catch (e) {
      addLog("error", `Virtual port failed: ${e}`);
    }
  }, [addLog, refreshPorts, portName]);

  const handleDestroyVirtualPort = useCallback(async () => {
    try {
      await invoke("destroy_virtual_port");
    } catch (e) {
      addLog("error", `Destroy virtual port error: ${e}`);
    }
    setVirtualActive(false);
    setVirtualPortPath(null);
    setVirtualTraffic([]);
    virtualUnlistenRef.current.forEach((fn) => fn());
    virtualUnlistenRef.current = [];
    addLog("info", "Virtual port destroyed");
    refreshPorts();
  }, [addLog, refreshPorts]);

  const handleVirtualSend = useCallback(async () => {
    if (!virtualSendData || !virtualActive) return;
    try {
      await invoke("send_virtual", { data: virtualSendData, format: virtualSendFormat });
      if (virtualSendFormat === "hex") {
        const normalized = virtualSendData.split(/\s+/).filter(Boolean).map((h) => h.toUpperCase()).join(" ");
        const ts = formatTimestamp(Date.now());
        setVirtualTraffic((prev) => [...prev.slice(-200), { direction: "TX", data: normalized, timestamp: ts }]);
        addLog("tx", `[VPort] ${normalized}`);
      } else {
        const hex = Array.from(new TextEncoder().encode(virtualSendData))
          .map((b) => b.toString(16).toUpperCase().padStart(2, "0"))
          .join(" ");
        const ts = formatTimestamp(Date.now());
        setVirtualTraffic((prev) => [...prev.slice(-200), { direction: "TX", data: hex, timestamp: ts }]);
        addLog("tx", `[VPort] ASCII: ${virtualSendData}`);
      }
      setVirtualSendData("");
    } catch (e) {
      addLog("error", `[VPort] Send failed: ${e}`);
    }
  }, [virtualSendData, virtualSendFormat, virtualActive, addLog]);

  const handleCopyPath = useCallback(async () => {
    if (virtualPortPath) {
      try {
        await navigator.clipboard.writeText(virtualPortPath);
        addLog("info", `Copied: ${virtualPortPath}`);
      } catch {
        addLog("error", "Copy failed");
      }
    }
  }, [virtualPortPath, addLog]);

  // -----------------------------------------------------------------------
  // Send handler
  // -----------------------------------------------------------------------

  const handleSend = useCallback(async () => {
    if (!sendData || !connected) return;
    try {
      await invoke("send", { data: sendData, format: sendFormat });
      if (sendFormat === "hex") {
        const normalized = sendData.split(/\s+/).filter(Boolean).map((h) => h.toUpperCase()).join(" ");
        const ts = formatTimestamp(Date.now());
        setTrafficData((prev) => [...prev.slice(-200), { direction: "TX", data: normalized, timestamp: ts }]);
        addLog("tx", normalized);
      } else {
        const hex = Array.from(new TextEncoder().encode(sendData))
          .map((b) => b.toString(16).toUpperCase().padStart(2, "0"))
          .join(" ");
        const ts = formatTimestamp(Date.now());
        setTrafficData((prev) => [...prev.slice(-200), { direction: "TX", data: hex, timestamp: ts }]);
        addLog("tx", `ASCII: ${sendData}`);
      }
      setSendData("");
    } catch (e) {
      addLog("error", `Send failed: ${e}`);
    }
  }, [sendData, sendFormat, connected, addLog]);

  // -----------------------------------------------------------------------
  // Cleanup on unmount
  // -----------------------------------------------------------------------

  useEffect(() => {
    return () => {
      unlistenRef.current.forEach((fn) => fn());
      virtualUnlistenRef.current.forEach((fn) => fn());
    };
  }, []);

  // -----------------------------------------------------------------------
  // Render
  // -----------------------------------------------------------------------

  const columns = [
    { key: "direction", label: "Dir", width: "56px" },
    { key: "data", label: "Data" },
    { key: "timestamp", label: "Time", width: "120px" },
  ];

  const renderTrafficCell = (key: string, row: Record<string, unknown>) => {
    if (key === "direction") {
      const dir = String(row.direction ?? "");
      return <Badge tone={dir === "TX" ? "info" : "success"}>{dir}</Badge>;
    }
    return String(row[key] ?? "");
  };

  const emptyMessage =
    sidebarMode === "virtual"
      ? "Virtual port is not active. Create one to capture loopback traffic."
      : "No traffic yet. Connect to a port or start the simulation.";

  const anyConnected = connected || virtualActive;

  const modeSegmented = (
    <Segmented
      label="Mode"
      block
      value={sidebarMode}
      onChange={(v) => { if (v === "virtual" && !requirePro("Virtual serial port")) return; setSidebarMode(v); }}
      options={[
        { value: "monitor", label: "Monitor" },
        { value: "virtual", label: <>Virtual port <ProBadge /></> },
      ]}
    />
  );

  const monitorSidebar = (
    <MonitorSidebar
      connected={connected} connecting={connecting} ports={ports}
      portName={portName} setPortName={setPortName}
      baudRate={baudRate} setBaudRate={setBaudRate}
      dataBits={dataBits} setDataBits={setDataBits}
      stopBits={stopBits} setStopBits={setStopBits}
      parity={parity} setParity={setParity}
      timeoutMs={timeoutMs} setTimeoutMs={setTimeoutMs}
      lastDataTime={lastDataTime}
      refreshPorts={refreshPorts}
      handleConnect={handleConnect} handleDisconnect={handleDisconnect}
      simTemplate={simTemplate} setSimTemplate={setSimTemplate}
      simRunning={simRunning} setSimRunning={setSimRunning}
      setTrafficData={setTrafficData} addLog={addLog}
    />
  );

  const virtualPortSidebar = (
    <VirtualPortSidebar
      virtualActive={virtualActive} virtualPortPath={virtualPortPath}
      handleCreateVirtualPort={handleCreateVirtualPort}
      handleDestroyVirtualPort={handleDestroyVirtualPort}
      handleCopyPath={handleCopyPath}
    />
  );

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100vh", background: "var(--bg)", color: "var(--text)" }}>
      <Toolbar
        title="Serial Monitor"
        icon={appIcon}
        connected={anyConnected}
        onDisconnect={() => {
          if (connected) handleDisconnect();
          if (virtualActive) handleDestroyVirtualPort();
        }}
        onInfo={() => setShowInfo(true)}
      />

      {isMobile ? (
        <div style={{ flex: 1, overflow: "hidden", display: "flex", flexDirection: "column" }}>
          <div style={{ flex: 1, overflow: "auto", minHeight: 0 }}>
            {mobileTab === "connect" && (
              <div className="vt-sidebar" style={{ borderBottom: "1px solid var(--border)" }}>
                <div style={{ padding: "12px 12px 0" }}>{modeSegmented}</div>
                {sidebarMode === "monitor" ? monitorSidebar : virtualPortSidebar}
              </div>
            )}
            {mobileTab === "data" && (
              <div style={{ display: "flex", flexDirection: "column", height: "100%", padding: 8, gap: 8 }}>
                <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
                  <DataTable
                    columns={columns}
                    data={sidebarMode === "virtual" ? virtualTraffic : trafficData}
                    selectable={false}
                    emptyMessage={emptyMessage}
                    renderCell={renderTrafficCell}
                  />
                </div>
                {sidebarMode === "monitor" ? (
                  <SendComposer
                    format={sendFormat} onFormatChange={setSendFormat}
                    value={sendData} onValueChange={setSendData}
                    onSend={handleSend} disabled={!connected}
                  />
                ) : (
                  <SendComposer
                    format={virtualSendFormat} onFormatChange={setVirtualSendFormat}
                    value={virtualSendData} onValueChange={setVirtualSendData}
                    onSend={handleVirtualSend} disabled={!virtualActive}
                  />
                )}
              </div>
            )}
            {mobileTab === "log" && (
              <LogPanel entries={log} beforeExport={() => requirePro("Log export")} onExport={(f) => addLog("info", `Log exported to ${f}`)} />
            )}
          </div>
          <nav style={{ display: "flex", borderTop: "1px solid var(--border)", background: "var(--surface)", flexShrink: 0, paddingBottom: "env(safe-area-inset-bottom, 0px)" }}>
            {(["connect", "data", "log"] as const).map((id) => (
              <button
                key={id}
                onClick={() => setMobileTab(id)}
                style={{
                  flex: 1, padding: "10px 0 8px", fontSize: "var(--fs-xs)", fontWeight: 600,
                  background: "none", border: "none", cursor: "pointer",
                  color: mobileTab === id ? "var(--accent)" : "var(--text-muted)",
                  borderTop: mobileTab === id ? "2px solid var(--accent)" : "2px solid transparent",
                }}
              >
                {id.charAt(0).toUpperCase() + id.slice(1)}
              </button>
            ))}
          </nav>
          <StatusBar connected={anyConnected} error={error ?? undefined} />
        </div>
      ) : (
        /* Desktop layout */
        <div style={{ display: "flex", flex: 1, overflow: "hidden", minHeight: 0 }}>
          <aside
            className="vt-sidebar"
            style={{
              width: leftWidth,
              flexShrink: 0,
              borderRight: "1px solid var(--border)",
              cursor: dragSide === "left" ? "col-resize" : undefined,
            }}
          >
            <div style={{ padding: "12px 12px 0" }}>{modeSegmented}</div>
            {sidebarMode === "monitor" ? monitorSidebar : virtualPortSidebar}
          </aside>
          {/* Left drag handle */}
          <div
            onMouseDown={(e) => handleDragStart("left", e)}
            style={{
              width: 4, cursor: "col-resize", flexShrink: 0, zIndex: 10,
              background: dragSide === "left" ? "var(--accent)" : "transparent",
              transition: "background 0.15s",
            }}
            onMouseEnter={(e) => { if (!dragSide) (e.target as HTMLElement).style.background = "var(--border)"; }}
            onMouseLeave={(e) => { if (!dragSide) (e.target as HTMLElement).style.background = "transparent"; }}
          />
          <main style={{ display: "flex", flex: 1, flexDirection: "column", overflow: "hidden", padding: 8, gap: 8, minWidth: 0 }}>
            <div style={{ height: tableHeight, minHeight: 80, display: "flex", flexDirection: "column", flexShrink: 0 }}>
              <DataTable
                columns={columns}
                data={sidebarMode === "virtual" ? virtualTraffic : trafficData}
                selectable={false}
                emptyMessage={emptyMessage}
                renderCell={renderTrafficCell}
              />
            </div>
            {sidebarMode === "monitor" ? (
              <SendComposer
                format={sendFormat} onFormatChange={setSendFormat}
                value={sendData} onValueChange={setSendData}
                onSend={handleSend} disabled={!connected}
              />
            ) : (
              <SendComposer
                format={virtualSendFormat} onFormatChange={setVirtualSendFormat}
                value={virtualSendData} onValueChange={setVirtualSendData}
                onSend={handleVirtualSend} disabled={!virtualActive}
              />
            )}
            {/* Horizontal drag handle */}
            <div
              onMouseDown={(e) => handleDragStart("table", e)}
              style={{
                height: 4, cursor: "row-resize", flexShrink: 0, zIndex: 10,
                background: dragSide === "table" ? "var(--accent)" : "transparent",
                transition: "background 0.15s",
              }}
              onMouseEnter={(e) => { if (!dragSide) (e.target as HTMLElement).style.background = "var(--border)"; }}
              onMouseLeave={(e) => { if (!dragSide) (e.target as HTMLElement).style.background = "transparent"; }}
            />
            <LogPanel entries={log} beforeExport={() => requirePro("Log export")} onExport={(f) => addLog("info", `Log exported to ${f}`)} />
          </main>
        </div>
      )}

      {!isMobile && <StatusBar connected={anyConnected} error={error ?? undefined} />}

      <InfoModal show={showInfo} onClose={() => setShowInfo(false)} />
    </div>
  );
}

export default App;
