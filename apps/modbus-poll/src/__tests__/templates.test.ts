import { describe, it, expect, beforeEach } from "vitest";
import {
  type DeviceProfile,
  type RegisterDef,
  initialValues,
  numRegisters,
  decodeRegisters,
  loadCustomTemplates,
  saveCustomTemplates,
} from "../templates";

// Minimal localStorage mock
const store: Record<string, string> = {};
beforeEach(() => {
  for (const key of Object.keys(store)) delete store[key];
});
Object.defineProperty(globalThis, "localStorage", {
  value: {
    getItem: (key: string) => store[key] ?? null,
    setItem: (key: string, value: string) => { store[key] = value; },
    removeItem: (key: string) => { delete store[key]; },
    clear: () => { for (const key of Object.keys(store)) delete store[key]; },
    get length() { return Object.keys(store).length; },
    key: (index: number) => Object.keys(store)[index] ?? null,
  },
  configurable: true,
});

function floatReg(name: string, initial: number, min: number, max: number, drift: number): RegisterDef {
  return { name, unit: "V", description: "", initial, min, max, drift, isCounter: false, counterRate: 0 };
}

function counterReg(name: string, initial: number, rate: number): RegisterDef {
  return { name, unit: "kWh", description: "", initial, min: 0, max: 999999, drift: 0, isCounter: true, counterRate: rate };
}

function makeProfile(registers: RegisterDef[], baseAddress = 0): DeviceProfile {
  return { id: "test", manufacturer: "Test", model: "Test", description: "Test", baseAddress, registers };
}

describe("initialValues", () => {
  it("returns initial values from profile", () => {
    const profile = makeProfile([floatReg("v1", 100, 0, 200, 1), floatReg("v2", 50, 0, 100, 0.5)]);
    expect(initialValues(profile)).toEqual([100, 50]);
  });

  it("returns empty array for profile with no registers", () => {
    const profile = makeProfile([]);
    expect(initialValues(profile)).toEqual([]);
  });
});

describe("numRegisters", () => {
  it("calculates 2 float32 + 1 counter = 8", () => {
    const profile = makeProfile([
      floatReg("v1", 0, 0, 100, 1),
      floatReg("v2", 0, 0, 100, 1),
      counterReg("energy", 0, 10),
    ]);
    expect(numRegisters(profile)).toBe(8);
  });

  it("single float = 2 registers", () => {
    const profile = makeProfile([floatReg("v", 0, 0, 100, 1)]);
    expect(numRegisters(profile)).toBe(2);
  });

  it("single counter = 4 registers", () => {
    const profile = makeProfile([counterReg("energy", 0, 10)]);
    expect(numRegisters(profile)).toBe(4);
  });

  it("empty profile = 0 registers", () => {
    expect(numRegisters(makeProfile([]))).toBe(0);
  });
});

describe("decodeRegisters", () => {
  it("round-trips float32 value", () => {
    const profile = makeProfile([floatReg("v", 0, 0, 300, 0)], 0);
    const buf = new ArrayBuffer(4);
    new Float32Array(buf)[0] = 220.5;
    const bits = new Uint32Array(buf)[0];
    const regs = [(bits >>> 16) & 0xffff, bits & 0xffff];
    const decoded = decodeRegisters(regs, profile);
    expect(decoded).toHaveLength(1);
    expect(decoded[0].name).toBe("v");
    expect(decoded[0].count).toBe(2);
    expect(decoded[0].decoded).toContain("220.5");
  });

  it("decodes a counter as big-endian float64 over 4 registers", () => {
    const profile = makeProfile([counterReg("energy", 0, 10)], 0);
    // 1234567.5 = 0x4132D68780000000 as IEEE 754 binary64, written AB CD EF GH:
    // 0x4132 0xD687 0x8000 0x0000
    const regs = [0x4132, 0xD687, 0x8000, 0x0000];
    const decoded = decodeRegisters(regs, profile);
    expect(decoded).toHaveLength(1);
    expect(decoded[0].count).toBe(4);
    expect(decoded[0].type).toBe("Float64 Counter BE");
    expect(decoded[0].decoded.replace(/[^0-9.]/g, "")).toBe("1234567.5");
  });

  it("matches the Rust simulator's counter layout (1000000.0 = 412E 8480 0000 0000)", () => {
    const profile = makeProfile([counterReg("energy", 0, 10)], 0);
    const decoded = decodeRegisters([0x412E, 0x8480, 0, 0], profile);
    expect(decoded[0].decoded.replace(/[^0-9.]/g, "")).toBe("1000000");
  });

  it("handles zero float32", () => {
    const profile = makeProfile([floatReg("v", 0, 0, 100, 0)], 0);
    const decoded = decodeRegisters([0, 0], profile);
    expect(decoded[0].decoded).toContain("0.0");
  });

  it("handles missing registers with zero defaults", () => {
    const profile = makeProfile([floatReg("v", 0, 0, 100, 0)], 0);
    // Empty register array — should still decode without crash
    const decoded = decodeRegisters([], profile);
    expect(decoded).toHaveLength(1);
  });
});

describe("custom template storage", () => {
  it("returns empty array when nothing stored", () => {
    expect(loadCustomTemplates()).toEqual([]);
  });

  it("round-trips templates", () => {
    const templates = [makeProfile([floatReg("v", 1, 0, 10, 0.1)])];
    saveCustomTemplates(templates);
    const loaded = loadCustomTemplates();
    expect(loaded).toHaveLength(1);
    expect(loaded[0].id).toBe("test");
    expect(loaded[0].registers).toHaveLength(1);
  });

  it("overwrites previous templates on save", () => {
    saveCustomTemplates([makeProfile([floatReg("v1", 0, 0, 10, 0)])]);
    saveCustomTemplates([makeProfile([floatReg("v2", 0, 0, 10, 0)]), makeProfile([floatReg("v3", 0, 0, 10, 0)])]);
    const loaded = loadCustomTemplates();
    expect(loaded).toHaveLength(2);
  });
});
