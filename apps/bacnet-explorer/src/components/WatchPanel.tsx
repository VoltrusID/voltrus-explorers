import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { RefreshCw, Star, Trash2, X } from "lucide-react";
import { Button, EmptyState, IconButton, ProBadge, Segmented, useProStatus } from "@mactools/ui";
import type { BacnetObjectDetail } from "../types";
import { formatObjectType } from "../types";

export interface WatchItem {
  key: string;
  type: string;
  instance: number;
  name: string;
}

interface WatchReading {
  name: string | null;
  value: string;
  status: string;
  error: string | null;
}

const INTERVALS = [
  { value: "1000", label: "1 s" },
  { value: "2000", label: "2 s" },
  { value: "5000", label: "5 s" },
  { value: "10000", label: "10 s" },
  { value: "30000", label: "30 s" },
] as const;

async function readItem(item: WatchItem): Promise<WatchReading> {
  try {
    const detail = await invoke<BacnetObjectDetail | null>("read_object_properties", {
      objectType: item.type,
      objectInstance: item.instance,
    });
    const pv = detail?.present_value ?? detail?.properties.find((p) => p.property_identifier === 85)?.raw_value ?? null;
    const name = detail?.object_name ?? null;
    if (pv == null) return { name, value: "—", status: detail?.status_flags ?? "", error: "The device returned no presentValue" };
    const unit = detail?.units_symbol ?? "";
    return { name, value: unit ? `${pv} ${unit}` : pv, status: detail?.status_flags ?? "", error: null };
  } catch (e) {
    return { name: null, value: "—", status: "", error: String(e) };
  }
}

interface WatchPanelProps {
  items: WatchItem[];
  onRemove: (key: string) => void;
  onClear: () => void;
  /** Phone width: drop the status column. */
  compact?: boolean;
}

/**
 * Objects picked in the object browser, read on demand. Auto-refresh is Pro. Reads go to the
 * device that is active in the backend, so the parent mounts one panel per active device.
 */
export function WatchPanel({ items, onRemove, onClear, compact = false }: WatchPanelProps) {
  const { requirePro } = useProStatus();
  const [readings, setReadings] = useState<Record<string, WatchReading>>({});
  const [refreshing, setRefreshing] = useState(false);
  const [updatedAt, setUpdatedAt] = useState<number | null>(null);
  const [auto, setAuto] = useState(false);
  const [intervalMs, setIntervalMs] = useState("2000");
  const itemsRef = useRef(items);
  itemsRef.current = items;
  const inFlight = useRef(false);
  const mounted = useRef(true);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const refresh = useCallback(async () => {
    if (inFlight.current) return;
    const batch = itemsRef.current;
    if (batch.length === 0) return;
    inFlight.current = true;
    setRefreshing(true);
    try {
      const results = await Promise.all(batch.map(readItem));
      if (!mounted.current) return;
      setReadings((prev) => {
        const next = { ...prev };
        batch.forEach((item, i) => {
          next[item.key] = results[i];
        });
        return next;
      });
      setUpdatedAt(Date.now());
    } finally {
      inFlight.current = false;
      if (mounted.current) setRefreshing(false);
    }
  }, []);

  // Read once on mount and whenever the list changes.
  const listSignature = items.map((i) => i.key).join("|");
  useEffect(() => {
    void refresh();
  }, [listSignature, refresh]);

  useEffect(() => {
    if (!auto) return;
    const id = setInterval(() => void refresh(), Number(intervalMs));
    return () => clearInterval(id);
  }, [auto, intervalMs, refresh]);

  const toggleAuto = () => {
    if (auto) {
      setAuto(false);
      return;
    }
    if (requirePro("Auto-refresh watch list")) setAuto(true);
  };

  if (items.length === 0) {
    return (
      <EmptyState
        icon={<Star size={20} strokeWidth={1.75} />}
        title="Watch list is empty"
        description="In the object browser, tap the star next to an object to add it here."
      />
    );
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", flex: 1, minHeight: 0, gap: 8 }}>
      <div style={{ display: "flex", alignItems: "center", gap: 8, flexWrap: "wrap" }}>
        <Button icon={<RefreshCw size={14} strokeWidth={1.75} />} loading={refreshing} onClick={() => void refresh()}>
          Refresh
        </Button>
        <Button variant={auto ? "primary" : "default"} onClick={toggleAuto} aria-pressed={auto}>
          Auto-refresh {!auto && <ProBadge />}
        </Button>
        {auto && (
          <Segmented
            label="Auto-refresh interval"
            size="sm"
            value={intervalMs}
            options={INTERVALS}
            onChange={setIntervalMs}
          />
        )}
        <span style={{ marginLeft: "auto", fontSize: "var(--fs-xs)", color: "var(--text-muted)", fontVariantNumeric: "tabular-nums" }}>
          {items.length} object{items.length === 1 ? "" : "s"}
          {updatedAt ? ` · ${new Date(updatedAt).toLocaleTimeString()}` : ""}
        </span>
        <Button variant="ghost" icon={<Trash2 size={14} strokeWidth={1.75} />} onClick={onClear}>
          Clear
        </Button>
      </div>

      <div style={{ flex: 1, minHeight: 0, overflow: "auto", background: "var(--card)", border: "1px solid var(--border)", borderRadius: "var(--radius-lg)" }}>
        <table className="vt-table">
          <thead>
            <tr>
              <th style={{ textAlign: "left" }}>Object</th>
              <th style={{ textAlign: "left" }}>Value</th>
              {!compact && <th style={{ textAlign: "left" }}>Status</th>}
              <th style={{ width: 44 }} aria-label="Remove" />
            </tr>
          </thead>
          <tbody>
            {items.map((item) => {
              const r = readings[item.key];
              return (
                <tr key={item.key}>
                  <td>
                    <div style={{ color: "var(--text)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{readings[item.key]?.name || item.name}</div>
                    <div style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)" }}>
                      {formatObjectType(item.type)} {item.instance}
                    </div>
                  </td>
                  <td className="is-mono" title={r?.error ?? undefined} style={{ color: r?.error ? "var(--danger)" : undefined }}>
                    {r ? r.value : "…"}
                  </td>
                  {!compact && <td style={{ color: "var(--text-dim)" }}>{r?.status ?? ""}</td>}
                  <td style={{ textAlign: "right" }}>
                    <IconButton size="sm" label="Remove from watch list" onClick={() => onRemove(item.key)}>
                      <X size={12} strokeWidth={1.75} />
                    </IconButton>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </div>
  );
}
