import { describe, expect, it } from "vitest";
import { encodeValue, exactText, isWideInteger } from "../wide";

describe("wide integer signals in the signal editor", () => {
  const u64 = { length: 64, valueType: "unsigned" as const, factor: 1, offset: 0 };
  const scaled = { ...u64, factor: 0.5 };
  const narrow = { ...u64, length: 16 };

  it("knows which signals need exact text", () => {
    expect(isWideInteger(u64)).toBe(true);
    expect(isWideInteger({ ...u64, valueType: "signed" })).toBe(true);
    expect(isWideInteger(scaled)).toBe(false);
    expect(isWideInteger(narrow)).toBe(false);
    expect(isWideInteger({ ...u64, valueType: "double" })).toBe(false);
  });

  it("sends whole numbers of wide signals as text, everything else as numbers", () => {
    expect(encodeValue(u64, " 18446744073709551557 ")).toBe("18446744073709551557");
    expect(encodeValue({ ...u64, valueType: "signed" }, "-9007199254740993")).toBe("-9007199254740993");
    expect(encodeValue(u64, "1.5")).toBe(1.5);
    expect(encodeValue(narrow, "42")).toBe(42);
    expect(encodeValue(scaled, "1000")).toBe(1000);
    expect(encodeValue(u64, "abc")).toBeNull();
  });

  it("starts the editor from the exact decoded text", () => {
    expect(exactText(u64, "18446744073709551557")).toBe("18446744073709551557");
    expect(exactText(u64, "Ready")).toBeNull();
    expect(exactText(narrow, "12")).toBeNull();
    expect(exactText(u64, null)).toBeNull();
  });
});
