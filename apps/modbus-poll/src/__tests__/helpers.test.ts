import { describe, it, expect } from "vitest";
import { typeStep } from "../helpers";

describe("typeStep", () => {
  it("returns 1 for uint16", () => {
    expect(typeStep("uint16")).toBe(1);
  });

  it("returns 1 for int16", () => {
    expect(typeStep("int16")).toBe(1);
  });

  it("returns 2 for float32", () => {
    expect(typeStep("float32")).toBe(2);
  });

  it("returns 2 for int32", () => {
    expect(typeStep("int32")).toBe(2);
  });

  it("returns 4 for float64", () => {
    expect(typeStep("float64")).toBe(4);
  });
});
