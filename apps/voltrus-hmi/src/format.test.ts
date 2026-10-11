import { describe, it, expect } from "vitest";
import { formatValue } from "./components/WidgetPreviews";

// Mirrors the exported runtime's fmtNum (export/widgets.rs num_mode).
describe("formatValue matches the runtime", () => {
  it("REAL shows one decimal", () => {
    expect(formatValue(3, "REAL")).toBe("3.0");
    expect(formatValue(2.345, "real")).toBe("2.3");
  });
  it("declared integer types show whole numbers", () => {
    expect(formatValue(42, "DINT")).toBe("42");
    expect(formatValue(41.6, "INT")).toBe("42");
  });
  it("every listed integer type rounds", () => {
    for (const t of ["BOOL", "BYTE", "SINT", "USINT", "INT", "UINT", "DINT", "UDINT", "LINT", "ULINT", "WORD", "DWORD", "LWORD", "TIME"]) {
      expect(formatValue(2.6, t)).toBe("3");
    }
  });
  it("LREAL keeps one decimal; unknown types fall back to auto", () => {
    expect(formatValue(4, "LREAL")).toBe("4.0");
    expect(formatValue(2.6, "FLOAT64")).toBe("2.6");
    expect(formatValue(5, "FLOAT64")).toBe("5");
  });
  it("untyped: integers as-is, fractions one decimal", () => {
    expect(formatValue(7)).toBe("7");
    expect(formatValue(7.25)).toBe("7.3");
  });
  it("non-numbers pass through", () => {
    expect(formatValue(true)).toBe("true");
  });
});
