import { useCallback, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Badge, Button, Callout, DataTable, EmptyState, IconButton, ProBadge, Tabs, saveTextFile } from "@mactools/ui";
import type { Tone } from "@mactools/ui";
import { Copy, Download, FileCode, Plus, RefreshCw, X } from "lucide-react";
import { formatValue, numericValue, sortConditions, toCsv, type Item } from "./mtc";
import { FREE_LIVE_ITEMS } from "./storage";
import type { AssetList, Component, Device, Devices, Observation } from "./types";

type Sub = "data" | "conditions" | "devices" | "assets";

interface Props {
  devices: Devices;
  items: Item[];
  obs: Map<string, Observation>;
  /** Items that update live; `null` when everything does (Pro). */
  liveIds: Set<string> | null;
  trending: Set<string>;
  onToggleTrend: (id: string) => void;
  onRefresh: () => void;
  requirePro: (feature: string) => boolean;
  addLog: (direction: "tx" | "rx" | "info" | "error", message: string) => void;
}

const levelTone: Record<string, Tone> = { Normal: "success", Warning: "warn", Fault: "danger", Unavailable: "neutral" };
const mono = (v: unknown) => <span className="mono">{String(v ?? "")}</span>;

export default function MtcView({ devices, items, obs, liveIds, trending, onToggleTrend, onRefresh, requirePro, addLog }: Props) {
  const [sub, setSub] = useState<Sub>("data");
  const conditions = useMemo(
    () => sortConditions([...obs.values()].filter((o) => o.category === "CONDITION")),
    [obs],
  );
  const faults = conditions.filter((o) => o.condition?.level === "Fault").length;
  const warnings = conditions.filter((o) => o.condition?.level === "Warning").length;

  return (
    <div style={{ display: "flex", flexDirection: "column", flex: 1, minHeight: 0, gap: 8 }}>
      <Tabs
        label="MTConnect views"
        value={sub}
        onChange={setSub}
        items={[
          { value: "data", label: "Data" },
          { value: "conditions", label: "Conditions", count: faults + warnings > 0 ? faults + warnings : undefined },
          { value: "devices", label: "Devices" },
          { value: "assets", label: "Assets" },
        ]}
      />
      {sub === "data" && (
        <DataTab items={items} obs={obs} liveIds={liveIds} trending={trending}
          onToggleTrend={onToggleTrend} onRefresh={onRefresh} requirePro={requirePro} addLog={addLog} />
      )}
      {sub === "conditions" && <ConditionsTab items={items} rows={conditions} />}
      {sub === "devices" && <DevicesTab devices={devices} />}
      {sub === "assets" && <AssetsTab addLog={addLog} />}
    </div>
  );
}

