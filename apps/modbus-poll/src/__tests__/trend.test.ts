import { describe, it, expect } from "vitest";
import { appendTrendPoints } from "../trend";
import type { TrendSeries } from "../TrendPanel";

const series = (...names: string[]) => new Map<string, TrendSeries>(names.map((n) => [n, { name: n, color: "#fff", points: [] }]));

describe("appendTrendPoints", () => {
  it("returns the same map when nothing is tracked", () => {
    const empty = series();
    expect(appendTrendPoints(empty, [{ name: "a", rawValue: 1 }], 1000, 60)).toBe(empty);
  });

  it("adds a point to tracked series only", () => {
    const next = appendTrendPoints(series("a"), [{ name: "a", rawValue: 1 }, { name: "b", rawValue: 2 }], 1000, 60);
    expect(next.get("a")!.points).toEqual([{ t: 1000, v: 1 }]);
    expect(next.has("b")).toBe(false);
  });

  it("skips a value that was not read and keeps the same map", () => {
    const start = series("a");
    expect(appendTrendPoints(start, [{ name: "a", rawValue: null }], 1000, 60)).toBe(start);
  });

  it("drops points older than the window and caps the history", () => {
    let current = series("a");
    for (let i = 0; i < 600; i++) current = appendTrendPoints(current, [{ name: "a", rawValue: i }], i * 10, 3600);
    expect(current.get("a")!.points).toHaveLength(500);
    const old = appendTrendPoints(current, [{ name: "a", rawValue: 0 }], 10_000_000, 60);
    expect(old.get("a")!.points).toEqual([{ t: 10_000_000, v: 0 }]);
  });
});
