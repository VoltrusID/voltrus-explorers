import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ValueLogWriter, valueLogHeader, valueLogRow } from "../valueLog";

const disk = vi.hoisted(() => ({ appended: [] as string[], failWrites: false }));

vi.mock("@mactools/ui", () => ({
  appendTextFile: async (_path: string, text: string) => {
    if (disk.failWrites) throw new Error("disk full");
    disk.appended.push(text);
  },
}));

describe("value log format", () => {
  it("names a column per tag and disambiguates duplicate names by address", () => {
    const tags = [
      { id: "1", name: "Temp", address: "DB1.DBD0", data_type: "Real" },
      { id: "2", name: "Temp", address: "DB2.DBD0", data_type: "Real" },
      { id: "3", name: "Run, state", address: "I0.0", data_type: "Bool" },
    ];
    expect(valueLogHeader(tags)).toBe('timestamp,Temp [DB1.DBD0],Temp [DB2.DBD0],"Run, state"');
  });

  it("writes a UTC timestamp then each value, booleans as 1 and 0", () => {
    const row = valueLogRow(new Date("2026-10-21T04:01:31.522Z"), [12.5, true, false, 7, "a,b"]);
    expect(row).toBe('2026-10-21T04:01:31.522Z,12.5,1,0,7,"a,b"');
  });
});

describe("ValueLogWriter", () => {
  beforeEach(() => {
    disk.appended.length = 0;
    disk.failWrites = false;
    vi.useFakeTimers();
  });
  afterEach(() => vi.useRealTimers());

  it("batches rows into one append per second", async () => {
    const writer = new ValueLogWriter("/tmp/log.csv", "sig", () => {});
    writer.push("a");
    writer.push("b");
    expect(disk.appended).toEqual([]);
    await vi.advanceTimersByTimeAsync(1000);
    expect(disk.appended).toEqual(["a\nb\n"]);
    expect(writer.rows).toBe(2);
  });

  it("flushes what is buffered on close and then ignores rows", async () => {
    const writer = new ValueLogWriter("/tmp/log.csv", "sig", () => {});
    writer.push("a");
    await writer.close();
    writer.push("late");
    await writer.flush();
    expect(disk.appended).toEqual(["a\n"]);
  });

  it("reports a failed write once and closes", async () => {
    disk.failWrites = true;
    const errors: unknown[] = [];
    const writer = new ValueLogWriter("/tmp/log.csv", "sig", (e) => errors.push(e));
    writer.push("a");
    await writer.flush();
    writer.push("b");
    await writer.flush();
    expect(errors).toHaveLength(1);
    expect(writer.closed).toBe(true);
  });
});
