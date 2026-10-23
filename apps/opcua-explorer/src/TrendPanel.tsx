import { useEffect, useRef, useState } from "react";
import { X } from "lucide-react";
import { Button, EmptyState, IconButton, ListGroup, Segmented, useIsPhone } from "@mactools/ui";
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

export interface TrendSeries {
  /** Unique per series; names can repeat across sessions. */
  id: string;
  name: string;
  color: string;
  points: { t: number; v: number }[];
}

const COLORS = ["#3b82f6", "#ef4444", "#22c55e", "#f59e0b", "#a855f7", "#ec4899", "#14b8a6", "#f97316"];

interface Props {
  series: TrendSeries[];
  onRemove: (id: string) => void;
  windowSec: number;
  onWindowChange: (sec: number) => void;
}

function getChartColors() {
  const isDark = document.documentElement.getAttribute("data-theme") !== "light";
  return {
    text: isDark ? "#6b7280" : "#9ca3af",
    grid: isDark ? "rgba(255,255,255,0.05)" : "rgba(0,0,0,0.06)",
    crosshair: isDark ? "rgba(255,255,255,0.08)" : "rgba(0,0,0,0.08)",
  };
}

export default function TrendPanel({ series, onRemove, windowSec, onWindowChange }: Props) {
  const phone = useIsPhone();
  const containerRef = useRef<HTMLDivElement>(null);
  const chartRef = useRef<IChartApi | null>(null);
  const seriesRefs = useRef<ISeriesApi<"Line">[]>([]);
  const [userScrolled, setUserScrolled] = useState(false);
  const prevCountRef = useRef<number>(0);
  const suppressNextRef = useRef(false);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const colors = getChartColors();
    const chart = createChart(container, {
      layout: {
        background: { color: "transparent" },
        textColor: colors.text,
        attributionLogo: false,
      },
      grid: {
        vertLines: { color: colors.grid },
        horzLines: { color: colors.grid },
      },
      crosshair: {
        mode: CrosshairMode.Normal,
        vertLine: { color: colors.crosshair, width: 1, style: 2 },
        horzLine: { color: colors.crosshair, width: 1, style: 2 },
      },
      rightPriceScale: {
        borderColor: colors.grid,
        scaleMargins: { top: 0.1, bottom: 0.1 },
      },
      timeScale: {
        borderColor: colors.grid,
        timeVisible: true,
        secondsVisible: true,
        rightOffset: 12,
        barSpacing: 6,
      },
      autoSize: true,
    });
    chartRef.current = chart;

    const onVisibleChange = () => {
      if (suppressNextRef.current) {
        suppressNextRef.current = false;
        return;
      }
      const range = chart.timeScale().getVisibleLogicalRange();
      if (!range) return;
      const visEnd = range.to;
      const rightOffset = 12;
      const isAtRightEdge = visEnd >= -rightOffset + 1;
      setUserScrolled(!isAtRightEdge);
    };

    chart.timeScale().subscribeVisibleLogicalRangeChange(onVisibleChange);

    return () => {
      chart.timeScale().unsubscribeVisibleLogicalRangeChange(onVisibleChange);
      chart.remove();
      chartRef.current = null;
      seriesRefs.current = [];
    };
  }, []);

  useEffect(() => {
    const chart = chartRef.current;
    if (!chart) return;

    for (const s of seriesRefs.current) chart.removeSeries(s);
    seriesRefs.current = [];

    const now = Date.now();
    const cutoff = now - windowSec * 1000;

    for (let i = 0; i < series.length; i++) {
      const s = series[i];
      const lineSeries = chart.addSeries(LineSeries, {
        color: s.color || COLORS[i % COLORS.length],
        lineWidth: 1.5 as never,
        lineType: LineType.Curved,
        lastValueVisible: false,
        priceLineVisible: false,
        title: s.name,
      });

      // Floor to integer seconds + dedup (keep LAST value per second).
      const bySecond = new Map<number, number>();
      for (const p of s.points) {
        if (p.t < cutoff) continue;
        bySecond.set(Math.floor(p.t / 1000), p.v);
      }
      const unique: LineData<Time>[] = Array.from(bySecond, ([t, v]) => ({
        time: t as Time,
        value: v,
      }));
      unique.sort((a, b) => (a.time as number) - (b.time as number));

      if (unique.length > 0) {
        lineSeries.setData(unique);
      }
      seriesRefs.current.push(lineSeries);
    }

    const countChanged = series.length !== prevCountRef.current;
    prevCountRef.current = series.length;
    if (series.length > 0 && (!userScrolled || countChanged)) {
      if (countChanged) setUserScrolled(false);
      suppressNextRef.current = true;
      chart.timeScale().fitContent();
    }
  }, [series, windowSec, userScrolled]);

  const resetView = () => {
    const chart = chartRef.current;
    if (!chart) return;
    setUserScrolled(false);
    suppressNextRef.current = true;
    chart.timeScale().fitContent();
  };

  if (series.length === 0) {
    return (
      <div role="status" style={{ flex: 1, display: "flex", alignItems: "center", justifyContent: "center" }}>
        <EmptyState
          compact
          title="No series in the trend"
          description={phone ? "Tap the chart icon on a value in Watch or Server to add it here." : "Click the chart icon on a table row to add that node to the trend."}
        />
      </div>
    );
  }

  if (phone) {
    return (
      <div className="oc-scroll" style={{ display: "flex", flexDirection: "column", gap: 12 }}>
        <Segmented
          block
          label="Trend window"
          value={String(windowSec)}
          onChange={(v) => onWindowChange(Number(v))}
          options={[
            { value: "30", label: "30 s" },
            { value: "60", label: "1 min" },
            { value: "300", label: "5 min" },
          ]}
        />
        <div className="oc-chart">
          <div ref={containerRef} style={{ flex: 1, minHeight: 0 }} role="img" aria-label="Trend chart showing live data" />
        </div>
        {userScrolled && (
          <Button variant="primary" block onClick={resetView}>
            Follow live data
          </Button>
        )}
        <ListGroup>
          {series.map((s, i) => (
            <div key={s.id} className="vm-row">
              <span className="oc-swatch" style={{ background: s.color || COLORS[i % COLORS.length] }} aria-hidden="true" />
              <span className="vm-row__text">
                <span className="vm-row__title">{s.name}</span>
              </span>
              <IconButton label={`Remove ${s.name} from trend`} onClick={() => onRemove(s.id)}>
                <X size={18} strokeWidth={1.75} />
              </IconButton>
            </div>
          ))}
        </ListGroup>
      </div>
    );
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", flex: 1, overflow: "hidden" }}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", padding: "4px 8px", borderBottom: "1px solid var(--border)", flexShrink: 0 }}>
        <div style={{ display: "flex", gap: 6, flexWrap: "wrap", alignItems: "center" }}>
          {series.map((s, i) => (
            <span key={s.id} style={{ fontSize: "var(--fs-xs)", display: "flex", alignItems: "center", gap: 4, background: "var(--bg)", padding: "2px 6px", borderRadius: "var(--radius-sm)", border: "1px solid var(--border)" }}>
              <span style={{ width: 8, height: 8, borderRadius: 2, background: s.color || COLORS[i % COLORS.length], display: "inline-block" }} aria-hidden="true" />
              {s.name}
              <button
                onClick={() => onRemove(s.id)}
                aria-label={`Remove ${s.name} from trend`}
                style={{ background: "none", border: "none", color: "var(--text-muted)", cursor: "pointer", padding: 0, display: "inline-flex", lineHeight: 1 }}
              >
                <X size={12} strokeWidth={1.75} />
              </button>
            </span>
          ))}
        </div>
        <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
          {userScrolled && (
            <Button size="sm" variant="primary" aria-label="Reset chart view to follow live data" onClick={resetView}>
              Reset
            </Button>
          )}
          <Segmented
            size="sm"
            label="Trend window"
            value={String(windowSec)}
            onChange={(v) => onWindowChange(Number(v))}
            options={[
              { value: "30", label: "30s" },
              { value: "60", label: "1m" },
              { value: "300", label: "5m" },
            ]}
          />
        </div>
      </div>
      <div ref={containerRef} style={{ flex: 1, minHeight: 0 }} role="img" aria-label="Trend chart showing live data" />
    </div>
  );
}
