import { describe, expect, it } from "vitest";
import { MAX_POINTS, MAX_SERIES, SeriesStore, extractNumeric, seriesKey } from "./charts";
import type { Decoded } from "./payload";

describe("extractNumeric", () => {
  it("reads bare numbers", () => {
    expect(extractNumeric("21.5")).toEqual([{ path: "", value: 21.5 }]);
    expect(extractNumeric("  -3e2\n")).toEqual([{ path: "", value: -300 }]);
    expect(extractNumeric(".5")).toEqual([{ path: "", value: 0.5 }]);
  });

  it("ignores things that only look like numbers", () => {
    for (const t of ["", "0x10", "NaN", "Infinity", "1,5", "12abc", "true", "1e999"]) {
      expect(extractNumeric(t), t).toEqual([]);
    }
  });

  it("flattens numeric JSON leaves with paths", () => {
    const got = extractNumeric('{"temp": 20.5, "room": {"hum": 40, "name": "a"}, "on": true, "xs": [1, 2]}');
    expect(got).toEqual([
      { path: "temp", value: 20.5 },
      { path: "room.hum", value: 40 },
      { path: "on", value: 1 },
      { path: "xs[0]", value: 1 },
      { path: "xs[1]", value: 2 },
    ]);
  });

  it("handles invalid JSON and caps the leaf count", () => {
    expect(extractNumeric("{not json")).toEqual([]);
    const big = JSON.stringify(Object.fromEntries(Array.from({ length: 200 }, (_, i) => [`k${i}`, i])));
    expect(extractNumeric(big).length).toBe(64);
  });

  it("reads Sparkplug metrics by name, with booleans and nested templates", () => {
    const decoded: Decoded = {
      kind: "sparkplug",
      value: {
        metrics: [
          { name: "Temp", value: 21.5 },
          { name: "Run", value: true },
          { name: "Label", value: "x" },
          { alias: 7, value: 3 },
          { name: "Nulled", value: 9, is_null: true },
          { name: "Motor", value: { metrics: [{ name: "rpm", value: 900 }] } },
          { name: "Arr", value: [1, 2] },
        ],
      },
    };
    expect(extractNumeric("ignored binary", decoded)).toEqual([
      { path: "Temp", value: 21.5 },
      { path: "Run", value: 1 },
      { path: "alias 7", value: 3 },
      { path: "Motor/rpm", value: 900 },
    ]);
  });

  it("reads numeric leaves of decoded protobuf and nothing from failed decodes", () => {
    expect(extractNumeric("", { kind: "protobuf", value: { sensor: "t1", value: 2.5, at_ms: 5 } })).toEqual([
      { path: "value", value: 2.5 },
      { path: "at_ms", value: 5 },
    ]);
    expect(extractNumeric("12", { kind: "sparkplug", error: "bad" })).toEqual([]);
  });
});

describe("SeriesStore", () => {
  it("records points per connection, topic and path", () => {
    const s = new SeriesStore();
    s.ingest("c1", "a", "1", undefined, 1000);
    s.ingest("c1", "a", "2", undefined, 2000);
    s.ingest("c2", "a", "9", undefined, 2000);
    s.ingest("c1", "b", '{"x": 4}', undefined, 3000);
    expect(s.get(seriesKey("c1", "a", ""))?.points).toEqual([
      { t: 1000, v: 1 },
      { t: 2000, v: 2 },
    ]);
    expect(s.get(seriesKey("c1", "a", ""))?.last).toBe(2);
    expect(s.list("c1").map((x) => x.key)).toEqual([seriesKey("c1", "a", ""), seriesKey("c1", "b", "x")]);
    expect(s.list("c2")).toHaveLength(1);
  });

  it("bumps version only when something changed", () => {
    const s = new SeriesStore();
    const v0 = s.version;
    s.ingest("c1", "a", "not numeric", undefined, 1);
    expect(s.version).toBe(v0);
    s.ingest("c1", "a", "1", undefined, 1);
    expect(s.version).toBeGreaterThan(v0);
  });

  it("keeps at most MAX_POINTS (plus the trim batch) per series", () => {
    const s = new SeriesStore();
    for (let i = 0; i < MAX_POINTS + 500; i++) s.ingest("c1", "a", String(i), undefined, i);
    const pts = s.get(seriesKey("c1", "a", ""))!.points;
    expect(pts.length).toBeLessThanOrEqual(MAX_POINTS + 200);
    expect(pts.length).toBeGreaterThanOrEqual(MAX_POINTS);
    expect(pts[pts.length - 1].v).toBe(MAX_POINTS + 499);
  });

  it("stops adding series at the cap but keeps feeding existing ones", () => {
    const s = new SeriesStore();
    for (let i = 0; i < MAX_SERIES + 10; i++) s.ingest("c1", `t${i}`, "1", undefined, 1);
    expect(s.list("c1")).toHaveLength(MAX_SERIES);
    s.ingest("c1", "t0", "5", undefined, 2);
    expect(s.get(seriesKey("c1", "t0", ""))?.points).toHaveLength(2);
  });

  it("clears one connection or everything", () => {
    const s = new SeriesStore();
    s.ingest("c1", "a", "1", undefined, 1);
    s.ingest("c2", "a", "1", undefined, 1);
    s.clear("c1");
    expect(s.list("c1")).toHaveLength(0);
    expect(s.list("c2")).toHaveLength(1);
    s.clear();
    expect(s.list("c2")).toHaveLength(0);
  });
});
