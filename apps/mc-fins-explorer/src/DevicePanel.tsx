import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Button,
  DataTable,
  Dialog,
  EmptyState,
  IconButton,
  LogPanel,
  ProBadge,
  StatusBar,
  StatusDot,
  Tabs,
  ValueEditor,
  saveTextFile,
  useProStatus,
} from "@mactools/ui";
import { FileDown, Minus, Plus, Square, Tags as TagsIcon } from "lucide-react";
import { useStore } from "./store";
import { usePlcDevice, toSpecs, type PlcValue } from "./usePlcDevice";
import type { Device } from "./types";
import { DATA_TYPES, PROTOCOL_NAME } from "./types";
import ConnectionPanel from "./ConnectionPanel";
import MonitorPanel from "./MonitorPanel";
import PlcPanel from "./PlcPanel";
import TrendPanel from "./TrendPanel";
import { useTrend } from "./useTrend";
import { FREE_TAG_LIMIT } from "./helpers";
import { ValueLogWriter, valueLogHeader, valueLogRow } from "./valueLog";

type LogDir = "info" | "rx" | "tx" | "error";

interface Props {
  device: Device;
  active: boolean;
  onLog: (dir: LogDir, msg: string) => void;
}

export function formatValue(v: PlcValue | undefined): string {
  if (v === undefined || v === null) return "—";
  if (typeof v === "boolean") return v ? "On (1)" : "Off (0)";
  if (typeof v === "number") return Number.isInteger(v) ? String(v) : String(Math.round(v * 1000) / 1000);
  return String(v);
}

