import { useCallback, useEffect, useRef } from "react";
import { useStore } from "../store";

/** What the trend needs of a table row: its name and the polled number (not the formatted text). */
interface TrendSource {
  name: string;
  rawValue: number | null;
}

export function useTrend(tableData: TrendSource[]) {
  const { trendSeries, setTrendSeries, trendWindow } = useStore();
  const trendSeriesRef = useRef(trendSeries);
  trendSeriesRef.current = trendSeries;
  const tableRef = useRef(tableData);
  tableRef.current = tableData;

  // Signature of current values — stable across renders unless a polled value
  // actually changed. Depending on `tableData` directly loops forever because
  // App rebuilds the array each render (not memoized) → effect fires → setState
  // → re-render → new array → fires again → Maximum update depth exceeded.
  const valueSig = tableData.map((r) => r.name + "\x00" + r.rawValue).join("\x01");

  // Capture trend data on every poll
  useEffect(() => {
    const ts = Date.now();
    const active = trendSeriesRef.current;
    if (active.size === 0) return;
    const table = tableRef.current;
    const next = new Map(active);
    let changed = false;
    for (const [name, s] of next) {
      const v = table.find((r) => r.name === name)?.rawValue;
      if (v === null || v === undefined) continue;
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
        const v = tableData.find((r) => r.name === name)?.rawValue;
        const colors = ["#3b82f6", "#ef4444", "#22c55e", "#f59e0b", "#a855f7", "#ec4899", "#14b8a6", "#f97316"];
        // No sample yet: the first poll adds the first point.
        next.set(name, { name, color: colors[next.size % colors.length], points: v === null || v === undefined ? [] : [{ t: Date.now(), v }] });
      }
      return next;
    });
  }, [tableData, setTrendSeries]);

  return { toggleTrend };
}
