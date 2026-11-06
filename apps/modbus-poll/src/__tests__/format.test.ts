import { describe, it, expect } from "vitest";
import { formatValue, displayAddr, extractRaw, extractError, registerBits } from "../utils/format";

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

  it("formats float64 hex as its 64 IEEE 754 bits", () => {
    expect(formatValue(100, "float64", "hex")).toBe("0x4059000000000000");
  });

  it("formats float32 hex and binary as its IEEE 754 bits, not the integer part", () => {
    // 220.5f32 = 0x435C8000
    expect(formatValue(220.5, "float32", "hex")).toBe("0x435C8000");
    expect(formatValue(220.5, "float32", "bin")).toBe("01000011010111001000000000000000");
  });

  it("formats uint32 above the int32 range in hex and decimal", () => {
    expect(formatValue(4275878552, "uint32", "hex")).toBe("0xFEDCBA98");
    expect(formatValue(4275878552, "uint32", "dec")).toBe("4275878552");
    expect(formatValue(4275878552, "uint32", "bin")).toHaveLength(32);
  });

  it("formats a negative int16 in hex as two's complement", () => {
    expect(formatValue(-2, "int16", "hex")).toBe("0xFFFE");
  });

  it("formats uint16 max hex", () => {
    expect(formatValue(65535, "uint16", "hex")).toBe("0xFFFF");
  });
});

describe("formatValue: ASCII", () => {
  it("shows two characters per register, high byte first", () => {
    // 0x4142 = "AB"
    expect(formatValue(0x4142, "uint16", "ascii")).toBe('"AB"');
  });

  it("shows four characters for a 32-bit value", () => {
    // 0x54454D50 = "TEMP"
    expect(formatValue(0x54454d50, "uint32", "ascii")).toBe('"TEMP"');
    expect(formatValue(0x54454d50, "int32", "ascii")).toBe('"TEMP"');
  });

  it("marks bytes outside printable ASCII", () => {
    expect(formatValue(0x4100, "uint16", "ascii")).toBe('"A·"');
    expect(formatValue(0, "uint16", "ascii")).toBe('"··"');
  });

  it("reads a negative int16 as its register bytes", () => {
    // -1 = 0xFFFF
    expect(formatValue(-1, "int16", "ascii")).toBe('"··"');
  });

  it("shows eight characters for a float64", () => {
    expect(formatValue(100, "float64", "ascii")).toHaveLength(10);
  });
});

describe("formatValue: BCD", () => {
  it("reads one decimal digit per nibble", () => {
    expect(formatValue(0x1234, "uint16", "bcd")).toBe("1234");
    expect(formatValue(0x0042, "uint16", "bcd")).toBe("42");
    expect(formatValue(0, "uint16", "bcd")).toBe("0");
  });

  it("reads eight digits from a 32-bit value", () => {
    expect(formatValue(0x20261106, "uint32", "bcd")).toBe("20261106");
    expect(formatValue(0x00009999, "int32", "bcd")).toBe("9999");
  });

  it("says when a nibble is not a decimal digit", () => {
    expect(formatValue(0x12ab, "uint16", "bcd")).toBe("not BCD");
    expect(formatValue(-1, "int16", "bcd")).toBe("not BCD");
  });
});

describe("registerBits", () => {
  it("gives the width of each type", () => {
    expect(registerBits(1, "uint16").width).toBe(16);
    expect(registerBits(1, "int16").width).toBe(16);
    expect(registerBits(1, "int32").width).toBe(32);
    expect(registerBits(1, "uint32").width).toBe(32);
    expect(registerBits(1, "float32").width).toBe(32);
    expect(registerBits(1, "float64").width).toBe(64);
  });

  it("gives IEEE 754 bits for floats", () => {
    expect(registerBits(1.5, "float64").bits).toBe(0x3ff8000000000000n);
    expect(registerBits(-2, "float32").bits).toBe(0xc0000000n);
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

  it("returns NaN for a failed read (a message, not a number)", () => {
    expect(extractRaw({ Failed: "Illegal Data Address (0x02) reading holding 64..64" })).toBeNaN();
  });

  it("returns NaN for a NaN float (serialized as null)", () => {
    expect(extractRaw({ Float64: null })).toBeNaN();
  });
});

describe("extractError", () => {
  it("returns the message of a failed value", () => {
    expect(extractError({ Failed: "Illegal Function (0x01) reading input 0..0" })).toBe("Illegal Function (0x01) reading input 0..0");
  });

  it("is undefined for numeric values", () => {
    expect(extractError({ Float64: 1.5 })).toBeUndefined();
    expect(extractError(3)).toBeUndefined();
    expect(extractError(undefined)).toBeUndefined();
  });
});
