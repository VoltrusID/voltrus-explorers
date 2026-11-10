import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
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
  noteDeviceError,
  noteDeviceSuccess,
  saveTextFile,
  useProStatus,
  withUtm,
} from "@mactools/ui";
import type { LogEntry } from "@mactools/ui";
import { Cpu, Eye, EyeOff, FolderOpen, Maximize2, Pencil, Play, Plus, RefreshCw, Save, Square, Trash2, X, ZoomIn, ZoomOut } from "lucide-react";
import appIcon from "../src-tauri/icons/128x128.png";
import DecoderDialog from "./DecoderDialog";
import ProtocolTable from "./ProtocolTable";
import Waveform, { type DecoderLane } from "./Waveform";
import {
  DECODER_NAMES, FREE_SAMPLES, LENGTHS, PRO_DECODERS, channelColor, clampView, cursorDelta, decoderChannels, decoderLabel,
  fitView, fmtRate, fmtSamples, fmtTime, makeSession, patternTrigger, readSession, sampleTime, tableCsv, viewOf, zoomAt, type PatternBit, type View,
} from "./logic";
import type {
  Annotation, CaptureInfo, ChannelInfo, DecoderConfig, DecoderView, Devices, FirmwareStatus, Measurement, Opened, Progress, ScenarioId, Source, TableRow, Trigger,
} from "./types";
import { check, hint, row } from "./ui";

type SourceKind = "simulator" | "fx2";
type TriggerKind = "none" | "edge" | "pattern";

interface StackItem {
  id: number;
  config: DecoderConfig;
  view: DecoderView | null;
  error: string | null;
  hidden: boolean;
  revision: number;
}

const MAX_LOG = 1000;
const CAPTURE_FILTERS = [
  { name: "Voltrus Logic capture", extensions: ["vla"] },
  { name: "sigrok session", extensions: ["sr"] },
];
const DEFAULT_CHANNELS: ChannelInfo[] = Array.from({ length: 8 }, (_, i) => ({ name: `D${i}` }));

