import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState, useCallback, useRef } from "react";
import {
  createChart,
  LineSeries,
  CrosshairMode,
  type IChartApi,
  type LineData,
  type Time,
} from "lightweight-charts";
import { LineChart, Download } from "lucide-react";
import { Button, Dialog, EmptyState, Tabs } from "@mactools/ui";

interface TrendLogDialogProps {
  objectInstance: number;
  onClose: () => void;
}

interface LogRecord {
  timestamp: string;
  value: string;
  status_flags: string | null;
}

interface TrendLogData {
  object_instance: number;
  records: LogRecord[];
  total_count: number;
}

const BATCH_SIZE = 60;

function parseChartRecords(records: LogRecord[]): LineData<Time>[] {
  const data: LineData<Time>[] = [];
  const seen = new Set<number>();
  for (const r of records) {
    const v = parseFloat(r.value);
    if (isNaN(v)) continue;
    const t = new Date(r.timestamp).getTime();
    if (isNaN(t) || seen.has(t)) continue;
    seen.add(t);
    data.push({ time: (t / 1000) as Time, value: v });
  }
  return data;
}

export function TrendLogDialog({ objectInstance, onClose }: TrendLogDialogProps) {
  const [totalCount, setTotalCount] = useState<number | null>(null);
  const [records, setRecords] = useState<LogRecord[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadingMore, setLoadingMore] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [hasMore, setHasMore] = useState(true);
  const [viewTab, setViewTab] = useState<"chart" | "table">("chart");

  const chartContainerRef = useRef<HTMLDivElement>(null);
  const chartRef = useRef<IChartApi | null>(null);

  const loadRecords = useCallback(
    async (startIndex: number, append: boolean) => {
      try {
        if (append) {
          setLoadingMore(true);
        } else {
          setLoading(true);
        }
        setError(null);

        const data: TrendLogData = await invoke("read_trend_log", {
          objectInstance,
          startIndex,
          count: -BATCH_SIZE,
        });

        if (append) {
          setRecords((prev) => [...data.records, ...prev]);
        } else {
          setRecords(data.records);
        }

        setHasMore(startIndex > 0);

        if (!append) {
          try {
            const count: number = await invoke("read_trend_log_count", {
              objectInstance,
            });
            setTotalCount(count);
          } catch {
            // Non-critical
          }
        }
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        setLoading(false);
        setLoadingMore(false);
      }
    },
    [objectInstance],
  );

  useEffect(() => {
    loadRecords(0, false);
  }, [loadRecords]);

  // Chart setup & update
  useEffect(() => {
    if (viewTab !== "chart" || !chartContainerRef.current) return;

    const container = chartContainerRef.current;
    const isDark = document.documentElement.getAttribute("data-theme") !== "light";
    const colors = {
      text: isDark ? "#6b7280" : "#9ca3af",
      grid: isDark ? "rgba(255,255,255,0.05)" : "rgba(0,0,0,0.06)",
      crosshair: isDark ? "rgba(255,255,255,0.08)" : "rgba(0,0,0,0.08)",
    };

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
        rightOffset: 5,
        barSpacing: 6,
      },
      autoSize: true,
    });
    chartRef.current = chart;

    const lineSeries = chart.addSeries(LineSeries, {
      color: "var(--accent)" === "var(--accent)" ? "#3b82f6" : "var(--accent)",
      lineWidth: 1.5 as never,
      lastValueVisible: false,
      priceLineVisible: false,
    });

    const chartData = parseChartRecords(records);
    if (chartData.length > 0) {
      lineSeries.setData(chartData);
      chart.timeScale().fitContent();
    }

    return () => {
      chart.remove();
      chartRef.current = null;
    };
  }, [viewTab, records]);

  const handleLoadMore = () => {
    const newStartIndex = records.length;
    loadRecords(newStartIndex, true);
  };

  const handleExportCSV = () => {
    if (records.length === 0) return;

    const header = "Timestamp,Value,Status Flags";
    const rows = records.map((r) => {
      const timestamp = `"${r.timestamp}"`;
      const value = `"${r.value}"`;
      const flags = r.status_flags ? `"${r.status_flags}"` : "";
      return `${timestamp},${value},${flags}`;
    });

    const csv = [header, ...rows].join("\n");
    const blob = new Blob([csv], { type: "text/csv;charset=utf-8;" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `trend-log-${objectInstance}.csv`;
    a.click();
    URL.revokeObjectURL(url);
  };

  const displayCount = totalCount ?? records.length;
  const numericCount = records.filter((r) => !isNaN(parseFloat(r.value))).length;

  return (
    <Dialog open onClose={onClose} title={
      <span style={{ display: "flex", alignItems: "baseline", gap: 6 }}>
        Trend log #{objectInstance}
        {displayCount > 0 && (
          <span style={{ fontSize: "var(--fs-sm)", color: "var(--text-muted)", fontWeight: 400 }}>
            ({displayCount} records)
          </span>
        )}
      </span>
    } width={720}>
        {/* Content */}
        {loading && !loadingMore && (
          <div style={{ display: "flex", alignItems: "center", justifyContent: "center", gap: 8, padding: "16px 0", fontSize: "var(--fs-sm)", color: "var(--text-muted)" }}>
            <span className="vt-spinner" aria-hidden="true" />
            Loading trend log…
          </div>
        )}

        {error && (
          <div style={{ padding: "6px 10px", fontSize: "var(--fs-sm)", color: "var(--danger)", background: "var(--danger-soft)", borderRadius: "var(--radius)" }} role="alert">
            {error}
          </div>
        )}

        {!loading && !error && (
          <>
            {records.length === 0 ? (
              <EmptyState
                icon={<LineChart size={20} strokeWidth={1.75} />}
                title="No records"
                description="This trend log contains no records."
              />
            ) : (
              <>
                {/* Tab bar */}
                <div style={{ marginBottom: 8 }}>
                  <Tabs
                    label="Trend log view"
                    value={viewTab}
                    onChange={setViewTab}
                    items={[
                      { value: "chart" as const, label: "Chart" },
                      { value: "table" as const, label: "Table" },
                    ]}
                  />
                </div>

                {/* Chart view */}
                {viewTab === "chart" && (
                  numericCount > 0 ? (
                    <div ref={chartContainerRef} style={{ height: 280, marginBottom: 8 }} role="img" aria-label="Trend log chart" />
                  ) : (
                    <EmptyState
                      compact
                      title="No numeric data"
                      description="Records do not contain numeric values to chart."
                    />
                  )
                )}

                {/* Table view */}
                {viewTab === "table" && (
                  <div style={{ overflowY: "auto", flex: 1, margin: "0 -18px", padding: "0 18px", minHeight: 0 }}>
                    <table className="vt-table">
                      <thead>
                        <tr>
                          <th style={{ textAlign: "left" }}>Timestamp</th>
                          <th style={{ textAlign: "left" }}>Value</th>
                          <th style={{ textAlign: "left" }}>Status flags</th>
                        </tr>
                      </thead>
                      <tbody>
                        {records.map((r, i) => (
                          <tr key={i}>
                            <td className="is-mono" style={{ whiteSpace: "nowrap" }}>
                              {r.timestamp}
                            </td>
                            <td className="is-mono">{r.value}</td>
                            <td style={{ color: "var(--text-dim)" }}>{r.status_flags || "—"}</td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                )}
              </>
            )}

            {/* Buttons */}
            <div style={{ display: "flex", justifyContent: "space-between", marginTop: 12, paddingTop: 12, borderTop: "1px solid var(--border)" }}>
              <Button onClick={handleLoadMore} loading={loadingMore} disabled={!hasMore}>
                {loadingMore ? "Loading…" : hasMore ? "Load more" : "All loaded"}
              </Button>

              <Button icon={<Download size={14} strokeWidth={1.75} />} disabled={records.length === 0} onClick={handleExportCSV}>
                Export CSV
              </Button>
            </div>
          </>
        )}
    </Dialog>
  );
}
