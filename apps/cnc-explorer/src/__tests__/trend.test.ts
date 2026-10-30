import { describe, it, expect } from "vitest";
import { TREND_COLORS, nextTrendColor, toLocalChartTime } from "../../../../packages/mt-ui/src/trend";

describe("nextTrendColor", () => {
  it("starts with the first colour", () => {
    expect(nextTrendColor([])).toBe(TREND_COLORS[0]);
  });
  it("reuses a freed colour instead of repeating one in use", () => {
    // series 0..2 added, series 0 removed, series 3 added, then 4 added again
    const inUse = [TREND_COLORS[1], TREND_COLORS[2]];
    const a = nextTrendColor(inUse);
    expect(a).toBe(TREND_COLORS[0]);
    const b = nextTrendColor([...inUse, a]);
    expect(b).toBe(TREND_COLORS[3]);
  });
  it("never duplicates while free slots remain", () => {
    const used: string[] = [];
    for (let i = 0; i < TREND_COLORS.length; i++) used.push(nextTrendColor(used));
    expect(new Set(used).size).toBe(TREND_COLORS.length);
  });
  it("cycles once the palette is exhausted", () => {
    expect(TREND_COLORS).toContain(nextTrendColor([...TREND_COLORS]));
  });
});

describe("toLocalChartTime", () => {
  it("shifts by the local UTC offset", () => {
    const t = 1_760_000_000;
    expect(toLocalChartTime(t)).toBe(t - new Date(t * 1000).getTimezoneOffset() * 60);
  });
});
