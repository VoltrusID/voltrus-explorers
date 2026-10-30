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
import { Activity, X } from "lucide-react";
import { Button, EmptyState, IconButton, Segmented, toLocalChartTime } from "@mactools/ui";

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

function currentScheme(): "dark" | "light" {
  const attr = document.documentElement.getAttribute("data-theme");
  if (attr) return attr === "light" ? "light" : "dark";
  return window.matchMedia?.("(prefers-color-scheme: light)").matches ? "light" : "dark";
}

function getChartColors() {
  const isDark = currentScheme() === "dark";
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
  const suppressNextRef = useRef(false);
  const [scheme, setScheme] = useState<"dark" | "light">(currentScheme);

  useEffect(() => {
    const mq = window.matchMedia("(prefers-color-scheme: light)");
    const onChange = (e: MediaQueryListEvent) => setScheme(e.matches ? "light" : "dark");
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, []);

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
  }, [theme, scheme]);

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
        bySecond.set(toLocalChartTime(Math.floor(p.t / 1000)), p.v);
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
  }, [series, windowSec, userScrolled, scheme]);

  const resetView = () => {
    const chart = chartRef.current;
    if (!chart) return;
    setUserScrolled(false);
    suppressNextRef.current = true;
    chart.timeScale().fitContent();
  };

  if (series.length === 0) {
    return (
      <div style={{ flex: 1, display: "flex", alignItems: "center", justifyContent: "center" }}>
        <EmptyState
          icon={<Activity size={20} strokeWidth={1.75} />}
          title="No trend series"
          description="Click the plus button on a point row to chart it here."
        />
      </div>
    );
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", flex: 1, overflow: "hidden" }}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", padding: "4px 8px", borderBottom: "1px solid var(--border)", flexShrink: 0, gap: 8 }}>
        <div style={{ display: "flex", gap: 6, flexWrap: "wrap", alignItems: "center" }}>
          {series.map((s, i) => (
            <span key={s.name} style={{ fontSize: "var(--fs-xs)", display: "flex", alignItems: "center", gap: 4, background: "var(--card)", padding: "1px 2px 1px 8px", borderRadius: "var(--radius-sm)", border: "1px solid var(--border)" }}>
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
            <Button size="sm" onClick={resetView}>Reset</Button>
          )}
          <Segmented
            size="sm"
            label="Trend window"
            value={String(windowSec)}
            onChange={(v) => onWindowChange(Number(v))}
            options={[30, 60, 300].map((sec) => ({ value: String(sec), label: sec >= 60 ? `${sec / 60}m` : `${sec}s` }))}
          />
        </div>
      </div>
      <div ref={containerRef} style={{ flex: 1, minHeight: 0 }} role="img" aria-label="Trend chart showing live data" />
    </div>
  );
}