function DataTab({ items, obs, liveIds, trending, onToggleTrend, onRefresh, requirePro, addLog }: Omit<Props, "devices">) {
  const rows = useMemo(
    () =>
      items
        .filter((i) => i.item.category !== "CONDITION")
        .map((i) => {
          const o = obs.get(i.id);
          return {
            id: i.id,
            name: i.name,
            type: i.item.subType ? `${i.item.type}.${i.item.subType}` : i.item.type,
            category: i.item.category,
            value: o ? formatValue(o.value) : "\u2014",
            units: i.item.units ?? "",
            timestamp: o?.timestamp ?? "",
            live: liveIds == null || liveIds.has(i.id),
            numeric: numericValue(o) != null,
          };
        }),
    [items, obs, liveIds],
  );

  const exportCsv = useCallback(async () => {
    if (!requirePro("CSV export")) return;
    try {
      const path = await saveTextFile(
        "mtconnect-data.csv",
        toCsv(["id", "name", "type", "category", "value", "units", "timestamp"], rows.map((r) => [r.id, r.name, r.type, r.category, r.value, r.units, r.timestamp])),
      );
      if (path) addLog("info", `Exported ${rows.length} rows to ${path}`);
    } catch (e) {
      addLog("error", `CSV export failed: ${String(e)}`);
    }
  }, [rows, requirePro, addLog]);

  const limited = liveIds != null && rows.length > FREE_LIVE_ITEMS;
  return (
    <>
      <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
        {limited && (
          <span style={{ flex: 1, fontSize: "var(--fs-xs)", color: "var(--text-muted)" }}>
            Live updates for {FREE_LIVE_ITEMS} of {rows.length} items. Refresh reads the rest once. <ProBadge /> streams them all.
          </span>
        )}
        <span style={{ flex: limited ? 0 : 1 }} />
        <Button size="sm" icon={<RefreshCw size={14} strokeWidth={1.75} />} onClick={onRefresh}>Refresh</Button>
        <Button size="sm" icon={<Download size={14} strokeWidth={1.75} />} onClick={exportCsv}>Export CSV <ProBadge /></Button>
      </div>
      <DataTable
        columns={[
          { key: "name", label: "Name", width: "170px" },
          { key: "type", label: "Type", width: "170px" },
          { key: "category", label: "Category", width: "86px" },
          { key: "value", label: "Value", align: "right" },
          { key: "units", label: "Units", width: "90px" },
          { key: "timestamp", label: "Timestamp", width: "210px" },
          { key: "trend", label: "Trend", width: "56px", align: "center" },
        ]}
        data={rows}
        emptyMessage="Waiting for data"
        getRowStyle={(r) => (r.live ? undefined : { opacity: 0.6 })}
        renderCell={(key, row) => {
          if (key === "trend") {
            if (!row.numeric || row.category !== "SAMPLE") return null;
            const id = String(row.id);
            const active = trending.has(id);
            return (
              <IconButton
                size="sm"
                label={active ? `Remove ${row.name} from trend` : `Add ${row.name} to trend`}
                pressed={active}
                onClick={(e) => { e.stopPropagation(); if (!active && !requirePro("Live trends")) return; onToggleTrend(id); }}
              >
                {active ? <X size={14} strokeWidth={1.75} /> : <Plus size={14} strokeWidth={1.75} />}
              </IconButton>
            );
          }
          if (key === "value" || key === "timestamp") return mono(row[key]);
          return String(row[key] ?? "");
        }}
      />

    </>
  );
}

function ConditionsTab({ items, rows }: { items: Item[]; rows: Observation[] }) {
  const names = useMemo(() => new Map(items.map((i) => [i.id, i.name])), [items]);
  const data = rows.map((o) => ({
    name: names.get(o.dataItemId) ?? o.dataItemId,
    level: o.condition?.level ?? "Unavailable",
    type: o.dataItemType ?? "",
    code: o.condition?.nativeCode ?? "",
    severity: o.condition?.nativeSeverity ?? "",
    message: o.condition?.message ?? "",
    timestamp: o.timestamp,
  }));
  return (
    <DataTable
      columns={[
        { key: "name", label: "Condition", width: "170px" },
        { key: "level", label: "State", width: "110px" },
        { key: "type", label: "Type", width: "150px" },
        { key: "code", label: "Code", width: "90px" },
        { key: "severity", label: "Severity", width: "80px" },
        { key: "message", label: "Message" },
        { key: "timestamp", label: "Timestamp", width: "210px" },
      ]}
      data={data}
      emptyMessage="No conditions reported"
      renderCell={(key, row) =>
        key === "level" ? <Badge tone={levelTone[String(row.level)]}>{String(row.level)}</Badge> : key === "timestamp" ? mono(row[key]) : String(row[key] ?? "")
      }
    />
  );
}

function ComponentNode({ c, depth }: { c: Component; depth: number }) {
  return (
    <details open={depth < 2} style={{ marginLeft: depth ? 14 : 0 }}>
      <summary style={{ cursor: "pointer", fontSize: "var(--fs-sm)", padding: "2px 0" }}>
        <span style={{ fontWeight: 600, color: "var(--text)" }}>{c.name ?? c.id}</span>{" "}
        <span style={{ color: "var(--text-muted)" }}>{c.element}</span>
      </summary>
      {c.dataItems.map((d) => (
        <div key={d.id} className="mono" style={{ marginLeft: 18, fontSize: "var(--fs-xs)", color: "var(--text-dim)", lineHeight: 1.7 }}>
          {d.category} {d.type}{d.subType ? `.${d.subType}` : ""} <span style={{ color: "var(--text)" }}>{d.name ?? d.id}</span>
          <span style={{ color: "var(--text-muted)" }}> ({d.id}){d.units ? ` ${d.units}` : ""}</span>
        </div>
      ))}
      {c.components.map((child) => <ComponentNode key={child.id} c={child} depth={depth + 1} />)}
    </details>
  );
}

