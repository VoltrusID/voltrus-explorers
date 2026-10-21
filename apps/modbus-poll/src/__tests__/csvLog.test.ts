import { describe, it, expect } from "vitest";
import { CSV_LOG_HEADER, csvLogFileName, csvLogLines } from "../csvLog";
import { effectivePlan } from "../limits";
import { buildTableRows } from "../utils/rows";
import type { ReadBlock } from "../types";

const blocks: ReadBlock[] = [
  { name: "Voltage", address: 10, quantity: 2, type: "float32", area: "holding" },
  { name: "Run", address: 0, quantity: 1, type: "uint16", area: "coil" },
];
const rowsFor = (values: unknown[], pro = true) =>
  buildTableRows(blocks, effectivePlan(blocks, pro), values, { useOneBased: false, displayFormat: "hex" });

describe("csv log", () => {
  it("header lists the columns written by csvLogLines", () => {
    expect(CSV_LOG_HEADER).toBe("timestamp,connection,name,area,address,type,value,error\n");
  });

  it("writes one line per value with numbers, not the display format", () => {
    const text = csvLogLines("10.0.0.5:502", Date.UTC(2026, 9, 21, 8, 30, 0, 123), rowsFor([{ Float64: 230.5 }, { Float64: 231 }, { Float64: 1 }]));
    expect(text.split("\n")).toEqual([
      "2026-10-21T08:30:00.123Z,10.0.0.5:502,Voltage_010,holding,10,float32,230.5,",
      "2026-10-21T08:30:00.123Z,10.0.0.5:502,Voltage_012,holding,12,float32,231,",
      "2026-10-21T08:30:00.123Z,10.0.0.5:502,Run_000,coil,0,bit,1,",
      "",
    ]);
  });

  it("leaves the value empty and fills the error column when the device rejected a read", () => {
    const text = csvLogLines("c", 0, rowsFor([{ Failed: "Illegal Data Address (0x02) reading 10" }, { Float64: 1 }, { Float64: 0 }]));
    const first = text.split("\n")[0];
    expect(first).toBe('1970-01-01T00:00:00.000Z,c,Voltage_010,holding,10,float32,,Illegal Data Address (0x02) reading 10');
  });

  it("quotes names and connection labels that contain commas or quotes", () => {
    const rows = buildTableRows([{ name: 'a,"b"', address: 0, quantity: 1, type: "uint16", area: "holding" }], effectivePlan([{ name: "x", address: 0, quantity: 1, type: "uint16", area: "holding" }], true), [{ Float64: 1 }], { useOneBased: false, displayFormat: "dec" });
    const line = csvLogLines("Plant, east", 0, rows);
    expect(line).toContain('"Plant, east"');
    expect(line).toContain('"a,""b""_000"');
  });

  it("never logs values beyond the free cap", () => {
    const many: ReadBlock[] = [{ name: "m", address: 0, quantity: 25, type: "uint16", area: "holding" }];
    const plan = effectivePlan(many, false);
    const rows = buildTableRows(many, plan, plan.tags.map(() => ({ Float64: 7 })), { useOneBased: false, displayFormat: "dec" });
    expect(rows).toHaveLength(25);
    expect(csvLogLines("c", 0, rows).trim().split("\n")).toHaveLength(20);
  });

  it("names files by start time without characters a file system dislikes", () => {
    expect(csvLogFileName(new Date("2026-10-21T08:30:05Z"))).toBe("modbus-log-2026-10-21T08-30-05.csv");
  });
});
