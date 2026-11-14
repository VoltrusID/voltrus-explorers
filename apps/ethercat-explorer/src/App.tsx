import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  Badge,
  Button,
  Callout,
  Dialog,
  EmptyState,
  Field,
  LogPanel,
  ProAboutRow,
  ProBadge,
  Section,
  Segmented,
  SheetSelect,
  StatusBar,
  StatusDot,
  Tabs,
  Toolbar,
  nextTrendColor,
  noteDeviceError,
  noteDeviceSuccess,
  saveTextFile,
  useProStatus,
  withUtm,
} from "@mactools/ui";
import type { LogEntry } from "@mactools/ui";
import { Cpu, Download, Play, Plug, RefreshCw, Square, X } from "lucide-react";
import appIcon from "../src-tauri/icons/128x128.png";
import DeviceDetail, { EntryTable } from "./DeviceDetail";
import FramesView from "./FramesView";
import SimView from "./SimView";
import TrendPanel, { type TrendSeries } from "./TrendPanel";
import { STATES, devicesCsv, entryNumber, extractBits, hex, interfaceLabel } from "./ecat";
import type { CycleStats, FrameView, IoEvent, IoImage, LinkSpec, NetInterface, PdoMapView, RawAccess, ScanResult, StateView, Template } from "./types";
import { StateBadge, dim, hint, row, scroller, th } from "./ui";

type Tab = "devices" | "process" | "frames" | "simulator";
type Mode = "interface" | "simulator";

const MAX_LOG = 2000;
const MAX_FRAMES = 5000;

const TEMPLATES: { value: Template; label: string; description: string }[] = [
  { value: "machine", label: "Machine", description: "EK1100 coupler, EL1008, EL2008, EL3102 and EL4102 terminals, and two CiA 402 servo drives." },
  { value: "ioStation", label: "I/O station", description: "EK1100 coupler with digital and analog input and output terminals." },
  { value: "drives", label: "Two servo drives", description: "Two CiA 402 drives: state machine, CSP, CSV and profile position." },
];

