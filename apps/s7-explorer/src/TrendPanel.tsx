import { useEffect, useRef, useState } from "react";
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
import { LineChart, RotateCcw, X } from "lucide-react";
import { Button, EmptyState, IconButton, Segmented, useIsPhone } from "@mactools/ui";

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
  const phone = useIsPhone();
  const containerRef = useRef<HTMLDivElement>(null);
  const chartRef = useRef<IChartApi | null>(null);
  const seriesRefs = useRef<ISeriesApi<"Line">[]>([]);
  const [userScrolled, setUserScrolled] = useState(false);
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

    if (series.length > 0 && !userScrolled) {
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
          icon={<LineChart size={18} strokeWidth={1.75} />}
          title="No trend series"
          description={phone ? "Tap the chart button on a row of the Watch tab to plot its value here." : "Click the add button on a tag row to plot it here."}
        />
      </div>
    );
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", flex: 1, overflow: "hidden" }}>
      {phone ? (
        <div style={{ display: "flex", flexDirection: "column", gap: 12, flexShrink: 0, paddingBottom: 12 }}>
          <Segmented
            block
            label="Trend window"
            value={String(windowSec)}
            onChange={(v) => onWindowChange(Number(v))}
            options={[{ value: "30", label: "30 s" }, { value: "60", label: "1 min" }, { value: "300", label: "5 min" }]}
          />
          <div style={{ display: "flex", gap: 8, alignItems: "center", overflowX: "auto" }}>
            {series.map((s, i) => (
              <span key={s.name} style={{ flexShrink: 0, display: "inline-flex", alignItems: "center", gap: 8, minHeight: 44, padding: "0 4px 0 14px", borderRadius: 999, background: "var(--card)", fontSize: "var(--fs-sm)", fontWeight: 600 }}>
                <span style={{ width: 10, height: 10, borderRadius: "50%", background: s.color || COLORS[i % COLORS.length] }} aria-hidden="true" />
                {s.name}
                <IconButton label={`Remove ${s.name} from trend`} onClick={() => onRemove(s.name)}>
                  <X size={16} strokeWidth={2} />
                </IconButton>
              </span>
            ))}
            {userScrolled && (
              <Button size="sm" variant="primary" onClick={resetView} aria-label="Reset chart view to follow live data">
                Follow live
              </Button>
            )}
          </div>
        </div>
      ) : (
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: 8, padding: "4px 8px", borderBottom: "1px solid var(--border)", flexShrink: 0 }}>
        <div style={{ display: "flex", gap: 6, flexWrap: "wrap", alignItems: "center" }}>
          {series.map((s, i) => (
            <span key={s.name} style={{ fontSize: "var(--fs-xs)", display: "inline-flex", alignItems: "center", gap: 6, background: "var(--sunken)", padding: "2px 4px 2px 8px", borderRadius: "var(--radius-sm)", border: "1px solid var(--border)" }}>
              <span style={{ width: 8, height: 8, borderRadius: 2, background: s.color || COLORS[i % COLORS.length], display: "inline-block" }} aria-hidden="true" />
              {s.name}
              <IconButton size="sm" label={`Remove ${s.name} from trend`} onClick={() => onRemove(s.name)}>
                <X size={12} strokeWidth={1.75} />
              </IconButton>
            </span>
          ))}
        </div>
        <div style={{ display: "flex", gap: 6, alignItems: "center", flexShrink: 0 }}>
          {userScrolled && (
            <Button
              size="sm"
              variant="primary"
              icon={<RotateCcw size={14} strokeWidth={1.75} />}
              onClick={resetView}
              aria-label="Reset chart view to follow live data"
            >
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
              { value: "60", label: "60s" },
              { value: "300", label: "5m" },
            ]}
          />
        </div>
      </div>
      )}
      <div ref={containerRef} style={{ flex: 1, minHeight: 0, ...(phone ? { borderRadius: "var(--radius-lg)", overflow: "hidden", background: "var(--card)" } : null) }} role="img" aria-label="Trend chart showing live data" />
    </div>
  );
}
