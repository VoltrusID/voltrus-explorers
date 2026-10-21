import { useState } from "react";
import { Button, EmptyState, saveTextFile } from "@mactools/ui";
import ChartToolbar, { useClock, type WindowSec } from "./ChartToolbar";
import LineChart, { SERIES_COLORS, type ChartSeries } from "./LineChart";
import { toCsv } from "./decode";
import { rssiStore } from "./store";
import { RSSI_UNKNOWN, type BleDevice } from "./types";

interface Props {
  devices: BleDevice[];
  onInfo: (message: string) => void;
  onError: (message: string) => void;
}

const label = (d: BleDevice) => d.name || d.address || d.id.substring(0, 17);

export default function RssiPanel({ devices, onInfo, onError }: Props) {
  const [windowSec, setWindowSec] = useState<WindowSec>("60");
  const [paused, setPaused] = useState(false);
  const [picked, setPicked] = useState<string[]>(() => devices.filter((d) => d.connected).map((d) => d.id));
  const now = useClock(paused);

  const tMin = windowSec === "0" ? -Infinity : now - Number(windowSec) * 1000;
  const byId = new Map(devices.map((d) => [d.id, d]));

  const series: ChartSeries[] = picked.map((id, i) => {
    const samples = rssiStore.get(id);
    const points: { t: number; v: number }[] = [];
    for (let j = samples.length - 1; j >= 0; j--) {
      const s = samples[j];
      if (s.t > now) continue;
      if (s.t < tMin) break;
      points.push({ t: s.t, v: s.v });
    }
    points.reverse();
    const d = byId.get(id);
    return { id, name: d ? label(d) : id, color: SERIES_COLORS[i % SERIES_COLORS.length], points };
  });

  const toggle = (id: string) => setPicked((prev) => (prev.includes(id) ? prev.filter((x) => x !== id) : [...prev, id]));

  const exportCsv = async () => {
    const rows: (string | number)[][] = [["timestamp", "device", "address", "rssi_dbm"]];
    for (const id of picked) {
      const d = byId.get(id);
      for (const s of rssiStore.get(id)) {
        rows.push([new Date(s.t).toISOString(), d ? label(d) : id, d?.address ?? "", s.v]);
      }
    }
    if (rows.length === 1) {
      onError("No RSSI samples recorded for the selected devices");
      return;
    }
    try {
      const path = await saveTextFile(`ble-rssi-${new Date().toISOString().slice(0, 19).replace(/[:T]/g, "-")}.csv`, toCsv(rows));
      if (path) onInfo(`RSSI samples exported to ${path}`);
    } catch (e) {
      onError(`Export failed: ${String(e)}`);
    }
  };

  if (devices.length === 0) {
    return (
      <EmptyState
        compact
        title="No devices to chart"
        description="Start a scan. Signal strength (RSSI) is sampled twice a second for every device that is advertising."
      />
    );
  }

  const strongest = [...devices]
    .filter((d) => d.rssi !== RSSI_UNKNOWN)
    .sort((a, b) => b.rssi - a.rssi)
    .slice(0, 4)
    .map((d) => d.id);

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
      <ChartToolbar
        windowSec={windowSec}
        onWindowChange={setWindowSec}
        paused={paused}
        onPausedChange={setPaused}
        onClear={() => rssiStore.clear()}
        onExport={exportCsv}
      />
      <div className="vt-panel" style={{ padding: 8 }}>
        <LineChart series={series} now={now} windowSec={Number(windowSec)} unit="dBm" label="Signal strength over time" />
      </div>
      <div style={{ display: "flex", gap: 6, alignItems: "center", flexWrap: "wrap" }}>
        <span style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)" }}>Plot:</span>
        <Button size="sm" onClick={() => setPicked(strongest)}>
          Strongest 4
        </Button>
        <Button size="sm" onClick={() => setPicked(devices.filter((d) => d.connected).map((d) => d.id))}>
          Connected
        </Button>
        <Button size="sm" variant="ghost" onClick={() => setPicked([])}>
          None
        </Button>
      </div>
      <div className="vt-panel" style={{ maxHeight: 260, overflowY: "auto" }}>
        {devices.map((d) => {
          const idx = picked.indexOf(d.id);
          return (
            <label key={d.id} className="vt-check ble-rssi-row">
              <input type="checkbox" checked={idx >= 0} onChange={() => toggle(d.id)} />
              <span
                aria-hidden="true"
                style={{ width: 10, height: 10, borderRadius: 2, background: idx >= 0 ? SERIES_COLORS[idx % SERIES_COLORS.length] : "var(--sunken)" }}
              />
              <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", color: "var(--text)" }}>
                {label(d)}
              </span>
              <span className="mono" style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)" }}>
                {d.rssi === RSSI_UNKNOWN ? "n/a" : `${d.rssi} dBm`}
              </span>
            </label>
          );
        })}
      </div>
    </div>
  );
}
