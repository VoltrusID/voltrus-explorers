import { describe, expect, it } from "vitest";
import { fmtValue, quality } from "../phone/format";
import { normalizePolicy } from "../phone/policies";

describe("fmtValue", () => {
  it("trims float noise to three decimals", () => {
    expect(fmtValue(184.60000001)).toBe("184.6");
    expect(fmtValue(10.4236)).toBe("10.424");
  });
  it("keeps a whole Double at one decimal so the digits do not jump", () => {
    expect(fmtValue(185, "Double")).toBe("185.0");
    expect(fmtValue(185, "UInt32")).toBe("185");
  });
  it("spells booleans and empty values", () => {
    expect(fmtValue(true)).toBe("True");
    expect(fmtValue(false)).toBe("False");
    expect(fmtValue(null)).toBe("No value");
    expect(fmtValue("")).toBe("No value");
  });
  it("shows objects as JSON", () => {
    expect(fmtValue({ low: 0, high: 10 })).toBe('{"low":0,"high":10}');
  });
});

describe("quality", () => {
  it("waits for the first value", () => {
    expect(quality(undefined, null).label).toBe("Waiting");
  });
  it("reads a value without a status as good", () => {
    expect(quality(undefined, 3)).toEqual({ label: "Good", tone: "success" });
  });
  it("maps OPC UA status codes", () => {
    expect(quality("Good", 1).tone).toBe("success");
    expect(quality("UncertainLastUsableValue", 1).tone).toBe("warn");
    expect(quality("BadNodeIdUnknown", 1).tone).toBe("danger");
  });
});

describe("normalizePolicy", () => {
  it("maps discovery names to the form's policy names", () => {
    expect(normalizePolicy("Aes128_Sha256_RsaOaep")).toBe("Aes128Sha256RsaOaep");
    expect(normalizePolicy("Basic256Sha256")).toBe("Basic256Sha256");
    expect(normalizePolicy("None")).toBe("None");
  });
  it("passes unknown names through", () => {
    expect(normalizePolicy("Custom")).toBe("Custom");
  });
});
