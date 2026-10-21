import type { TrendSeries } from "./TrendPanel";

/** What a trend needs of a table row: its name and the polled number. */
export interface TrendSample {
  name: string;
  rawValue: number | null;
}

const MAX_POINTS = 500;

/** Add one point per tracked series from this poll. Returns `series` itself when nothing was added. */
export function appendTrendPoints(
  series: Map<string, TrendSeries>,
  rows: TrendSample[],
  timestampMs: number,
  windowSec: number,
): Map<string, TrendSeries> {
  if (series.size === 0) return series;
  const byName = new Map(rows.map((r) => [r.name, r.rawValue]));
  const cutoff = timestampMs - windowSec * 1000 - 5000;
  let changed = false;
  const next = new Map(series);
  for (const [name, s] of series) {
    const v = byName.get(name);
    if (v === null || v === undefined) continue;
    const points = [...s.points, { t: timestampMs, v }].filter((p) => p.t > cutoff).slice(-MAX_POINTS);
    next.set(name, { ...s, points });
    changed = true;
  }
  return changed ? next : series;
}