function App() {
  const { requirePro, pro, status: proStatus } = useProStatus();
  const [log, setLog] = useState<LogEntry[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [showInfo, setShowInfo] = useState(false);
  const [showFirmware, setShowFirmware] = useState(false);

  // Device
  const [devices, setDevices] = useState<Devices | null>(null);
  const [firmware, setFirmware] = useState<FirmwareStatus | null>(null);
  const [sourceKind, setSourceKind] = useState<SourceKind>("simulator");
  const [scenario, setScenario] = useState<ScenarioId>("modbusRtu");
  const [boardId, setBoardId] = useState("");
  const [rate, setRate] = useState(1_000_000);
  const [samples, setSamples] = useState(1_000_000);
  const [downloading, setDownloading] = useState(false);

  // Trigger
  const [triggerKind, setTriggerKind] = useState<TriggerKind>("edge");
  const [edgeCh, setEdgeCh] = useState(2);
  const [edgeType, setEdgeType] = useState<"rising" | "falling" | "any">("rising");
  const [pattern, setPattern] = useState<PatternBit[]>(Array(8).fill("x"));
  const [preTrigger, setPreTrigger] = useState(0.1);

  // Capture
  const [running, setRunning] = useState(false);
  const [progress, setProgress] = useState<Progress | null>(null);
  const [info, setInfo] = useState<CaptureInfo | null>(null);
  const [channels, setChannels] = useState<ChannelInfo[]>(DEFAULT_CHANNELS);
  const [hidden, setHidden] = useState<Set<number>>(new Set());
  const [view, setViewState] = useState<View>({ start: 0, spp: 1 });
  const [plotW, setPlotWState] = useState(800);
  const plotWRef = useRef(800);
  /** The view shows the whole capture: keep it that way when the width changes. */
  const fitted = useRef(false);
  const samplesRef = useRef(0);
  const setPlotW = useCallback((w: number) => {
    plotWRef.current = w;
    setPlotWState(w);
    if (fitted.current && samplesRef.current > 0) setViewState(fitView(samplesRef.current, w));
  }, []);
  const [cursors, setCursors] = useState<[number | null, number | null]>([null, null]);
  const [measureCh, setMeasureCh] = useState(0);
  const [measurement, setMeasurement] = useState<Measurement | null>(null);

  // Decoders
  const [stack, setStack] = useState<StackItem[]>([]);
  const stackRef = useRef(stack);
  stackRef.current = stack;
  const nextId = useRef(1);
  /** The stack came from a simulator scenario and has not been edited: replace it when the scenario changes. */
  const autoStack = useRef<ScenarioId | null>(null);
  const [editing, setEditing] = useState<{ id: number | null; config: DecoderConfig | null } | null>(null);
  const [bottomTab, setBottomTab] = useState<string>("log");
  const [selectedRow, setSelectedRow] = useState<{ id: number; index: number } | null>(null);
  const [highlight, setHighlight] = useState<{ start: number; end: number } | null>(null);

  const addLog = useCallback((direction: LogEntry["direction"], message: string) => {
    setLog((prev) => {
      const next = [...prev, { timestamp: Date.now(), direction, message }];
      return next.length > MAX_LOG ? next.slice(-MAX_LOG) : next;
    });
  }, []);

  const scenarioInfo = devices?.scenarios.find((s) => s.id === scenario) ?? null;
  /** The scenario the running capture belongs to (read by the capture event). */
  const autoStackFor = useRef(scenarioInfo);
  const setView = useCallback((v: View) => { fitted.current = false; setViewState(v); }, []);

  // ── Devices and firmware ──
  const refresh = useCallback(async () => {
    try {
      const d = await invoke<Devices>("la_devices");
      setDevices(d);
      if (d.usbError) addLog("info", `USB: ${d.usbError}`);
      setBoardId((b) => (b && d.boards.some((x) => x.info.id === b) ? b : d.boards[0]?.info.id ?? ""));
    } catch (e) {
      addLog("error", String(e));
    }
    try { setFirmware(await invoke<FirmwareStatus>("la_firmware_status")); } catch { /* ignore */ }
  }, [addLog]);
  useEffect(() => { void refresh(); }, [refresh]);

  const board = devices?.boards.find((b) => b.info.id === boardId) ?? null;
  const boardFirmware = board ? firmware?.files.find(([n]) => n === board.board.firmware)?.[1] ?? null : null;

  // ── Decoders ──
  const runDecoder = useCallback(async (item: StackItem): Promise<StackItem> => {
    try {
      const view = await invoke<DecoderView>("la_decode", { id: item.id, config: item.config });
      return { ...item, view, error: null, revision: item.revision + 1 };
    } catch (e) {
      return { ...item, view: null, error: String(e), revision: item.revision + 1 };
    }
  }, []);

  const rerunAll = useCallback(async (items: StackItem[]) => {
    const done = await Promise.all(items.map(runDecoder));
    setStack(done);
  }, [runDecoder]);

  const addDecoders = useCallback(async (configs: DecoderConfig[], replace: boolean) => {
    if (replace) for (const d of stackRef.current) void invoke("la_decoder_remove", { id: d.id });
    const items = configs.map((config) => ({ id: nextId.current++, config, view: null, error: null, hidden: false, revision: 0 }));
    const base = replace ? [] : stackRef.current;
    setStack([...base, ...items]);
    const done = await Promise.all(items.map(runDecoder));
    setStack((s) => s.map((x) => done.find((d) => d.id === x.id) ?? x));
    if (done[0]?.view?.table) setBottomTab(`dec-${done[0].id}`);
  }, [runDecoder]);

  const saveDecoder = useCallback(async (config: DecoderConfig) => {
    if (PRO_DECODERS.includes(config.kind) && !requirePro(`The ${DECODER_NAMES[config.kind]} decoder`)) return;
    if (config.kind === "spi" && config.device === "mcp2515" && !requirePro("MCP2515 decoding")) return;
    autoStack.current = null;
    const e = editing;
    setEditing(null);
    if (e?.id != null) {
      const item = stackRef.current.find((x) => x.id === e.id);
      if (!item) return;
      const done = await runDecoder({ ...item, config });
      setStack((s) => s.map((x) => (x.id === done.id ? done : x)));
    } else if (info) {
      await addDecoders([config], false);
    }
  }, [editing, info, requirePro, runDecoder, addDecoders]);

  const removeDecoder = useCallback((id: number) => {
    autoStack.current = null;
    void invoke("la_decoder_remove", { id });
    setStack((s) => s.filter((x) => x.id !== id));
    setBottomTab((t) => (t === `dec-${id}` ? "log" : t));
  }, []);

  // ── Capture events ──
  const afterCapture = useCallback(async (ci: CaptureInfo, keepView = false) => {
    setInfo(ci);
    setChannels(ci.channels.length ? ci.channels : DEFAULT_CHANNELS);
    setCursors([null, null]);
    setHighlight(null);
    setSelectedRow(null);
    samplesRef.current = ci.samples;
    if (!keepView) {
      fitted.current = true;
      setViewState(fitView(ci.samples, plotWRef.current));
    }
  }, []);

  useEffect(() => {
    const subs = [
      listen<Progress>("la-progress", (e) => setProgress(e.payload)),
      listen<{ direction: LogEntry["direction"]; message: string }>("la-log", (e) => addLog(e.payload.direction, e.payload.message)),
      listen<CaptureInfo>("la-capture", (e) => {
        setRunning(false);
        setProgress(null);
        const ci = e.payload;
        addLog("info", `Captured ${fmtSamples(ci.samples)} samples at ${fmtRate(ci.sampleRate)} (${fmtTime(ci.samples / ci.sampleRate)}) from ${ci.source}${ci.trigger != null ? `, trigger at ${fmtTime(ci.trigger / ci.sampleRate)}` : ""}`);
        noteDeviceSuccess();
        void afterCapture(ci).then(async () => {
          const sc = ci.source.startsWith("Simulator") ? autoStackFor.current : null;
          if (sc && (stackRef.current.length === 0 || autoStack.current !== null)) {
            autoStack.current = sc.id;
            await addDecoders(sc.decoders, true);
          } else {
            await rerunAll(stackRef.current);
          }
        });
      }),
      listen<{ message: string }>("la-error", (e) => {
        setRunning(false);
        setProgress(null);
        setError(e.payload.message);
        addLog("error", e.payload.message);
        noteDeviceError();
      }),
    ];
    return () => { subs.forEach((s) => s.then((un) => un())); };
  }, [addLog, afterCapture, addDecoders, rerunAll]);

  // ── Run ──
  const trigger: Trigger = useMemo(() => {
    if (triggerKind === "edge") return { kind: "edge", channel: edgeCh, edge: edgeType };
    if (triggerKind === "pattern") return patternTrigger(pattern);
    return { kind: "none" };
  }, [triggerKind, edgeCh, edgeType, pattern]);

  const run = useCallback(async () => {
    setError(null);
    if (samples > FREE_SAMPLES && !requirePro(`Captures longer than ${fmtSamples(FREE_SAMPLES)} samples`)) return;
    if (trigger.kind === "pattern" && !requirePro("Pattern triggers")) return;
    let source: Source;
    let chans = channels;
    if (sourceKind === "simulator") {
      if (!scenarioInfo) return;
      source = { kind: "simulator", scenario };
      // A different scenario than the capture on screen: take its channel names.
      if (!info?.source.includes(scenarioInfo.name)) chans = scenarioInfo.channels;
      autoStackFor.current = scenarioInfo;
    } else {
      if (!board) { setError("No FX2 board found. Plug one in and press refresh."); return; }
      source = { kind: "fx2", id: board.info.id };
      autoStackFor.current = null;
    }
    try {
      await invoke("la_capture_start", { request: { source, settings: { sampleRate: rate, samples, trigger, preTrigger: trigger.kind === "none" ? 0 : preTrigger }, channels: chans } });
      setRunning(true);
      setProgress({ phase: trigger.kind === "none" ? "recording" : "waiting", samples: 0, total: samples });
    } catch (e) {
      setError(String(e));
    }
  }, [samples, trigger, requirePro, sourceKind, scenarioInfo, scenario, info, channels, board, rate, preTrigger]);

  const stop = useCallback(() => { void invoke("la_capture_stop"); }, []);

  // Picking a scenario sets its sample rate and channel names for the trigger list.
  const pickScenario = (id: ScenarioId) => {
    setScenario(id);
    const s = devices?.scenarios.find((x) => x.id === id);
    if (s) {
      setRate(s.defaultRate);
      if (!info) setChannels(s.channels);
      setSamples(Math.min(Math.max(Math.round(s.defaultRate * s.duration), 100_000), FREE_SAMPLES));
      setMeasureCh(id === "modbusRtu" ? 5 : 0);
      if (id === "modbusRtu") { setTriggerKind("edge"); setEdgeCh(2); setEdgeType("rising"); }
      else if (id === "canBus" || id === "uartConsole") { setTriggerKind("edge"); setEdgeCh(0); setEdgeType("falling"); }
      else if (id === "spiMcp2515") { setTriggerKind("edge"); setEdgeCh(0); setEdgeType("falling"); }
      else { setTriggerKind("edge"); setEdgeCh(1); setEdgeType("falling"); }
    }
  };
  useEffect(() => { if (devices && !info) pickScenario("modbusRtu"); /* first load */ // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [devices]);

  // ── Channels ──
  const renameChannel = (i: number, patch: Partial<ChannelInfo>) => {
    const next = channels.map((c, k) => (k === i ? { ...c, ...patch } : c));
    setChannels(next);
    if (info) void invoke("la_set_channels", { channels: next });
  };

  // ── Measurement between cursors (or over the view) ──
  useEffect(() => {
    if (!info) { setMeasurement(null); return; }
    const [a, b] = cursors;
    const start = a != null && b != null ? Math.min(a, b) : Math.floor(view.start);
    const end = a != null && b != null ? Math.max(a, b) : Math.ceil(view.start + view.spp * plotW);
    const t = setTimeout(() => {
      invoke<Measurement | null>("la_measure", { channel: measureCh, start, end }).then(setMeasurement).catch(() => setMeasurement(null));
    }, 120);
    return () => clearTimeout(t);
  }, [info, cursors, view, plotW, measureCh]);

  // ── Files ──
  const session = useCallback(() => makeSession(stack.map((d) => ({ id: d.id, config: d.config, hidden: d.hidden })), cursors, view), [stack, cursors, view]);

  const saveCapture = useCallback(async () => {
    if (!info) return;
    const path = await saveDialog({ defaultPath: `capture-${new Date().toISOString().slice(0, 19).replace(/[:T]/g, "-")}.vla`, filters: [...CAPTURE_FILTERS, { name: "Value Change Dump", extensions: ["vcd"] }] });
    if (!path) return;
    const ext = path.split(".").pop()?.toLowerCase();
    if ((ext === "sr" || ext === "vcd") && !requirePro(ext === "sr" ? "Export to sigrok .sr" : "Export to VCD")) return;
    try {
      await invoke("la_save", { path, session: session() });
      addLog("info", `Saved to ${path}`);
    } catch (e) {
      setError(String(e));
    }
  }, [info, session, requirePro, addLog]);

  const openCapture = useCallback(async () => {
    const path = await openDialog({ multiple: false, directory: false, filters: [{ name: "Captures", extensions: ["vla", "sr"] }, ...CAPTURE_FILTERS] });
    if (!path || Array.isArray(path)) return;
    try {
      const o = await invoke<Opened>("la_open", { path });
      for (const w of o.warnings) addLog("info", w);
      await afterCapture(o.info);
      addLog("info", `Opened ${o.info.source}: ${fmtSamples(o.info.samples)} samples at ${fmtRate(o.info.sampleRate)}`);
      const s = readSession(o.session);
      autoStack.current = null;
      for (const d of stackRef.current) void invoke("la_decoder_remove", { id: d.id });
      if (s) {
        const items = s.decoders.map((d) => ({ id: nextId.current++, config: d.config, view: null, error: null, hidden: !!d.hidden, revision: 0 }));
        await rerunAll(items);
        setCursors(s.cursors);
        if (s.view) { fitted.current = false; setViewState(clampView(s.view, o.info.samples, plotWRef.current)); }
      } else {
        setStack([]);
      }
    } catch (e) {
      setError(String(e));
    }
  }, [addLog, afterCapture, rerunAll]);

  // ── Firmware ──
  const downloadFirmware = useCallback(async () => {
    setDownloading(true);
    try {
      const files = await invoke<string[]>("la_firmware_download");
      addLog("info", `Installed fx2lafw ${firmware?.release ?? ""} from sigrok.org: ${files.join(", ")}`);
      setFirmware(await invoke<FirmwareStatus>("la_firmware_status"));
    } catch (e) {
      setError(String(e));
    } finally {
      setDownloading(false);
    }
  }, [addLog, firmware]);

  const chooseFirmware = useCallback(async () => {
    const path = await openDialog({ multiple: false, directory: false, filters: [{ name: "FX2 firmware", extensions: ["fw", "bin"] }] });
    if (!path || Array.isArray(path)) return;
    try { setFirmware(await invoke<FirmwareStatus>("la_firmware_choose", { path })); } catch (e) { setError(String(e)); }
  }, []);

  // ── Table and annotations ──
  const selectRow = useCallback((id: number, index: number, r: TableRow) => {
    setSelectedRow({ id, index });
    setHighlight({ start: r.start, end: r.end });
    fitted.current = false;
    if (info) setViewState(viewOf(r.start, r.end, info.samples, plotW));
  }, [info, plotW]);

  const onAnnotationClick = useCallback((id: number, a: Annotation) => {
    const item = stackRef.current.find((d) => d.id === id);
    const rows = item?.view?.table?.rows;
    setHighlight({ start: a.start, end: a.end });
    if (!rows) return;
    const index = rows.findIndex((r) => a.start >= r.start && a.start < r.end);
    if (index >= 0) {
      setSelectedRow({ id, index });
      setBottomTab(`dec-${id}`);
    }
  }, []);

  const exportTable = useCallback(async (item: StackItem) => {
    if (!info || !item.view?.table) return;
    if (!requirePro("Protocol table export")) return;
    try {
      const p = await saveTextFile(`${DECODER_NAMES[item.config.kind].toLowerCase().replace(/ /g, "-")}-table.csv`, tableCsv(item.view.table, info));
      if (p) addLog("info", `Table exported to ${p}`);
    } catch (e) { setError(String(e)); }
  }, [info, requirePro, addLog]);

  const lanes: DecoderLane[] = useMemo(
    () => stack.filter((d) => !d.hidden && d.view).map((d) => ({ id: d.id, title: `${decoderLabel(d.config, channels)} — ${d.view!.summary}`, rows: d.view!.rows, revision: d.revision })),
    [stack, channels],
  );

  const openPrivacy = useCallback(async () => {
    try { await invoke("open_url", { url: await withUtm("https://voltrus.id/privacy/", "privacy") }); } catch (e) { addLog("error", String(e)); }
  }, [addLog]);

  // ── Sidebar ──
  const rateOptions = (sourceKind === "fx2" ? devices?.sampleRates ?? [] : [...(devices?.sampleRates ?? []), 32_000_000, 48_000_000, 64_000_000])
    .map((r) => ({ value: String(r), label: fmtRate(r) }));
  const chOptions = channels.map((c, i) => ({ value: String(i), label: `D${i} · ${c.name}` }));

  const deviceSection = (
    <Section title="Device" actions={<Button size="sm" variant="ghost" aria-label="Refresh devices" icon={<RefreshCw size={14} strokeWidth={1.75} />} onClick={() => void refresh()} disabled={running} />}>
      <Segmented label="Source" block value={sourceKind} onChange={(v) => setSourceKind(v)}
        options={[{ value: "simulator", label: "Simulator" }, { value: "fx2", label: "USB board" }]} />
      {sourceKind === "simulator" ? (
        <>
          <Field label="Scenario">
            <SheetSelect aria-label="Scenario" sheetTitle="Scenario" value={scenario} disabled={running}
              options={(devices?.scenarios ?? []).map((s) => ({ value: s.id, label: s.name }))} onChange={(v) => pickScenario(v as ScenarioId)} />
          </Field>
          {scenarioInfo && <span style={hint}>{scenarioInfo.description}</span>}
        </>
      ) : (
        <>
          <Field label="Board">
            <SheetSelect aria-label="Board" sheetTitle="FX2 board" value={boardId} disabled={running}
              options={devices?.boards.length ? devices.boards.map((b) => ({ value: b.info.id, label: `${b.board.name}${b.firmwareLoaded ? " · fx2lafw running" : ""}` })) : [{ value: "", label: "No FX2 board found" }]}
              onChange={(v) => setBoardId(String(v))} />
          </Field>
          {board ? (
            <div style={row}>
              <Badge tone={board.firmwareLoaded || boardFirmware ? "success" : "warn"}>
                {board.firmwareLoaded ? "Firmware running" : boardFirmware ? "Firmware found" : "Firmware needed"}
              </Badge>
              <span style={{ flex: 1 }} />
              <Button size="sm" variant="ghost" onClick={() => setShowFirmware(true)}>Firmware…</Button>
            </div>
          ) : (
            <span style={hint}>
              Plug in a Cypress FX2 logic analyzer (the 24 MHz 8-channel boards sold as Saleae compatibles, VID:PID 0925:3881, or a CY7C68013A board). The hardware path is new and has not been
              tested on a real board yet: please report how it goes.
              {devices?.usbError ? ` USB: ${devices.usbError}` : ""}
            </span>
          )}
          {!board && <Button size="sm" variant="ghost" onClick={() => setShowFirmware(true)}>Firmware…</Button>}
        </>
      )}
      <Field label="Sample rate">
        <SheetSelect aria-label="Sample rate" sheetTitle="Sample rate" value={String(rate)} disabled={running} options={rateOptions} onChange={(v) => setRate(Number(v))} />
      </Field>
      <Field label={`Length · ${fmtTime(samples / rate)}`}>
        <SheetSelect aria-label="Capture length" sheetTitle="Capture length" value={String(samples)} disabled={running}
          options={LENGTHS.map((n) => ({ value: String(n), label: `${fmtSamples(n)} samples (${fmtTime(n / rate)})${!pro && n > FREE_SAMPLES ? " · Pro" : ""}` }))}
          onChange={(v) => setSamples(Number(v))} />
      </Field>
      {sourceKind === "simulator" && scenarioInfo && rate < scenarioInfo.minRate && (
        <Callout tone="warn">{fmtRate(rate)} is too slow to decode this scenario; use at least {fmtRate(scenarioInfo.minRate)}.</Callout>
      )}
    </Section>
  );

  const triggerSection = (
    <Section title="Trigger">
      <Segmented label="Trigger" block value={triggerKind} onChange={(v) => setTriggerKind(v)}
        options={[{ value: "none", label: "None" }, { value: "edge", label: "Edge" }, { value: "pattern", label: <>Pattern <ProBadge /></> }]} />
      {triggerKind === "edge" && (
        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 6 }}>
          <SheetSelect aria-label="Trigger channel" sheetTitle="Trigger channel" value={String(edgeCh)} options={chOptions} onChange={(v) => setEdgeCh(Number(v))} />
          <SheetSelect aria-label="Edge" sheetTitle="Edge" value={edgeType} onChange={(v) => setEdgeType(v as typeof edgeType)}
            options={[{ value: "rising", label: "Rising ↑" }, { value: "falling", label: "Falling ↓" }, { value: "any", label: "Either ↕" }]} />
        </div>
      )}
      {triggerKind === "pattern" && (
        <div style={{ display: "grid", gridTemplateColumns: "repeat(8, 1fr)", gap: 3 }}>
          {pattern.map((b, i) => (
            <button key={i} type="button" className="vt-btn vt-btn--sm" title={`D${i} ${channels[i]?.name}: ${b === "x" ? "don't care" : b}`} aria-label={`D${i} pattern bit ${b}`}
              onClick={() => setPattern((p) => p.map((x, k) => (k === i ? (x === "x" ? "1" : x === "1" ? "0" : "x") : x)))}
              style={{ padding: 0, justifyContent: "center", fontFamily: "var(--font-mono)", borderColor: channelColor(channels[i], i) }}>
              {b === "x" ? "·" : b}
            </button>
          ))}
        </div>
      )}
      {triggerKind === "pattern" && <span style={hint}>Click a channel to cycle don't care (·), 1 and 0. Fires when the pattern becomes true.</span>}
      {triggerKind !== "none" && (
        <Field label={`Before the trigger · ${Math.round(preTrigger * 100)} %`}>
          <input type="range" min={0} max={0.9} step={0.05} value={preTrigger} onChange={(e) => setPreTrigger(Number(e.target.value))} aria-label="Pre-trigger share" />
        </Field>
      )}
    </Section>
  );

  const channelSection = (
    <Section title="Channels">
      {channels.map((c, i) => (
        <div key={i} style={row}>
          <input type="color" aria-label={`D${i} colour`} value={channelColor(c, i)} onChange={(e) => renameChannel(i, { color: e.target.value })}
            style={{ width: 22, height: 22, padding: 0, border: "none", background: "none", flexShrink: 0 }} />
          <span className="mono" style={{ fontSize: 11, color: "var(--text-muted)", width: 18 }}>D{i}</span>
          <input className="vt-input" aria-label={`D${i} name`} value={c.name} onChange={(e) => renameChannel(i, { name: e.target.value })} style={{ minWidth: 0 }} />
          <Button size="sm" variant="ghost" aria-label={hidden.has(i) ? `Show D${i}` : `Hide D${i}`}
            icon={hidden.has(i) ? <EyeOff size={14} strokeWidth={1.75} /> : <Eye size={14} strokeWidth={1.75} />}
            onClick={() => setHidden((h) => { const n = new Set(h); if (n.has(i)) n.delete(i); else n.add(i); return n; })} />
        </div>
      ))}
      <Button size="sm" variant="ghost" onClick={() => { setChannels((cs) => cs.map((c) => ({ ...c, color: null }))); if (info) void invoke("la_set_channels", { channels: channels.map((c) => ({ ...c, color: null })) }); }}>
        Default colours
      </Button>
    </Section>
  );

  const decoderSection = (
    <Section title="Decoders" actions={<Button size="sm" icon={<Plus size={14} strokeWidth={1.75} />} disabled={!info} onClick={() => setEditing({ id: null, config: null })}>Add</Button>}>
      {stack.length === 0 && <span style={hint}>{info ? "Add Modbus RTU, UART, SPI, I2C or CAN to decode the capture." : "Capture first, then add decoders."}</span>}
      {stack.map((d) => (
        <div key={d.id} style={{ display: "flex", flexDirection: "column", gap: 2, padding: "6px 8px", border: "1px solid var(--border)", borderRadius: "var(--radius)", background: "var(--card)" }}>
          <div style={row}>
            <strong style={{ fontSize: "var(--fs-sm)", flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }} title={decoderLabel(d.config, channels)}>
              {decoderLabel(d.config, channels)}
            </strong>
            <Button size="sm" variant="ghost" aria-label="Show or hide" icon={d.hidden ? <EyeOff size={13} strokeWidth={1.75} /> : <Eye size={13} strokeWidth={1.75} />}
              onClick={() => setStack((s) => s.map((x) => (x.id === d.id ? { ...x, hidden: !x.hidden } : x)))} />
            <Button size="sm" variant="ghost" aria-label="Edit" icon={<Pencil size={13} strokeWidth={1.75} />} onClick={() => setEditing({ id: d.id, config: d.config })} />
            <Button size="sm" variant="ghost" aria-label="Remove" icon={<Trash2 size={13} strokeWidth={1.75} />} onClick={() => removeDecoder(d.id)} />
          </div>
          <span style={{ ...hint, color: d.error ? "var(--danger)" : undefined }}>
            {d.error ?? d.view?.summary ?? "Decoding…"}
            {!d.error && decoderChannels(d.config).some((c) => c >= channels.length) ? " (channel missing)" : ""}
          </span>
        </div>
      ))}
    </Section>
  );

  // ── Main area ──
  const delta = info && cursors[0] != null && cursors[1] != null ? cursorDelta(info, cursors[0], cursors[1]) : null;
  const measureBar = info && (
    <div style={{ display: "flex", gap: 10, alignItems: "center", flexWrap: "wrap", fontSize: "var(--fs-xs)", color: "var(--text-dim)" }}>
      <Badge tone="neutral">{info.source}</Badge>
      <span className="mono">{fmtSamples(info.samples)} @ {fmtRate(info.sampleRate)} · {fmtTime(info.samples / info.sampleRate)}</span>
      <span style={{ color: "#22d3ee" }} className="mono">A {cursors[0] != null ? fmtTime(sampleTime(info, cursors[0])) : "—"}</span>
      <span style={{ color: "#e879f9" }} className="mono">B {cursors[1] != null ? fmtTime(sampleTime(info, cursors[1])) : "—"}</span>
      {delta && <strong className="mono" style={{ color: "var(--text)" }}>Δ {fmtTime(delta.dt)}{delta.freq ? ` · ${fmtRate(delta.freq)}` : ""} · {delta.samples.toLocaleString()} samples</strong>}
      {(cursors[0] != null || cursors[1] != null) && <Button size="sm" variant="ghost" onClick={() => setCursors([null, null])}>Clear cursors</Button>}
      <span style={{ flex: 1 }} />
      <span>Measure</span>
      <div style={{ width: 150 }}>
        <SheetSelect aria-label="Measured channel" sheetTitle="Measured channel" value={String(measureCh)} options={chOptions} onChange={(v) => setMeasureCh(Number(v))} />
      </div>
      <span className="mono" title={cursors[0] != null && cursors[1] != null ? "Between the cursors" : "Over the visible window"}>
        {measurement ? `${fmtRate(measurement.frequency)} · duty ${(measurement.duty * 100).toFixed(1)} % · ${measurement.periods} periods` : "no full period"}
      </span>
      <Button size="sm" variant="ghost" aria-label="Zoom in" icon={<ZoomIn size={14} strokeWidth={1.75} />} onClick={() => { fitted.current = false; setViewState((v) => zoomAt(v, 2, plotW / 2, info.samples, plotW)); }} />
      <Button size="sm" variant="ghost" aria-label="Zoom out" icon={<ZoomOut size={14} strokeWidth={1.75} />} onClick={() => { fitted.current = false; setViewState((v) => zoomAt(v, 0.5, plotW / 2, info.samples, plotW)); }} />
      <Button size="sm" variant="ghost" aria-label="Fit" icon={<Maximize2 size={14} strokeWidth={1.75} />} onClick={() => { fitted.current = true; setViewState(fitView(info.samples, plotW)); }} />
    </div>
  );

  const welcome = (
    <div style={{ flex: 1, display: "flex", alignItems: "center", justifyContent: "center" }}>
      <EmptyState
        icon={<Cpu size={20} strokeWidth={1.75} />}
        title="Capture a bus"
        description="Probe RS-485 (through a transceiver's RO pin), CAN_RX, SPI, I2C or UART lines with an FX2 USB logic analyzer, or run the built-in simulator: a PLC polling Modbus RTU drives, a CAN bus, an MCP2515 on SPI."
        action={
          <div style={{ display: "flex", gap: 8 }}>
            <Button variant="primary" icon={<Play size={14} strokeWidth={1.75} />} disabled={running} onClick={() => { setSourceKind("simulator"); void run(); }}>Capture the Modbus simulator</Button>
            <Button icon={<FolderOpen size={14} strokeWidth={1.75} />} onClick={() => void openCapture()}>Open a capture</Button>
          </div>
        }
      />
    </div>
  );

  const tables = stack.filter((d) => d.view?.table);
  const activeTable = tables.find((d) => `dec-${d.id}` === bottomTab) ?? null;
  const pct = progress && progress.total > 0 ? Math.round((progress.samples / progress.total) * 100) : 0;

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100vh", background: "var(--bg)", color: "var(--text)" }}>
      <Toolbar
        title="Voltrus Logic Analyzer"
        icon={appIcon}
        connected={running}
        onInfo={() => setShowInfo(true)}
        actions={
          <div style={{ display: "flex", gap: 6 }}>
            <Button size="sm" variant="ghost" icon={<FolderOpen size={14} strokeWidth={1.75} />} onClick={() => void openCapture()}>Open</Button>
            <Button size="sm" variant="ghost" icon={<Save size={14} strokeWidth={1.75} />} disabled={!info} onClick={() => void saveCapture()}>Save</Button>
          </div>
        }
      />
      <div style={{ display: "flex", flex: 1, overflow: "hidden" }}>
        <aside className="vt-sidebar" style={{ width: 300, flexShrink: 0, borderRight: "1px solid var(--border)", overflowY: "auto" }}>
          <Section>
            {running ? (
              <Button variant="danger" block icon={<Square size={14} strokeWidth={1.75} />} onClick={stop}>
                {progress?.phase === "waiting" ? "Stop (waiting for trigger)" : `Stop (${pct} %)`}
              </Button>
            ) : (
              <Button variant="primary" block icon={<Play size={14} strokeWidth={1.75} />} onClick={() => void run()}>Run capture</Button>
            )}
            {running && progress && (
              <div style={{ height: 4, borderRadius: 2, background: "var(--border)", overflow: "hidden" }}>
                <div style={{ width: progress.phase === "waiting" ? "100%" : `${pct}%`, height: "100%", background: progress.phase === "waiting" ? "var(--warn)" : "var(--accent)", opacity: progress.phase === "waiting" ? 0.5 : 1 }} />
              </div>
            )}
          </Section>
          {deviceSection}
          {triggerSection}
          {decoderSection}
          {channelSection}
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
          {info ? (
            <>
              {measureBar}
              <Waveform
                info={info}
                channels={channels}
                hidden={hidden}
                view={view}
                setView={setView}
                decoders={lanes}
                cursors={cursors}
                setCursors={setCursors}
                highlight={highlight}
                onAnnotationClick={onAnnotationClick}
                onWidth={setPlotW}
              />
            </>
          ) : welcome}
          <div style={{ height: 250, flexShrink: 0, display: "flex", flexDirection: "column", gap: 6 }}>
            <Tabs
              label="Bottom panel"
              value={activeTable ? bottomTab : "log"}
              onChange={setBottomTab}
              items={[
                ...tables.map((d) => ({ value: `dec-${d.id}`, label: DECODER_NAMES[d.config.kind], count: d.view!.table!.rows.length })),
                { value: "log", label: "Log" },
              ]}
            />
            {activeTable ? (
              <ProtocolTable
                table={activeTable.view!.table}
                selected={selectedRow?.id === activeTable.id ? selectedRow.index : null}
                onSelect={(i, r) => selectRow(activeTable.id, i, r)}
                onExport={() => void exportTable(activeTable)}
                pro={pro}
              />
            ) : (
              <LogPanel entries={log} onExport={(f) => addLog("info", `Log exported to ${f}`)} onClear={() => setLog([])} exportPrefix="logic-analyzer-log" />
            )}
          </div>
        </main>
      </div>
      <StatusBar connected={running} label={running ? (progress?.phase === "waiting" ? "Waiting for trigger" : "Capturing") : info ? `${fmtSamples(info.samples)} samples · ${info.source}` : "Idle"} error={error} />

      <DecoderDialog
        open={editing != null}
        initial={editing?.config ?? null}
        channels={channels}
        pro={pro}
        onClose={() => setEditing(null)}
        onSave={(d) => void saveDecoder(d)}
      />

      <Dialog open={showFirmware} onClose={() => setShowFirmware(false)} title="FX2 firmware (fx2lafw)" width={540} footer={<Button onClick={() => setShowFirmware(false)}>Close</Button>}>
        <div style={{ display: "flex", flexDirection: "column", gap: 10, fontSize: "var(--fs-sm)", color: "var(--text-dim)", lineHeight: 1.55 }}>
          <p style={{ margin: 0 }}>
            FX2 boards have no firmware of their own: it is loaded into the chip each time the board is plugged in. This app uses fx2lafw, the open firmware of the sigrok project
            ({firmware?.licence ?? "GPL-2.0-or-later"}), and ships the official {firmware?.release} images. It is a separate program that runs on the board, not part of the app. To
            use another version, download it from sigrok.org or choose a .fw file.
          </p>
          <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
            {firmware?.files.map(([name, found]) => (
              <div key={name} style={row}>
                <StatusDot tone={found ? "success" : "neutral"} />
                <span className="mono" style={{ fontSize: 11.5, flex: 1 }}>{name}</span>
                <span style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)" }} title={found?.path}>
                  {found ? `${found.source}${found.verified === "official" ? ", verified" : ", not the official image"}` : "not found"}
                </span>
              </div>
            ))}
          </div>
          {firmware?.userFile && <span style={hint}>Chosen file (used for every board): {firmware.userFile}</span>}
          <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
            <Button disabled={downloading} onClick={() => void downloadFirmware()}>{downloading ? "Downloading…" : "Download from sigrok.org"}</Button>
            <Button onClick={() => void chooseFirmware()}>Choose a .fw file…</Button>
            {firmware?.userFile && <Button variant="ghost" onClick={async () => setFirmware(await invoke<FirmwareStatus>("la_firmware_choose", { path: null }))}>Forget chosen file</Button>}
          </div>
          <span style={hint}>
            Source code of the bundled firmware:{" "}
            <a href="#" onClick={(e) => { e.preventDefault(); if (firmware) void invoke("open_url", { url: firmware.mirrorSourceUrl }); }}>{firmware?.mirrorSourceUrl}</a>
            {" "}(also at sigrok.org). For three years after we last distribute it, JoyoDigitama will send a copy of that source on request at no more than the cost of doing so:
            support@voltrus.id. Its licence texts are next to the images{firmware?.bundledDir ? ` in ${firmware.bundledDir}` : ""}. The download fetches {firmware?.downloadUrl} once,
            checks its SHA-256, and keeps the images with their licence in {firmware?.appDir ?? "the app's data folder"}.
          </span>
        </div>
      </Dialog>

      <Dialog open={showInfo} onClose={() => setShowInfo(false)} title="About Voltrus Logic Analyzer" width={480} footer={(
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
            A logic analyzer for Cypress FX2 USB boards, made for the wires of a plant: Modbus RTU on RS-485 with request and response pairing and timing, CAN at bit level, SPI with
            MCP2515 decoding, I2C and UART. The built-in simulator works without a board.
          </p>
          <p style={{ margin: "0 0 12px" }}>
            Pro adds the CAN decoder, MCP2515 decoding, pattern triggers, captures over {fmtSamples(FREE_SAMPLES)} samples, protocol table CSV export and sigrok .sr and VCD export.
          </p>
          <div style={{ borderTop: "1px solid var(--border)", paddingTop: 12, marginBottom: 12 }}>
            <ProAboutRow />
          </div>
          <p style={{ margin: "0 0 12px", fontSize: "var(--fs-xs)", color: "var(--text-muted)" }}>
            FX2 boards run fx2lafw, sigrok's open firmware (GPL-2.0-or-later). The app ships its unmodified images and uploads them to the board; its source is offered under
            Device, Firmware. No sigrok software is part of the app itself. Modbus is a trademark of Schneider Electric, and Saleae, Cypress and Microchip are their owners' trademarks; this app is not affiliated with or endorsed
            by any of them.
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
