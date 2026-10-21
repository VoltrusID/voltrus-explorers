import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Play, Plus, Square, X } from "lucide-react";
import { Button, Dialog, Field, IconButton, ProAboutRow, ProBadge, Section, StatusDot, Toolbar, useProStatus } from "@mactools/ui";
import appIcon from "../src-tauri/icons/128x128.png";
import { useStore } from "./store";
import { uid } from "./helpers";
import DevicePanel from "./DevicePanel";
import TagsPanel from "./TagsPanel";

// ── Simulator templates ──

interface SimPoint {
  key: string;
  address: string;
  data_type: string;
  base: number;
  variance: number;
  noise: number;
  period: number;
  min?: number;
  max?: number;
  decimals?: number;
  isBool?: boolean;
  invert?: number; // seconds between boolean toggles (0 = static)
}

const TEMPLATES: Record<string, { name: string; description: string; points: SimPoint[] }> = {
  motor: {
    name: "S7-1500 Motor Control (DB1)",
    description: "VFD motor control data block: speed, current, temp, torque, power, status flags.",
    points: [
      { key: "speed", address: "DB1.DBD0", data_type: "Real", base: 1450, variance: 12, noise: 3, period: 15, min: 1400, max: 1500, decimals: 1 },
      { key: "current", address: "DB1.DBD4", data_type: "Real", base: 12.4, variance: 0.8, noise: 0.2, period: 12, min: 8, max: 18, decimals: 1 },
      { key: "temp", address: "DB1.DBD8", data_type: "Real", base: 67.2, variance: 1.5, noise: 0.3, period: 60, min: 50, max: 90, decimals: 1 },
      { key: "torque", address: "DB1.DBD12", data_type: "Real", base: 45.8, variance: 2.5, noise: 0.8, period: 14, min: 30, max: 60, decimals: 1 },
      { key: "power", address: "DB1.DBD16", data_type: "Real", base: 7.5, variance: 0.4, noise: 0.1, period: 13, min: 5, max: 10, decimals: 2 },
      { key: "running", address: "DB1.DBX20.0", data_type: "Bool", base: 1, variance: 0, noise: 0, period: 1, isBool: true },
      { key: "fault", address: "DB1.DBX20.1", data_type: "Bool", base: 0, variance: 0, noise: 0, period: 1, isBool: true },
      { key: "runtime", address: "DB1.DBW22", data_type: "Word", base: 15234, variance: 0, noise: 0, period: 3600, decimals: 0 },
      { key: "freq", address: "DB1.DBD24", data_type: "Real", base: 50, variance: 0.02, noise: 0.01, period: 10, min: 49.9, max: 50.1, decimals: 2 },
    ],
  },
  process: {
    name: "S7-300 Process Control (DB1)",
    description: "Chemical reactor: temperatures, pressures, levels, flow, pH, agitator/valve states.",
    points: [
      { key: "rtemp", address: "DB1.DBD0", data_type: "Real", base: 185.4, variance: 1.2, noise: 0.3, period: 25, min: 175, max: 195, decimals: 1 },
      { key: "rpress", address: "DB1.DBD4", data_type: "Real", base: 4.21, variance: 0.08, noise: 0.02, period: 20, min: 3.5, max: 5, decimals: 2 },
      { key: "rlvl", address: "DB1.DBD8", data_type: "Real", base: 72.5, variance: 1.5, noise: 0.5, period: 30, min: 50, max: 90, decimals: 1 },
      { key: "feed", address: "DB1.DBD12", data_type: "Real", base: 12.4, variance: 0.3, noise: 0.1, period: 18, min: 10, max: 15, decimals: 1 },
      { key: "ph", address: "DB1.DBD16", data_type: "Real", base: 7.12, variance: 0.05, noise: 0.02, period: 15, min: 6.5, max: 7.8, decimals: 2 },
      { key: "agitator", address: "DB1.DBX20.0", data_type: "Bool", base: 1, variance: 0, noise: 0, period: 1, isBool: true },
      { key: "valve_in", address: "DB1.DBX20.1", data_type: "Bool", base: 1, variance: 0, noise: 0, period: 1, isBool: true },
      { key: "valve_out", address: "DB1.DBX20.2", data_type: "Bool", base: 0, variance: 0, noise: 0, period: 1, isBool: true },
      { key: "cycle", address: "DB1.DBW24", data_type: "Word", base: 1247, variance: 0, noise: 0, period: 60, decimals: 0 },
    ],
  },
};

