import { describe, expect, it } from "vitest";
import { CPU_PRESETS, CUSTOM_CPU, cpuAccessHint, cpuPresetFor } from "../phone/cpuPresets";
import { blockKindName, byteRows, dbWindowAddresses, groupBlocks } from "../phone/blockGroups";

describe("cpuPresetFor", () => {
  it("names the family a rack and slot belong to", () => {
    expect(cpuPresetFor(0, 1)).toBe("s7-1200-1500");
    expect(cpuPresetFor(0, 2)).toBe("s7-300");
    expect(cpuPresetFor(0, 3)).toBe("s7-400");
  });

  it("falls back to custom for anything else", () => {
    expect(cpuPresetFor(1, 1)).toBe(CUSTOM_CPU);
    expect(cpuPresetFor(0, 0)).toBe(CUSTOM_CPU);
  });

  it("round-trips every preset", () => {
    for (const p of CPU_PRESETS) expect(cpuPresetFor(p.rack, p.slot)).toBe(p.id);
  });

  it("warns about PUT/GET only for S7-1200 and S7-1500", () => {
    expect(cpuAccessHint("s7-1200-1500")).toContain("PUT/GET");
    expect(cpuAccessHint(CUSTOM_CPU)).toBe("");
  });
});

describe("groupBlocks", () => {
  const blocks = [
    { block_type: "DB", block_number: 20 },
    { block_type: "OB", block_number: 100 },
    { block_type: "DB", block_number: 1 },
    { block_type: "FC", block_number: 12 },
    { block_type: "OB", block_number: 1 },
    { block_type: "SDB", block_number: 0 },
    { block_type: "FB", block_number: 10 },
  ];

  it("orders types OB, FC, FB, DB, then the rest", () => {
    expect(groupBlocks(blocks).map((g) => g.type)).toEqual(["OB", "FC", "FB", "DB", "SDB"]);
  });

  it("sorts the numbers inside a type", () => {
    const db = groupBlocks(blocks).find((g) => g.type === "DB")!;
    expect(db.blocks.map((b) => b.block_number)).toEqual([1, 20]);
    expect(db.name).toBe("Data blocks");
  });

  it("keeps the type before a slash and names unknown types as they are", () => {
    const groups = groupBlocks([{ block_type: "DB/Instance", block_number: 3 }, { block_type: "XYZ", block_number: 1 }]);
    expect(groups.map((g) => g.type)).toEqual(["DB", "XYZ"]);
    expect(blockKindName("XYZ")).toBe("XYZ");
  });

  it("returns nothing for no blocks", () => {
    expect(groupBlocks([])).toEqual([]);
  });
});

describe("data block window", () => {
  it("lists byte addresses from the start offset", () => {
    expect(dbWindowAddresses(10, 32, 3)).toEqual(["DB10.DBB32", "DB10.DBB33", "DB10.DBB34"]);
  });

  it("shows bytes as hex rows of four and marks a missed byte", () => {
    expect(byteRows(0, [0, 31, 160, 60, 255, "x"])).toEqual([
      [0, "00 1F A0 3C"],
      [4, "FF --"],
    ]);
  });
});
