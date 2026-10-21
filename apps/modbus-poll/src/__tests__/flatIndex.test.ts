import { describe, it, expect } from "vitest";
import { typeStep } from "../helpers";

/**
 * Tests for the flatIndex calculation logic used in App.tsx tableData.
 * The formula: blocks.slice(0, blockIdx).reduce((sum, prev) => sum + prev.quantity * typeStep(prev.type), 0) + i * step
 * This replaces the old buggy: reduce((sum, prev) => sum + prev.quantity, 0) + i
 */

interface Block {
  name: string;
  address: number;
  quantity: number;
  type: string;
  area: string;
}

/**
 * Reproduces the flatIndex calculation from App.tsx tableData.
 * This tests the CORRECTED formula.
 */
function computeFlatIndex(blocks: Block[], blockIdx: number, i: number): number {
  const step = typeStep(blocks[blockIdx].type);
  return blocks.slice(0, blockIdx).reduce((sum, prev) => sum + prev.quantity * typeStep(prev.type), 0) + i * step;
}

/**
 * The old BUGGY formula for comparison.
 */
function computeFlatIndexOld(blocks: Block[], blockIdx: number, i: number): number {
  return blocks.slice(0, blockIdx).reduce((sum, prev) => sum + prev.quantity, 0) + i;
}

describe("flatIndex calculation", () => {
  it("single uint16 block — old and new agree", () => {
    const blocks: Block[] = [
      { name: "v", address: 0, quantity: 3, type: "uint16", area: "holding" },
    ];
    // 3 values, each 1 register
    expect(computeFlatIndex(blocks, 0, 0)).toBe(0);
    expect(computeFlatIndex(blocks, 0, 1)).toBe(1);
    expect(computeFlatIndex(blocks, 0, 2)).toBe(2);
    // Old formula gives same result for uint16
    expect(computeFlatIndexOld(blocks, 0, 0)).toBe(0);
    expect(computeFlatIndexOld(blocks, 0, 2)).toBe(2);
  });

  it("single float32 block — new formula accounts for step=2", () => {
    const blocks: Block[] = [
      { name: "voltage", address: 0, quantity: 3, type: "float32", area: "holding" },
    ];
    // 3 values, each 2 registers → offsets 0, 2, 4
    expect(computeFlatIndex(blocks, 0, 0)).toBe(0);
    expect(computeFlatIndex(blocks, 0, 1)).toBe(2);
    expect(computeFlatIndex(blocks, 0, 2)).toBe(4);
    // Old formula gave wrong: 0, 1, 2
    expect(computeFlatIndexOld(blocks, 0, 1)).toBe(1); // WRONG
    expect(computeFlatIndexOld(blocks, 0, 2)).toBe(2); // WRONG
  });

  it("uint16 block followed by float32 block — correct offset", () => {
    const blocks: Block[] = [
      { name: "status", address: 0, quantity: 2, type: "uint16", area: "holding" },
      { name: "voltage", address: 2, quantity: 3, type: "float32", area: "holding" },
    ];
    // Block 0: 2 uint16 values = 2 registers (indices 0, 1)
    // Block 1 starts at flat index 2
    expect(computeFlatIndex(blocks, 1, 0)).toBe(2);
    expect(computeFlatIndex(blocks, 1, 1)).toBe(4);
    expect(computeFlatIndex(blocks, 1, 2)).toBe(6);
    // Old formula: block 0 contributes quantity=2, so start at 2, then +0,+1,+2
    expect(computeFlatIndexOld(blocks, 1, 0)).toBe(2);  // happens to be correct
    expect(computeFlatIndexOld(blocks, 1, 1)).toBe(3);  // WRONG — should be 4
    expect(computeFlatIndexOld(blocks, 1, 2)).toBe(4);  // WRONG — should be 6
  });

  it("float32 block followed by uint16 block — correct offset", () => {
    const blocks: Block[] = [
      { name: "voltage", address: 0, quantity: 3, type: "float32", area: "holding" },
      { name: "status", address: 6, quantity: 2, type: "uint16", area: "holding" },
    ];
    // Block 0: 3 float32 values = 6 registers (indices 0,2,4 with step)
    // Block 1 starts at flat index 6
    expect(computeFlatIndex(blocks, 1, 0)).toBe(6);
    expect(computeFlatIndex(blocks, 1, 1)).toBe(7);
    // Old formula: block 0 contributes quantity=3, so start at 3
    expect(computeFlatIndexOld(blocks, 1, 0)).toBe(3);  // WRONG — should be 6
    expect(computeFlatIndexOld(blocks, 1, 1)).toBe(4);  // WRONG — should be 7
  });

  it("float64 block — step=4 per value", () => {
    const blocks: Block[] = [
      { name: "energy", address: 0, quantity: 2, type: "float64", area: "holding" },
    ];
    expect(computeFlatIndex(blocks, 0, 0)).toBe(0);
    expect(computeFlatIndex(blocks, 0, 1)).toBe(4);
  });

  it("mixed blocks: float32 + float64 + uint16", () => {
    const blocks: Block[] = [
      { name: "voltage", address: 0, quantity: 2, type: "float32", area: "holding" },
      { name: "energy", address: 4, quantity: 1, type: "float64", area: "holding" },
      { name: "status", address: 8, quantity: 3, type: "uint16", area: "holding" },
    ];
    // Block 0: 2×2 = 4 registers
    // Block 1: starts at 4, 1×4 = 4 registers
    // Block 2: starts at 8, 3×1 = 3 registers
    expect(computeFlatIndex(blocks, 0, 0)).toBe(0);
    expect(computeFlatIndex(blocks, 0, 1)).toBe(2);
    expect(computeFlatIndex(blocks, 1, 0)).toBe(4);
    expect(computeFlatIndex(blocks, 2, 0)).toBe(8);
    expect(computeFlatIndex(blocks, 2, 1)).toBe(9);
    expect(computeFlatIndex(blocks, 2, 2)).toBe(10);
  });

  it("total register count matches sum of flatIndex + step for last element", () => {
    const blocks: Block[] = [
      { name: "voltage", address: 0, quantity: 3, type: "float32", area: "holding" },
      { name: "current", address: 6, quantity: 2, type: "float32", area: "holding" },
    ];
    // Total registers = 3×2 + 2×2 = 10
    const lastIdx = computeFlatIndex(blocks, 1, 1);
    const step = typeStep(blocks[1].type);
    expect(lastIdx + step).toBe(10);
  });
});

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
