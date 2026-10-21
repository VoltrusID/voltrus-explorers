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
import { Button, Dialog, EmptyState, Tabs, saveTextFile } from "@mactools/ui";

interface TrendLogDialogProps {
  objectType: "trendLog" | "trendLogMultiple";
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
  item_count: number;
  first_item: boolean;
  last_item: boolean;
  more_items: boolean;
  first_sequence_number: number | null;
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

export function TrendLogDialog({ objectType, objectInstance, onClose }: TrendLogDialogProps) {
  const [totalCount, setTotalCount] = useState<number | null>(null);
  const [records, setRecords] = useState<LogRecord[]>([]);
  // 1-based position of the oldest loaded record; everything below it is unloaded.
  const [oldestIndex, setOldestIndex] = useState(1);
  // Sequence number of the oldest loaded record, when the device reports it.
  const [oldestSeq, setOldestSeq] = useState<number | null>(null);
  // The oldest loaded record is the first record of the log (ReadRange FIRST_ITEM flag).
  const [reachedStart, setReachedStart] = useState(false);
  const [loading, setLoading] = useState(true);
  const [loadingMore, setLoadingMore] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [viewTab, setViewTab] = useState<"chart" | "table">("chart");

  const chartContainerRef = useRef<HTMLDivElement>(null);
  const chartRef = useRef<IChartApi | null>(null);
  // Bumped on every (re)load and on unmount so late responses are ignored.
  const generation = useRef(0);

  const hasMore = !reachedStart && (oldestSeq !== null ? oldestSeq > 1 : oldestIndex > 1);

  // Records per ReadRange. A device that cannot segment Aborts a reply larger than one
  // APDU (ASHRAE 135 clause 18.9), so the page size is halved until a reply fits.
  const batchSize = useRef(BATCH_SIZE);
  const readPage = useCallback(async (fetch: (batch: number) => Promise<TrendLogData>) => {
    for (;;) {
      try {
        return await fetch(batchSize.current);
      } catch (e) {
        const message = e instanceof Error ? e.message : String(e);
        if (!message.includes("BACnet Abort from device") || batchSize.current <= 1) throw e;
        batchSize.current = Math.ceil(batchSize.current / 2);
      }
    }
  }, []);

  // Newest page: ReadRange byPosition, referenceIndex = total, negative count reads
  // the BATCH_SIZE records that END at the reference index.
  const loadInitial = useCallback(async () => {
    const gen = ++generation.current;
    setLoading(true);
    setLoadingMore(false);
    setError(null);
    try {
      const total: number = await invoke("read_trend_log_count", { objectType, objectInstance });
      if (gen !== generation.current) return;
      if (total === 0) {
        setTotalCount(0);
        setRecords([]);
        setOldestIndex(1);
        setOldestSeq(null);
        setReachedStart(true);
        return;
      }
      const data = await readPage((batch) =>
        invoke<TrendLogData>("read_trend_log", {
          objectType,
          objectInstance,
          referenceIndex: total,
          count: -Math.min(batch, total),
        }),
      );
      if (gen !== generation.current) return;
      if (data.records.length === 0) {
        throw new Error(`Device reported ${total} records but returned none`);
      }
      setTotalCount(total);
      setRecords(data.records);
      setOldestIndex(total - data.records.length + 1);
      setOldestSeq(data.first_sequence_number);
      setReachedStart(data.first_item);
    } catch (e) {
      if (gen !== generation.current) return;
      setRecords([]);
      setTotalCount(null);
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      if (gen === generation.current) setLoading(false);
    }
  }, [objectType, objectInstance, readPage]);

  // Older page: the records ending just before the oldest one already loaded. A live
  // circular log shifts record positions, so when the device reports sequence numbers the
  // page is requested bySequenceNumber (stable); otherwise byPosition.
  const loadMore = useCallback(async () => {
    if (!hasMore) return;
    const gen = generation.current;
    setLoadingMore(true);
    setError(null);
    try {
      let data: TrendLogData;
      if (oldestSeq !== null) {
        data = await readPage((batch) =>
          invoke<TrendLogData>("read_trend_log_by_sequence", {
            objectType,
            objectInstance,
            referenceSequenceNumber: oldestSeq - 1,
            count: -batch,
          }),
        );
      } else {
        const reference = oldestIndex - 1;
        data = await readPage((batch) =>
          invoke<TrendLogData>("read_trend_log", {
            objectType,
            objectInstance,
            referenceIndex: reference,
            count: -Math.min(batch, reference),
          }),
        );
      }
      if (gen !== generation.current) return;
      if (data.records.length === 0) {
        if (data.first_item) {
          setReachedStart(true);
          return;
        }
        throw new Error("Device returned no older records");
      }
      if (oldestSeq !== null && data.first_sequence_number === null) {
        throw new Error("Device did not report a sequence number for the returned records");
      }
      setRecords((prev) => [...data.records, ...prev]);
      setOldestIndex(oldestIndex - data.records.length);
      setOldestSeq(data.first_sequence_number);
      setReachedStart(data.first_item);
    } catch (e) {
      if (gen !== generation.current) return;
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      if (gen === generation.current) setLoadingMore(false);
    }
  }, [objectType, objectInstance, hasMore, oldestIndex, oldestSeq, readPage]);

  useEffect(() => {
    loadInitial();
    return () => {
      generation.current++;
    };
  }, [loadInitial]);

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

  const handleExportCSV = async () => {
    if (records.length === 0) return;

    const header = "Timestamp,Value,Status Flags";
    const rows = records.map((r) => {
      const timestamp = `"${r.timestamp}"`;
      const value = `"${r.value}"`;
      const flags = r.status_flags ? `"${r.status_flags}"` : "";
      return `${timestamp},${value},${flags}`;
    });

    const csv = [header, ...rows].join("\n");
    const filename = `${objectType === "trendLogMultiple" ? "trend-log-multiple" : "trend-log"}-${objectInstance}.csv`;
    try {
      // null means the user cancelled the save panel; a failed write rejects.
      await saveTextFile(filename, csv);
    } catch (e) {
      setError(`Export CSV failed: ${e instanceof Error ? e.message : String(e)}`);
    }
  };

  const displayCount = totalCount ?? records.length;
  const numericCount = records.filter((r) => !isNaN(parseFloat(r.value))).length;

  return (
    <Dialog open onClose={onClose} title={
      <span style={{ display: "flex", alignItems: "baseline", gap: 6 }}>
        {objectType === "trendLogMultiple" ? "Trend log multiple" : "Trend log"} #{objectInstance}
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
          <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 8, padding: "6px 10px", fontSize: "var(--fs-sm)", color: "var(--danger)", background: "var(--danger-soft)", borderRadius: "var(--radius)" }} role="alert">
            <span>{error}</span>
            {records.length === 0 && (
              <Button onClick={() => void loadInitial()}>Retry</Button>
            )}
          </div>
        )}

        {!loading && !(error && records.length === 0) && (
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
              <Button onClick={() => void loadMore()} loading={loadingMore} disabled={!hasMore}>
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
