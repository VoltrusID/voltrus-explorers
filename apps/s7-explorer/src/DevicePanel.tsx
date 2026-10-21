import { useCallback, useEffect, useMemo, useState } from "react";
import {
  Button,
  ConnectionForm,
  DataTable,
  Dialog,
  EmptyState,
  IconButton,
  LogPanel,
  ProBadge,
  StatusBar,
  Tabs,
  ValueEditor,
  useProStatus,
} from "@mactools/ui";
import { Minus, Plus, Tags as TagsIcon } from "lucide-react";
import { useStore } from "./store";
import { useS7Device, toSpecs, type S7Value } from "./useS7Device";
import type { Device } from "./types";
import { DATA_TYPES } from "./types";
import DiagnosticsPanel from "./DiagnosticsPanel";
import BlocksPanel from "./BlocksPanel";
import TrendPanel from "./TrendPanel";
import { useTrend } from "./useTrend";

type LogDir = "info" | "rx" | "tx" | "error";

function formatS7Value(v: S7Value | undefined): string {
  if (v === undefined || v === null) return "—";
  if (typeof v === "boolean") return v ? "On (1)" : "Off (0)";
  if (typeof v === "number") return Number.isInteger(v) ? String(v) : v.toFixed(3);
  return String(v);
}

interface Props {
  device: Device;
  active: boolean;
  onLog: (dir: LogDir, msg: string) => void;
}

