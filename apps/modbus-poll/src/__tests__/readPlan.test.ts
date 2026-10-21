import { describe, it, expect } from "vitest";
import { blockStep, blockType, buildReadPlan, readPlanKey, validateBlock } from "../helpers";
import { buildTableRows } from "../utils/rows";
import type { ReadBlock } from "../types";

// The app's default blocks (PM5560 demo against the built-in simulator).
const DEFAULT_BLOCKS: ReadBlock[] = [
  { name: "Voltage", address: 2999, quantity: 3, type: "float32", area: "holding" },
  { name: "Current", address: 3005, quantity: 3, type: "float32", area: "holding" },
  { name: "Power", address: 3011, quantity: 3, type: "float32", area: "holding" },
  { name: "Power Quality", address: 3017, quantity: 4, type: "float32", area: "holding" },
  { name: "Temperature", address: 3025, quantity: 1, type: "float32", area: "holding" },
  { name: "Energy", address: 3027, quantity: 2, type: "float64", area: "holding" },
];

const opts = { useOneBased: false, displayFormat: "dec" as const };

/** What `read_tags` returns: one serialized ModbusValue per tag. */
const f64 = (n: number) => ({ Float64: n });

describe("buildReadPlan", () => {
  it("emits one tag per value, addressed by value step, not one per register", () => {
    const plan = buildReadPlan(DEFAULT_BLOCKS);
    expect(plan.tags).toHaveLength(3 + 3 + 3 + 4 + 1 + 2);
    expect(plan.tags.slice(0, 3).map((t) => t.address)).toEqual([2999, 3001, 3003]);
    expect(plan.tags.slice(0, 3).map((t) => t.name)).toEqual(["Voltage_2999", "Voltage_3001", "Voltage_3003"]);
    // float64 values are 4 registers apart
    expect(plan.tags.slice(-2).map((t) => t.address)).toEqual([3027, 3031]);
  });

  it("firstTag is an index into the tag list (and so into the returned values)", () => {
    const plan = buildReadPlan(DEFAULT_BLOCKS);
    // Register-offset arithmetic (the old bug) would give 6, 12, 18 … here.
    expect(plan.firstTag).toEqual([0, 3, 6, 9, 13, 14]);
  });

  it("mixed widths: indices count values, not registers", () => {
    const blocks: ReadBlock[] = [
      { name: "a", address: 0, quantity: 2, type: "uint16", area: "holding" },
      { name: "b", address: 2, quantity: 3, type: "float32", area: "holding" },
      { name: "c", address: 8, quantity: 1, type: "float64", area: "input" },
      { name: "d", address: 20, quantity: 2, type: "int32", area: "holding" },
    ];
    const plan = buildReadPlan(blocks);
    expect(plan.firstTag).toEqual([0, 2, 5, 6]);
    expect(plan.tags[plan.firstTag[3] + 1]).toMatchObject({ name: "d_022", address: 22, type: "int32" });
    expect(plan.tags[plan.firstTag[2]]).toMatchObject({ register_area: "input", type: "float64", address: 8 });
  });

  it("coil and discrete blocks are single bits whatever type was stored", () => {
    const blocks: ReadBlock[] = [{ name: "c", address: 10, quantity: 3, type: "float32", area: "coil" }];
    expect(blockType(blocks[0])).toBe("uint16");
    expect(blockStep(blocks[0])).toBe(1);
    expect(buildReadPlan(blocks).tags.map((t) => t.address)).toEqual([10, 11, 12]);
  });

  it("plan key changes whenever the tag list changes", () => {
    const a = readPlanKey(buildReadPlan(DEFAULT_BLOCKS));
    expect(readPlanKey(buildReadPlan([...DEFAULT_BLOCKS]))).toBe(a);
    expect(readPlanKey(buildReadPlan(DEFAULT_BLOCKS.slice(1)))).not.toBe(a);
    expect(readPlanKey(buildReadPlan([{ ...DEFAULT_BLOCKS[0], type: "int32" }, ...DEFAULT_BLOCKS.slice(1)]))).not.toBe(a);
  });
});

