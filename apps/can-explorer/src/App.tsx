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
  appendTextFile,
  nextTrendColor,
  noteDeviceError,
  noteDeviceSuccess,
  openTextFile,
  pickSavePath,
  saveTextFile,
  useProStatus,
  withUtm,
} from "@mactools/ui";
import type { LogEntry } from "@mactools/ui";
import { Cable, Play, Plug, Plus, RefreshCw, RotateCcw, Square, Trash2, X } from "lucide-react";
import appIcon from "../src-tauri/icons/128x128.png";
import MonitorView from "./MonitorView";
import ReplayView from "./ReplayView";
import SignalsView from "./SignalsView";
import SimDashboard from "./SimView";
import TraceView from "./TraceView";
import TransmitView from "./TransmitView";
import TrendPanel, { type TrendSeries } from "./TrendPanel";
import {
  BITRATES, DATA_BITRATES, IFACE_DEFAULTS, TRACE_MAX, candumpLine, configOf, foldMonitor, ifaceProblem, loadProfiles, loadTxList, needsPro,
  pushCapped, rateText, sameIface, saveProfiles, saveTxList, toCandump, traceCsv, type IfaceForm, type MonitorRow, type Profile, type TxItem,
} from "./can";
import type { ConnInfo, DbcView, FrameView, GsUsbDevice, Layer, PortInfo, Stats } from "./types";
import { hint } from "./ui";

type Tab = "monitor" | "trace" | "transmit" | "signals" | "replay";
type Mode = "bus" | "simulator";

const MAX_LOG = 2000;
const LOG_FILTER = { name: "candump log", extensions: ["log", "txt", "candump"] };
const DBC_FILTER = { name: "DBC", extensions: ["dbc"] };

const LAYERS: { value: Layer; label: string }[] = [
  { value: "none", label: "None" },
  { value: "j1939", label: "J1939" },
  { value: "canopen", label: "CANopen" },
  { value: "obd2", label: "OBD-II" },
];

const toForm = (p: Profile): IfaceForm => ({
  kind: p.kind, port: p.port, bitrate: p.bitrate, dataBitrate: p.dataBitrate, serialBaud: p.serialBaud, listenOnly: p.listenOnly, ifname: p.ifname, gsDevice: p.gsDevice, channel: p.channel,
});