function App() {
  const { requirePro, pro, status: proStatus } = useProStatus();
  const [mode, setMode] = useState<Mode>("simulator");
  const [tab, setTab] = useState<Tab>("devices");
  const [bottomTab, setBottomTab] = useState<"log" | "trend">("log");
  const [showInfo, setShowInfo] = useState(false);
  const [log, setLog] = useState<LogEntry[]>([]);
  const [error, setError] = useState<string | null>(null);

  const [ifaces, setIfaces] = useState<NetInterface[]>([]);
  const [iface, setIface] = useState("");
  const [access, setAccess] = useState<RawAccess | null>(null);
  const [template, setTemplate] = useState<Template>("machine");

  const [scan, setScan] = useState<ScanResult | null>(null);
  const [linkKind, setLinkKind] = useState<Mode | null>(null);
  const [connecting, setConnecting] = useState(false);
  const [busy, setBusy] = useState(false);
  const [selected, setSelected] = useState(0);
  const [states, setStates] = useState<StateView[]>([]);
  const [images, setImages] = useState<IoImage[]>([]);
  const [stats, setStats] = useState<CycleStats | null>(null);
  const [pdos, setPdos] = useState<Map<number, PdoMapView>>(new Map());
  const [cycleMs, setCycleMs] = useState("10");
  const [frames, setFrames] = useState<FrameView[]>([]);
  const startedAt = useRef(Date.now());

  const [trend, setTrend] = useState<Map<string, TrendSeries>>(new Map());
  const trendRef = useRef(trend);
  trendRef.current = trend;
  const pdosRef = useRef(pdos);
  pdosRef.current = pdos;
  const [trendWindow, setTrendWindow] = useState(60);

  const addLog = useCallback((direction: LogEntry["direction"], message: string) => {
    setLog((prev) => {
      const next = [...prev, { timestamp: Date.now(), direction, message }];
      return next.length > MAX_LOG ? next.slice(-MAX_LOG) : next;
    });
  }, []);

  const connected = scan !== null;

  // ── Events from Rust ──
  useEffect(() => {
    const subs = [
      listen<{ direction: LogEntry["direction"]; message: string }>("ecat-log", (e) => addLog(e.payload.direction, e.payload.message)),
      listen<FrameView[]>("ecat-frames", (e) => setFrames((prev) => [...prev, ...e.payload].slice(-MAX_FRAMES))),
      listen<IoEvent>("ecat-io", (e) => {
        const { stats: s, devices } = e.payload;
        setStats(s);
        setImages(devices);
        if (trendRef.current.size === 0) return;
        const now = Date.now();
        setTrend((prev) => {
          const next = new Map(prev);
          for (const [key, series] of prev) {
            const [pos, index, sub, bit] = key.split(":").map(Number);
            const map = pdosRef.current.get(pos);
            const entry = map && [...map.inputs, ...map.outputs].flatMap((p) => p.entries).find((x) => x.index === index && x.sub === sub && x.bitOffset === bit);
            if (!entry) continue;
            const isIn = map.inputs.some((p) => p.entries.includes(entry));
            const img = isIn ? devices[pos]?.inputs : devices[pos]?.outputs;
            if (!img || img.length * 8 < entry.bitOffset + entry.bits) continue;
            const v = entryNumber(entry, extractBits(img, entry.bitOffset, entry.bits));
            if (v === null) continue;
            const cutoff = now - trendWindow * 1000 - 5000;
            const points = [...series.points, { t: now, v }].filter((p) => p.t > cutoff);
            next.set(key, { ...series, points: points.length > 3000 ? points.slice(-3000) : points });
          }
          return next;
        });
      }),
    ];
    return () => { subs.forEach((s) => s.then((un) => un())); };
  }, [addLog, trendWindow]);

  // ── Interfaces and raw access ──
  const refreshIfaces = useCallback(async () => {
    try {
      const list = await invoke<NetInterface[]>("ecat_interfaces");
      setIfaces(list);
      setIface((cur) => (cur && list.some((i) => i.name === cur) ? cur : list.find((i) => !i.wireless)?.name ?? ""));
      setAccess(await invoke<RawAccess>("ecat_raw_access"));
    } catch (e) {
      addLog("error", `Listing network interfaces failed: ${String(e)}`);
    }
  }, [addLog]);
  useEffect(() => { void refreshIfaces(); }, [refreshIfaces]);

  const grant = async () => {
    try {
      await invoke("ecat_grant_raw_access");
      setAccess(await invoke<RawAccess>("ecat_raw_access"));
      addLog("info", "Raw Ethernet access granted until this Mac restarts");
    } catch (e) {
      if (String(e) !== "Cancelled") setError(String(e));
    }
  };

  // ── Connect ──
  const loadPdos = useCallback(async (r: ScanResult) => {
    const m = new Map<number, PdoMapView>();
    for (const d of r.devices) {
      try {
        m.set(d.position, await invoke<PdoMapView>("ecat_pdo_map", { position: d.position }));
        setPdos(new Map(m));
      } catch (e) {
        addLog("error", `${d.name}: PDO mapping: ${String(e)}`);
      }
    }
  }, [addLog]);

  const applyScan = useCallback((r: ScanResult, kind: Mode) => {
    setScan(r);
    setLinkKind(kind);
    setSelected(0);
    setStates(r.devices.map((d) => ({ position: d.position, state: d.state, alCode: d.alCode, alText: d.alText })));
    setImages([]);
    setStats(null);
    setPdos(new Map());
    setTrend(new Map());
    setFrames([]);
    startedAt.current = Date.now();
    if (r.initError) addLog("error", `EtherCAT init failed (${r.initError}); the list comes from a raw scan`);
    else addLog("info", `${r.link}: ${r.devices.length} devices`);
    void loadPdos(r);
  }, [addLog, loadPdos]);

  const connect = useCallback(async (spec: LinkSpec) => {
    setConnecting(true);
    setError(null);
    try {
      const r = await invoke<ScanResult>("ecat_connect", { link: spec });
      applyScan(r, spec.kind);
      setTab("devices");
      noteDeviceSuccess();
    } catch (e) {
      setError(String(e));
      addLog("error", String(e));
      noteDeviceError();
    } finally {
      setConnecting(false);
    }
  }, [addLog, applyScan]);

  const disconnect = useCallback(async () => {
    try { await invoke("ecat_disconnect"); } catch { /* not connected */ }
    setScan(null);
    setLinkKind(null);
    setStats(null);
    setImages([]);
  }, []);

  const rescan = useCallback(async () => {
    if (!linkKind) return;
    setConnecting(true);
    try {
      applyScan(await invoke<ScanResult>("ecat_rescan"), linkKind);
    } catch (e) {
      setError(String(e));
    } finally {
      setConnecting(false);
    }
  }, [applyScan, linkKind]);

  // AL states (with status codes) read from every device once a second.
  useEffect(() => {
    if (!connected) return;
    const t = setInterval(async () => {
      try { setStates(await invoke<StateView[]>("ecat_states")); } catch { /* gone */ }
    }, 1000);
    return () => clearInterval(t);
  }, [connected]);

  const requestState = useCallback(async (state: string, positions: number[]) => {
    if (state === "OP" && !pro && !requirePro("OP with outputs")) return;
    setBusy(true);
    addLog("tx", `Request ${state} for ${positions.length ? positions.map((p) => `device ${p}`).join(", ") : "all devices"}`);
    try {
      const res = await invoke<StateView[]>("ecat_request_state", { state, positions });
      setStates((prev) => {
        const next = [...prev];
        for (const v of res) next[v.position] = v;
        return next;
      });
    } catch (e) {
      setError(String(e));
      addLog("error", String(e));
    } finally {
      setBusy(false);
    }
  }, [addLog, pro, requirePro]);

  const applyCycle = async () => {
    const ms = Number(cycleMs);
    if (!Number.isFinite(ms) || ms < 1 || ms > 1000) { setError("Cycle time between 1 and 1000 ms"); return; }
    try { await invoke("ecat_set_cycle", { ms }); addLog("info", `Cycle time ${ms} ms`); } catch (e) { setError(String(e)); }
  };

  const toggleCycle = async () => {
    try { await invoke("ecat_cycle", { run: !stats?.running }); } catch (e) { setError(String(e)); }
  };

  const toggleTrend = useCallback((key: string, label: string) => {
    if (!trendRef.current.has(key) && !requirePro("Live trends")) return;
    setTrend((prev) => {
      const next = new Map(prev);
      if (next.has(key)) next.delete(key);
      else next.set(key, { name: label, color: nextTrendColor(Array.from(next.values(), (s) => s.color)), points: [] });
      return next;
    });
    setBottomTab("trend");
  }, [requirePro]);

  const exportDevices = async () => {
    if (!scan || !requirePro("Device list export")) return;
    const path = await saveTextFile("ethercat-devices.csv", devicesCsv(scan.devices));
    if (path) addLog("info", `Device list saved to ${path}`);
  };

  const openPrivacy = useCallback(async () => {
    try { await invoke("open_url", { url: await withUtm("https://voltrus.id/privacy/", "privacy") }); } catch (e) { addLog("error", String(e)); }
  }, [addLog]);

  const stateCounts = useMemo(() => {
    const m = new Map<string, number>();
    for (const s of states) {
      const k = s.alCode != null ? `${s.state} + error` : s.state;
      m.set(k, (m.get(k) ?? 0) + 1);
    }
    return [...m.entries()];
  }, [states]);

  // ── Sidebar ──
  const ifaceOptions = [
    ...(iface && !ifaces.some((i) => i.name === iface) ? [{ value: iface, label: `${iface} (not present)` }] : []),
    ...ifaces.map((i) => ({ value: i.name, label: interfaceLabel(i) })),
  ];

  const networkSection = (
    <Section title="Network">
      <Segmented
        label="Link"
        block
        value={mode}
        onChange={(v) => { if (!connected) setMode(v); }}
        options={[{ value: "interface", label: "Ethernet port" }, { value: "simulator", label: "Simulator" }]}
      />
      {mode === "interface" ? (
        <>
          <Field label="Interface">
            <div style={{ display: "flex", gap: 6 }}>
              <SheetSelect aria-label="Network interface" sheetTitle="Network interface" value={iface} disabled={connected}
                options={ifaceOptions.length ? ifaceOptions : [{ value: "", label: "No wired interface found" }]} onChange={(v) => setIface(String(v))} />
              <Button size="sm" variant="ghost" aria-label="Refresh interfaces" icon={<RefreshCw size={14} strokeWidth={1.75} />} onClick={() => void refreshIfaces()} disabled={connected} />
            </div>
          </Field>
          {access && !access.ok && (
            <Callout tone="warn">
              <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
                <span>{access.detail}</span>
                {access.canGrant && <Button size="sm" onClick={() => void grant()}>Grant access</Button>}
              </div>
            </Callout>
          )}
          <span style={hint}>
            Use a wired port (a USB or Thunderbolt Ethernet adapter works) on its own EtherCAT line. This computer becomes the master: unplug the PLC first, there
            can be only one master on a line.
          </span>
          {!connected && (
            <Button variant="primary" block icon={<Plug size={14} strokeWidth={1.75} />} disabled={connecting || !iface} onClick={() => void connect({ kind: "interface", name: iface })}>
              {connecting ? "Scanning" : "Connect and scan"}
            </Button>
          )}
        </>
      ) : (
        <>
          <Field label="Simulated machine">
            <SheetSelect aria-label="Simulated machine" sheetTitle="Simulated machine" value={template} disabled={connected}
              options={TEMPLATES.map((t) => ({ value: t.value, label: t.label }))} onChange={(v) => setTemplate(v as Template)} />
          </Field>
          <span style={hint}>{TEMPLATES.find((t) => t.value === template)?.description} Devices answer frames at register level, as real ones do, so everything in the app works on them.</span>
          {!connected && (
            <Button variant="primary" block icon={<Play size={14} strokeWidth={1.75} />} disabled={connecting} onClick={() => void connect({ kind: "simulator", template })}>
              {connecting ? "Starting" : "Start the simulator"}
            </Button>
          )}
        </>
      )}
      {connected && (
        <div style={{ display: "flex", gap: 6 }}>
          <Button block icon={<RefreshCw size={14} strokeWidth={1.75} />} disabled={connecting || busy} onClick={() => void rescan()}>Rescan</Button>
          <Button block variant="danger" icon={<X size={14} strokeWidth={1.75} />} onClick={() => void disconnect()}>Disconnect</Button>
        </div>
      )}
    </Section>
  );

  const busSection = connected && (
    <Section title="Bus state" meta={busy ? <StatusDot tone="warn" label="Changing" /> : undefined}>
      <div style={{ display: "flex", gap: 4, flexWrap: "wrap" }}>
        {stateCounts.map(([s, n]) => <Badge key={s} tone={s.includes("error") || s === "no answer" ? "danger" : s === "OP" ? "success" : s === "SAFE-OP" ? "info" : "warn"}>{n} {s}</Badge>)}
      </div>
      <div style={{ display: "grid", gridTemplateColumns: "repeat(4, 1fr)", gap: 4 }}>
        {STATES.map((s) => (
          <Button key={s} size="sm" disabled={busy} onClick={() => void requestState(s, [])}>{s}{s === "OP" && !pro && <ProBadge />}</Button>
        ))}
      </div>
      <span style={hint}>All devices. SAFE-OP starts the process data exchange (inputs live); OP also applies outputs.</span>
      <Field label="Cycle time (ms)">
        <div style={{ display: "flex", gap: 6 }}>
          <input className="vt-input vt-input--mono" aria-label="Cycle time in milliseconds" value={cycleMs} onChange={(e) => setCycleMs(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter") void applyCycle(); }} />
          <Button size="sm" onClick={() => void applyCycle()}>Set</Button>
          <Button size="sm" variant="ghost" aria-label={stats?.running ? "Stop the exchange" : "Start the exchange"} icon={stats?.running ? <Square size={14} strokeWidth={1.75} /> : <Play size={14} strokeWidth={1.75} />} onClick={() => void toggleCycle()} />
        </div>
      </Field>
      {stats && (
        <div style={{ display: "grid", gridTemplateColumns: "max-content 1fr", columnGap: 10, rowGap: 2, fontSize: "var(--fs-xs)" }}>
          <span style={dim}>Exchange</span>
          <span>{stats.running ? <StatusDot tone="success" label="running" /> : "stopped"} {stats.cycles.toLocaleString()} cycles</span>
          <span style={dim}>Working counter</span>
          <span className="mono" style={{ color: stats.wkc !== stats.expectedWkc ? "var(--danger)" : undefined }}>{stats.wkc} of {stats.expectedWkc}</span>
          <span style={dim}>WKC errors</span>
          <span className="mono">{stats.wkcErrors}</span>
          <span style={dim}>Lost frames</span>
          <span className="mono">{stats.timeouts}</span>
          <span style={dim}>Round trip</span>
          <span className="mono">{stats.avgUs.toFixed(0)} µs (min {stats.minUs.toFixed(0)}, max {stats.maxUs.toFixed(0)})</span>
        </div>
      )}
    </Section>
  );

  // ── Main ──
  const welcome = (
    <div style={{ flex: 1, display: "flex", alignItems: "center", justifyContent: "center" }}>
      <EmptyState
        icon={<Cpu size={20} strokeWidth={1.75} />}
        title="Scan an EtherCAT network"
        description="Plug the first device of an EtherCAT line into a wired Ethernet port, pick the port and scan: every device with its state, identity, EEPROM, registers, object dictionary and process data. No hardware here? The simulator has a whole machine."
        action={
          <div style={{ display: "flex", gap: 8 }}>
            <Button variant="primary" icon={<Play size={14} strokeWidth={1.75} />} onClick={() => { setMode("simulator"); void connect({ kind: "simulator", template }); }}>Try the simulator</Button>
            <Button onClick={() => setMode("interface")}>Use an Ethernet port</Button>
          </div>
        }
      />
    </div>
  );

  const devices = scan?.devices ?? [];
  const device = devices[selected];
  const devicesTab = (
    <div className="ecat-split">
      <div className="ecat-list">
        <div style={row}>
          <Badge>{devices.length} devices</Badge>
          <span style={hint}>in cable order from this computer</span>
          <span style={{ flex: 1 }} />
          <Button size="sm" variant="ghost" icon={<Download size={14} strokeWidth={1.75} />} onClick={() => void exportDevices()}>CSV{!pro && <ProBadge />}</Button>
        </div>
        <div style={scroller}>
          <table className="vt-table" style={{ width: "100%" }}>
            <thead>
              <tr>
                <th style={{ ...th, width: 34 }}>#</th>
                <th style={{ ...th, width: 62 }}>Address</th>
                <th style={th}>Device</th>
                <th style={{ ...th, width: 92 }}>State</th>
              </tr>
            </thead>
            <tbody>
              {devices.map((d) => {
                const st = states[d.position];
                return (
                  <tr key={d.position} onClick={() => setSelected(d.position)} aria-selected={d.position === selected}
                    style={{ cursor: "pointer", background: d.position === selected ? "color-mix(in srgb, var(--accent) 16%, transparent)" : undefined }}>
                    <td className="mono">{d.position}</td>
                    <td className="mono">{hex(d.address, 4).slice(2)}</td>
                    <td>
                      <div style={{ fontWeight: 600 }}>{d.name || `Device ${d.position}`}</div>
                      <div style={{ ...hint, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis", maxWidth: 190 }}>{d.description || (d.vendorName ?? hex(d.vendorId, 8))}</div>
                    </td>
                    <td>
                      <StateBadge state={st?.state ?? d.state} />
                      {st?.alCode != null && <div style={{ fontSize: "var(--fs-xs)", color: "var(--danger)" }}>{hex(st.alCode, 4)}</div>}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      </div>
      <div style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column", minHeight: 0 }}>
        {device ? (
          <DeviceDetail
            key={device.position}
            device={device}
            state={states[device.position]}
            pdo={pdos.get(device.position)}
            image={images[device.position]}
            pro={pro}
            requirePro={requirePro}
            addLog={addLog}
            onRequestState={(s, p) => void requestState(s, p)}
            busy={busy}
            trending={new Set(trend.keys())}
            onToggleTrend={toggleTrend}
          />
        ) : (
          <EmptyState title="No devices" />
        )}
      </div>
    </div>
  );

  const processTab = (
    <div style={{ ...scroller, display: "flex", flexDirection: "column", gap: 4 }}>
      {devices.filter((d) => { const m = pdos.get(d.position); return m && (m.inputs.length || m.outputs.length); }).map((d) => {
        const m = pdos.get(d.position)!;
        const st = states[d.position];
        return (
          <div key={d.position} style={{ borderBottom: "1px solid var(--border)", paddingBottom: 6 }}>
            <div style={{ ...row, padding: "8px 10px 2px" }}>
              <strong>{d.position}: {d.name}</strong>
              <StateBadge state={st?.state ?? d.state} alText={st?.alText} />
            </div>
            {m.inputs.length > 0 && (
              <EntryTable position={d.position} deviceName={d.name} title="Inputs" pdos={m.inputs} img={images[d.position]?.inputs ?? []} trending={new Set(trend.keys())} onToggleTrend={toggleTrend} />
            )}
            {m.outputs.length > 0 && (
              <EntryTable position={d.position} deviceName={d.name} title="Outputs" pdos={m.outputs} img={images[d.position]?.outputs ?? []}
                writable={st?.state === "OP"} pro={pro} requirePro={requirePro} addLog={addLog} trending={new Set(trend.keys())} onToggleTrend={toggleTrend} />
            )}
          </div>
        );
      })}
      {pdos.size === 0 && <EmptyState title="Reading PDO mappings" />}
    </div>
  );

  const connectedMain = (
    <div style={{ display: "flex", flexDirection: "column", flex: 1, minHeight: 0, gap: 8 }}>
      {scan?.initError && (
        <Callout tone="warn">
          EtherCAT init did not complete: {scan.initError}. The list below comes from a raw scan (identity and state only). Check the device the error names, then rescan.
        </Callout>
      )}
      <Tabs
        label="Views"
        value={tab}
        onChange={setTab}
        items={[
          { value: "devices", label: "Devices", count: devices.length || undefined },
          { value: "process", label: "Process data" },
          { value: "frames", label: "Frames", count: frames.length || undefined },
          ...(linkKind === "simulator" ? [{ value: "simulator" as const, label: "Simulator" }] : []),
        ]}
      />
      {tab === "devices" && devicesTab}
      {tab === "process" && processTab}
      {tab === "frames" && (
        <FramesView frames={frames} onClear={() => setFrames([])} simulator={linkKind === "simulator"} startedAt={startedAt.current} pro={pro} requirePro={requirePro} addLog={addLog} />
      )}
      {tab === "simulator" && linkKind === "simulator" && <SimView addLog={addLog} />}
    </div>
  );

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100vh", background: "var(--bg)", color: "var(--text)" }}>
      <Toolbar title="EtherCAT Explorer" icon={appIcon} connected={connected} onDisconnect={() => { void disconnect(); }} onInfo={() => setShowInfo(true)} />
      <div style={{ display: "flex", flex: 1, overflow: "hidden" }}>
        <aside className="vt-sidebar" style={{ width: 300, flexShrink: 0, borderRight: "1px solid var(--border)" }}>
          {networkSection}
          {busSection}
        </aside>
        <main style={{ display: "flex", flex: 1, flexDirection: "column", overflow: "hidden", padding: 8, minWidth: 0, gap: 8 }}>
          {error && (
            <Callout tone="danger">
              <div style={{ display: "flex", gap: 8, alignItems: "flex-start" }}>
                <span style={{ flex: 1 }}>{error}</span>
                <button type="button" className="vt-btn vt-btn--sm vt-btn--ghost" aria-label="Dismiss" onClick={() => setError(null)}><X size={14} /></button>
              </div>
            </Callout>
          )}
          <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>{connected ? connectedMain : welcome}</div>
          <div style={{ height: 200, flexShrink: 0, display: "flex", flexDirection: "column", gap: 6 }}>
            <Tabs
              label="Bottom panel"
              value={bottomTab}
              onChange={setBottomTab}
              items={[{ value: "log", label: "Log" }, { value: "trend", label: <>Trend <ProBadge /></>, count: trend.size || undefined }]}
            />
            {bottomTab === "log" ? (
              <LogPanel entries={log} onExport={(f) => addLog("info", `Log exported to ${f}`)} onClear={() => setLog([])} />
            ) : trend.size === 0 ? (
              <div style={{ ...hint, padding: 8 }}>Click the trend icon next to a process data entry.</div>
            ) : (
              <TrendPanel
                series={Array.from(trend.values())}
                onRemove={(name) => setTrend((prev) => { const next = new Map(prev); for (const [k, s] of next) if (s.name === name) next.delete(k); return next; })}
                windowSec={trendWindow}
                onWindowChange={setTrendWindow}
                theme="dark"
              />
            )}
          </div>
        </main>
      </div>
      <StatusBar connected={connected} label={scan?.link} error={error} />

      <Dialog open={showInfo} onClose={() => setShowInfo(false)} title="About EtherCAT Explorer" width={480} footer={(
        <>
          {proStatus && !proStatus.store && (
            <Button variant="ghost" onClick={() => invoke("open_url", { url: "https://github.com/VoltrusID/voltrus-explorers" })}>Source on GitHub</Button>
          )}
          <Button variant="ghost" onClick={openPrivacy}>Privacy policy</Button>
          <Button block={!proStatus || proStatus.store} onClick={() => setShowInfo(false)}>Close</Button>
        </>
      )}>
        <div style={{ fontSize: "var(--fs-sm)", color: "var(--text-dim)", lineHeight: 1.6 }}>
          <p style={{ margin: "0 0 12px" }}>
            An EtherCAT network explorer for commissioning and troubleshooting: scan a line from a Mac's Ethernet port and see every device's identity, state and AL
            status code, ports, EEPROM, ESC registers and error counters, its CoE object dictionary and its process data, live.
          </p>
          <p style={{ margin: "0 0 12px" }}>
            Pro adds OP with output writes, SDO and register writes, error counter resets, live trends, EEPROM backup, pcap export of captured frames and the device
            list export.
          </p>
          <div style={{ borderTop: "1px solid var(--border)", paddingTop: 12, marginBottom: 12 }}>
            <ProAboutRow />
          </div>
          <p style={{ margin: "0 0 12px", fontSize: "var(--fs-xs)", color: "var(--text-muted)" }}>
            EtherCAT® is a registered trademark and patented technology, licensed by Beckhoff Automation GmbH, Germany. The simulator imitates Beckhoff EK1100,
            EL1008, EL2008, EL3102 and EL4102 terminals by their published identities; Beckhoff is a trademark of Beckhoff Automation GmbH. EtherCAT Explorer is an
            independent tool built on the open-source EtherCrab master; it is not affiliated with or endorsed by Beckhoff or the EtherCAT Technology Group.
          </p>
          <span style={{ color: "var(--text-muted)" }}>Part of the </span>
          <strong style={{ color: "var(--accent-text)" }}>Voltrus</strong>
          <span style={{ color: "var(--text-muted)" }}> suite by </span>
          <strong style={{ color: "var(--text)" }}>JoyoDigitama</strong>
        </div>
      </Dialog>
    </div>
  );
}

export default App;
