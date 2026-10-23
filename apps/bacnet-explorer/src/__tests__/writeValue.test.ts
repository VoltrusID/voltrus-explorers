import { describe, expect, it } from "vitest";
import { PRIORITY_OPTIONS, RELINQUISH_VALUE, parsePriority, parseWriteValue } from "../writeValue";

describe("parsePriority", () => {
  it("treats the empty choice as the device default", () => {
    expect(parsePriority("")).toBeNull();
  });

  it("accepts 1 to 16", () => {
    expect(parsePriority("1")).toBe(1);
    expect(parsePriority("8")).toBe(8);
    expect(parsePriority("16")).toBe(16);
  });

  it("rejects anything else", () => {
    for (const bad of ["0", "17", "-1", "2.5", "abc"]) {
      expect(() => parsePriority(bad), bad).toThrow(/between 1 and 16/);
    }
  });
});

describe("PRIORITY_OPTIONS", () => {
  it("offers the device default and every priority once", () => {
    expect(PRIORITY_OPTIONS.map((o) => o.value)).toEqual(["", ...Array.from({ length: 16 }, (_, i) => String(i + 1))]);
  });

  it("only offers values parsePriority accepts", () => {
    for (const option of PRIORITY_OPTIONS) expect(() => parsePriority(option.value)).not.toThrow();
  });
});

describe("relinquish", () => {
  it("writes Null, the value that clears a priority slot", () => {
    expect(RELINQUISH_VALUE).toEqual({ Null: null });
  });

  it("is not something a typed value can produce", () => {
    expect(parseWriteValue("real", "21.5")).toEqual({ Real: 21.5 });
    expect(() => parseWriteValue("real", "")).toThrow();
  });
});
