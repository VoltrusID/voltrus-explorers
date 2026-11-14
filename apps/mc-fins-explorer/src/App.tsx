import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Play, Plus, Square, X } from "lucide-react";
import { Button, Dialog, Field, IconButton, type LogEntry, ProAboutRow, ProBadge, Section, StatusDot, Toolbar, useProStatus } from "@mactools/ui";
import appIcon from "../src-tauri/icons/128x128.png";
import { useStore } from "./store";
import { uid } from "./helpers";
import type { Protocol } from "./types";
import DevicePanel from "./DevicePanel";
import TagsPanel from "./TagsPanel";
import { TEMPLATES, isAnimated, simValue } from "./simTemplates";

interface FrameEvent {
  device_id: string;
  direction: "tx" | "rx";
  bytes: string;
  summary: string;
}

function App() {
  const store = useStore();
  const { devices, activeDeviceId, addDevice, setActiveDevice, addLog, sim, setSim } = store;
  const { requirePro, status: proStatus } = useProStatus();
  const [showInfo, setShowInfo] = useState(false);
  const [removing, setRemoving] = useState<string | null>(null);
  const [ports, setPorts] = useState({ mc: 5000, fins: 9600, node: 10 });
  const [simBusy, setSimBusy] = useState(false);
  const [simError, setSimError] = useState<string | null>(null);
  const [simMode, setSimMode] = useState<{ mc_run: boolean; fins_mode: string } | null>(null);
  const simStart = useRef(0);

  const activeDevice = devices.find((d) => d.id === activeDeviceId) ?? devices[0];

  // Simulator log lines and frames from the backend go to the comm log. Frames are buffered and added a few times a
  // second: one render per frame made polling of a long tag list slow.
  useEffect(() => {
    let disposed = false;
    const offs: (() => void)[] = [];
    const keep = (p: Promise<() => void>) => void p.then((off) => (disposed ? off() : offs.push(off)));
    const pending: LogEntry[] = [];
    keep(listen<{ level: string; message: string }>("sim-log", (e) => {
      pending.push({ timestamp: Date.now(), direction: e.payload.level === "error" ? "error" : "info", message: e.payload.message });
    }));
    keep(listen<FrameEvent>("frame", (e) => {
      const st = useStore.getState();
      if (!st.showFrames) return;
      const label = st.devices.find((d) => d.id === e.payload.device_id)?.label ?? "PLC";
      pending.push({ timestamp: Date.now(), direction: e.payload.direction, message: `[${label}] ${e.payload.summary} · ${e.payload.bytes}` });
    }));
    const flush = setInterval(() => useStore.getState().addLogs(pending.splice(0)), 250);
    return () => {
      disposed = true;
      clearInterval(flush);
      offs.forEach((f) => f());
    };
  }, []);

  const startSim = async () => {
    if (!requirePro("Built-in PLC simulator")) return;
    setSimBusy(true);
    setSimError(null);
    try {
      const info = await invoke<{ mc_port: number; fins_port: number; fins_node: number }>("sim_start", {
        mcPort: ports.mc,
        finsPort: ports.fins,
        finsNode: ports.node,
      });
      simStart.current = Date.now();
      setSim({ running: true, mcPort: info.mc_port, finsPort: info.fins_port, finsNode: info.fins_node });
    } catch (e) {
      setSimError(String(e));
      addLog("error", String(e));
    } finally {
      setSimBusy(false);
    }
  };

  const stopSim = async () => {
    await invoke("sim_stop").catch(() => undefined);
    setSim({ running: false, mcPort: null, finsPort: null, finsNode: null });
    setSimMode(null);
    addLog("info", "Simulator stopped");
  };

  // Animate both templates once a second while the simulated CPU is in RUN (a remote STOP freezes it).
  useEffect(() => {
    if (!sim.running) return;
    // Fixed points are seeded once, so a value a client writes to them stays.
    let seeded = false;
    const t = setInterval(async () => {
      try {
        const st = await invoke<{ running: boolean; mc_run: boolean; fins_mode: string }>("sim_status");
        setSimMode({ mc_run: st.mc_run, fins_mode: st.fins_mode });
        const secs = (Date.now() - simStart.current) / 1000;
        const writes = (["mc", "fins"] as Protocol[])
          .filter((p) => (p === "mc" ? st.mc_run : st.fins_mode !== "PROGRAM"))
          .flatMap((p) => TEMPLATES[p].points.filter((pt) => !seeded || isAnimated(pt)).map((pt) => ({ protocol: p, address: pt.address, data_type: pt.data_type, value: simValue(pt, secs) })));
        if (writes.length > 0) await invoke("sim_set", { writes });
        seeded = true;
      } catch (e) {
        addLog("error", `Simulator: ${String(e)}`);
      }
    }, 1000);
    return () => clearInterval(t);
  }, [sim.running, addLog]);

  /** Point the active PLC tab at the simulator for its protocol and load the template's tags. */
  const connectToSim = (p: Protocol) => {
    const port = p === "mc" ? sim.mcPort : sim.finsPort;
    if (!port) return;
    store.updateDevice(activeDevice.id, {
      protocol: p,
      host: "127.0.0.1",
      port,
      label: p === "mc" ? "MC simulator" : "FINS simulator",
    });
    loadSimTags(p);
  };

  const loadSimTags = (p: Protocol) => {
    const pts = TEMPLATES[p].points.map((pt) => ({ id: uid(), name: pt.key, address: pt.address, data_type: pt.data_type }));
    store.setDeviceTags(activeDevice.id, pts);
    addLog("info", `Loaded ${pts.length} tags from the ${TEMPLATES[p].name} template`);
  };

  const addPlc = () => {
    if (requirePro("Several PLC connections")) addDevice(activeDevice.protocol);
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100vh", background: "var(--bg)", color: "var(--text)" }}>
      <Toolbar title="MC & FINS Explorer" icon={appIcon} connected={false} onInfo={() => setShowInfo(true)} />

      <div className="vt-tabs" role="tablist" aria-label="PLC connections" style={{ background: "var(--surface)" }}>
        {devices.map((d) => (
          <div key={d.id} style={{ display: "flex", alignItems: "center", gap: 2 }}>
            <button role="tab" aria-selected={d === activeDevice} className="vt-tab" onClick={() => setActiveDevice(d.id)} title={`${d.host}:${d.port}`}>
              <span className="vt-dot" aria-hidden="true" />
              {d.label}
              <span style={{ color: "var(--text-muted)", fontSize: "var(--fs-xs)", marginLeft: 6 }}>{d.protocol === "mc" ? "MC" : "FINS"}</span>
            </button>
            {devices.length > 1 && (
              <IconButton size="sm" label={`Close ${d.label}`} onClick={() => setRemoving(d.id)}>
                <X size={14} strokeWidth={1.75} />
              </IconButton>
            )}
          </div>
        ))}
        <IconButton label="Add PLC (Pro for more than one)" onClick={addPlc}>
          <Plus size={16} strokeWidth={1.75} />
        </IconButton>
      </div>

      <div style={{ flex: 1, overflow: "hidden", display: "flex", minHeight: 0 }}>
        <div style={{ flex: 1, overflow: "hidden", display: "flex", flexDirection: "column", minWidth: 0 }}>
          {devices.map((d) => (
            <DevicePanel key={d.id} device={d} active={d === activeDevice} onLog={addLog} />
          ))}
        </div>
        <aside className="vt-sidebar" style={{ width: 290, flexShrink: 0, borderLeft: "1px solid var(--border)", overflowY: "auto" }}>
          <Section
            title="Simulator"
            meta={
              <span style={{ display: "inline-flex", alignItems: "center", gap: 6 }}>
                <ProBadge />
                <StatusDot tone={sim.running ? "success" : "neutral"} label={sim.running ? "Simulator running" : "Simulator stopped"} />
                {sim.running ? "Running" : "Stopped"}
              </span>
            }
          >
            <div style={{ display: "flex", flexDirection: "column", gap: 10, padding: 14 }}>
              <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr 0.8fr", gap: 8 }}>
                <Field label="MC port">
                  {(id) => <input id={id} type="number" className="vt-input" value={ports.mc} disabled={sim.running} onChange={(e) => setPorts({ ...ports, mc: Number(e.target.value) })} />}
                </Field>
                <Field label="FINS port">
                  {(id) => <input id={id} type="number" className="vt-input" value={ports.fins} disabled={sim.running} onChange={(e) => setPorts({ ...ports, fins: Number(e.target.value) })} />}
                </Field>
                <Field label="Node">
                  {(id) => <input id={id} type="number" min={1} max={254} className="vt-input" value={ports.node} disabled={sim.running} onChange={(e) => setPorts({ ...ports, node: Number(e.target.value) })} />}
                </Field>
              </div>
              {!sim.running ? (
                <Button variant="primary" block loading={simBusy} icon={<Play size={14} strokeWidth={1.75} />} onClick={() => void startSim()}>
                  Start simulator
                </Button>
              ) : (
                <Button variant="danger" block icon={<Square size={14} strokeWidth={1.75} />} onClick={() => void stopSim()}>
                  Stop simulator
                </Button>
              )}
              {simError && <div role="alert" style={{ color: "var(--danger)", fontSize: "var(--fs-xs)", lineHeight: 1.5 }}>{simError}</div>}
              {sim.running ? (
                <div style={{ display: "flex", flexDirection: "column", gap: 8, padding: 10, background: "var(--sunken)", borderRadius: "var(--radius)", fontSize: "var(--fs-xs)", color: "var(--text-muted)", lineHeight: 1.5 }}>
                  <div>
                    <strong style={{ color: "var(--text)" }}>{TEMPLATES.mc.name}</strong>: MC protocol 3E/4E, binary or ASCII, on TCP and UDP <span className="mono" style={{ color: "var(--text)" }}>{sim.mcPort}</span>
                    {simMode && !simMode.mc_run && <> · <span style={{ color: "var(--warning)" }}>STOP</span></>}
                  </div>
                  <Button size="sm" onClick={() => connectToSim("mc")}>Connect this tab to the MC simulator</Button>
                  <div>
                    <strong style={{ color: "var(--text)" }}>{TEMPLATES.fins.name}</strong>: FINS/UDP and FINS/TCP <span className="mono" style={{ color: "var(--text)" }}>{sim.finsPort}</span>, node {sim.finsNode}
                    {simMode && simMode.fins_mode !== "RUN" && <> · <span style={{ color: "var(--warning)" }}>{simMode.fins_mode}</span></>}
                  </div>
                  <Button size="sm" onClick={() => connectToSim("fins")}>Connect this tab to the FINS simulator</Button>
                  <div>Other tools (SCADA, HMI, Node-RED) can connect to this computer's IP on the same ports.</div>
                </div>
              ) : (
                <div style={{ fontSize: "var(--fs-xs)", color: "var(--text-dim)", lineHeight: 1.5 }}>
                  Serves a simulated Mitsubishi CPU (MC protocol) and Omron CPU (FINS) from this computer, with live values, so you can try the app or test a SCADA or HMI without hardware.
                </div>
              )}
            </div>
          </Section>
          <TagsPanel device={activeDevice} onLoadSim={sim.running ? () => loadSimTags(activeDevice.protocol) : undefined} />
        </aside>
      </div>

      <Dialog
        open={removing !== null}
        onClose={() => setRemoving(null)}
        title={`Remove ${devices.find((d) => d.id === removing)?.label ?? "PLC"}?`}
        footer={
          <>
            <Button onClick={() => setRemoving(null)}>Cancel</Button>
            <Button variant="danger" onClick={() => { if (removing) { void invoke("disconnect", { deviceId: removing }); store.removeDevice(removing); } setRemoving(null); }}>
              Remove
            </Button>
          </>
        }
      >
        <p style={{ margin: 0, fontSize: "var(--fs-md)", color: "var(--text-dim)", lineHeight: 1.5 }}>
          This deletes the saved connection and its tag list. The PLC itself is not touched.
        </p>
      </Dialog>

      <Dialog
        open={showInfo}
        onClose={() => setShowInfo(false)}
        title="About MC & FINS Explorer"
        width={420}
        footer={
          <>
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
          <p style={{ margin: 0 }}>
            Read and write Mitsubishi MELSEC PLCs over MC protocol (SLMP 3E and 4E frames, binary or ASCII, TCP or UDP) and Omron PLCs over FINS/UDP and FINS/TCP.
          </p>
          <p style={{ margin: 0 }}>
            <strong style={{ color: "var(--text)", fontWeight: 600 }}>Comm log:</strong> every request and reply as bytes, with the command and the end code explained.
          </p>
          <p style={{ margin: 0 }}>
            <strong style={{ color: "var(--text)", fontWeight: 600 }}>Simulator:</strong> a simulated Mitsubishi and Omron CPU on this computer for testing clients.
          </p>
          <p style={{ margin: 0, color: "var(--text-muted)" }}>
            Part of the <strong style={{ color: "var(--text)" }}>Voltrus</strong> suite by JoyoDigitama. Mitsubishi Electric, MELSEC, iQ-R, iQ-F and SLMP are trademarks of Mitsubishi Electric
            Corporation; OMRON, SYSMAC and FINS are trademarks of OMRON Corporation. MC & FINS Explorer is an independent tool and is not affiliated with or endorsed by either.
          </p>
          <ProAboutRow />
        </div>
      </Dialog>
    </div>
  );
}

export default App;