export default function DevicePanel({ device, active, onLog }: Props) {
  const plc = usePlcDevice(device.id);
  const { pro, requirePro } = useProStatus();
  const updateDevice = useStore((s) => s.updateDevice);
  const log = useStore((s) => s.log);
  const [pollMs, setPollMs] = useState(1000);
  const [mainTab, setMainTab] = useState<"tags" | "monitor" | "plc">("tags");
  const [bottomTab, setBottomTab] = useState<"log" | "trend">("log");
  const [writing, setWriting] = useState<{ address: string; dataType: string; value: string } | null>(null);
  const [writeError, setWriteError] = useState<string | null>(null);

  const allTags = device.tags;
  const tags = pro ? allTags : allTags.slice(0, FREE_TAG_LIMIT);
  const tagSig = JSON.stringify(toSpecs(tags));

  useEffect(() => {
    if (plc.error) onLog("error", `[${device.label}] ${plc.error}`);
  }, [plc.error, device.label, onLog]);

  useEffect(() => {
    if (plc.connected && tags.length > 0) {
      plc.startPolling(toSpecs(tags), pollMs);
      return () => plc.stopPolling();
    }
  }, [plc.connected, tagSig, pollMs, device.id]); // eslint-disable-line react-hooks/exhaustive-deps

  // ── Value logging to CSV (Pro) ──
  const logWriterRef = useRef<ValueLogWriter | null>(null);
  const [logging, setLogging] = useState<{ path: string; rows: number } | null>(null);

  const stopLogging = useCallback(
    async (reason?: string) => {
      const writer = logWriterRef.current;
      if (!writer) return;
      logWriterRef.current = null;
      setLogging(null);
      await writer.close();
      onLog("info", `[${device.label}] Value logging stopped${reason ? ` (${reason})` : ""}: ${writer.rows} rows in ${writer.path}`);
    },
    [device.label, onLog],
  );

  const startLogging = async () => {
    if (!requirePro("Logging values to CSV")) return;
    if (logWriterRef.current || tags.length === 0) return;
    try {
      const path = await saveTextFile(`${device.label}-values.csv`, valueLogHeader(tags) + "\n");
      if (!path) return;
      logWriterRef.current = new ValueLogWriter(path, tagSig, (e) => {
        logWriterRef.current = null;
        setLogging(null);
        onLog("error", `[${device.label}] Value logging stopped, could not write ${path}: ${String(e)}`);
      });
      setLogging({ path, rows: 0 });
      onLog("info", `[${device.label}] Logging ${tags.length} tags to ${path}, one row per poll`);
    } catch (e) {
      onLog("error", `[${device.label}] Could not start value logging: ${String(e)}`);
    }
  };

  useEffect(() => {
    if (logWriterRef.current && logWriterRef.current.signature !== tagSig) void stopLogging("tag list changed");
  }, [tagSig, stopLogging]);

  useEffect(() => {
    const writer = logWriterRef.current;
    if (!writer || plc.values.length !== tags.length) return;
    writer.push(valueLogRow(new Date(), plc.values));
    setLogging((cur) => (cur ? { ...cur, rows: writer.rows } : cur));
  }, [plc.values]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => () => { void logWriterRef.current?.close(); }, []);

  const handleConnect = async () => {
    onLog("info", `[${device.label}] Connecting to ${device.host}:${device.port} (${PROTOCOL_NAME[device.protocol]})…`);
    const summary = await plc.connect(device);
    if (summary) onLog("rx", `[${device.label}] Connected: ${summary}`);
  };

  const handleDisconnect = async () => {
    await stopLogging("disconnected");
    await plc.disconnect();
    onLog("info", `[${device.label}] Disconnected`);
  };

  const columns = useMemo(
    () => [
      { key: "name", label: "Name", width: "180px" },
      { key: "address", label: "Address", width: "120px" },
      { key: "type", label: "Type", width: "90px" },
      { key: "value", label: "Value" },
      { key: "trend", label: "Trend", width: "60px" },
    ],
    [],
  );

  const tableData = useMemo(
    () =>
      tags.map((t, i) => ({
        name: t.name,
        address: t.address,
        type: t.data_type,
        value: plc.values.length === tags.length ? formatValue(plc.values[i]) : "—",
      })),
    [tags, plc.values],
  );

  const { trendSeries, setTrendSeries, trendWindow, setTrendWindow, toggleTrend } = useTrend(
    tableData.map((r) => ({ name: r.name, value: r.value.replace(/^On \(1\)$/, "1").replace(/^Off \(0\)$/, "0") })),
  );

  const writeValue = async (address: string, dataType: string, value: string | number | boolean) => {
    const err = await plc.writeTag(address, dataType, value);
    if (err) onLog("error", `[${device.label}] Write ${value} to ${address} failed: ${err}`);
    else {
      onLog("tx", `[${device.label}] Wrote ${value} (${dataType}) to ${address}`);
      if (plc.connected && tags.length > 0) void plc.readTags(toSpecs(tags));
    }
    return err;
  };

  const onRowClick = (row: Record<string, unknown>) => {
    if (!plc.connected) return;
    if (!requirePro("Writing values to the PLC")) return;
    setWriteError(null);
    setWriting({ address: String(row.address), dataType: String(row.type), value: row.value === "—" ? "" : String(row.value) });
  };

  const lockedCount = allTags.length - tags.length;

  const writeDialog =
    writing &&
    (writing.dataType === "Bool" ? (
      <Dialog open onClose={() => setWriting(null)} title="Write bit" width={300}>
        <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
          <div className="mono" style={{ fontSize: "var(--fs-md)", color: "var(--text)" }}>{writing.address}</div>
          {writeError && <div role="alert" style={{ color: "var(--danger)", fontSize: "var(--fs-sm)" }}>{writeError}</div>}
          <div style={{ display: "flex", gap: 8 }}>
            <Button variant="primary" style={{ flex: 1 }} onClick={async () => { const e = await writeValue(writing.address, "Bool", true); if (e) setWriteError(e); else setWriting(null); }}>Set ON</Button>
            <Button style={{ flex: 1 }} onClick={async () => { const e = await writeValue(writing.address, "Bool", false); if (e) setWriteError(e); else setWriting(null); }}>Reset OFF</Button>
          </div>
        </div>
      </Dialog>
    ) : (
      <ValueEditor
        address={writing.address}
        currentValue={writing.value || undefined}
        currentDataType={writing.dataType}
        types={DATA_TYPES.filter((d) => d !== "Bool").map((d) => ({ value: d, label: d }))}
        error={writeError}
        onSubmit={async (value, dataType) => {
          const e = await writeValue(writing.address, dataType, value);
          if (e) setWriteError(e);
          else setWriting(null);
        }}
        onCancel={() => setWriting(null)}
      />
    ));

  const tagsView = (
    <>
      {!pro && allTags.length > 0 && (
        <div style={{ display: "flex", alignItems: "center", gap: 8, padding: "6px 14px", fontSize: "var(--fs-xs)", color: "var(--text-muted)", borderBottom: "1px solid var(--border)" }}>
          <span>{Math.min(allTags.length, FREE_TAG_LIMIT)} of {FREE_TAG_LIMIT} live tags{lockedCount > 0 ? `, ${lockedCount} not polled` : ""}</span>
          <button type="button" style={{ background: "none", border: 0, padding: 0, color: "var(--accent)", cursor: "pointer", font: "inherit" }} onClick={() => requirePro("More than 10 live tags")}>Pro for more</button>
        </div>
      )}
      <div style={{ display: "flex", alignItems: "center", gap: 8, padding: "6px 14px", fontSize: "var(--fs-xs)", color: "var(--text-muted)", borderBottom: "1px solid var(--border)" }}>
        {logging ? (
          <>
            <StatusDot tone="success" label="Logging values" />
            <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }} title={logging.path}>
              Logging {logging.rows} rows to <span className="mono">{logging.path.split(/[\\/]/).pop()}</span>
            </span>
            <Button size="sm" icon={<Square size={14} strokeWidth={1.75} />} onClick={() => void stopLogging()}>Stop logging</Button>
          </>
        ) : (
          <>
            <span style={{ flex: 1 }}>{plc.connected ? "Click a value to write it." : "Connect to poll these tags."}</span>
            <Button size="sm" icon={<FileDown size={14} strokeWidth={1.75} />} disabled={!plc.connected || tags.length === 0} onClick={() => void startLogging()}>
              Log values to CSV <ProBadge />
            </Button>
          </>
        )}
      </div>
      {tags.length === 0 ? (
        <EmptyState compact icon={<TagsIcon size={18} strokeWidth={1.75} />} title="No tags" description="Add tags in the Tags panel on the right, or load the simulator's tags." />
      ) : (
        <DataTable
          columns={columns}
          data={tableData}
          onRowClick={onRowClick}
          renderCell={(key, row) => {
            if (key !== "trend") return String(row[key] ?? "");
            const name = String(row.name);
            const on = trendSeries.has(name);
            return (
              <IconButton
                size="sm"
                pressed={on}
                label={on ? `Remove ${name} from trend` : `Add ${name} to trend`}
                onClick={(e) => { e.stopPropagation(); if (on || requirePro("Live trends")) toggleTrend(name); }}
              >
                {on ? <Minus size={14} strokeWidth={1.75} /> : <Plus size={14} strokeWidth={1.75} />}
              </IconButton>
            );
          }}
        />
      )}
    </>
  );

  return (
    <div style={{ display: active ? "flex" : "none", flex: 1, overflow: "hidden", flexDirection: "column", minHeight: 0 }}>
      <div style={{ display: "flex", flex: 1, overflow: "hidden", minHeight: 0 }}>
        <aside className="vt-sidebar" style={{ width: 272, flexShrink: 0, borderRight: "1px solid var(--border)", overflowY: "auto" }}>
          <ConnectionPanel
            device={device}
            connected={plc.connected}
            loading={plc.loading}
            pollMs={pollMs}
            onPollMs={setPollMs}
            onChange={(patch) => updateDevice(device.id, patch)}
            onConnect={() => void handleConnect()}
            onDisconnect={() => void handleDisconnect()}
          />
        </aside>
        <main style={{ display: "flex", flex: 1, flexDirection: "column", overflow: "hidden", minWidth: 0 }}>
          <div style={{ flex: 1.3, minHeight: 160, overflow: "hidden", display: "flex", flexDirection: "column" }}>
            <Tabs
              label="Device views"
              value={mainTab}
              onChange={setMainTab}
              items={[
                { value: "tags", label: "Tags", count: tags.length || undefined },
                { value: "monitor", label: "Device monitor" },
                { value: "plc", label: "PLC" },
              ]}
            />
            <div style={{ flex: 1, minHeight: 0, overflow: "auto", display: "flex", flexDirection: "column" }}>
              {mainTab === "tags" && tagsView}
              {mainTab === "monitor" && (
                <MonitorPanel device={device} connected={plc.connected} pollMs={pollMs} onLog={onLog} onWrite={(a, t, v) => plc.writeTag(a, t, v)} />
              )}
              {mainTab === "plc" && <PlcPanel device={device} connected={plc.connected} onLog={onLog} />}
            </div>
          </div>
          <div style={{ display: "flex", flexDirection: "column", flex: 1, minHeight: 140, borderTop: "1px solid var(--border)", overflow: "hidden" }}>
            <BottomTabs value={bottomTab} onChange={(v) => { if (v === "trend" && !requirePro("Live trends")) return; setBottomTab(v); }} trendCount={trendSeries.size} />
            <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column", overflow: "hidden" }}>
              {bottomTab === "log" ? (
                <LogPanel entries={log} onExport={(f) => onLog("info", `Log exported to ${f}`)} />
              ) : (
                <TrendPanel
                  series={Array.from(trendSeries.values())}
                  onRemove={(n) => setTrendSeries((p) => { const m = new Map(p); m.delete(n); return m; })}
                  windowSec={trendWindow}
                  onWindowChange={setTrendWindow}
                  theme="dark"
                />
              )}
            </div>
          </div>
        </main>
      </div>
      <StatusBar connected={plc.connected} latency={plc.latency} pollCount={plc.pollCount} error={plc.error} />
      {writeDialog}
    </div>
  );
}

function BottomTabs({ value, onChange, trendCount }: { value: "log" | "trend"; onChange: (v: "log" | "trend") => void; trendCount: number }) {
  const showFrames = useStore((s) => s.showFrames);
  const setShowFrames = useStore((s) => s.setShowFrames);
  return (
    <div style={{ display: "flex", alignItems: "center" }}>
      <div style={{ flex: 1, minWidth: 0 }}>
        <Tabs
          label="Bottom views"
          value={value}
          onChange={onChange}
          items={[
            { value: "log", label: "Comm log" },
            { value: "trend", label: <>Trend <ProBadge /></>, count: trendCount > 0 ? trendCount : undefined },
          ]}
        />
      </div>
      <label style={{ display: "flex", alignItems: "center", gap: 6, padding: "0 14px", fontSize: "var(--fs-xs)", color: "var(--text-muted)", whiteSpace: "nowrap" }}>
        <input type="checkbox" checked={showFrames} onChange={(e) => setShowFrames(e.target.checked)} />
        Show frames
      </label>
    </div>
  );
}
