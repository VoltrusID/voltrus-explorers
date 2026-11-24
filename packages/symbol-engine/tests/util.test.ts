import { describe, expect, it } from "vitest";
import { clamp, newId } from "../src";

describe("newId", () => {
  it("returns opaque string ids", () => {
    const id = newId();
    expect(typeof id).toBe("string");
    expect(id.length).toBeGreaterThan(0);
  });

  it("generates unique ids", () => {
    const ids = new Set(Array.from({ length: 1000 }, () => newId()));
    expect(ids.size).toBe(1000);
  });
});

describe("clamp", () => {
  it("keeps in-range values unchanged", () => {
    expect(clamp(5, 0, 10)).toBe(5);
    expect(clamp(0, 0, 10)).toBe(0);
    expect(clamp(10, 0, 10)).toBe(10);
  });

  it("clamps below the low bound", () => {
    expect(clamp(-1, 0, 10)).toBe(0);
  });

  it("clamps above the high bound", () => {
    expect(clamp(11, 0, 10)).toBe(10);
  });
});
