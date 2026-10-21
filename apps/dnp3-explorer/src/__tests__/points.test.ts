import { describe, it, expect } from "vitest";
import { parsePointsCsv, pointsToCsv, type Dnp3Point } from "../points";

const POINTS: Dnp3Point[] = [
  { name: "Voltage", group: 30, variation: 4, index: 0 },
  { name: "Breaker, main", group: 10, variation: 2, index: 3 },
  { name: 'Quote "A"', group: 20, variation: 1, index: 12 },
];

describe("point list CSV", () => {
  it("writes the header and one line per point, quoting names that need it", () => {
    expect(pointsToCsv(POINTS)).toBe(
      'name,group,variation,index\nVoltage,30,4,0\n"Breaker, main",10,2,3\n"Quote ""A""",20,1,12\n',
    );
  });

  it("reads back what it wrote", () => {
    expect(parsePointsCsv(pointsToCsv(POINTS))).toEqual({ points: POINTS, errors: [] });
  });

  it("accepts CRLF line endings and a BOM", () => {
    const text = "\uFEFFname,group,variation,index\r\nA,1,2,7\r\n";
    expect(parsePointsCsv(text).points).toEqual([{ name: "A", group: 1, variation: 2, index: 7 }]);
  });

  it("rejects bad lines with their line numbers and keeps the good ones", () => {
    const text = [
      "name,group,variation,index",
      "good,30,4,0",
      "short,30,4",
      ",30,4,1",
      "dup,30,4,2",
      "dup,30,4,3",
      "badtype,99,1,0",
      "empty,30,4,",
      "negative,30,4,-1",
      "huge,30,4,70000",
      "",
      "good2,1,2,5",
    ].join("\n");
    const { points, errors } = parsePointsCsv(text);
    expect(points.map((p) => p.name)).toEqual(["good", "dup", "good2"]);
    expect(errors).toHaveLength(7);
    expect(errors[0]).toMatch(/^Line 3: expected/);
    expect(errors[1]).toMatch(/^Line 4: the point name is empty/);
    expect(errors[2]).toMatch(/^Line 6: duplicate point name 'dup'/);
    expect(errors[3]).toMatch(/^Line 7: unsupported object g99v1/);
    expect(errors[4]).toMatch(/^Line 8: index/);
    expect(errors[5]).toMatch(/^Line 9: index/);
    expect(errors[6]).toMatch(/^Line 10: index/);
  });
});
