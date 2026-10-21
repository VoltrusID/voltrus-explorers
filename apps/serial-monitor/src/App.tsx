import { useState, useCallback, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  StatusBar,
  Toolbar,
  LogPanel,
  Segmented,
  ProBadge,
  useProStatus,
  pickSavePath,
} from "@mactools/ui";
import type { LogEntry } from "@mactools/ui";
import appIcon from "../src-tauri/icons/128x128.png";

import type { PortInfo, SerialConfig, Session, SidebarMode, TrafficEntry } from "./types";
import { TEMPLATES } from "./constants";
import {
  FREE_LOG_LINES, loadMacros, loadProfiles, saveMacros, saveProfiles,
  type PortProfile, type SendMacro,
} from "./profiles";
import {
  FREE_SESSION_LIMIT, appendTraffic, defaultConfig, newSession, pickFreePort, shortPortName,
} from "./sessions";
import { formatTimestamp } from "./components/ConnectionHealth";
import { MonitorSidebar } from "./components/MonitorSidebar";
import { VirtualPortSidebar } from "./components/VirtualPortSidebar";
import { SendComposer } from "./components/SendComposer";
import { SessionPanel } from "./components/SessionPanel";
import { SessionsSection } from "./components/SessionsSection";
import { CaptureSection, type CaptureState } from "./components/CaptureSection";
import { TrafficTable } from "./components/TrafficTable";
import { InfoModal } from "./components/InfoModal";

interface DataEvent { id: string; hex: string; bytes: number[]; timestamp: number }
interface DisconnectEvent { id: string; reason: string }

const bytesToHex = (bytes: number[] | Uint8Array) =>
  Array.from(bytes).map((b) => b.toString(16).toUpperCase().padStart(2, "0")).join(" ");

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