describe("buildTableRows", () => {
  it("shows every value under its own name (default blocks against the PM5560 simulator)", () => {
    const plan = buildReadPlan(DEFAULT_BLOCKS);
    // The simulator's profile order: V1 V2 V3, I1 I2 I3, P Q S, PF Hz THDv THDi, T, Energy in/out.
    const values = [
      230.2, 229.8, 230.5,
      142.5, 138.7, 145.3,
      95000, -12000, 98000,
      0.96, 50, 1.8, 6.2,
      38.5,
      1587329, 2451,
    ].map(f64);
    const rows = buildTableRows(DEFAULT_BLOCKS, plan, values, opts);
    const byName = Object.fromEntries(rows.map((r) => [r.name, r.rawValue]));
    expect(byName.Voltage_2999).toBe(230.2);
    expect(byName.Voltage_3001).toBe(229.8);
    expect(byName.Voltage_3003).toBe(230.5);
    expect(byName.Current_3005).toBe(142.5);
    expect(byName.Power_3011).toBe(95000);
    expect(byName["Power Quality_3019"]).toBe(50);
    expect(byName.Temperature_3025).toBe(38.5);
    expect(byName.Energy_3027).toBe(1587329);
    expect(byName.Energy_3031).toBe(2451);
    expect(rows.every((r) => r.rawValue !== null)).toBe(true);
  });

  it("rows beyond the polled values are empty, not shifted", () => {
    const plan = buildReadPlan(DEFAULT_BLOCKS);
    const rows = buildTableRows(DEFAULT_BLOCKS, plan, [f64(1), f64(2)], opts);
    expect(rows[0].rawValue).toBe(1);
    expect(rows[1].rawValue).toBe(2);
    expect(rows[2].rawValue).toBeNull();
    expect(rows[2].value).toBe("—");
  });

  it("carries the raw number regardless of the display format", () => {
    const blocks: ReadBlock[] = [{ name: "u", address: 0, quantity: 1, type: "uint16", area: "holding" }];
    const plan = buildReadPlan(blocks);
    const hex = buildTableRows(blocks, plan, [f64(255)], { ...opts, displayFormat: "hex" })[0];
    const bin = buildTableRows(blocks, plan, [f64(255)], { ...opts, displayFormat: "bin" })[0];
    expect(hex.value).toBe("0x00FF");
    expect(bin.value).toBe("0000000011111111");
    expect(hex.rawValue).toBe(255);
    expect(bin.rawValue).toBe(255);
  });

  it("surfaces a device exception on the rows it covers", () => {
    const blocks: ReadBlock[] = [{ name: "u", address: 64, quantity: 2, type: "uint16", area: "holding" }];
    const message = "Illegal Data Address (0x02) reading holding 64..65";
    const rows = buildTableRows(blocks, buildReadPlan(blocks), [{ Failed: message }, f64(7)], opts);
    expect(rows[0].error).toBe(message);
    expect(rows[0].rawValue).toBeNull();
    expect(rows[1].error).toBeUndefined();
    expect(rows[1].rawValue).toBe(7);
  });

  it("coil rows read ON/OFF and keep 0/1 as the raw value", () => {
    const blocks: ReadBlock[] = [{ name: "c", address: 0, quantity: 2, type: "uint16", area: "coil" }];
    const rows = buildTableRows(blocks, buildReadPlan(blocks), [f64(1), f64(0)], opts);
    expect(rows.map((r) => r.value)).toEqual(["ON", "OFF"]);
    expect(rows.map((r) => r.rawValue)).toEqual([1, 0]);
    expect(rows[0].type).toBe("bit");
  });

  it("1-based display addresses follow the area", () => {
    const blocks: ReadBlock[] = [{ name: "u", address: 0, quantity: 1, type: "uint16", area: "holding" }];
    const rows = buildTableRows(blocks, buildReadPlan(blocks), [f64(1)], { ...opts, useOneBased: true });
    expect(rows[0].address).toBe("40001");
    expect(rows[0].rawAddress).toBe(0);
  });
});

describe("validateBlock", () => {
  const ok: ReadBlock = { name: "x", address: 0, quantity: 1, type: "uint16", area: "holding" };

  it("accepts a plain block", () => {
    expect(validateBlock(ok)).toBeNull();
  });

  it("rejects the unsupported string type", () => {
    expect(validateBlock({ ...ok, type: "string" })).toMatch(/Unsupported type/);
  });

  it("rejects unknown areas, bad numbers and ranges past 65535", () => {
    expect(validateBlock({ ...ok, area: "register" })).toMatch(/Unsupported area/);
    expect(validateBlock({ ...ok, address: NaN })).not.toBeNull();
    expect(validateBlock({ ...ok, address: -1 })).not.toBeNull();
    expect(validateBlock({ ...ok, quantity: 0 })).not.toBeNull();
    expect(validateBlock({ ...ok, quantity: 1.5 })).not.toBeNull();
    expect(validateBlock({ ...ok, name: "  " })).not.toBeNull();
    expect(validateBlock({ ...ok, address: 65535, type: "float32" })).toMatch(/past address 65535/);
    expect(validateBlock({ ...ok, address: 65534, type: "float32" })).toBeNull();
  });
});