function App() {
  const store = useStore();
  const {
    devices, activeDeviceId, addDevice, setActiveDevice,
    isMobile, setIsMobile, mobileTab, setMobileTab,
    log, addLog,
    sim, setSim,
  } = store;

  const { requirePro, status: proStatus } = useProStatus();
  const [showInfo, setShowInfo] = useState(false);

  // Sim UI
  const [simPort, setSimPort] = useState(1102);
  const [simStarting, setSimStarting] = useState(false);
  const simStartRef = useRef(0);

  // ── Mobile detection ──
  useEffect(() => {
    const onResize = () => setIsMobile(window.innerWidth < 768);
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, [setIsMobile]);

  // ── Sim log listener ──
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    listen<{ level: string; message: string }>("sim-log", (e) => {
      addLog(e.payload.level === "error" ? "error" : "info", e.payload.message);
    }).then((fn) => { unlisten = fn; });
    return () => { unlisten?.(); };
  }, [addLog]);

  // ── Start the real S7 simulator ──
  const startSim = async () => {
    const tmpl = sim.template;
    if (!tmpl || !requirePro("S7 simulator")) return;
    setSimStarting(true);
    try {
      const points = TEMPLATES[tmpl].points.map((p) => ({
        address: p.address,
        data_type: p.data_type,
        value: p.isBool ? p.base : p.base,
      }));
      const info = await invoke<{ port: number; point_count: number }>("sim_start", { points, port: simPort });
      simStartRef.current = Date.now();
      setSim({ running: true, port: info.port });
      addLog("info", `Simulator started: ${TEMPLATES[tmpl].name} on port ${info.port}`);
    } catch (e) {
      addLog("error", `Simulator failed to start: ${String(e)}`);
    } finally {
      setSimStarting(false);
    }
  };

  const stopSim = async () => {
    try {
      await invoke("sim_stop");
      setSim({ running: false, port: null });
      addLog("info", "Simulator stopped");
    } catch (e) {
      addLog("error", `Simulator stop failed: ${String(e)}`);
    }
  };

  // Populate a device's tag list from the running simulator template so the
  // addresses + data types line up with the simulated image.
  const loadSimTags = (deviceId: string, template: string) => {
    const pts = TEMPLATES[template].points.map((p) => ({
      id: uid(),
      name: p.key,
      address: p.address,
      data_type: p.data_type,
    }));
    store.setDeviceTags(deviceId, pts);
    addLog("info", `Loaded ${pts.length} tags from simulator`);
  };

  // ── Animate sim values into the backend image ──
  useEffect(() => {
    if (!sim.running || !sim.template) return;
    const tmpl = TEMPLATES[sim.template];
    const interval = setInterval(async () => {
      const t = (Date.now() - simStartRef.current) / 1000;
      for (const p of tmpl.points) {
        try {
          if (p.isBool) {
            // Static initial state; toggle fault every 30s for liveliness.
            if (p.key === "fault") {
              await invoke("sim_set_bool", { address: p.address, active: Math.floor(t / 30) % 2 === 1 });
            } else {
              await invoke("sim_set_bool", { address: p.address, active: p.base >= 0.5 });
            }
          } else {
            let v = p.base + Math.sin((t / p.period) * Math.PI * 2) * p.variance + (Math.random() - 0.5) * p.noise;
            if (p.min !== undefined) v = Math.max(p.min, v);
            if (p.max !== undefined) v = Math.min(p.max, v);
            await invoke("sim_set_value", { address: p.address, dataType: p.data_type, value: v });
          }
        } catch { /* device may be disconnecting */ }
      }
    }, 1000);
    return () => clearInterval(interval);
  }, [sim.running, sim.template]);

  // Fall back to the first device so a fresh install (or a stale persisted
  // active id) always shows a pane instead of a blank workspace.
  const activeDevice = devices.find((d) => d.id === activeDeviceId) ?? devices[0];

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100vh", background: "var(--bg)", color: "var(--text)" }}>
      <Toolbar title="S7 Explorer" icon={appIcon} connected={false} onInfo={() => setShowInfo(true)} />

      {/* Device tab strip */}
      <div className="vt-tabs" role="tablist" aria-label="PLC connections" style={{ background: "var(--surface)" }}>
        {devices.map((d) => (
          <div key={d.id} style={{ display: "flex", alignItems: "center", gap: 2 }}>
            <button
              role="tab"
              aria-selected={d === activeDevice}
              className="vt-tab"
              onClick={() => setActiveDevice(d.id)}
              title={`${d.host}:${d.port}`}
            >
              <span className="vt-dot" aria-hidden="true" />
              {d.label}
            </button>
            {devices.length > 1 && (
              <IconButton
                size="sm"
                label={`Close ${d.label}`}
                onClick={() => { if (confirm(`Remove ${d.label}?`)) store.removeDevice(d.id); }}
              >
                <X size={14} strokeWidth={1.75} />
              </IconButton>
            )}
          </div>
        ))}
        <IconButton label="Add PLC (Pro for more than one)" onClick={() => { if (devices.length === 0 || requirePro("Multiple PLC connections")) addDevice(); }}>
          <Plus size={16} strokeWidth={1.75} />
        </IconButton>
      </div>

      {/* Workspace: devices (left/center) + simulator (right sidebar / mobile bottom) */}
      <div style={{ flex: 1, overflow: "hidden", display: "flex", flexDirection: isMobile ? "column" : "row", minHeight: 0 }}>
        <div style={{ flex: 1, overflow: "hidden", display: "flex", flexDirection: "column", minWidth: 0 }}>
          {devices.map((d) => (
            <DevicePanel key={d.id} device={d} active={d === activeDevice} onLog={addLog} />
          ))}
        </div>
        <aside
          className="vt-sidebar"
          style={{
            width: isMobile ? undefined : 280,
            flexShrink: 0,
            borderLeft: isMobile ? "none" : "1px solid var(--border)",
            borderTop: isMobile ? "1px solid var(--border)" : "none",
          }}
        >
          <SimulatorPanel
            template={sim.template}
            running={sim.running}
            port={sim.port}
            portInput={simPort}
            starting={simStarting}
            onPort={setSimPort}
            onTemplate={(t) => setSim({ template: t })}
            onStart={startSim}
            onStop={stopSim}
          />
          <TagsPanel
            device={activeDevice}
            onLoadSim={sim.template ? () => loadSimTags(activeDevice.id, sim.template!) : undefined}
          />
        </aside>
      </div>

      {/* Mobile tab bar */}
      {isMobile && (
        <nav style={{ display: "flex", borderTop: "1px solid var(--border)", background: "var(--surface)", flexShrink: 0 }}>
          {(["connect", "data", "diagnostics", "blocks", "log", "trend"] as const).map((t) => (
            <button key={t} onClick={() => { if (t === "blocks" && !requirePro("Block manager")) return; if (t === "trend" && !requirePro("Live trends")) return; setMobileTab(t); }} style={{
              flex: 1, padding: "10px 0 8px", fontSize: "var(--fs-xs)", fontWeight: 600, background: "none", border: "none",
              color: mobileTab === t ? "var(--accent-text)" : "var(--text-muted)",
              borderTop: mobileTab === t ? "2px solid var(--accent)" : "2px solid transparent", cursor: "pointer",
            }}>{t === "connect" ? "Connect" : t[0].toUpperCase() + t.slice(1)}</button>
          ))}
        </nav>
      )}

      <Dialog
        open={showInfo}
        onClose={() => setShowInfo(false)}
        title="About S7 Explorer"
        width={400}
        footer={
          <>
            {/* No outbound source link in App Store builds: review treats links to builds that unlock paid
                features elsewhere as steering away from in-app purchase. */}
            {proStatus && !proStatus.store && (
              <Button size="sm" variant="ghost" onClick={() => invoke("open_url", { url: "https://github.com/VoltrusID/voltrus-explorers" })}>
                Source on GitHub
              </Button>
            )}
            <Button size="sm" onClick={() => setShowInfo(false)}>Close</Button>
          </>
        }
      >
        <div style={{ display: "flex", flexDirection: "column", gap: 10, fontSize: "var(--fs-sm)", color: "var(--text-dim)", lineHeight: 1.6 }}>
          <p style={{ margin: 0 }}>Connect to S7-compatible PLCs (S7-300, S7-400, S7-1200, S7-1500) over Ethernet (ISO-on-TCP, port 102).</p>
          <p style={{ margin: 0 }}><strong style={{ color: "var(--text)", fontWeight: 600 }}>Multi-PLC:</strong> open several PLCs in tabs, each polls independently.</p>
          <p style={{ margin: 0 }}><strong style={{ color: "var(--text)", fontWeight: 600 }}>Simulator:</strong> run a real S7 server on this Mac for testing clients.</p>
          <p style={{ margin: 0 }}>Read/write DB, I, Q, M, T, C. Diagnostics, block manager, live trend.</p>
          <p style={{ margin: 0, color: "var(--text-muted)" }}>
            Part of the <strong style={{ color: "var(--text)" }}>Voltrus</strong> suite by JoyoDigitama. Siemens, SIMATIC, S7-300, S7-400, S7-1200,
            S7-1500 and TIA Portal are trademarks of Siemens AG. S7 Explorer is an independent tool and is not affiliated with or endorsed by Siemens.
          </p>
          <ProAboutRow />
        </div>
      </Dialog>
    </div>
  );
}

