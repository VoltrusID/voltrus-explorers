import { useEffect, useMemo, useRef, useState } from "react";
import {
  createChart,
  LineSeries,
  LineType,
  CrosshairMode,
  type IChartApi,
  type ISeriesApi,
  type LineData,
  type Time,
} from "lightweight-charts";
import { Activity } from "lucide-react";
import { Button, EmptyState, Segmented } from "@mactools/ui";
import { seriesLabel, type SeriesStore } from "./charts";

const COLORS = ["#3b82f6", "#ef4444", "#22c55e", "#f59e0b", "#a855f7", "#ec4899", "#14b8a6", "#f97316"];
export const MAX_CHARTED = COLORS.length;

function currentScheme(): "dark" | "light" {
  const attr = document.documentElement.getAttribute("data-theme");
  if (attr) return attr === "light" ? "light" : "dark";
  return window.matchMedia?.("(prefers-color-scheme: light)").matches ? "light" : "dark";
}

function chartColors(scheme: "dark" | "light") {
  const dark = scheme === "dark";
  return {
    text: dark ? "#6b7280" : "#9ca3af",
    grid: dark ? "rgba(255,255,255,0.05)" : "rgba(0,0,0,0.06)",
    crosshair: dark ? "rgba(255,255,255,0.08)" : "rgba(0,0,0,0.08)",
  };
}

function fmtValue(v: number): string {
  return Number.isInteger(v) ? String(v) : String(Number(v.toPrecision(6)));
}

interface Props {
  store: SeriesStore;
  /** Connection id whose series are listed. */
  conn: string;
  /** Keys of the series drawn on the chart. */
  selected: string[];
  onSelectedChange: (keys: string[]) => void;
}

