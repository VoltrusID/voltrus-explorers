import { useCallback, useEffect, useRef, useState } from "react";
import type { TrendSeries } from "./TrendPanel";

export function useTrend(tableData: { name: string; value: string | number }[]) {
  const [trendSeries, setTrendSeries] = useState<Map<string, TrendSeries>>(new Map());
  const [trendWindow, setTrendWindow] = useState(60);
  const trendSeriesRef = useRef(trendSeries);
  trendSeriesRef.current = trendSeries;
  const tableRef = useRef(tableData);
  tableRef.current = tableData;

  const valueSig = tableData.map((r) => r.name + "\x00" + r.value).join("\x01");

  useEffect(() => {
    const ts = Date.now();
    const active = trendSeriesRef.current;
    if (active.size === 0) return;
    const table = tableRef.current;
    const next = new Map(active);
    let changed = false;
    for (const [name, s] of next) {
      const row = table.find((r) => r.name === name);
      if (!row) continue;
      const v = typeof row.value === "number" ? row.value : parseFloat(String(row.value));
      if (isNaN(v)) continue;
      const points = [...s.points, { t: ts, v }];
      const cutoff = ts - trendWindow * 1000 - 5000;
      next.set(name, { ...s, points: points.filter((p) => p.t > cutoff).slice(-500) });
      changed = true;
    }
    if (changed) setTrendSeries(next);
  }, [valueSig, trendWindow, setTrendSeries]);

  const toggleTrend = useCallback((name: string) => {
    setTrendSeries((prev) => {
      const next = new Map(prev);
      if (next.has(name)) {
        next.delete(name);
      } else {
        const row = tableData.find((r) => r.name === name);
        const v = row ? (typeof row.value === "number" ? row.value : parseFloat(String(row.value))) : 0;
        const colors = ["#3b82f6", "#ef4444", "#22c55e", "#f59e0b", "#a855f7", "#ec4899", "#14b8a6", "#f97316"];
        next.set(name, { name, color: colors[next.size % colors.length], points: [{ t: Date.now(), v: isNaN(v) ? 0 : v }] });
      }
      return next;
    });
  }, [tableData, setTrendSeries]);

  return { trendSeries, setTrendSeries, trendWindow, setTrendWindow, toggleTrend };
}
