import { useEffect, useRef, useState } from "react";
import { Activity, RotateCcw, X } from "lucide-react";
import { Button, EmptyState, IconButton, Segmented } from "@mactools/ui";
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
  name: string;
  color: string;
  points: { t: number; v: number }[];
}

const COLORS = ["#3b82f6", "#ef4444", "#22c55e", "#f59e0b", "#a855f7", "#ec4899", "#14b8a6", "#f97316"];

interface Props {
  series: TrendSeries[];
  onRemove: (name: string) => void;
  windowSec: number;
  onWindowChange: (sec: number) => void;
  theme: string;
}

function getChartColors() {
  const isDark = document.documentElement.getAttribute("data-theme") !== "light";
  return {
    text: isDark ? "#6b7280" : "#9ca3af",
    grid: isDark ? "rgba(255,255,255,0.05)" : "rgba(0,0,0,0.06)",
    crosshair: isDark ? "rgba(255,255,255,0.08)" : "rgba(0,0,0,0.08)",
  };
}

export default function TrendPanel({ series, onRemove, windowSec, onWindowChange, theme }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const chartRef = useRef<IChartApi | null>(null);
  const seriesRefs = useRef<ISeriesApi<"Line">[]>([]);
  const [userScrolled, setUserScrolled] = useState(false);
  const lastBarCountRef = useRef<number>(0);
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
  }, [theme]);

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
      // lightweight-charts setData REQUIRES strictly ascending unique timestamps;
      // fractional/duplicate times throw "data must be asc ordered by time".
      const bySecond = new Map<number, number>();
      for (const p of s.points) {
        if (p.t < cutoff) continue;
        bySecond.set(Math.floor(p.t / 1000), p.v);
      }
      const unique: LineData<Time>[] = Array.from(bySecond, ([t, v]) => ({
        time: t as Time,
        value: v,
      }));
      // Guard ascending (points append oldest→newest, but be bulletproof).
      unique.sort((a, b) => (a.time as number) - (b.time as number));

      if (unique.length > 0) {
        lineSeries.setData(unique);
      }
      seriesRefs.current.push(lineSeries);
    }

    // Re-fit whenever a series is added/removed (count changed), even if the
    // user had scrolled — otherwise newly added data lands off-screen and the
    // chart looks blank until remount.
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
      <EmptyState
        icon={<Activity size={20} />}
        title="No trend series"
        description="Click the trend button on a tag row to chart its value here."
      />
    );
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", flex: 1, overflow: "hidden" }}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: 8, padding: "4px 8px", borderBottom: "1px solid var(--border)", flexShrink: 0 }}>
        <div style={{ display: "flex", gap: 6, flexWrap: "wrap", alignItems: "center" }}>
          {series.map((s, i) => (
            <span key={s.name} style={{ fontSize: 11, display: "flex", alignItems: "center", gap: 4, background: "var(--bg)", padding: "1px 4px 1px 6px", borderRadius: 999, border: "1px solid var(--border)" }}>
              <span style={{ width: 8, height: 8, borderRadius: 2, background: s.color || COLORS[i % COLORS.length], display: "inline-block" }} aria-hidden="true" />
              <span className="mono">{s.name}</span>
              <IconButton size="sm" label={`Remove ${s.name} from trend`} onClick={() => onRemove(s.name)}>
                <X size={10} />
              </IconButton>
            </span>
          ))}
        </div>
        <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
          {userScrolled && (
            <Button size="sm" variant="ghost" icon={<RotateCcw size={12} />} onClick={resetView} aria-label="Reset chart view to follow live data">
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
