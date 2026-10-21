import { describe, it, expect } from "vitest";
import { formatValue, displayAddr, extractRaw } from "../utils/format";

describe("formatValue", () => {
  it("returns '---' for NaN", () => {
    expect(formatValue(NaN, "uint16", "dec")).toBe("---");
  });

  it("formats uint16 decimal", () => {
    expect(formatValue(1000, "uint16", "dec")).toBe("1000");
  });

  it("formats uint16 zero", () => {
    expect(formatValue(0, "uint16", "dec")).toBe("0");
  });

  it("formats float32 with precision 4", () => {
    expect(formatValue(220.5, "float32", "dec")).toBe("220.5");
  });

  it("formats float64 with precision 15", () => {
    const pi = 3.141592653589793;
    const result = formatValue(pi, "float64", "dec");
    expect(result).toBe(pi.toPrecision(15));
  });

  it("formats uint16 hex", () => {
    expect(formatValue(255, "uint16", "hex")).toBe("0x00FF");
  });

  it("formats float32 hex with 8 chars", () => {
    const result = formatValue(220.5, "float32", "hex");
    expect(result).toMatch(/^0x[0-9A-F]{8}$/);
  });

  it("formats uint16 binary with 16 chars", () => {
    const result = formatValue(5, "uint16", "bin");
    expect(result).toBe("0000000000000101");
    expect(result).toHaveLength(16);
  });

  it("formats float32 binary with 32 chars", () => {
    const result = formatValue(220.5, "float32", "bin");
    expect(result).toHaveLength(32);
  });

  it("passes through string type", () => {
    expect(formatValue(42, "string", "dec")).toBe("42");
  });

  it("formats integer float64 without decimal", () => {
    expect(formatValue(100, "float64", "dec")).toBe("100");
  });

  it("formats negative int16 decimal", () => {
    // -1 in uint16 is 0xFFFF = 65535, but the raw value passed is the JS number
    expect(formatValue(-1, "int16", "dec")).toBe("-1");
  });

  it("formats int32 hex with 8 chars", () => {
    const result = formatValue(1000, "int32", "hex");
    expect(result).toMatch(/^0x[0-9A-F]{8}$/);
  });

  it("formats float64 hex with 8 chars", () => {
    const result = formatValue(100, "float64", "hex");
    expect(result).toMatch(/^0x[0-9A-F]{8}$/);
  });

  it("formats uint16 max hex", () => {
    expect(formatValue(65535, "uint16", "hex")).toBe("0xFFFF");
  });
});

describe("displayAddr", () => {
  it("holding + 0-based = 40001", () => {
    expect(displayAddr(0, "holding", true)).toBe("40001");
  });

  it("holding + 0-based offset 5 = 40006", () => {
    expect(displayAddr(5, "holding", true)).toBe("40006");
  });

  it("input + 0-based = 30001", () => {
    expect(displayAddr(0, "input", true)).toBe("30001");
  });

  it("input offset 5 = 30006", () => {
    expect(displayAddr(5, "input", true)).toBe("30006");
  });

  it("coil + 0-based = 1", () => {
    expect(displayAddr(0, "coil", true)).toBe("1");
  });

  it("discrete + 0-based = 10001", () => {
    expect(displayAddr(0, "discrete", true)).toBe("10001");
  });

  it("zero-based returns raw address", () => {
    expect(displayAddr(5, "holding", false)).toBe("5");
  });

  it("zero-based coil returns raw address", () => {
    expect(displayAddr(10, "coil", false)).toBe("10");
  });

  it("unknown area with one-based returns base + 1", () => {
    expect(displayAddr(5, "unknown", true)).toBe("6");
  });

  it("holding address 0 zero-based returns '0'", () => {
    expect(displayAddr(0, "holding", false)).toBe("0");
  });
});

describe("extractRaw", () => {
  it("returns number as-is", () => {
    expect(extractRaw(42)).toBe(42);
  });

  it("parses string number", () => {
    expect(extractRaw("100")).toBe(100);
  });

  it("extracts number from object value", () => {
    expect(extractRaw({ Float64: 3.14 })).toBe(3.14);
  });

  it("returns NaN for null", () => {
    expect(extractRaw(null)).toBeNaN();
  });

  it("returns NaN for undefined", () => {
    expect(extractRaw(undefined)).toBeNaN();
  });

  it("returns 0 for numeric 0", () => {
    expect(extractRaw(0)).toBe(0);
  });

  it("extracts first number from multi-key object", () => {
    expect(extractRaw({ foo: "bar", val: 99 })).toBe(99);
  });

  it("returns NaN for empty object", () => {
    expect(extractRaw({})).toBeNaN();
  });
});