// ── Simulator panel (right sidebar) ──

interface SimPanelProps {
  template: string | null;
  running: boolean;
  port: number | null;
  portInput: number;
  starting: boolean;
  onPort: (n: number) => void;
  onTemplate: (t: string | null) => void;
  onStart: () => void;
  onStop: () => void;
}

function SimulatorPanel({ template, running, port, portInput, starting, onPort, onTemplate, onStart, onStop }: SimPanelProps) {
  return (
    <Section
      title="Simulator"
      meta={
        <span style={{ display: "inline-flex", alignItems: "center", gap: 6 }}>
          <ProBadge />
          <StatusDot tone={running ? "success" : "neutral"} label={running ? "Simulator running" : "Simulator stopped"} />
          {running ? `Port ${port ?? ""}` : "Stopped"}
        </span>
      }
    >
      <div style={{ display: "flex", flexDirection: "column", gap: 10, padding: 14 }}>
        <Field label="Template" hint={template ? TEMPLATES[template].description : undefined}>
          {(id) => (
            <select
              id={id}
              className="vt-select"
              value={template ?? ""}
              disabled={running}
              onChange={(e) => onTemplate(e.target.value || null)}
            >
              <option value="">Select…</option>
              {Object.entries(TEMPLATES).map(([tid, t]) => <option key={tid} value={tid}>{t.name}</option>)}
            </select>
          )}
        </Field>

        <Field label="TCP port">
          {(id) => (
            <input
              id={id}
              type="number"
              min={1}
              max={65535}
              className="vt-input"
              value={portInput}
              disabled={running}
              onChange={(e) => onPort(Number(e.target.value))}
            />
          )}
        </Field>

        {!running ? (
          <Button variant="primary" block icon={<Play size={14} strokeWidth={1.75} />} disabled={!template || starting} onClick={onStart}>
            {starting ? "Starting…" : "Start server"}
          </Button>
        ) : (
          <Button variant="danger" block icon={<Square size={14} strokeWidth={1.75} />} onClick={onStop}>
            Stop server
          </Button>
        )}

        {running && port ? (
          <div style={{ display: "flex", flexDirection: "column", gap: 4, padding: 10, background: "var(--sunken)", borderRadius: "var(--radius)", fontSize: "var(--fs-xs)", color: "var(--text-muted)", lineHeight: 1.5 }}>
            <span style={{ display: "inline-flex", alignItems: "center", gap: 6, color: "var(--text)", fontWeight: 600 }}>
              <StatusDot tone="success" label="Simulator live" />
              Live on port {port}
            </span>
            <span>
              Point a client at <span className="mono" style={{ color: "var(--text)" }}>127.0.0.1:{port}</span> (or this Mac's LAN IP). Rack 0, slot 0.
            </span>
          </div>
        ) : (
          <div style={{ fontSize: "var(--fs-xs)", color: "var(--text-dim)", lineHeight: 1.5 }}>
            Start the server, then add a PLC tab pointing at <span className="mono">127.0.0.1</span> on the bound port to self-test, or let external S7 clients (snap7, TIA Portal) connect.
          </div>
        )}
      </div>
    </Section>
  );
}

export default App;
