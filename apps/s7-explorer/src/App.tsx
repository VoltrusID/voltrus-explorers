import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Activity, ChartLine, Eye, Layers, Play, Plug, Plus, ScrollText, Server, Square, X } from "lucide-react";
import {
  Button,
  Dialog,
  Field,
  IconButton,
  LogPanel,
  PageTitle,
  PhoneTabBar,
  ProAboutRow,
  ProBadge,
  Section,
  StatusDot,
  Tabs,
  Toolbar,
  usePhoneShell,
  useProStatus,
  type PhoneTabItem,
} from "@mactools/ui";
import appIcon from "../src-tauri/icons/128x128.png";
import { useStore } from "./store";
import { uid } from "./helpers";
import type { Device } from "./types";
import DevicePanel from "./DevicePanel";
import TagsPanel from "./TagsPanel";
import { TEMPLATES } from "./simTemplates";
import type { PhoneTab } from "./types";
import PhoneSimulator from "./phone/PhoneSimulator";
import { useSystemScheme } from "./phone/scheme";

function App() {
  const store = useStore();
  const {
    devices, activeDeviceId, addDevice, setActiveDevice,
    phoneTab, setPhoneTab, live,
    log, addLog,
    sim, setSim,
  } = store;

  const { requirePro, status: proStatus } = useProStatus();
  const [showInfo, setShowInfo] = useState(false);
  const [removing, setRemoving] = useState<Device | null>(null);

  // Sim UI
  const [simPort, setSimPort] = useState(1102);
  const [simStarting, setSimStarting] = useState(false);
  const [simError, setSimError] = useState<string | null>(null);
  const simStartRef = useRef(0);

  // Below 768 px the phone shell is on. It follows the system colour scheme (iOS draws the status bar from it), so the
  // page sets data-theme from it; the desktop layout keeps the dark default.
  const phone = usePhoneShell();
  const scheme = useSystemScheme();
  useEffect(() => {
    if (!phone) return;
    document.documentElement.setAttribute("data-theme", scheme);
    return () => document.documentElement.removeAttribute("data-theme");
  }, [phone, scheme]);

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
    setSimError(null);
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
      setSimError(`The simulator did not start: ${String(e)}`);
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
      setSimError(`The simulator did not stop: ${String(e)}`);
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

  const addPlc = () => {
    if (devices.length === 0 || requirePro("Multiple PLC connections")) addDevice();
  };

  const useSimulator = () => {
    if (!sim.port) return;
    store.updateDevice(activeDevice.id, { host: "127.0.0.1", port: sim.port, rack: 0, slot: 0 });
    setPhoneTab("connect");
  };

  // ── Phone shell: header, one page per tab, pill tab bar ──
  const ICON = { size: 22, strokeWidth: 2 } as const;
  const phoneTabs: PhoneTabItem<PhoneTab>[] = [
    { id: "connect", label: "Connect", icon: <Plug {...ICON} />, dot: live.connected },
    { id: "watch", label: "Watch", icon: <Eye {...ICON} /> },
    { id: "blocks", label: "Blocks", icon: <Layers {...ICON} /> },
    { id: "trend", label: "Trend", icon: <ChartLine {...ICON} />, badge: live.trendCount },
    { id: "diagnostics", label: "Diagnostics", icon: <Activity {...ICON} /> },
    { id: "sim", label: "Simulator", icon: <Server {...ICON} />, badge: sim.running ? 1 : undefined },
    { id: "log", label: "Log", icon: <ScrollText {...ICON} /> },
  ];
  const selectPhoneTab = (t: PhoneTab) => {
    if (t === "blocks" && !requirePro("Block manager")) return;
    if (t === "trend" && !requirePro("Live trends")) return;
    setPhoneTab(t);
  };

  // The saved PLCs as chips on the Connect page.
  const phoneChips = (
    <Tabs
      className="vt-tabs--conn"
      label="PLC connections"
      value={activeDevice.id}
      onChange={setActiveDevice}
      items={devices.map((d) => ({
        value: d.id,
        label: (
          <>
            <span className={d.id === activeDevice.id && live.connected ? "vt-dot vt-dot--success" : "vt-dot"} aria-hidden="true" />
            {d.label || "PLC"}
          </>
        ),
      }))}
      trailing={
        <Button size="sm" variant="ghost" icon={<Plus size={16} strokeWidth={1.75} />} onClick={addPlc} aria-label="Add PLC">
          PLC <ProBadge />
        </Button>
      }
    />
  );

  const panels = devices.map((d) => (
    <DevicePanel
      key={d.id}
      device={d}
      active={d === activeDevice}
      onLog={addLog}
      phoneChips={phoneChips}
      onRemove={devices.length > 1 ? () => setRemoving(d) : undefined}
      onLoadSim={sim.template ? () => { loadSimTags(d.id, sim.template!); setPhoneTab("watch"); } : undefined}
    />
  ));

  return (
    <div className={phone ? "vm-app" : undefined} style={phone ? undefined : { display: "flex", flexDirection: "column", height: "100vh", background: "var(--bg)", color: "var(--text)" }}>
      <Toolbar
        title="S7 Explorer"
        icon={appIcon}
        connected={phone ? live.connected : false}
        context={phone && live.connected ? `${activeDevice.host}:${activeDevice.port}` : undefined}
        latency={phone ? live.latency : undefined}
        pollCount={phone ? live.pollCount : undefined}
        error={phone ? live.error : undefined}
        onInfo={() => setShowInfo(true)}
      />

      {phone ? (
        <>
          {panels}
          {phoneTab === "sim" && (
            <PhoneSimulator
              template={sim.template}
              running={sim.running}
              port={sim.port}
              portInput={simPort}
              starting={simStarting}
              error={simError}
              onPort={setSimPort}
              onTemplate={(t) => setSim({ template: t })}
              onStart={startSim}
              onStop={stopSim}
              onUse={useSimulator}
              onLoadTags={() => {
                if (!sim.template) return;
                loadSimTags(activeDevice.id, sim.template);
                setPhoneTab("watch");
              }}
            />
          )}
          {phoneTab === "log" && (
            <div className="vm-body vm-body--fill">
              <PageTitle title="Log" caption="Every request and reply" />
              <div className="vm-fill">
                <LogPanel entries={log} onExport={(f) => addLog("info", `Log exported to ${f}`)} />
              </div>
            </div>
          )}
          <PhoneTabBar tabs={phoneTabs} active={phoneTab} onChange={selectPhoneTab} />
        </>
      ) : (
        <>
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
            <IconButton label="Add PLC (Pro for more than one)" onClick={addPlc}>
              <Plus size={16} strokeWidth={1.75} />
            </IconButton>
          </div>

          {/* Workspace: devices (left/center) + simulator and tags (right sidebar) */}
          <div style={{ flex: 1, overflow: "hidden", display: "flex", flexDirection: "row", minHeight: 0 }}>
            <div style={{ flex: 1, overflow: "hidden", display: "flex", flexDirection: "column", minWidth: 0 }}>
              {panels}
            </div>
            <aside className="vt-sidebar" style={{ width: 280, flexShrink: 0, borderLeft: "1px solid var(--border)", borderTop: "none" }}>
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
        </>
      )}

      <Dialog
        open={removing !== null}
        onClose={() => setRemoving(null)}
        title={`Remove ${removing?.label ?? "PLC"}?`}
        footer={
          <>
            <Button onClick={() => setRemoving(null)}>Cancel</Button>
            <Button
              variant="danger"
              onClick={() => {
                if (removing) store.removeDevice(removing.id);
                setRemoving(null);
              }}
            >
              Remove
            </Button>
          </>
        }
      >
        <p style={{ margin: 0, fontSize: "var(--fs-md)", color: "var(--text-dim)", lineHeight: 1.5 }}>
          This deletes the saved connection and its tag list from this device. The PLC itself is not touched.
        </p>
      </Dialog>

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
          <p style={{ margin: 0 }}>
            <strong style={{ color: "var(--text)", fontWeight: 600 }}>Multi-PLC:</strong>{" "}
            {phone ? "save several PLCs and switch between them on the Connect tab, each polls independently." : "open several PLCs in tabs, each polls independently."}
          </p>
          <p style={{ margin: 0 }}><strong style={{ color: "var(--text)", fontWeight: 600 }}>Simulator:</strong> run a real S7 server on this {phone ? "phone" : "Mac"} for testing clients.</p>
          <p style={{ margin: 0 }}>Read DB, I, Q, M, T and C addresses (timers and counters are read-only). Diagnostics, block manager, live trend.</p>
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