export default function DevicePanel({ device, active, onLog }: Props) {
  const s7 = useS7Device(device.id);
  const { requirePro } = useProStatus();
  const updateDevice = useStore((st) => st.updateDevice);
  const isMobile = useStore((st) => st.isMobile);
  const mobileTab = useStore((st) => st.mobileTab);

  const [pollMs, setPollMs] = useState(1000);
  const [mainTab, setMainTab] = useState<"tags" | "diagnostics" | "blocks">("tags");
  const [bottomTab, setBottomTab] = useState<"log" | "trend">("log");
  const [writing, setWriting] = useState<{ address: string; dataType: string; value: string } | null>(null);

  const tags = device.tags ?? [];
  const log = useStore((st) => st.log);

  // Surface backend errors in the log.
  useEffect(() => {
    if (s7.error) onLog("error", `[${device.label}] ${s7.error}`);
  }, [s7.error, device.label, onLog]);

  // Start/stop polling on connection + tag changes.
  useEffect(() => {
    if (s7.connected && tags.length > 0) {
      s7.startPolling(toSpecs(tags), pollMs);
      return () => s7.stopPolling();
    }
  }, [s7.connected, tags.length, pollMs, device.id]); // eslint-disable-line react-hooks/exhaustive-deps

  // Restart polling when the rate changes.
  useEffect(() => {
    if (s7.connected && tags.length > 0) s7.startPolling(toSpecs(tags), pollMs);
  }, [pollMs]); // eslint-disable-line react-hooks/exhaustive-deps

  const handleConnect = useCallback(
    async (values: Record<string, string | number>) => {
      const config = {
        host: String(values.host),
        port: Number(values.port),
        rack: Number(values.rack),
        slot: Number(values.slot),
      };
      updateDevice(device.id, config);
      onLog("info", `[${device.label}] Connecting to ${config.host}:${config.port} (rack=${config.rack}, slot=${config.slot})…`);
      const ok = await s7.connect(config);
      if (ok) onLog("rx", `[${device.label}] Connected`);
    },
    [device.id, device.label, s7, updateDevice, onLog],
  );

  const handleDisconnect = useCallback(async () => {
    await s7.disconnect();
    onLog("info", `[${device.label}] Disconnected`);
  }, [s7, device.label, onLog]);

  const columns = useMemo(
    () => [
      { key: "name", label: "Name", width: "120px" },
      { key: "address", label: "Address", width: "120px" },
      { key: "type", label: "Type", width: "90px" },
      { key: "value", label: "Value" },
      { key: "trend", label: "Trend", width: "60px" },
    ],
    [],
  );

  const tableData = useMemo(() => {
    if (s7.values.length === tags.length) {
      return tags.map((t, i) => ({
        name: t.name,
        address: t.address,
        type: t.data_type,
        value: formatS7Value(s7.values[i]),
      }));
    }
    return tags.map((t) => ({ name: t.name, address: t.address, type: t.data_type, value: "—" }));
  }, [tags, s7.values]);

  const { trendSeries, setTrendSeries, trendWindow, setTrendWindow, toggleTrend } = useTrend(tableData);

  const onRowClick = (row: Record<string, unknown>) => {
    if (!requirePro("Writing values to the PLC")) return;
    setWriting({
      address: String(row.address ?? ""),
      dataType: String(row.type ?? ""),
      value: row.value === "—" ? "" : String(row.value ?? ""),
    });
  };

  const renderTrendCell = (name: string) => {
    const isActive = trendSeries.has(name);
    return (
      <IconButton
        size="sm"
        pressed={isActive}
        label={isActive ? `Remove ${name} from trend` : `Add ${name} to trend`}
        onClick={(e) => { e.stopPropagation(); if (isActive || requirePro("Live trends")) toggleTrend(name); }}
      >
        {isActive ? <Minus size={14} strokeWidth={1.75} /> : <Plus size={14} strokeWidth={1.75} />}
      </IconButton>
    );
  };

  const dataTableEl = (
    <DataTable
      columns={columns}
      data={tableData}
      onRowClick={onRowClick}
      emptyMessage="No tags configured. Add tags in the Tags panel."
      renderCell={(key, row) => {
        if (key === "trend") return renderTrendCell(String(row.name));
        return String(row[key] ?? "");
      }}
    />
  );

  // ── Sidebar: connection + poll rate (tags live in the right sidebar now) ──
  const sidebar = (
    <>
      <div style={{ display: "flex", flexDirection: "column", gap: 10, padding: 14 }}>
        <ConnectionForm
          fields={[
            { name: "host", label: "IP address", type: "text", default: device.host },
            { name: "port", label: "Port", type: "number", default: device.port },
            { name: "rack", label: "Rack", type: "number", default: device.rack },
            { name: "slot", label: "Slot", type: "number", default: device.slot },
          ]}
          onSubmit={handleConnect}
          onDisconnect={handleDisconnect}
          connected={s7.connected}
          loading={s7.loading}
        />
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <label className="vt-label" htmlFor={`poll-${device.id}`} style={{ marginBottom: 0 }}>Poll interval (ms)</label>
          <input
            id={`poll-${device.id}`}
            type="number"
            min={100}
            step={100}
            value={pollMs}
            onChange={(e) => setPollMs(Math.max(100, Number(e.target.value)))}
            className="vt-input"
            style={{ width: 84 }}
          />
        </div>
      </div>
    </>
  );

  // ── Main: tabs (Tags / Diagnostics / Blocks) + bottom (Comm log / Trend) ──
  const main = (
    <>
      <Tabs
        label="Device views"
        value={mainTab}
        onChange={(v) => { if (v === "blocks" && !requirePro("Block manager")) return; setMainTab(v); }}
        items={[
          { value: "tags", label: "Tags" },
          { value: "diagnostics", label: "Diagnostics" },
          { value: "blocks", label: <>Blocks <ProBadge /></> },
        ]}
      />
      <div style={{ flex: 1, minHeight: 0, overflow: "auto", display: "flex", flexDirection: "column" }}>
        {mainTab === "tags" && (tags.length === 0
          ? (
            <EmptyState
              compact
              icon={<TagsIcon size={18} strokeWidth={1.75} />}
              title="No tags configured"
              description="Add tags in the Tags panel on the right, or load them from the simulator."
            />
          )
          : dataTableEl)}
        {mainTab === "diagnostics" && <DiagnosticsPanel deviceId={device.id} connected={s7.connected} onLog={onLog} deviceLabel={device.label} />}
        {mainTab === "blocks" && <BlocksPanel deviceId={device.id} connected={s7.connected} />}
      </div>
    </>
  );

  return (
    <div style={{ display: active ? "flex" : "none", flex: 1, overflow: "hidden", flexDirection: "column", minHeight: 0 }}>
      {isMobile ? (
        <>
          <div style={{ flex: 1, overflow: "auto", padding: 16, minHeight: 0 }}>
            {mobileTab === "connect" && sidebar}
            {mobileTab === "data" && dataTableEl}
            {mobileTab === "diagnostics" && <DiagnosticsPanel deviceId={device.id} connected={s7.connected} onLog={onLog} deviceLabel={device.label} />}
            {mobileTab === "blocks" && <BlocksPanel deviceId={device.id} connected={s7.connected} />}
            {mobileTab === "log" && <LogPanel entries={log} onExport={(f) => onLog("info", `Log exported to ${f}`)} />}
            {mobileTab === "trend" && <div style={{ display: "flex", flexDirection: "column", flex: 1, minHeight: 300 }}><TrendPanel series={Array.from(trendSeries.values())} onRemove={(n) => setTrendSeries((p) => { const m = new Map(p); m.delete(n); return m; })} windowSec={trendWindow} onWindowChange={setTrendWindow} theme="dark" /></div>}
          </div>
          <StatusBar connected={s7.connected} latency={s7.latency} pollCount={s7.pollCount} error={s7.error} />
        </>
      ) : (
        <>
          <div style={{ display: "flex", flex: 1, overflow: "hidden", minHeight: 0 }}>
            <aside className="vt-sidebar" style={{ width: 264, flexShrink: 0, borderRight: "1px solid var(--border)" }}>
              {sidebar}
            </aside>
            <main style={{ display: "flex", flex: 1, flexDirection: "column", overflow: "hidden", minWidth: 0 }}>
              <div style={{ flex: 1, minHeight: 160, overflow: "hidden", display: "flex", flexDirection: "column" }}>
                {main}
              </div>
              <div style={{ display: "flex", flexDirection: "column", flex: 1, minHeight: 140, borderTop: "1px solid var(--border)", overflow: "hidden" }}>
                <Tabs
                  label="Bottom views"
                  value={bottomTab}
                  onChange={(v) => { if (v === "trend" && !requirePro("Live trends")) return; setBottomTab(v); }}
                  items={[
                    { value: "log", label: "Comm log" },
                    { value: "trend", label: <>Trend <ProBadge /></>, count: trendSeries.size > 0 ? trendSeries.size : undefined },
                  ]}
                />
                <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column", overflow: "hidden" }}>
                  {bottomTab === "log"
                    ? <LogPanel entries={log} onExport={(f) => onLog("info", `Log exported to ${f}`)} />
                    : <TrendPanel series={Array.from(trendSeries.values())} onRemove={(n) => setTrendSeries((p) => { const m = new Map(p); m.delete(n); return m; })} windowSec={trendWindow} onWindowChange={setTrendWindow} theme="dark" />}
                </div>
              </div>
            </main>
          </div>
          <StatusBar connected={s7.connected} latency={s7.latency} pollCount={s7.pollCount} error={s7.error} />
        </>
      )}

      {writing && (() => {
        const doWrite = async (value: string | number | boolean, dataType: string) => {
          const ok = await s7.writeTag(writing.address, dataType, value);
          if (ok) onLog("tx", `[${device.label}] Write ${value} (${dataType}) to ${writing.address}`);
          else onLog("error", `[${device.label}] Write failed to ${writing.address}`);
          if (ok && s7.connected && tags.length > 0) void s7.readTags(toSpecs(tags));
          setWriting(null);
        };
        if (writing.dataType === "Bool") {
          return (
            <Dialog open onClose={() => setWriting(null)} title="Write BOOL" width={280}>
              <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
                <div className="mono" style={{ fontSize: "var(--fs-md)", color: "var(--text)" }}>{writing.address}</div>
                <div style={{ display: "flex", gap: 8 }}>
                  <Button variant="primary" style={{ flex: 1 }} onClick={() => doWrite(true, "Bool")}>Write TRUE</Button>
                  <Button style={{ flex: 1 }} onClick={() => doWrite(false, "Bool")}>Write FALSE</Button>
                </div>
              </div>
            </Dialog>
          );
        }
        return (
          <ValueEditor
            address={writing.address}
            currentValue={writing.value || undefined}
            currentDataType={writing.dataType}
            types={DATA_TYPES.filter((d) => d !== "Bool").map((d) => ({ value: d, label: d }))}
            onSubmit={(value, dataType) => doWrite(value, dataType)}
            onCancel={() => setWriting(null)}
          />
        );
      })()}
    </div>
  );
}
