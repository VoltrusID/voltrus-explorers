import { useCallback } from "react";
import { useStore } from "../store";

/** What the trend needs of a table row: its name and the polled number (not the formatted text). */
interface TrendSource {
  name: string;
  rawValue: number | null;
}

const COLORS = ["#3b82f6", "#ef4444", "#22c55e", "#f59e0b", "#a855f7", "#ec4899", "#14b8a6", "#f97316"];

/** Adding and removing trend series. Points are appended by each connection's poll (see connections.ts). */
export function useTrend(tableData: TrendSource[]) {
  const setTrendSeries = useStore((s) => s.setTrendSeries);

  const toggleTrend = useCallback((name: string) => {
    setTrendSeries((prev) => {
      const next = new Map(prev);
      if (next.has(name)) {
        next.delete(name);
      } else {
        const v = tableData.find((r) => r.name === name)?.rawValue;
        // No sample yet: the first poll adds the first point.
        next.set(name, { name, color: COLORS[next.size % COLORS.length], points: v === null || v === undefined ? [] : [{ t: Date.now(), v }] });
      }
      return next;
    });
  }, [tableData, setTrendSeries]);

  return { toggleTrend };
}
