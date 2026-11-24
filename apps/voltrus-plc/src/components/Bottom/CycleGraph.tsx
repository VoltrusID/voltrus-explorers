import { useState, useMemo } from "react";
import { useStore } from "../../store";

const WIDTH = 600;
const HEIGHT = 160;
const PAD_L = 44;
const PAD_R = 12;
const PAD_T = 8;
const PAD_B = 20;
const PLOT_W = WIDTH - PAD_L - PAD_R;
const PLOT_H = HEIGHT - PAD_T - PAD_B;

export default function CycleGraph() {
  const cycleHistory = useStore((s) => s.cycleHistory);
  const maxCycleMs = useStore((s) => s.maxCycleMs);
  const lastCycleMs = useStore((s) => s.lastCycleMs);
  const avgCycleMs = useStore((s) => s.avgCycleMs);
  const cycleCount = useStore((s) => s.cycleCount);
  const [tooltip, setTooltip] = useState<{ x: number; y: number; value: number; index: number } | null>(null);

  const pathD = useMemo(() => {
    if (cycleHistory.length < 2) return "";
    const max = Math.max(maxCycleMs * 1.1, Math.max(...cycleHistory, 1));
    const xStep = PLOT_W / Math.max(cycleHistory.length - 1, 1);

    return cycleHistory
      .map((v, i) => {
        const x = PAD_L + i * xStep;
        const y = PAD_T + PLOT_H - (v / max) * PLOT_H;
        return `${i === 0 ? "M" : "L"} ${x.toFixed(1)} ${y.toFixed(1)}`;
      })
      .join(" ");
  }, [cycleHistory, maxCycleMs]);

  const areaD = useMemo(() => {
    if (cycleHistory.length < 2) return "";
    const max = Math.max(maxCycleMs * 1.1, Math.max(...cycleHistory, 1));
    const xStep = PLOT_W / Math.max(cycleHistory.length - 1, 1);
    const lastIdx = cycleHistory.length - 1;

    const line = cycleHistory
      .map((v, i) => {
        const x = PAD_L + i * xStep;
        const y = PAD_T + PLOT_H - (v / max) * PLOT_H;
        return `${i === 0 ? "M" : "L"} ${x.toFixed(1)} ${y.toFixed(1)}`;
      })
      .join(" ");

    const baseY = PAD_T + PLOT_H;
    return `${line} L ${PAD_L + lastIdx * xStep} ${baseY} L ${PAD_L} ${baseY} Z`;
  }, [cycleHistory, maxCycleMs]);

  const yAxisLabels = useMemo(() => {
    const max = Math.max(maxCycleMs * 1.1, 1);
    const labels: { value: number; y: number }[] = [];
    const steps = 4;
    for (let i = 0; i <= steps; i++) {
      const value = (max / steps) * i;
      labels.push({
        value,
        y: PAD_T + PLOT_H - (value / max) * PLOT_H,
      });
    }
    return labels;
  }, [maxCycleMs, ...cycleHistory]);

  const points = useMemo(() => {
    if (cycleHistory.length === 0) return [];
    const max = Math.max(maxCycleMs * 1.1, Math.max(...cycleHistory, 1));
    const xStep = PLOT_W / Math.max(cycleHistory.length - 1, 1);
    return cycleHistory.map((v, i) => ({
      x: PAD_L + i * xStep,
      y: PAD_T + PLOT_H - (v / max) * PLOT_H,
      value: v,
      index: i,
    }));
  }, [cycleHistory, maxCycleMs]);

  const watchdogY = useMemo(() => {
    const max = Math.max(maxCycleMs * 1.1, Math.max(...cycleHistory, maxCycleMs + 10));
    return PAD_T + PLOT_H - (maxCycleMs / max) * PLOT_H;
  }, [cycleHistory, maxCycleMs]);

  return (
    <div className="cycle-graph">
      <div className="cycle-graph-stats">
        <span className="cycle-stat">
          Cycles: <strong>{cycleCount.toLocaleString()}</strong>
        </span>
        <span className="cycle-stat">
          Last: <strong>{lastCycleMs.toFixed(2)}ms</strong>
        </span>
        <span className="cycle-stat">
          Avg: <strong>{avgCycleMs.toFixed(2)}ms</strong>
        </span>
        <span className="cycle-stat">
          Max: <strong style={{ color: lastCycleMs > maxCycleMs ? "var(--danger)" : undefined }}>
            {Math.max(...cycleHistory, 0).toFixed(2)}ms
          </strong>
        </span>
      </div>

      <svg
        viewBox={`0 0 ${WIDTH} ${HEIGHT}`}
        className="cycle-graph-svg"
        preserveAspectRatio="xMidYMid meet"
      >
        {/* Grid lines */}
        {yAxisLabels.map((l) => (
          <g key={l.value}>
            <line
              x1={PAD_L} y1={l.y} x2={WIDTH - PAD_R} y2={l.y}
              stroke="var(--border)" strokeWidth="0.5" strokeDasharray="3 2"
            />
            <text
              x={PAD_L - 6} y={l.y + 3}
              textAnchor="end" fontSize="9" fill="var(--text-muted)"
            >
              {l.value.toFixed(1)}
            </text>
          </g>
        ))}

        {/* Y-axis label */}
        <text
          x={10} y={PAD_T + PLOT_H / 2}
          textAnchor="middle" fontSize="8" fill="var(--text-muted)"
          transform={`rotate(-90, 10, ${PAD_T + PLOT_H / 2})`}
        >
          ms
        </text>

        {/* Watchdog line */}
        {cycleHistory.length > 0 && (
          <>
            <line
              x1={PAD_L} y1={watchdogY} x2={WIDTH - PAD_R} y2={watchdogY}
              stroke="var(--danger)" strokeWidth="1" strokeDasharray="6 3"
              opacity="0.7"
            />
            <text
              x={WIDTH - PAD_R - 4} y={watchdogY - 4}
              textAnchor="end" fontSize="8" fill="var(--danger)" opacity="0.8"
            >
              watchdog {maxCycleMs}ms
            </text>
          </>
        )}

        {/* Area fill */}
        {areaD && (
          <path d={areaD} fill="var(--accent)" opacity="0.1" />
        )}

        {/* Line */}
        {pathD && (
          <path d={pathD} fill="none" stroke="var(--accent)" strokeWidth="1.5" />
        )}

        {/* Interactive area for tooltip */}
        <rect
          x={PAD_L} y={PAD_T} width={PLOT_W} height={PLOT_H}
          fill="transparent"
          onMouseMove={(e) => {
            const rect = (e.currentTarget as SVGRectElement).getBoundingClientRect();
            const svgX = (e.clientX - rect.left) / rect.width * PLOT_W;
            const idx = Math.round((svgX / PLOT_W) * (points.length - 1));
            if (idx >= 0 && idx < points.length) {
              const pt = points[idx];
              setTooltip({
                x: pt.x,
                y: pt.y,
                value: pt.value,
                index: pt.index,
              });
            }
          }}
          onMouseLeave={() => setTooltip(null)}
        />

        {/* Tooltip dot and label */}
        {tooltip && (
          <>
            <circle cx={tooltip.x} cy={tooltip.y} r="3" fill="var(--accent)" />
            <rect
              x={Math.min(Math.max(tooltip.x - 40, PAD_L), WIDTH - PAD_R - 80)}
              y={tooltip.y - 20}
              width="80" height="16" rx="3"
              fill="var(--surface)" stroke="var(--border)" strokeWidth="0.5"
            />
            <text
              x={Math.min(Math.max(tooltip.x, PAD_L + 40), WIDTH - PAD_R - 40)}
              y={tooltip.y - 8}
              textAnchor="middle" fontSize="9" fill="var(--text)"
            >
              #{tooltip.index}: {tooltip.value.toFixed(2)}ms
            </text>
          </>
        )}

        {/* Empty state */}
        {cycleHistory.length === 0 && (
          <text
            x={WIDTH / 2} y={HEIGHT / 2}
            textAnchor="middle" fontSize="12" fill="var(--text-muted)"
          >
            Run the PLC to see cycle timing
          </text>
        )}
      </svg>
    </div>
  );
}