function App() {
  const { requirePro, pro, status: proStatus } = useProStatus();
  const [mode, setMode] = useState<Mode>("bus");
  const [tab, setTab] = useState<Tab>("monitor");
  const [bottomTab, setBottomTab] = useState<"log" | "trend">("log");
  const [showInfo, setShowInfo] = useState(false);
  const [log, setLog] = useState<LogEntry[]>([]);
  const [error, setError] = useState<string | null>(null);

  // Interface
  const [form, setForm] = useState<IfaceForm>(IFACE_DEFAULTS);
  const [ports, setPorts] = useState<PortInfo[]>([]);
  const [socketcan, setSocketcan] = useState<string[]>([]);
  const [gsDevices, setGsDevices] = useState<GsUsbDevice[]>([]);
  const [conn, setConn] = useState<ConnInfo | null>(null);
  const [connVirtual, setConnVirtual] = useState(false);
  const [loading, setLoading] = useState(false);
  const [stats, setStats] = useState<Stats | null>(null);

  // Profiles
  const [profiles, setProfiles] = useState<Profile[]>(loadProfiles);
  const [activeProfile, setActiveProfile] = useState<string | null>(null);
  const [profileName, setProfileName] = useState("");

  // Data
  const monitorRef = useRef(new Map<string, MonitorRow>());
  const [monitorRows, setMonitorRows] = useState<MonitorRow[]>([]);
  const [trace, setTrace] = useState<FrameView[]>([]);
  const [traceSource, setTraceSource] = useState<string | null>(null);
  const [paused, setPaused] = useState(false);
  const pausedRef = useRef(paused);
  pausedRef.current = paused;
  const [filter, setFilter] = useState("");
  const [latest, setLatest] = useState<Map<string, FrameView>>(new Map());
  const [dbc, setDbc] = useState<DbcView | null>(null);
  const [layer, setLayer] = useState<Layer>("none");
  const [recording, setRecording] = useState<string | null>(null);
  const recordRef = useRef<{ path: string; iface: string; buf: string[] } | null>(null);

  // Transmit
  const [txItems, setTxItemsState] = useState<TxItem[]>(loadTxList);
  const setTxItems = useCallback((items: TxItem[]) => { setTxItemsState(items); saveTxList(items); }, []);
  const [txRunning, setTxRunning] = useState<Set<string>>(new Set());

  // Simulator
  const [simRunning, setSimRunning] = useState(false);

  // Trend: key `Message.Signal`
  const [trend, setTrend] = useState<Map<string, TrendSeries>>(new Map());
  const trendRef = useRef(trend);
  trendRef.current = trend;
  const [trendWindow, setTrendWindow] = useState(60);

  const addLog = useCallback((direction: LogEntry["direction"], message: string) => {
    setLog((prev) => pushCapped(prev, [{ timestamp: Date.now(), direction, message }], MAX_LOG));
  }, []);

  // ── Events from Rust ──
  useEffect(() => {
    const subs = [
      listen<{ direction: LogEntry["direction"]; message: string }>("can-log", (e) => addLog(e.payload.direction, e.payload.message)),
      listen<{ frames: FrameView[]; dropped: number }>("can-frames", (e) => {
        const { frames } = e.payload;
        if (frames.length === 0) return;
        foldMonitor(monitorRef.current, frames);
        setMonitorRows(Array.from(monitorRef.current.values()));
        if (!pausedRef.current) setTrace((prev) => pushCapped(prev, frames, TRACE_MAX));
        setLatest((prev) => {
          let next: Map<string, FrameView> | null = null;
          for (const f of frames) if (f.msg && f.dir === "rx") { next ??= new Map(prev); next.set(f.msg, f); }
          return next ?? prev;
        });
        if (trendRef.current.size > 0) {
          setTrend((prev) => {
            const next = new Map(prev);
            let changed = false;
            for (const f of frames) {
              if (!f.msg || !f.sigs) continue;
              for (const [name, value] of f.sigs) {
                const key = `${f.msg}.${name}`;
                const s = next.get(key);
                if (!s) continue;
                const cutoff = f.ts / 1000 - trendWindow * 1000 - 5000;
                const points = [...s.points, { t: f.ts / 1000, v: value }];
                next.set(key, { ...s, points: (points.length > 3000 ? points.slice(-3000) : points).filter((p) => p.t > cutoff) });
                changed = true;
              }
            }
            return changed ? next : prev;
          });
        }
        const rec = recordRef.current;
        if (rec) for (const f of frames) rec.buf.push(candumpLine(f, rec.iface));
      }),
      listen<Stats>("can-stats", (e) => setStats(e.payload)),
      listen<{ state: "closed"; reason: string | null }>("can-conn", (e) => {
        setConn(null);
        setConnVirtual(false);
        setTxRunning(new Set());
        if (e.payload.reason) {
          setError(e.payload.reason);
          addLog("error", e.payload.reason);
          noteDeviceError();
        }
      }),
      listen<{ key: string; stopped: boolean; error?: string }>("can-periodic", (e) => {
        if (e.payload.stopped) setTxRunning((prev) => { const n = new Set(prev); n.delete(e.payload.key); return n; });
        if (e.payload.error) addLog("error", `Cyclic ${e.payload.key}: ${e.payload.error}`);
      }),
    ];
    return () => { subs.forEach((s) => s.then((un) => un())); };
  }, [addLog, trendWindow]);

  // Flush the recording buffer twice a second.
  useEffect(() => {
    if (!recording) return;
    const t = setInterval(() => {
      const rec = recordRef.current;
      if (!rec || rec.buf.length === 0) return;
      const text = rec.buf.join("\n") + "\n";
      rec.buf = [];
      appendTextFile(rec.path, text).catch((e) => {
        addLog("error", `Recording stopped: ${String(e)}`);
        recordRef.current = null;
        setRecording(null);
      });
    }, 500);
    return () => clearInterval(t);
  }, [recording, addLog]);

  // ── Ports ──
  const refreshPorts = useCallback(async () => {
    try {
      const p = await invoke<PortInfo[]>("can_list_ports");
      setPorts(p);
      setForm((f) => (f.port || p.length === 0 ? f : { ...f, port: p[0].name }));
    } catch (e) {
      addLog("error", `Listing serial ports failed: ${String(e)}`);
    }
    try { setSocketcan(await invoke<string[]>("can_list_socketcan")); } catch { /* not Linux */ }
    try { setGsDevices(await invoke<GsUsbDevice[]>("can_list_gsusb")); } catch (e) { addLog("error", `Listing USB adapters failed: ${String(e)}`); }
  }, [addLog]);
  useEffect(() => { void refreshPorts(); }, [refreshPorts]);

  // ── Connect ──
  const clearData = useCallback(() => {
    monitorRef.current = new Map();
    setMonitorRows([]);
    setTrace([]);
    setTraceSource(null);
    setLatest(new Map());
  }, []);

  const connectWith = useCallback(async (f: IfaceForm) => {
    const problem = ifaceProblem(f);
    if (problem) { setError(problem); return; }
    setLoading(true);
    setError(null);
    try {
      const info = await invoke<ConnInfo>("can_connect", { config: configOf(f) });
      clearData();
      setConn(info);
      setConnVirtual(f.kind === "virtual");
      setStats(null);
      noteDeviceSuccess();
    } catch (e) {
      setError(String(e));
      addLog("error", String(e));
      noteDeviceError();
    } finally {
      setLoading(false);
    }
  }, [addLog, clearData]);

  const disconnect = useCallback(async () => {
    try { await invoke("can_disconnect"); } catch { /* not connected */ }
    setConn(null);
    setConnVirtual(false);
    setTxRunning(new Set());
  }, []);

  // ── DBC and layer ──
  const loadDbc = useCallback(async () => {
    try {
      const file = await openTextFile(DBC_FILTER);
      if (!file) return;
      const v = await invoke<DbcView>("dbc_load", { name: file.path.split(/[\\/]/).pop() ?? file.path, text: file.text });
      setDbc(v);
      setLatest(new Map());
      if (v.skipped > 0) addLog("info", `${v.skipped} DBC statements not used (attribute definitions, environment variables, signal groups)`);
    } catch (e) {
      setError(String(e));
      addLog("error", `DBC: ${String(e)}`);
    }
  }, [addLog]);

  const loadSimDbc = useCallback(async () => {
    try { setDbc(await invoke<DbcView>("dbc_load_sim")); setLatest(new Map()); } catch (e) { setError(String(e)); }
  }, []);

  const saveSimDbc = useCallback(async () => {
    try {
      const path = await saveTextFile("can-explorer-simulator.dbc", await invoke<string>("dbc_sim_text"), DBC_FILTER);
      if (path) addLog("info", `Simulator DBC saved to ${path}`);
    } catch (e) { addLog("error", String(e)); }
  }, [addLog]);

  const clearDbc = useCallback(async () => {
    await invoke("dbc_clear");
    setDbc(null);
    setLatest(new Map());
    setTrend(new Map());
  }, []);

  const changeLayer = useCallback(async (l: Layer) => {
    setLayer(l);
    try { await invoke("can_set_layer", { layer: l }); } catch (e) { addLog("error", String(e)); }
  }, [addLog]);

  // ── Simulator ──
  const startSim = useCallback(async () => {
    if (!requirePro("The bus simulator")) return false;
    try { await invoke("sim_start"); setSimRunning(true); return true; } catch (e) { setError(String(e)); return false; }
  }, [requirePro]);

  const stopSim = useCallback(async () => {
    await invoke("sim_stop");
    setSimRunning(false);
  }, []);

  const monitorSim = useCallback(async () => {
    if (!simRunning && !(await startSim())) return;
    const f: IfaceForm = { ...form, kind: "virtual" };
    setForm(f);
    setActiveProfile(null);
    if (!dbc) await loadSimDbc();
    if (layer === "none") await changeLayer("j1939");
    setMode("bus");
    setTab("monitor");
    await connectWith(f);
  }, [simRunning, startSim, form, dbc, loadSimDbc, layer, changeLayer, connectWith]);

  // ── Trace files ──
  const exportTrace = useCallback(async (kind: "candump" | "csv", frames: FrameView[]) => {
    if (!requirePro("Trace export")) return;
    try {
      const path = kind === "candump"
        ? await saveTextFile("can-trace.log", toCandump(frames, conn?.logName ?? "can0"), LOG_FILTER)
        : await saveTextFile("can-trace.csv", traceCsv(frames));
      if (path) addLog("info", `Exported ${frames.length} frames to ${path}`);
    } catch (e) {
      addLog("error", `Export failed: ${String(e)}`);
    }
  }, [requirePro, conn, addLog]);

  const importTrace = useCallback(async () => {
    try {
      const file = await openTextFile(LOG_FILTER);
      if (!file) return;
      const frames = await invoke<FrameView[]>("trace_decode", { text: file.text });
      setPaused(true);
      setTrace(frames.slice(-TRACE_MAX));
      setTraceSource(file.path);
      monitorRef.current = new Map();
      foldMonitor(monitorRef.current, frames);
      setMonitorRows(Array.from(monitorRef.current.values()));
      addLog("info", `Opened ${frames.length} frames from ${file.path}${frames.length > TRACE_MAX ? ` (the last ${TRACE_MAX} are in the trace)` : ""}. The trace is paused; resume to see live frames again.`);
    } catch (e) {
      setError(String(e));
    }
  }, [addLog]);

  const startRecording = useCallback(async () => {
    if (!requirePro("Recording to a file")) return;
    const stamp = new Date().toISOString().replace(/[:T]/g, "-").slice(0, 19);
    const path = await pickSavePath(`can-${stamp}.log`, LOG_FILTER);
    if (!path) return;
    try {
      recordRef.current = { path, iface: conn?.logName ?? "can0", buf: [] };
      await appendTextFile(path, "");
      setRecording(path);
      addLog("info", `Recording every frame to ${path}`);
    } catch (e) {
      recordRef.current = null;
      addLog("error", `Cannot record to ${path}: ${String(e)}`);
    }
  }, [requirePro, conn, addLog]);

  const stopRecording = useCallback(async () => {
    const rec = recordRef.current;
    recordRef.current = null;
    setRecording(null);
    if (rec && rec.buf.length > 0) {
      try { await appendTextFile(rec.path, rec.buf.join("\n") + "\n"); } catch (e) { addLog("error", String(e)); }
    }
    if (rec) addLog("info", `Recording saved to ${rec.path}`);
  }, [addLog]);

  // ── Trend ──
  const toggleTrend = useCallback((key: string, label: string) => {
    if (!trendRef.current.has(key) && !requirePro("Live signal trends")) return;
    setTrend((prev) => {
      const next = new Map(prev);
      if (next.has(key)) next.delete(key);
      else next.set(key, { name: label, color: nextTrendColor(Array.from(next.values(), (s) => s.color)), points: [] });
      return next;
    });
    setBottomTab("trend");
  }, [requirePro]);

  // ── Profiles ──
  const saveProfile = useCallback((name: string) => {
    const n = name.trim();
    if (!n) return;
    if (needsPro(profiles, n, pro) && !requirePro("More than one saved interface profile")) return;
    const next = [...profiles.filter((p) => p.name !== n), { ...form, name: n }];
    setProfiles(next);
    saveProfiles(next);
    setActiveProfile(n);
    setProfileName("");
  }, [profiles, pro, requirePro, form]);

  const loadProfile = useCallback((name: string) => {
    const p = profiles.find((x) => x.name === name);
    if (!p) return;
    setForm(toForm(p));
    setActiveProfile(p.name);
  }, [profiles]);

  const deleteProfile = useCallback((name: string) => {
    const next = profiles.filter((p) => p.name !== name);
    setProfiles(next);
    saveProfiles(next);
    if (activeProfile === name) setActiveProfile(null);
  }, [profiles, activeProfile]);

  const profileDirty = useMemo(() => {
    const saved = profiles.find((p) => p.name === activeProfile);
    return !!saved && !sameIface(toForm(saved), form);
  }, [profiles, activeProfile, form]);

  const openPrivacy = useCallback(async () => {
    try { await invoke("open_url", { url: await withUtm("https://voltrus.id/privacy/", "privacy") }); } catch (e) { addLog("error", String(e)); }
  }, [addLog]);

  const connected = conn != null;
  const canSend = conn?.canSend ?? false;
  const set = (patch: Partial<IfaceForm>) => setForm((f) => ({ ...f, ...patch }));

  // ── Sidebar ──
  const portOptions = [
    ...(form.port && !ports.some((p) => p.name === form.port) ? [{ value: form.port, label: `${form.port} (not present)` }] : []),
    ...ports.map((p) => ({ value: p.name, label: `${p.likelyCan ? "● " : ""}${p.name}${p.description ? ` · ${p.description}` : ""}` })),
  ];

  const busSidebar = (
    <>
      <Section title="Profiles">
        <SheetSelect
          aria-label="Interface profile"
          sheetTitle="Interface profile"
          value={activeProfile ?? ""}
          options={[{ value: "", label: "New profile" }, ...profiles.map((p) => ({ value: p.name, label: p.name }))]}
          onChange={(v) => { if (v) loadProfile(v); else setActiveProfile(null); }}
          disabled={connected}
        />
        {activeProfile ? (
          <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
            <Badge tone={profileDirty ? "warn" : "success"}>{profileDirty ? "Modified" : "Saved"}</Badge>
            <span style={{ flex: 1 }} />
            {profileDirty && <Button size="sm" onClick={() => saveProfile(activeProfile)}>Save</Button>}
            {profileDirty && <Button size="sm" icon={<RotateCcw size={14} strokeWidth={1.75} />} onClick={() => loadProfile(activeProfile)}>Revert</Button>}
            <Button size="sm" variant="danger" icon={<Trash2 size={14} strokeWidth={1.75} />} onClick={() => deleteProfile(activeProfile)}>Delete</Button>
          </div>
        ) : (
          <div style={{ display: "flex", gap: 6 }}>
            <input className="vt-input" aria-label="Profile name" placeholder="Profile name" value={profileName}
              onChange={(e) => setProfileName(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter") saveProfile(profileName); }} />
            <Button size="sm" icon={<Plus size={14} strokeWidth={1.75} />} disabled={!profileName.trim()} onClick={() => saveProfile(profileName)}>Save</Button>
          </div>
        )}
        {!pro && <span style={hint}>One saved profile. More <ProBadge /></span>}
      </Section>
      <Section title="Interface">
        <Segmented
          label="Interface kind"
          block
          value={form.kind}
          onChange={(k) => set({ kind: k })}
          options={[{ value: "slcan", label: "SLCAN" }, { value: "gsusb", label: "gs_usb" }, { value: "socketcan", label: "SocketCAN" }, { value: "virtual", label: "Virtual" }]}
        />
        {form.kind === "slcan" && (
          <>
            <Field label="Serial port">
              <div style={{ display: "flex", gap: 6 }}>
                <SheetSelect aria-label="Serial port" sheetTitle="Serial port" value={form.port} disabled={connected}
                  options={portOptions.length > 0 ? portOptions : [{ value: "", label: "No serial ports found" }]} onChange={(v) => set({ port: String(v) })} />
                <Button size="sm" variant="ghost" aria-label="Refresh ports" icon={<RefreshCw size={14} strokeWidth={1.75} />} onClick={() => void refreshPorts()} disabled={connected} />
              </div>
            </Field>
            <Field label="Bit rate">
              <SheetSelect aria-label="Bit rate" sheetTitle="Bit rate" value={String(form.bitrate)} disabled={connected}
                options={BITRATES.map((b) => ({ value: String(b), label: rateText(b) }))} onChange={(v) => set({ bitrate: Number(v) })} />
            </Field>
            <Field label="CAN FD data rate">
              <SheetSelect aria-label="CAN FD data rate" sheetTitle="CAN FD data rate" value={String(form.dataBitrate)} disabled={connected}
                options={[{ value: "0", label: "Classic CAN (no FD)" }, ...DATA_BITRATES.map((b) => ({ value: String(b), label: rateText(b) }))]} onChange={(v) => set({ dataBitrate: Number(v) })} />
            </Field>
            <label style={{ display: "flex", gap: 6, alignItems: "center", fontSize: "var(--fs-sm)" }}>
              <input type="checkbox" checked={form.listenOnly} disabled={connected} onChange={(e) => set({ listenOnly: e.target.checked })} /> Listen only (no ACK, no sending)
            </label>
            <span style={hint}>SLCAN adapters: CANable and CANable 2 (slcan firmware), Lawicel CANUSB and CAN232, USBtin and compatibles. ● marks a known CAN adapter.</span>
          </>
        )}
        {form.kind === "gsusb" && (
          <>
            <Field label="Adapter">
              <div style={{ display: "flex", gap: 6 }}>
                <SheetSelect aria-label="gs_usb adapter" sheetTitle="gs_usb adapter" value={form.gsDevice} disabled={connected}
                  options={[
                    { value: "", label: gsDevices.length > 0 ? "First adapter found" : "No gs_usb adapter found" },
                    ...(form.gsDevice && !gsDevices.some((d) => d.id === form.gsDevice) ? [{ value: form.gsDevice, label: `${form.gsDevice} (not present)` }] : []),
                    ...gsDevices.map((d) => ({ value: d.id, label: d.description })),
                  ]}
                  onChange={(v) => set({ gsDevice: String(v) })} />
                <Button size="sm" variant="ghost" aria-label="Refresh adapters" icon={<RefreshCw size={14} strokeWidth={1.75} />} onClick={() => void refreshPorts()} disabled={connected} />
              </div>
            </Field>
            <Field label="Channel">
              <SheetSelect aria-label="Channel" sheetTitle="Channel" value={String(form.channel)} disabled={connected}
                options={[0, 1, 2].map((c) => ({ value: String(c), label: `Channel ${c + 1}` }))} onChange={(v) => set({ channel: Number(v) })} />
            </Field>
            <Field label="Bit rate">
              <SheetSelect aria-label="Bit rate" sheetTitle="Bit rate" value={String(form.bitrate)} disabled={connected}
                options={BITRATES.map((b) => ({ value: String(b), label: rateText(b) }))} onChange={(v) => set({ bitrate: Number(v) })} />
            </Field>
            <Field label="CAN FD data rate">
              <SheetSelect aria-label="CAN FD data rate" sheetTitle="CAN FD data rate" value={String(form.dataBitrate)} disabled={connected}
                options={[{ value: "0", label: "Classic CAN (no FD)" }, ...DATA_BITRATES.map((b) => ({ value: String(b), label: rateText(b) }))]} onChange={(v) => set({ dataBitrate: Number(v) })} />
            </Field>
            <label style={{ display: "flex", gap: 6, alignItems: "center", fontSize: "var(--fs-sm)" }}>
              <input type="checkbox" checked={form.listenOnly} disabled={connected} onChange={(e) => set({ listenOnly: e.target.checked })} /> Listen only (no ACK, no sending)
            </label>
            <span style={hint}>
              candleLight firmware over USB, no driver needed: CANable and CANable 2 with candleLight, candleLight, CANtact, Geschwister Schneider USB/CAN and compatibles.
              On Linux the kernel's gs_usb driver is detached while the app uses the adapter.
            </span>
          </>
        )}
        {form.kind === "socketcan" && (
          <>
            <Field label="Interface">
              <input className="vt-input vt-input--mono" value={form.ifname} disabled={connected} list="socketcan-ifaces" onChange={(e) => set({ ifname: e.target.value })} />
              <datalist id="socketcan-ifaces">{socketcan.map((s) => <option key={s} value={s} />)}</datalist>
            </Field>
            <span style={hint}>
              Linux only. Bring the interface up first, e.g. <span className="mono">sudo ip link set can0 up type can bitrate 500000</span>, or a virtual one with{" "}
              <span className="mono">sudo ip link add vcan0 type vcan && sudo ip link set vcan0 up</span>.
              {socketcan.length > 0 ? ` Found: ${socketcan.join(", ")}.` : ""}
            </span>
          </>
        )}
        {form.kind === "virtual" && <span style={hint}>A bus inside the app, 500 kbit/s. The simulator sits on it; start it from the Simulator mode.</span>}
        {connected ? (
          <Button variant="danger" block icon={<X size={14} strokeWidth={1.75} />} onClick={() => void disconnect()}>Disconnect</Button>
        ) : (
          <Button variant="primary" block icon={<Plug size={14} strokeWidth={1.75} />} disabled={loading} onClick={() => void connectWith(form)}>
            {loading ? "Opening" : "Connect"}
          </Button>
        )}
      </Section>
      <Section title="Decoding">
        <Field label="Protocol labels">
          <SheetSelect aria-label="Protocol labels" sheetTitle="Protocol labels" value={layer} options={LAYERS} onChange={(v) => void changeLayer(v as Layer)} />
        </Field>
        {dbc ? (
          <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
            <Badge tone="success" title={dbc.name}>{dbc.name}</Badge>
            <span style={hint}>{dbc.messages.length} msgs</span>
            <span style={{ flex: 1 }} />
            <Button size="sm" variant="ghost" onClick={() => void clearDbc()}>Unload</Button>
          </div>
        ) : (
          <div style={{ display: "flex", gap: 6 }}>
            <Button size="sm" onClick={() => void loadDbc()}>Open DBC</Button>
            <Button size="sm" variant="ghost" onClick={() => void loadSimDbc()}>Simulator DBC</Button>
          </div>
        )}
      </Section>
    </>
  );

  const simSidebar = (
    <Section title="Bus simulator" meta={simRunning ? <StatusDot tone="success" label="Running" /> : undefined}>
      <span style={hint}>
        A diesel genset on the virtual bus: a J1939 engine ECU, a generator controller, a multiplexed pump drive, a CANopen I/O node and OBD-II answers. The made-up
        signals are described by the simulator's DBC.
      </span>
      {simRunning ? (
        <>
          <Button variant="primary" block icon={<Cable size={14} strokeWidth={1.75} />} disabled={connVirtual} onClick={() => void monitorSim()}>
            {connVirtual ? "Monitoring the virtual bus" : "Monitor it"}
          </Button>
          <Button variant="danger" block icon={<Square size={14} strokeWidth={1.75} />} onClick={() => void stopSim()}>Stop simulator</Button>
        </>
      ) : (
        <Button variant="primary" block icon={<Play size={14} strokeWidth={1.75} />} onClick={() => void startSim()}>Start simulator</Button>
      )}
      <Button block variant="ghost" onClick={() => void saveSimDbc()}>Save the simulator's DBC</Button>
    </Section>
  );

  // ── Main ──
  const header = connected && (
    <div style={{ display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap" }}>
      <Badge tone="success">{conn.description}</Badge>
      {!conn.canSend && <Badge tone="warn">Listen only</Badge>}
      {stats && (
        <>
          <span className="mono" style={{ fontSize: "var(--fs-xs)", color: "var(--text-dim)" }}>
            RX {stats.rx.toLocaleString()} ({stats.rxPerSec}/s) · TX {stats.tx.toLocaleString()} ({stats.txPerSec}/s)
          </span>
          {stats.load != null && <Badge tone={stats.load > 70 ? "danger" : stats.load > 40 ? "warn" : "neutral"}>Load {stats.load.toFixed(1)}%</Badge>}
          {stats.errors > 0 && <Badge tone="danger">{stats.errors} errors</Badge>}
          {stats.dropped > 0 && <Badge tone="warn" title="Frames the screen could not keep up with; they still count in RX">{stats.dropped} not shown</Badge>}
        </>
      )}
      <span style={{ flex: 1 }} />
      <input className="vt-input vt-input--mono" aria-label="Filter" placeholder="Filter: 7E8, 700-7FF, EEC1, !100" value={filter} onChange={(e) => setFilter(e.target.value)} style={{ width: 240 }} />
    </div>
  );

  const welcome = (
    <div style={{ flex: 1, display: "flex", alignItems: "center", justifyContent: "center" }}>
      <EmptyState
        icon={<Cable size={20} strokeWidth={1.75} />}
        title="Connect to a CAN bus"
        description="Plug in a USB CAN adapter (SLCAN: CANable, CANUSB, USBtin; or gs_usb / candleLight), pick it and the bus bit rate, and connect. On Linux, SocketCAN interfaces work too. No hardware? Try the built-in simulator."
        action={
          <div style={{ display: "flex", gap: 8 }}>
            <Button variant="primary" icon={<Play size={14} strokeWidth={1.75} />} onClick={() => void monitorSim()}>Try the simulator <ProBadge /></Button>
            <Button onClick={() => void importTrace()}>Open a candump log</Button>
          </div>
        }
      />
    </div>
  );

  const showBusTabs = connected || trace.length > 0;
  const busMain = !showBusTabs ? welcome : (
    <div style={{ display: "flex", flexDirection: "column", flex: 1, minHeight: 0, gap: 8 }}>
      {header}
      {!connected && (
        <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
          <Badge tone="neutral">Not connected</Badge>
          <span style={{ flex: 1 }} />
          <input className="vt-input vt-input--mono" aria-label="Filter" placeholder="Filter: 7E8, 700-7FF, EEC1, !100" value={filter} onChange={(e) => setFilter(e.target.value)} style={{ width: 240 }} />
        </div>
      )}
      <Tabs
        label="Views"
        value={tab}
        onChange={setTab}
        items={[
          { value: "monitor", label: "Monitor", count: monitorRows.length || undefined },
          { value: "trace", label: "Trace", count: trace.length || undefined },
          { value: "transmit", label: "Transmit", count: txRunning.size || undefined },
          { value: "signals", label: "Signals" },
          { value: "replay", label: "Replay" },
        ]}
      />
      {tab === "monitor" && (
        <MonitorView
          rows={monitorRows}
          filter={filter}
          onClear={() => { monitorRef.current = new Map(); setMonitorRows([]); }}
          trending={new Set(trend.keys())}
          onToggleTrend={toggleTrend}
        />
      )}
      {tab === "trace" && (
        <TraceView
          frames={trace}
          filter={filter}
          paused={paused}
          onPause={(p) => { setPaused(p); if (!p && traceSource) { setTrace([]); setTraceSource(null); } }}
          onClear={() => { setTrace([]); setTraceSource(null); }}
          onExport={(k, f) => void exportTrace(k, f)}
          onImport={() => void importTrace()}
          recording={recording}
          onRecord={() => void startRecording()}
          onStopRecord={() => void stopRecording()}
          source={traceSource}
        />
      )}
      {tab === "transmit" && (
        connected ? (
          <TransmitView items={txItems} setItems={setTxItems} running={txRunning} setRunning={setTxRunning} canSend={canSend} requirePro={requirePro} addLog={addLog} />
        ) : <EmptyState title="Not connected" description="Connect to a bus to send frames." />
      )}
      {tab === "signals" && (
        <SignalsView dbc={dbc} latest={latest} canSend={canSend} connected={connected} requirePro={requirePro} addLog={addLog} onLoad={() => void loadDbc()} onLoadSim={() => void loadSimDbc()} />
      )}
      {tab === "replay" && <ReplayView connected={connected} canSend={canSend} requirePro={requirePro} addLog={addLog} />}
    </div>
  );

  const simMain = simRunning ? (
    <SimDashboard addLog={addLog} onConnect={() => void monitorSim()} connectedToVirtual={connVirtual} />
  ) : (
    <EmptyState
      icon={<Cable size={20} strokeWidth={1.75} />}
      title="Simulator not running"
      description="Starts a genset that talks J1939, CANopen and OBD-II on the virtual bus. Monitor it from this app."
      action={<Button variant="primary" icon={<Play size={14} strokeWidth={1.75} />} onClick={() => void startSim()}>Start simulator <ProBadge /></Button>}
    />
  );

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100vh", background: "var(--bg)", color: "var(--text)" }}>
      <Toolbar
        title="CAN Explorer"
        icon={appIcon}
        connected={connected}
        onDisconnect={() => { void disconnect(); }}
        onInfo={() => setShowInfo(true)}
      />
      <div style={{ display: "flex", flex: 1, overflow: "hidden" }}>
        <aside className="vt-sidebar" style={{ width: 290, flexShrink: 0, borderRight: "1px solid var(--border)" }}>
          <Section>
            <Segmented
              label="Mode"
              block
              value={mode}
              onChange={(v) => { if (v === "simulator" && !requirePro("The bus simulator")) return; setMode(v); }}
              options={[
                { value: "bus", label: "Bus" },
                { value: "simulator", label: (<>Simulator <ProBadge />{simRunning && <StatusDot tone="success" label="Simulator running" />}</>) },
              ]}
            />
          </Section>
          {mode === "bus" ? busSidebar : simSidebar}
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
          <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
            {mode === "bus" ? busMain : simMain}
          </div>
          <div style={{ height: 220, flexShrink: 0, display: "flex", flexDirection: "column", gap: 6 }}>
            <Tabs
              label="Bottom panel"
              value={bottomTab}
              onChange={setBottomTab}
              items={[
                { value: "log", label: "Log" },
                { value: "trend", label: <>Trend <ProBadge /></>, count: trend.size || undefined },
              ]}
            />
            {bottomTab === "log" ? (
              <LogPanel entries={log} onExport={(f) => addLog("info", `Log exported to ${f}`)} onClear={() => setLog([])} />
            ) : trend.size === 0 ? (
              <div style={{ ...hint, padding: 8 }}>Open a decoded row in the Monitor and click the trend icon next to a signal.</div>
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
      <StatusBar connected={connected} label={conn?.description} error={error} />

      <Dialog open={showInfo} onClose={() => setShowInfo(false)} title="About CAN Explorer" width={460} footer={(
        <>
          {proStatus && !proStatus.store && (
            <Button variant="ghost" onClick={() => invoke("open_url", { url: "https://github.com/VoltrusID/voltrus-explorers" })}>
              Source on GitHub
            </Button>
          )}
          <Button variant="ghost" onClick={openPrivacy}>Privacy policy</Button>
          <Button block={!proStatus || proStatus.store} onClick={() => setShowInfo(false)}>Close</Button>
        </>
      )}>
        <div style={{ fontSize: "var(--fs-sm)", color: "var(--text-dim)", lineHeight: 1.6 }}>
          <p style={{ margin: "0 0 12px" }}>
            A CAN and CAN FD bus monitor. Connect through a USB SLCAN adapter or Linux SocketCAN, watch every identifier and the trace, decode frames with a DBC file and
            label J1939, CANopen and OBD-II traffic, and send frames.
          </p>
          <p style={{ margin: "0 0 12px" }}>
            Pro adds cyclic transmit, sending by signal values, live signal trends, trace recording, candump and CSV export, replay of candump logs, the bus simulator and
            unlimited interface profiles.
          </p>
          <div style={{ borderTop: "1px solid var(--border)", paddingTop: 12, marginBottom: 12 }}>
            <ProAboutRow />
          </div>
          <p style={{ margin: "0 0 12px", fontSize: "var(--fs-xs)", color: "var(--text-muted)" }}>
            CAN Explorer is an independent tool. CANopen is a registered trademark of CAN in Automation e.V., J1939 is an SAE standard, and CANable, CANUSB and USBtin
            are their makers' products; it is not affiliated with or endorsed by any of them.
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
