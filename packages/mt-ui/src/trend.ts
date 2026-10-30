/** Shared palette for live trend series. */
export const TREND_COLORS = ["#3b82f6", "#ef4444", "#22c55e", "#f59e0b", "#a855f7", "#ec4899", "#14b8a6", "#f97316"];

/**
 * Colour for a newly added trend series: the first palette colour not used by `inUse`.
 * Existing series keep theirs; the palette only cycles once every colour is taken.
 */
export function nextTrendColor(inUse: Iterable<string>, palette: readonly string[] = TREND_COLORS): string {
  const used = new Map<string, number>();
  let total = 0;
  for (const c of inUse) {
    const k = c.toLowerCase();
    used.set(k, (used.get(k) ?? 0) + 1);
    total++;
  }
  const free = palette.find((c) => !used.has(c.toLowerCase()));
  if (free) return free;
  // Palette exhausted: reuse the least-used colour.
  let best = palette[0];
  let bestCount = Infinity;
  for (const c of palette) {
    const n = used.get(c.toLowerCase()) ?? 0;
    if (n < bestCount) { best = c; bestCount = n; }
  }
  return total === 0 ? palette[0] : best;
}

/**
 * lightweight-charts renders its time axis in UTC. Shift a unix-seconds timestamp by the
 * local UTC offset (at that instant) so the axis shows local wall-clock time.
 */
export function toLocalChartTime(unixSec: number): number {
  return unixSec - new Date(unixSec * 1000).getTimezoneOffset() * 60;
}
