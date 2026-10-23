import { describe, expect, it } from "vitest";
import { defaultBlocks, FREE_VALUES } from "../constants";
import { buildReadPlan, validateBlock } from "../helpers";
import { effectivePlan } from "../limits";
import { useStore } from "../store";
import { BUILTIN_PROFILES } from "../templates";

describe("default read blocks of a fresh install", () => {
  it("is one generic block: ten holding registers from address 0", () => {
    expect(useStore.getState().blocks).toEqual([{ name: "Registers", address: 0, quantity: 10, type: "uint16", area: "holding" }]);
  });

  it("does not ask a device for the PM5560 map (holding 2999..3034 gets Illegal Data Address from any other device)", () => {
    const plan = buildReadPlan(useStore.getState().blocks);
    expect(plan.tags.map((t) => t.address)).toEqual([0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
    expect(plan.tags.every((t) => t.address < 2999)).toBe(true);
  });

  it("can be polled and fits the free cap", () => {
    const [block] = defaultBlocks();
    expect(validateBlock(block)).toBeNull();
    expect(effectivePlan(defaultBlocks(), false).tags).toHaveLength(10);
    expect(10).toBeLessThanOrEqual(FREE_VALUES);
  });

  it("hands out a fresh copy every time, so editing one connection's blocks never edits the default", () => {
    const a = defaultBlocks();
    a[0].quantity = 99;
    expect(defaultBlocks()[0].quantity).toBe(10);
  });

  it("keeps the PM5560 map as a slave simulator template the user picks", () => {
    const pm = BUILTIN_PROFILES.find((p) => p.id === "pm5560");
    expect(pm?.baseAddress).toBe(2999);
  });
});