export function ChartPanel({ store, conn, selected, onSelectedChange }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const chartRef = useRef<IChartApi | null>(null);
  const lineRefs = useRef<Map<string, ISeriesApi<"Line">>>(new Map());
  const [windowSec, setWindowSec] = useState(300);
  const [filter, setFilter] = useState("");
  const [tick, setTick] = useState(0);
  const [scheme, setScheme] = useState(currentScheme);

  // The store is mutable and fed outside React; redraw at most twice a second when it changed.
  useEffect(() => {
    let seen = -1;
    const id = window.setInterval(() => {
      if (store.version !== seen) {
        seen = store.version;
        setTick((t) => t + 1);
      }
    }, 500);
    return () => window.clearInterval(id);
  }, [store]);

  useEffect(() => {
    const mq = window.matchMedia("(prefers-color-scheme: light)");
    const onChange = () => setScheme(currentScheme());
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, []);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;
    const c = chartColors(scheme);
    const chart = createChart(container, {
      layout: { background: { color: "transparent" }, textColor: c.text, attributionLogo: false },
      grid: { vertLines: { color: c.grid }, horzLines: { color: c.grid } },
      crosshair: {
        mode: CrosshairMode.Normal,
        vertLine: { color: c.crosshair, width: 1, style: 2 },
        horzLine: { color: c.crosshair, width: 1, style: 2 },
      },
      rightPriceScale: { borderColor: c.grid, scaleMargins: { top: 0.1, bottom: 0.1 } },
      timeScale: { borderColor: c.grid, timeVisible: true, secondsVisible: true, rightOffset: 4 },
      autoSize: true,
    });
    chartRef.current = chart;
    const lines = lineRefs.current;
    return () => {
      chart.remove();
      chartRef.current = null;
      lines.clear();
    };
  }, [scheme, selected.length === 0]);

  const all = useMemo(() => store.list(conn), [store, conn, tick]);

  // Redraw selected series.
  useEffect(() => {
    const chart = chartRef.current;
    if (!chart) return;
    const lines = lineRefs.current;
    for (const [key, line] of lines) {
      if (!selected.includes(key)) {
        chart.removeSeries(line);
        lines.delete(key);
      }
    }
    const cutoff = Date.now() - windowSec * 1000;
    selected.forEach((key, i) => {
      const s = store.get(key);
      if (!s) return;
      let line = lines.get(key);
      if (!line) {
        line = chart.addSeries(LineSeries, {
          color: COLORS[i % COLORS.length],
          lineWidth: 2,
          lineType: LineType.WithSteps,
          lastValueVisible: false,
          priceLineVisible: false,
          title: seriesLabel(s),
        });
        lines.set(key, line);
      } else {
        line.applyOptions({ color: COLORS[i % COLORS.length] });
      }
      // lightweight-charts wants strictly increasing whole seconds; keep the last value in each second.
      const bySecond = new Map<number, number>();
      for (const p of s.points) if (p.t >= cutoff) bySecond.set(Math.floor(p.t / 1000), p.v);
      const data: LineData<Time>[] = Array.from(bySecond, ([t, v]) => ({ time: t as Time, value: v }));
      data.sort((a, b) => (a.time as number) - (b.time as number));
      line.setData(data);
    });
    chart.timeScale().scrollToRealTime();
  }, [store, selected, windowSec, tick, scheme]);

  const visible = all.filter((s) => seriesLabel(s).toLowerCase().includes(filter.toLowerCase()));

  const toggle = (key: string) => {
    if (selected.includes(key)) onSelectedChange(selected.filter((k) => k !== key));
    else if (selected.length < MAX_CHARTED) onSelectedChange([...selected, key]);
  };

  if (all.length === 0) {
    return (
      <div style={{ flex: 1, display: "flex", alignItems: "center", justifyContent: "center" }}>
        <EmptyState
          compact
          icon={<Activity size={18} strokeWidth={1.75} />}
          title="No numeric values yet"
          description="Numbers appear here when a payload is a plain number, JSON with numeric fields, or a decoded Sparkplug B or protobuf message. Values received before Pro was active are not charted."
        />
      </div>
    );
  }

  return (
    <div style={{ display: "flex", flex: 1, minHeight: 0, gap: 8 }}>
      <div style={{ width: 230, flexShrink: 0, display: "flex", flexDirection: "column", gap: 6, minHeight: 0 }}>
        <input
          className="vt-input vt-input--sm"
          placeholder="Filter series"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          aria-label="Filter series"
        />
        <div
          style={{
            flex: 1,
            overflow: "auto",
            border: "1px solid var(--border)",
            borderRadius: "var(--radius-md)",
            background: "var(--card)",
          }}
        >
          {visible.map((s) => {
            const idx = selected.indexOf(s.key);
            const on = idx >= 0;
            return (
              <button
                key={s.key}
                type="button"
                onClick={() => toggle(s.key)}
                aria-pressed={on}
                title={seriesLabel(s)}
                disabled={!on && selected.length >= MAX_CHARTED}
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: 6,
                  width: "100%",
                  padding: "3px 6px",
                  border: "none",
                  background: on ? "var(--border)" : "transparent",
                  color: "var(--text)",
                  fontSize: "var(--fs-xs)",
                  fontFamily: "var(--font-mono)",
                  textAlign: "left",
                  cursor: "pointer",
                }}
              >
                <span
                  aria-hidden="true"
                  style={{
                    width: 8,
                    height: 8,
                    borderRadius: 2,
                    flexShrink: 0,
                    background: on ? COLORS[idx % COLORS.length] : "transparent",
                    border: "1px solid var(--border)",
                  }}
                />
                <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                  {seriesLabel(s)}
                </span>
                <span style={{ color: "var(--text-muted)", flexShrink: 0 }}>{fmtValue(s.last)}</span>
              </button>
            );
          })}
        </div>
        <span style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)" }}>
          {selected.length} of {MAX_CHARTED} on the chart
        </span>
      </div>
      <div style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column", minHeight: 0 }}>
        <div style={{ display: "flex", justifyContent: "flex-end", gap: 6, paddingBottom: 4 }}>
          <Button
            size="sm"
            variant="ghost"
            onClick={() => {
              store.clear(conn);
              onSelectedChange([]);
            }}
          >
            Clear series
          </Button>
          <Segmented
            size="sm"
            label="Chart window"
            value={String(windowSec)}
            onChange={(v) => setWindowSec(Number(v))}
            options={[60, 300, 900].map((sec) => ({ value: String(sec), label: `${sec / 60}m` }))}
          />
        </div>
        {selected.length === 0 ? (
          <div style={{ flex: 1, display: "flex", alignItems: "center", justifyContent: "center" }}>
            <span style={{ fontSize: "var(--fs-sm)", color: "var(--text-muted)" }}>
              Pick one or more series on the left.
            </span>
          </div>
        ) : (
          <div
            ref={containerRef}
            style={{ flex: 1, minHeight: 0 }}
            role="img"
            aria-label="Chart of the selected topic values"
          />
        )}
      </div>
    </div>
  );
}
