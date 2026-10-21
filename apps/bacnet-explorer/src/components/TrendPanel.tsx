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
import { X } from "lucide-react";
import { IconButton } from "@mactools/ui";

export interface TrendSeries {
  id?: string;
  name: string;
  color: string;
  points: { t: number; v: number }[];
  source?: "poll" | "cov";
}

const COLORS = ["#3b82f6", "#ef4444", "#22c55e", "#f59e0b", "#a855f7", "#ec4899", "#14b8a6", "#f97316"];

interface Props {
  series: TrendSeries[];
  onRemove: (name: string) => void;
  windowSec: number;
  onWindowChange: (sec: number) => void;
  theme: string;
  pollMs?: number;
  onPollMsChange?: (ms: number) => void;
  onSourceChange?: (name: string, source: "poll" | "cov") => void;
}

function getChartColors() {
  const isDark = document.documentElement.getAttribute("data-theme") !== "light";
  return {
    text: isDark ? "#6b7280" : "#9ca3af",
    grid: isDark ? "rgba(255,255,255,0.05)" : "rgba(0,0,0,0.06)",
    crosshair: isDark ? "rgba(255,255,255,0.08)" : "rgba(0,0,0,0.08)",
  };
}

export default function TrendPanel({ series, onRemove, windowSec, onWindowChange, theme, pollMs, onPollMsChange, onSourceChange }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const chartRef = useRef<IChartApi | null>(null);
  const seriesRefs = useRef<ISeriesApi<"Line">[]>([]);
  const [userScrolled, setUserScrolled] = useState(false);
  const [covNotice, setCovNotice] = useState(false);
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

      const data: LineData<Time>[] = s.points
        .filter((p) => p.t >= cutoff)
        .slice(1)
        .map((p) => ({
          time: (p.t / 1000) as Time,
          value: p.v,
        }));

      const seen = new Set<number>();
      const unique = data.filter((d) => {
        const t = d.time as unknown as number;
        if (seen.has(t)) return false;
        seen.add(t);
        return true;
      });

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

  return (
    <div style={{ display: "flex", flexDirection: "column", flex: 1, overflow: "hidden" }}>
      {covNotice && (
        <div style={{ display: "flex", alignItems: "center", gap: 8, padding: "4px 8px", fontSize: "var(--fs-xs)", color: "var(--text-muted)", background: "var(--accent-soft)", borderBottom: "1px solid var(--border)", flexShrink: 0 }}>
          <span>COV subscriptions are under development, using Poll for now.</span>
          <button className="vt-btn vt-btn--sm" style={{ marginLeft: "auto" }} onClick={() => setCovNotice(false)}>Dismiss</button>
        </div>
      )}
      {series.length > 0 && (
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", padding: "4px 8px", borderBottom: "1px solid var(--border)", flexShrink: 0, flexWrap: "wrap", gap: 4 }}>
        <div style={{ display: "flex", gap: 6, flexWrap: "wrap", alignItems: "center" }}>
          {series.map((s, i) => {
            const sid = s.id ?? s.name;
            return (
            <span key={sid} style={{ fontSize: "var(--fs-xs)", display: "flex", alignItems: "center", gap: 4, padding: "2px 4px 2px 6px", background: "var(--surface)", borderRadius: "var(--radius-sm)", border: "1px solid var(--border)" }}>
              <span style={{ width: 8, height: 8, borderRadius: 2, background: s.color || COLORS[i % COLORS.length], display: "inline-block" }} aria-hidden="true" />
              {s.name}
              {onSourceChange && (
                <button
                  className="vt-btn vt-btn--sm"
                  aria-pressed={s.source === "poll"}
                  onClick={() => onSourceChange(sid, "poll")}
                  title="Data source: Poll"
                >
                  Poll
                </button>
              )}
              <button
                className="vt-btn vt-btn--sm"
                onClick={() => setCovNotice(true)}
                title="COV, under development"
              >
                COV
              </button>
              <IconButton size="sm" label={`Remove ${s.name} from trend`} onClick={() => onRemove(sid)}>
                <X size={12} strokeWidth={1.75} />
              </IconButton>
            </span>
            );
          })}
        </div>
        <div style={{ display: "flex", gap: 4, alignItems: "center", flexWrap: "wrap" }}>
          {onPollMsChange && (
            <label style={{ display: "flex", alignItems: "center", gap: 4, fontSize: "var(--fs-xs)", color: "var(--text-muted)" }}>
              Poll
              <input
                className="vt-input vt-input--sm"
                type="number"
                min={100}
                step={100}
                value={pollMs ?? 1000}
                onChange={(e) => onPollMsChange(Math.max(100, Number(e.target.value) || 1000))}
                style={{ width: 56 }}
              />
              ms
            </label>
          )}
          {userScrolled && (
            <button className="vt-btn vt-btn--sm" onClick={resetView} aria-label="Reset chart view to follow live data">
              Reset
            </button>
          )}
          {[30, 60, 300].map((sec) => (
            <button
              key={sec}
              className="vt-btn vt-btn--sm"
              aria-pressed={windowSec === sec}
              aria-label={`Show last ${sec >= 60 ? `${sec / 60} minutes` : `${sec} seconds`}`}
              onClick={() => onWindowChange(sec)}
            >
              {sec >= 60 ? `${sec / 60}m` : `${sec}s`}
            </button>
          ))}
        </div>
      </div>
      )}
      <div ref={containerRef} style={{ flex: 1, minHeight: 0 }} role="img" aria-label="Trend chart showing live data" />
    </div>
  );
}
