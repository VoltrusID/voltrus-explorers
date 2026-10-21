import { describe, expect, it } from "vitest";
import { ValueLog, csvCell, samplesToCsv, valuesToCsv } from "../valueLog";

describe("csvCell", () => {
  it("quotes commas, quotes and newlines", () => {
    expect(csvCell("a,b")).toBe('"a,b"');
    expect(csvCell('say "hi"')).toBe('"say ""hi"""');
    expect(csvCell("a\nb")).toBe('"a\nb"');
  });

  it("renders null, undefined, objects and booleans", () => {
    expect(csvCell(null)).toBe("");
    expect(csvCell(undefined)).toBe("");
    expect(csvCell(true)).toBe("true");
    expect(csvCell({ a: 1 })).toBe('"{""a"":1}"');
  });

  it("neutralises spreadsheet formulas but not numbers", () => {
    expect(csvCell("=1+1")).toBe("'=1+1");
    expect(csvCell("@SUM(A1)")).toBe("'@SUM(A1)");
    expect(csvCell("-cmd")).toBe("'-cmd");
    expect(csvCell(-4.5)).toBe("-4.5");
    expect(csvCell("-4.5")).toBe("-4.5");
    expect(csvCell("1e-3")).toBe("1e-3");
  });
});

describe("valuesToCsv", () => {
  it("writes a header and one row per node with ISO timestamps", () => {
    const out = valuesToCsv([
      { session: "plc:4840", nodeId: "ns=2;s=T", name: "Temp", dataType: "Double", value: 21.5, timestamp: 1700000000000 },
      { session: "plc:4840", nodeId: "ns=2;s=R", name: "Running", dataType: "Boolean", value: null, timestamp: 0 },
    ]);
    expect(out).toBe(
      "Session,Node ID,Browse Name,Data Type,Value,Timestamp\r\n" +
        "plc:4840,ns=2;s=T,Temp,Double,21.5,2023-11-14T22:13:20.000Z\r\n" +
        "plc:4840,ns=2;s=R,Running,Boolean,,\r\n",
    );
  });
});

describe("ValueLog", () => {
  it("records only changes per node and session", () => {
    const log = new ValueLog();
    log.record("s1", "A", "n1", "N1", 1, 10);
    log.record("s1", "A", "n1", "N1", 1, 11);
    log.record("s1", "A", "n1", "N1", 2, 12);
    log.record("s2", "B", "n1", "N1", 1, 13);
    expect(log.all().map((s) => [s.session, s.value, s.t])).toEqual([
      ["A", 1, 10],
      ["A", 2, 12],
      ["B", 1, 13],
    ]);
  });

  it("records the same value again after forget()", () => {
    const log = new ValueLog();
    log.record("s1", "A", "n1", "N1", 1, 10);
    log.forget("s1");
    log.record("s1", "A", "n1", "N1", 1, 20);
    expect(log.size).toBe(2);
  });

  it("keeps the newest samples when over the cap", () => {
    const log = new ValueLog(3);
    for (let i = 0; i < 5; i++) log.record("s", "A", "n", "N", i, i);
    expect(log.all().map((s) => s.value)).toEqual([2, 3, 4]);
  });

  it("exports samples as CSV in order", () => {
    const log = new ValueLog();
    log.record("s", "A", "n", "N", "x,y", 1700000000000);
    expect(samplesToCsv(log.all())).toBe(
      "Timestamp,Session,Node ID,Browse Name,Value\r\n2023-11-14T22:13:20.000Z,A,n,N,\"x,y\"\r\n",
    );
  });
});