function App() {
  const { pro, status, requirePro, openUpsell } = useProStatus();
  // The PTY-based virtual port cannot be created inside the App Sandbox, so store builds do not offer it.
  const virtualAvailable = status !== null && !status.store;

  // Open ports. Free keeps one, Pro several side by side.
  const [sessions, setSessions] = useState<Session[]>(() => [newSession("s1", defaultConfig())]);
  const [activeId, setActiveId] = useState("s1");
  const sessionsRef = useRef(sessions);
  sessionsRef.current = sessions;
  const nextSessionId = useRef(2);
  const active = sessions.find((s) => s.id === activeId) ?? sessions[0];

  // Port enumeration
  const [ports, setPorts] = useState<PortInfo[]>([]);

  // Data display
  const [log, setLog] = useState<LogEntry[]>([]);

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

  // Saved settings and send macros
  const [profiles, setProfilesState] = useState<PortProfile[]>(loadProfiles);
  const [macros, setMacrosState] = useState<SendMacro[]>(loadMacros);
  const setProfiles = useCallback((list: PortProfile[]) => setProfilesState(saveProfiles(list)), []);
  const setMacros = useCallback((list: SendMacro[]) => setMacrosState(saveMacros(list)), []);
  const proRef = useRef(pro);
  proRef.current = pro;

  // Simulation (pushes canned frames into one session's table)
  const [simTemplate, setSimTemplate] = useState<string | null>(null);
  const [simRunning, setSimRunning] = useState(false);
  const [simTarget, setSimTarget] = useState<string | null>(null);

  // Capture to file
  const [capture, setCapture] = useState<CaptureState | null>(null);

  const virtualUnlistenRef = useRef<UnlistenFn[]>([]);

  // -----------------------------------------------------------------------
  // Helpers
  // -----------------------------------------------------------------------

  const addLog = useCallback((direction: LogEntry["direction"], message: string, sessionId?: string) => {
    const list = sessionsRef.current;
    const port = sessionId && list.length > 1 ? list.find((s) => s.id === sessionId)?.config.port_name : undefined;
    const text = port ? `[${shortPortName(port)}] ${message}` : message;
    setLog((prev) => {
      const next = [...prev, { timestamp: Date.now(), direction, message: text }];
      return proRef.current || next.length <= FREE_LOG_LINES ? next : next.slice(-FREE_LOG_LINES);
    });
  }, []);

  const updateSession = useCallback((id: string, patch: Partial<Session>) => {
    setSessions((prev) => prev.map((s) => (s.id === id ? { ...s, ...patch } : s)));
  }, []);

  const setConfig = useCallback((patch: Partial<SerialConfig>) => {
    setSessions((prev) => prev.map((s) => (s.id === activeId ? { ...s, config: { ...s.config, ...patch } } : s)));
  }, [activeId]);

  const pushTraffic = useCallback((id: string, direction: string, data: string, timestamp: string) => {
    setSessions((prev) => prev.map((s) => (
      s.id === id ? { ...s, traffic: appendTraffic(s.traffic, { direction, data, timestamp }) } : s
    )));
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
      if (list.length === 0) return;
      setSessions((prev) => prev.map((s) => (
        s.config.port_name ? s : { ...s, config: { ...s.config, port_name: list[0].port_name } }
      )));
    });
  }, [refreshPorts]);

  // -----------------------------------------------------------------------
  // Mobile detection
  // -----------------------------------------------------------------------

  useEffect(() => {
    const onResize = () => setIsMobile(window.innerWidth < 768);
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  // -----------------------------------------------------------------------
  // Backend events (one listener set for every session)
  // -----------------------------------------------------------------------

  useEffect(() => {
    let disposed = false;
    const unlisteners: UnlistenFn[] = [];
    const register = async () => {
      const fns = await Promise.all([
        listen<DataEvent>("serial-data", ({ payload }) => {
          pushTraffic(payload.id, "RX", payload.hex, formatTimestamp(payload.timestamp));
          updateSession(payload.id, { lastDataTime: Date.now() });
          addLog("rx", payload.hex, payload.id);
        }),
        listen<DisconnectEvent>("serial-disconnect", ({ payload }) => {
          updateSession(payload.id, { connected: false, connecting: false, lastDataTime: null, repeating: false });
          addLog("error", `Disconnected: ${payload.reason}`, payload.id);
        }),
        listen<{ reason: string }>("capture-stopped", ({ payload }) => {
          setCapture(null);
          addLog("error", payload.reason);
        }),
      ]);
      if (disposed) fns.forEach((fn) => fn());
      else unlisteners.push(...fns);
    };
    register();
    return () => {
      disposed = true;
      unlisteners.forEach((fn) => fn());
    };
  }, [pushTraffic, updateSession, addLog]);

  // -----------------------------------------------------------------------
  // Simulation effect
  // -----------------------------------------------------------------------

  useEffect(() => {
    if (!simRunning || !simTemplate || !simTarget) return;
    const tmpl = TEMPLATES[simTemplate];
    const timers: number[] = [];

    const cycle = () => {
      tmpl.frames.forEach((frame) => {
        const timer = window.setTimeout(() => {
          pushTraffic(simTarget, frame.direction, frame.hexData, formatTimestamp(Date.now()));
          addLog(frame.direction === "TX" ? "tx" : "rx", `${frame.label}: ${frame.hexData}`, simTarget);
        }, frame.intervalMs);
        timers.push(timer);
      });

      const maxInterval = Math.max(...tmpl.frames.map((f) => f.intervalMs));
      const nextCycle = window.setTimeout(cycle, maxInterval + 2000);
      timers.push(nextCycle);
    };

    addLog("info", `Simulation started: ${tmpl.name}`, simTarget);
    cycle();

    return () => {
      timers.forEach(clearTimeout);
    };
  }, [simRunning, simTemplate, simTarget, addLog, pushTraffic]);

  const startSimulation = useCallback(() => {
    updateSession(activeId, { traffic: [] });
    setSimTarget(activeId);
    setSimRunning(true);
  }, [activeId, updateSession]);

  const stopSimulation = useCallback(() => {
    setSimRunning(false);
    if (simTarget) updateSession(simTarget, { traffic: [] });
    addLog("info", "Simulation stopped");
  }, [simTarget, updateSession, addLog]);

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

  const handleConnect = useCallback(async (id: string) => {
    const s = sessionsRef.current.find((x) => x.id === id);
    if (!s || s.connected || s.connecting) return;
    updateSession(id, { connecting: true });
    try {
      await invoke("connect", { id, config: s.config });
      updateSession(id, { connected: true, connecting: false, lastDataTime: null });
      addLog("info", `Opened ${s.config.port_name} @ ${s.config.baud_rate} baud (timeout: ${s.config.timeout_ms}ms)`);
    } catch (e) {
      updateSession(id, { connecting: false });
      addLog("error", `Connection failed: ${e}`, id);
    }
  }, [addLog, updateSession]);

  const handleDisconnect = useCallback(async (id: string) => {
    const s = sessionsRef.current.find((x) => x.id === id);
    try {
      await invoke("disconnect", { id });
    } catch (e) {
      addLog("error", `Disconnect error: ${e}`, id);
    }
    updateSession(id, { connected: false, lastDataTime: null, repeating: false });
    addLog("info", `Closed ${s?.config.port_name ?? "port"}`);
  }, [addLog, updateSession]);

  const addSession = useCallback(() => {
    const list = sessionsRef.current;
    if (list.length >= FREE_SESSION_LIMIT && !requirePro("Several ports side by side")) return;
    const id = `s${nextSessionId.current++}`;
    const base = list.find((s) => s.id === activeId)?.config ?? defaultConfig();
    setSessions((prev) => [...prev, newSession(id, { ...base, port_name: pickFreePort(ports, prev) })]);
    setActiveId(id);
  }, [activeId, ports, requirePro]);

  const closeSession = useCallback(async (id: string) => {
    const list = sessionsRef.current;
    if (list.length < 2) return;
    if (list.find((s) => s.id === id)?.connected) await handleDisconnect(id);
    if (simTarget === id) setSimRunning(false);
    setSessions((prev) => prev.filter((s) => s.id !== id));
    setActiveId((cur) => (cur === id ? list.find((s) => s.id !== id)!.id : cur));
  }, [handleDisconnect, simTarget]);

  const disconnectAll = useCallback(() => {
    sessionsRef.current.filter((s) => s.connected).forEach((s) => void handleDisconnect(s.id));
  }, [handleDisconnect]);

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
      const current = sessionsRef.current.find((s) => s.id === activeId)?.config.port_name;
      if (updatedList.length > 0 && !updatedList.find((p) => p.port_name === current)) {
        setConfig({ port_name: updatedList[0].port_name });
      }

      const unlistenData = await listen<{ hex: string; bytes: number[]; timestamp: number }>(
        "virtual-data",
        (event) => {
          const ts = formatTimestamp(event.payload.timestamp);
          setVirtualTraffic((prev) => appendTraffic(prev, { direction: "RX", data: event.payload.hex, timestamp: ts }));
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
  }, [addLog, refreshPorts, activeId, setConfig]);

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
      const ts = formatTimestamp(Date.now());
      if (virtualSendFormat === "hex") {
        const normalized = virtualSendData.split(/\s+/).filter(Boolean).map((h) => h.toUpperCase()).join(" ");
        setVirtualTraffic((prev) => appendTraffic(prev, { direction: "TX", data: normalized, timestamp: ts }));
        addLog("tx", `[VPort] ${normalized}`);
      } else {
        const hex = bytesToHex(new TextEncoder().encode(virtualSendData));
        setVirtualTraffic((prev) => appendTraffic(prev, { direction: "TX", data: hex, timestamp: ts }));
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

  const handleSend = useCallback(async (id: string, keepInput: boolean) => {
    const s = sessionsRef.current.find((x) => x.id === id);
    if (!s || !s.sendData || !s.connected) return;
    try {
      await invoke("send", { id, data: s.sendData, format: s.sendFormat });
      const ts = formatTimestamp(Date.now());
      if (s.sendFormat === "hex") {
        const normalized = s.sendData.split(/\s+/).filter(Boolean).map((h) => h.toUpperCase()).join(" ");
        pushTraffic(id, "TX", normalized, ts);
        addLog("tx", normalized, id);
      } else {
        pushTraffic(id, "TX", bytesToHex(new TextEncoder().encode(s.sendData)), ts);
        addLog("tx", `ASCII: ${s.sendData}`, id);
      }
      if (!keepInput) updateSession(id, { sendData: "" });
    } catch (e) {
      addLog("error", `Send failed: ${e}`, id);
      updateSession(id, { repeating: false });
    }
  }, [addLog, pushTraffic, updateSession]);

  const applyProfile = useCallback((p: PortProfile) => {
    setConfig({
      ...(p.port_name ? { port_name: p.port_name } : {}),
      baud_rate: p.baud_rate,
      data_bits: p.data_bits,
      stop_bits: p.stop_bits,
      parity: p.parity,
      timeout_ms: p.timeout_ms,
    });
  }, [setConfig]);

  // -----------------------------------------------------------------------
  // Capture to file
  // -----------------------------------------------------------------------

  const startCapture = useCallback(async () => {
    if (!requirePro("Capture to file")) return;
    try {
      const stamp = new Date().toISOString().slice(0, 19).replace(/:/g, "-");
      const path = await pickSavePath(`serial-capture-${stamp}.csv`);
      if (!path) return;
      await invoke("start_capture", { path });
      setCapture({ path, rows: 0 });
      addLog("info", `Capturing to ${path}`);
    } catch (e) {
      addLog("error", `Capture failed: ${e}`);
    }
  }, [requirePro, addLog]);

  const stopCapture = useCallback(async () => {
    try {
      const path = capture?.path;
      const rows = await invoke<number | null>("stop_capture");
      addLog("info", `Capture stopped: ${rows ?? 0} rows saved to ${path}`);
    } catch (e) {
      addLog("error", `Stopping capture failed: ${e}`);
    }
    setCapture(null);
  }, [capture, addLog]);

  const capturing = capture !== null;
  useEffect(() => {
    if (!capturing) return;
    const timer = setInterval(() => {
      invoke<CaptureState | null>("capture_status").then(setCapture).catch(() => {});
    }, 1000);
    return () => clearInterval(timer);
  }, [capturing]);

  // -----------------------------------------------------------------------
  // Cleanup on unmount
  // -----------------------------------------------------------------------

  useEffect(() => {
    return () => {
      virtualUnlistenRef.current.forEach((fn) => fn());
    };
  }, []);

  // -----------------------------------------------------------------------
  // Render
  // -----------------------------------------------------------------------

  const logNote =
    !pro && log.length >= FREE_LOG_LINES ? (
      <div style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)", flexShrink: 0 }}>
        Showing the last {FREE_LOG_LINES} log lines.{" "}
        <button
          onClick={() => openUpsell("Unlimited log scrollback")}
          style={{ background: "none", border: "none", padding: 0, color: "var(--accent)", cursor: "pointer", font: "inherit" }}
        >
          Pro keeps the full log
        </button>
      </div>
    ) : null;

  const anyConnected = sessions.some((s) => s.connected) || virtualActive;
  const monitorMode = sidebarMode === "monitor";

  const modeSegmented = virtualAvailable ? (
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
  ) : null;

  const monitorSidebar = (
    <>
      <SessionsSection sessions={sessions} activeId={active.id} onSelect={setActiveId} onAdd={addSession} />
      <MonitorSidebar
        connected={active.connected} connecting={active.connecting} ports={ports}
        portName={active.config.port_name} setPortName={(port_name) => setConfig({ port_name })}
        baudRate={active.config.baud_rate} setBaudRate={(baud_rate) => setConfig({ baud_rate })}
        dataBits={active.config.data_bits} setDataBits={(data_bits) => setConfig({ data_bits })}
        stopBits={active.config.stop_bits} setStopBits={(stop_bits) => setConfig({ stop_bits })}
        parity={active.config.parity} setParity={(parity) => setConfig({ parity })}
        timeoutMs={active.config.timeout_ms} setTimeoutMs={(timeout_ms) => setConfig({ timeout_ms })}
        lastDataTime={active.lastDataTime}
        refreshPorts={refreshPorts}
        handleConnect={() => handleConnect(active.id)} handleDisconnect={() => handleDisconnect(active.id)}
        simTemplate={simTemplate} setSimTemplate={setSimTemplate}
        simRunning={simRunning} setSimRunning={setSimRunning}
        onSimStart={startSimulation} onSimStop={stopSimulation}
        busyPorts={sessions.filter((s) => s.id !== active.id).map((s) => s.config.port_name)}
        profiles={profiles} setProfiles={setProfiles} applyProfile={applyProfile}
      />
      <CaptureSection capture={capture} onStart={startCapture} onStop={stopCapture} />
    </>
  );

  const virtualPortSidebar = (
    <VirtualPortSidebar
      virtualActive={virtualActive} virtualPortPath={virtualPortPath}
      handleCreateVirtualPort={handleCreateVirtualPort}
      handleDestroyVirtualPort={handleDestroyVirtualPort}
      handleCopyPath={handleCopyPath}
    />
  );

  /** Traffic tables and send bars: every port side by side (desktop) or the selected one (mobile). */
  const panels = (fill: boolean) => (
    <>
      <div
        style={fill
          ? { flex: 1, minHeight: 0, display: "flex", flexDirection: "column", gap: 8 }
          : { display: "flex", gap: 8, flexShrink: 0, overflowX: "auto" }}
      >
        {sessions.map((s) => (
          <SessionPanel
            key={s.id}
            session={s}
            active={s.id === active.id}
            showHeader={sessions.length > 1}
            visible={monitorMode && (!fill || s.id === active.id)}
            fill={fill}
            tableHeight={tableHeight}
            onSelect={() => setActiveId(s.id)}
            onClose={() => closeSession(s.id)}
            onUpdate={(patch) => updateSession(s.id, patch)}
            onSend={(keepInput) => handleSend(s.id, keepInput)}
            macros={macros}
            onMacrosChange={setMacros}
          />
        ))}
      </div>
      {!monitorMode && (
        <>
          <div
            style={fill
              ? { flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }
              : { height: tableHeight, minHeight: 80, display: "flex", flexDirection: "column", flexShrink: 0 }}
          >
            <TrafficTable
              data={virtualTraffic}
              emptyMessage="Virtual port is not active. Create one to capture loopback traffic."
            />
          </div>
          <SendComposer
            format={virtualSendFormat} onFormatChange={setVirtualSendFormat}
            value={virtualSendData} onValueChange={setVirtualSendData}
            onSend={handleVirtualSend} disabled={!virtualActive}
          />
        </>
      )}
    </>
  );

  const logPanel = (
    <>
      {logNote}
      <LogPanel
        entries={log}
        beforeCopyAll={() => requirePro("Copy the full log")}
        beforeExport={() => requirePro("Log export")}
        onExport={(f) => addLog("info", `Log exported to ${f}`)}
      />
    </>
  );

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100vh", background: "var(--bg)", color: "var(--text)" }}>
      <Toolbar
        title="Serial Monitor"
        icon={appIcon}
        connected={anyConnected}
        onDisconnect={() => {
          disconnectAll();
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
                {monitorMode ? monitorSidebar : virtualPortSidebar}
              </div>
            )}
            {/* Stays mounted on the other tabs so repeat-send keeps running. */}
            <div
              style={{
                display: mobileTab === "data" ? "flex" : "none",
                flexDirection: "column", height: "100%", padding: 8, gap: 8,
              }}
            >
              {panels(true)}
            </div>
            {mobileTab === "log" && logPanel}
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
          <StatusBar connected={anyConnected} />
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
            {monitorMode ? monitorSidebar : virtualPortSidebar}
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
            {panels(false)}
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
            {logPanel}
          </main>
        </div>
      )}

      {!isMobile && <StatusBar connected={anyConnected} />}

      <InfoModal show={showInfo} onClose={() => setShowInfo(false)} />
    </div>
  );
}

export default App;
