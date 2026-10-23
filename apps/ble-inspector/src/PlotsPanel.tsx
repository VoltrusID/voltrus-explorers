import { useState } from "react";
import { X } from "lucide-react";
import { Badge, EmptyState, IconButton, SheetSelect, saveTextFile } from "@mactools/ui";
import ChartToolbar, { NumInput, useClock, type WindowSec } from "./ChartToolbar";
import LineChart, { SERIES_COLORS, type ChartSeries } from "./LineChart";
import { NUMERIC_FORMATS, NUMERIC_FORMAT_OPTIONS, describeValue, hexToBytes, plotValue, toCsv } from "./decode";
import { defaultDecode, gattName, shortUuid } from "./gatt";
import { notifyStore, type DecodePref } from "./store";

interface Props {
  /** `deviceId|charUuid` keys with a live subscription. */
  subscribed: ReadonlySet<string>;
  deviceName: (deviceId: string) => string;
  formats: Record<string, DecodePref>;
  onFormat: (charUuid: string, pref: DecodePref) => void;
  onInfo: (message: string) => void;
  onError: (message: string) => void;
}

/** Settings used to plot a characteristic: the saved ones, a known default, or plain uint8. */
function plotPref(charUuid: string, formats: Record<string, DecodePref>): DecodePref {
  const saved = formats[charUuid];
  if (saved && NUMERIC_FORMATS.some((f) => f.id === saved.format)) return saved;
  const known = defaultDecode(charUuid);
  const numeric = known && NUMERIC_FORMATS.some((f) => f.id === known.format);
  return { format: numeric ? known.format : "uint8", offset: saved?.offset ?? known?.offset ?? 0, scale: saved?.scale ?? known?.scale ?? 1 };
}

export default function PlotsPanel({ subscribed, deviceName, formats, onFormat, onInfo, onError }: Props) {
  const [windowSec, setWindowSec] = useState<WindowSec>("60");
  const [paused, setPaused] = useState(false);
  const [hidden, setHidden] = useState<Set<string>>(new Set());
  const now = useClock(paused);

  const keys = [...new Set([...notifyStore.keys(), ...subscribed])].sort();
  const tMin = windowSec === "0" ? -Infinity : now - Number(windowSec) * 1000;

  const series: ChartSeries[] = keys.map((key, i) => {
    const charUuid = key.slice(key.indexOf("|") + 1);
    const pref = plotPref(charUuid, formats);
    const points: { t: number; v: number }[] = [];
    const samples = notifyStore.get(key);
    for (let j = samples.length - 1; j >= 0; j--) {
      const s = samples[j];
      if (s.t > now) continue;
      if (s.t < tMin) break;
      const bytes = hexToBytes(s.v);
      const v = bytes ? plotValue(bytes, pref) : null;
      if (v !== null) points.push({ t: s.t, v });
    }
    points.reverse();
    return {
      id: key,
      name: gattName(charUuid) ?? shortUuid(charUuid),
      color: SERIES_COLORS[i % SERIES_COLORS.length],
      points,
    };
  });
  const visible = series.filter((s) => !hidden.has(s.id));

  const toggle = (id: string) =>
    setHidden((prev) => {
      const next = new Set(prev);
      if (!next.delete(id)) next.add(id);
      return next;
    });

  const clearAll = () => {
    notifyStore.clear();
    setHidden(new Set());
  };

  const exportCsv = async () => {
    const rows: (string | number)[][] = [["timestamp", "device", "characteristic", "raw_hex", "value"]];
    for (const key of keys) {
      const sep = key.indexOf("|");
      const deviceId = key.slice(0, sep);
      const charUuid = key.slice(sep + 1);
      const pref = plotPref(charUuid, formats);
      for (const s of notifyStore.get(key)) {
        const bytes = hexToBytes(s.v);
        const v = bytes ? plotValue(bytes, pref) : null;
        rows.push([new Date(s.t).toISOString(), deviceName(deviceId), charUuid, s.v, v ?? ""]);
      }
    }
    if (rows.length === 1) {
      onError("No notifications recorded yet");
      return;
    }
    try {
      const path = await saveTextFile(`ble-notify-${new Date().toISOString().slice(0, 19).replace(/[:T]/g, "-")}.csv`, toCsv(rows));
      if (path) onInfo(`Notify samples exported to ${path}`);
    } catch (e) {
      onError(`Export failed: ${String(e)}`);
    }
  };

  if (keys.length === 0) {
    return (
      <EmptyState
        compact
        title="No notify streams yet"
        description="Connect to a device, press Notify on a characteristic, and its numeric values are plotted here as they arrive."
      />
    );
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
      <ChartToolbar
        windowSec={windowSec}
        onWindowChange={setWindowSec}
        paused={paused}
        onPausedChange={setPaused}
        onClear={clearAll}
        onExport={exportCsv}
      />
      <div className="vt-panel" style={{ padding: 8 }}>
        <LineChart series={visible} now={now} windowSec={Number(windowSec)} label="Notification values over time" />
      </div>
      <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
        {series.map((s) => {
          const sep = s.id.indexOf("|");
          const deviceId = s.id.slice(0, sep);
          const charUuid = s.id.slice(sep + 1);
          const pref = plotPref(charUuid, formats);
          const history = notifyStore.get(s.id);
          const last = history[history.length - 1];
          const lastBytes = last ? hexToBytes(last.v) : null;
          return (
            <div key={s.id} className="vt-panel ble-stream">
              <label className="vt-check" title={hidden.has(s.id) ? "Show in chart" : "Hide from chart"}>
                <input type="checkbox" checked={!hidden.has(s.id)} onChange={() => toggle(s.id)} aria-label={`Plot ${s.name}`} />
                <span aria-hidden="true" style={{ width: 10, height: 10, borderRadius: 2, background: s.color }} />
              </label>
              <div style={{ flex: 1, minWidth: 120 }}>
                <div style={{ fontSize: "var(--fs-sm)", fontWeight: 500, display: "flex", gap: 6, alignItems: "center" }}>
                  {s.name}
                  {subscribed.has(s.id) && <Badge tone="success">Live</Badge>}
                </div>
                <div className="mono" style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)" }}>
                  {deviceName(deviceId)} · {notifyStore.get(s.id).length} samples
                </div>
              </div>
              <SheetSelect
                size="sm"
                style={{ width: 120 }}
                aria-label={`Number format for ${s.name}`}
                sheetTitle="Number format"
                value={pref.format}
                options={NUMERIC_FORMAT_OPTIONS}
                onChange={(v) => onFormat(charUuid, { ...pref, format: v })}
              />
              <span style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)" }}>at byte</span>
              <NumInput
                label={`Byte offset for ${s.name}`}
                value={pref.offset}
                min={0}
                integer
                width={48}
                onCommit={(offset) => onFormat(charUuid, { ...pref, offset })}
              />
              <span style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)" }}>× scale</span>
              <NumInput
                label={`Scale for ${s.name}`}
                value={pref.scale}
                width={64}
                onCommit={(scale) => onFormat(charUuid, { ...pref, scale })}
              />
              <span
                className="mono"
                style={{ minWidth: 90, textAlign: "right", fontSize: "var(--fs-sm)", color: "var(--text)" }}
                title="Latest value"
              >
                {lastBytes ? describeValue(lastBytes, pref) : "no data"}
              </span>
              <IconButton size="sm" label={`Remove ${s.name} history`} onClick={() => notifyStore.clear(s.id)}>
                <X size={13} strokeWidth={1.75} />
              </IconButton>
            </div>
          );
        })}
      </div>
    </div>
  );
}