function DeviceNode({ d }: { d: Device }) {
  const desc = d.description;
  return (
    <details open style={{ marginBottom: 8 }}>
      <summary style={{ cursor: "pointer", fontSize: "var(--fs-sm)", fontWeight: 600, color: "var(--text)" }}>
        {d.name} {d.isAgent && <Badge>Agent</Badge>}
        {desc && (desc.manufacturer || desc.model) && (
          <span style={{ fontWeight: 400, color: "var(--text-muted)" }}> {[desc.manufacturer, desc.model].filter(Boolean).join(" ")}</span>
        )}
      </summary>
      <div style={{ marginLeft: 14 }}>
        {d.dataItems.map((i) => (
          <div key={i.id} className="mono" style={{ marginLeft: 18, fontSize: "var(--fs-xs)", color: "var(--text-dim)", lineHeight: 1.7 }}>
            {i.category} {i.type} <span style={{ color: "var(--text)" }}>{i.name ?? i.id}</span>
          </div>
        ))}
        {d.components.map((c) => <ComponentNode key={c.id} c={c} depth={0} />)}
      </div>
    </details>
  );
}

function DevicesTab({ devices }: { devices: Devices }) {
  return (
    <div style={{ flex: 1, overflow: "auto", background: "var(--card)", border: "1px solid var(--border)", borderRadius: "var(--radius-lg)", padding: 12 }}>
      <div style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)", marginBottom: 8 }}>
        Agent schema {devices.header.version ?? "?"}, instance {devices.header.instanceId}, buffer {devices.header.bufferSize ?? "?"}
      </div>
      {devices.devices.map((d) => <DeviceNode key={d.id} d={d} />)}
    </div>
  );
}

function AssetsTab({ addLog }: { addLog: Props["addLog"] }) {
  const [list, setList] = useState<AssetList | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      const a = await invoke<AssetList>("mtc_assets");
      setList(a);
      setSelected((s) => (s && a.assets.some((x) => x.assetId === s) ? s : a.assets[0]?.assetId ?? null));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }, []);

  const asset = list?.assets.find((a) => a.assetId === selected);
  return (
    <div style={{ display: "flex", flexDirection: "column", flex: 1, minHeight: 0, gap: 8 }}>
      <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
        <Button size="sm" icon={<RefreshCw size={14} strokeWidth={1.75} />} disabled={busy} onClick={load}>{list ? "Reload assets" : "Load assets"}</Button>
        {list && <span style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)" }}>{list.assets.length} asset(s)</span>}
      </div>
      {error && <Callout tone="danger">{error}</Callout>}
      {!list ? (
        <EmptyState compact icon={<FileCode size={20} strokeWidth={1.75} />} title="Assets not loaded" description="Cutting tools, files and other assets the agent holds. Select one to see its XML." />
      ) : (
        <div style={{ display: "flex", flex: 1, minHeight: 0, gap: 8 }}>
          <div style={{ width: 420, flexShrink: 0, display: "flex", flexDirection: "column", minHeight: 0 }}>
            <DataTable
              columns={[
                { key: "assetId", label: "Asset id" },
                { key: "assetType", label: "Type", width: "120px" },
                { key: "timestamp", label: "Timestamp", width: "170px" },
              ]}
              data={list.assets.map((a) => ({ assetId: a.assetId, assetType: a.assetType + (a.removed ? " (removed)" : ""), timestamp: a.timestamp ?? "" }))}
              emptyMessage="This agent holds no assets"
              onRowClick={(r) => setSelected(String(r.assetId))}
            />
          </div>
          <div style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column", background: "var(--sunken)", border: "1px solid var(--border)", borderRadius: "var(--radius-lg)" }}>
            {asset ? (
              <>
                <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", padding: "6px 10px", borderBottom: "1px solid var(--border)", fontSize: "var(--fs-xs)", color: "var(--text-muted)" }}>
                  <span>{asset.assetType} {asset.assetId}</span>
                  <IconButton size="sm" label="Copy XML" onClick={() => { void navigator.clipboard.writeText(asset.rawXml); addLog("info", `Copied XML of asset ${asset.assetId}`); }}>
                    <Copy size={14} strokeWidth={1.75} />
                  </IconButton>
                </div>
                <pre className="mono" style={{ margin: 0, padding: 10, overflow: "auto", flex: 1, fontSize: "var(--fs-xs)", lineHeight: 1.5, color: "var(--text)" }}>{asset.rawXml}</pre>
              </>
            ) : (
              <EmptyState compact title="No asset selected" />
            )}
          </div>
        </div>
      )}
    </div>
  );
}
