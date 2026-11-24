import { useMemo } from "react";
import { useStore } from "../../store";

const WIDTH = 600;
const HEIGHT = 160;
const PAD_L = 44;
const PAD_R = 12;
const PAD_T = 8;
const PAD_B = 24;
const PLOT_W = WIDTH - PAD_L - PAD_R;
const PLOT_H = HEIGHT - PAD_T - PAD_B;
const BIN_COUNT = 12;

function percentile(sorted: number[], p: number): number {
  if (sorted.length === 0) return 0;
  const idx = (p / 100) * (sorted.length - 1);
  const lo = Math.floor(idx);
  const hi = Math.ceil(idx);
  if (lo === hi) return sorted[lo];
  return sorted[lo] + (sorted[hi] - sorted[lo]) * (idx - lo);
}

export default function Histogram() {
  const cycleHistory = useStore((s) => s.cycleHistory);
  const maxCycleMs = useStore((s) => s.maxCycleMs);

  const stats = useMemo(() => {
    if (cycleHistory.length === 0) return null;
    const sorted = [...cycleHistory].sort((a, b) => a - b);
    return {
      min: sorted[0],
      max: sorted[sorted.length - 1],
      avg: sorted.reduce((a, b) => a + b, 0) / sorted.length,
      p50: percentile(sorted, 50),
      p95: percentile(sorted, 95),
      p99: percentile(sorted, 99),
    };
  }, [cycleHistory]);

  const bins = useMemo(() => {
    if (cycleHistory.length === 0 || !stats) return [];
    const upper = Math.max(stats.max * 1.1, maxCycleMs * 0.5, 1);
    const binWidth = upper / BIN_COUNT;
    const counts = new Array(BIN_COUNT).fill(0);
    for (const v of cycleHistory) {
      const b = Math.min(Math.floor(v / binWidth), BIN_COUNT - 1);
      counts[b]++;
    }
    const maxCount = Math.max(...counts, 1);
    return counts.map((count, i) => ({
      count,
      label: (i * binWidth).toFixed(1),
      lo: i * binWidth,
      hi: (i + 1) * binWidth,
      ratio: count / maxCount,
      danger: (i + 1) * binWidth > maxCycleMs * 0.8,
    }));
  }, [cycleHistory, maxCycleMs, stats]);

  const barW = PLOT_W / BIN_COUNT - 2;

  return (
    <div className="cycle-graph">
      {stats ? (
        <div className="cycle-graph-stats">
          <span className="cycle-stat">
            Min: <strong style={{ color: stats.min < 1 ? "var(--accent)" : undefined }}>
              {stats.min.toFixed(2)}ms
            </strong>
          </span>
          <span className="cycle-stat">
            Avg: <strong>{stats.avg.toFixed(2)}ms</strong>
          </span>
          <span className="cycle-stat">
            P50: <strong>{stats.p50.toFixed(2)}ms</strong>
          </span>
          <span className="cycle-stat">
            P95: <strong>{stats.p95.toFixed(2)}ms</strong>
          </span>
          <span className="cycle-stat">
            P99: <strong style={{ color: stats.p99 > maxCycleMs * 0.9 ? "var(--danger)" : undefined }}>
              {stats.p99.toFixed(2)}ms
            </strong>
          </span>
          <span className="cycle-stat">
            Max: <strong style={{ color: stats.max > maxCycleMs ? "var(--danger)" : undefined }}>
              {stats.max.toFixed(2)}ms
            </strong>
          </span>
        </div>
      ) : (
        <div className="cycle-graph-stats">
          <span className="cycle-stat" style={{ color: "var(--text-muted)" }}>
            Run the PLC to see cycle distribution
          </span>
        </div>
      )}

      <svg
        viewBox={`0 0 ${WIDTH} ${HEIGHT}`}
        className="cycle-graph-svg"
        preserveAspectRatio="xMidYMid meet"
      >
        {bins.map((bin, i) => {
          const barH = bin.ratio * PLOT_H;
          const x = PAD_L + i * (PLOT_W / BIN_COUNT) + 1;
          const y = PAD_T + PLOT_H - barH;
          const fill = bin.danger ? "var(--danger)" : "var(--accent)";
          return (
            <g key={i}>
              <rect
                x={x} y={y} width={barW} height={barH}
                fill={fill} opacity={0.7} rx="1"
              />
              {bin.count > 0 && (
                <text
                  x={x + barW / 2} y={y - 3}
                  textAnchor="middle" fontSize="7" fill="var(--text-muted)"
                >
                  {bin.count}
                </text>
              )}
              <text
                x={x + barW / 2} y={PAD_T + PLOT_H + 12}
                textAnchor="middle" fontSize="7" fill="var(--text-muted)"
              >
                {bin.label}
              </text>
            </g>
          );
        })}

        {/* Y-axis label */}
        <text
          x={10} y={PAD_T + PLOT_H / 2}
          textAnchor="middle" fontSize="8" fill="var(--text-muted)"
          transform={`rotate(-90, 10, ${PAD_T + PLOT_H / 2})`}
        >
          count
        </text>

        {/* X-axis label */}
        <text
          x={PAD_L + PLOT_W / 2} y={HEIGHT - 2}
          textAnchor="middle" fontSize="8" fill="var(--text-muted)"
        >
          cycle time (ms)
        </text>

        {bins.length === 0 && (
          <text
            x={WIDTH / 2} y={HEIGHT / 2}
            textAnchor="middle" fontSize="12" fill="var(--text-muted)"
          >
            No cycle data yet
          </text>
        )}
      </svg>
    </div>
  );
}
