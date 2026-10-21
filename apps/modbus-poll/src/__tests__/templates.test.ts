import { describe, it, expect, beforeEach } from "vitest";
import {
  type DeviceProfile,
  type RegisterDef,
  initialValues,
  numRegisters,
  tickValues,
  valuesToRegisters,
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

describe("tickValues", () => {
  it("counter increments", () => {
    const profile = makeProfile([counterReg("energy", 1000, 10)]);
    const values = [1000];
    const result = tickValues(profile, values, 0);
    expect(result.values[0]).toBeGreaterThan(1000);
  });

  it("clamps to min/max", () => {
    const profile = makeProfile([floatReg("v", 50, 40, 60, 10000)]);
    let values = [50];
    for (let i = 0; i < 50; i++) {
      const result = tickValues(profile, values, 0);
      values = result.values;
      expect(values[0]).toBeGreaterThanOrEqual(40);
      expect(values[0]).toBeLessThanOrEqual(60);
    }
  });

  it("advances loadFactor", () => {
    const profile = makeProfile([floatReg("v", 50, 0, 100, 0)]);
    const result = tickValues(profile, [50], 0);
    expect(result.loadFactor).not.toBe(0);
  });

  it("multiple registers tick independently", () => {
    const profile = makeProfile([
      floatReg("v", 50, 0, 100, 1),
      counterReg("energy", 0, 5),
    ]);
    const values = [50, 0];
    const result = tickValues(profile, values, 0);
    // Counter should increase
    expect(result.values[1]).toBeGreaterThan(0);
    // Float should stay in bounds
    expect(result.values[0]).toBeGreaterThanOrEqual(0);
    expect(result.values[0]).toBeLessThanOrEqual(100);
  });

  it("counter with zero rate stays near zero", () => {
    const profile = makeProfile([counterReg("energy", 0, 0)]);
    const result = tickValues(profile, [0], 0);
    // With rate 0, the variation is (random - 0.5) * 0 * 0.1 = 0
    expect(result.values[0]).toBe(0);
  });
});

describe("valuesToRegisters", () => {
  it("encodes float32 220.5 correctly", () => {
    const profile = makeProfile([floatReg("v", 220.5, 0, 300, 0)], 0);
    const writes = valuesToRegisters([220.5], profile);
    // 220.5f32 = 0x435C8000
    expect(writes).toHaveLength(2);
    expect(writes[0]).toEqual({ address: 0, value: 0x435C });
    expect(writes[1]).toEqual({ address: 1, value: 0x8000 });
  });

  it("encodes counter correctly", () => {
    const profile = makeProfile([counterReg("energy", 1000000, 10)], 0);
    const writes = valuesToRegisters([1000000], profile);
    expect(writes).toHaveLength(4);
    expect(writes[0]).toEqual({ address: 0, value: 0 }); // hi16(hi32)
    expect(writes[1]).toEqual({ address: 1, value: 0 }); // lo16(hi32)
    expect(writes[2]).toEqual({ address: 2, value: 0x000F }); // hi16(lo32)
    expect(writes[3]).toEqual({ address: 3, value: 0x4240 }); // lo16(lo32)
  });

  it("uses base address offset", () => {
    const profile = makeProfile([floatReg("v", 220.5, 0, 300, 0)], 100);
    const writes = valuesToRegisters([220.5], profile);
    expect(writes[0].address).toBe(100);
    expect(writes[1].address).toBe(101);
  });

  it("encodes zero float32 correctly", () => {
    const profile = makeProfile([floatReg("v", 0, 0, 100, 0)], 0);
    const writes = valuesToRegisters([0], profile);
    expect(writes).toHaveLength(2);
    expect(writes[0]).toEqual({ address: 0, value: 0 });
    expect(writes[1]).toEqual({ address: 1, value: 0 });
  });

  it("encodes zero counter correctly", () => {
    const profile = makeProfile([counterReg("energy", 0, 10)], 0);
    const writes = valuesToRegisters([0], profile);
    expect(writes).toHaveLength(4);
    expect(writes.every(w => w.value === 0)).toBe(true);
  });

  it("multi-register profile produces correct offsets", () => {
    const profile = makeProfile([
      floatReg("v1", 100, 0, 200, 0),
      counterReg("energy", 500, 10),
    ], 10);
    const writes = valuesToRegisters([100, 500], profile);
    // float at base 10: regs 10-11
    expect(writes[0].address).toBe(10);
    expect(writes[1].address).toBe(11);
    // counter at base+2: regs 12-15
    expect(writes[2].address).toBe(12);
    expect(writes[3].address).toBe(13);
    expect(writes[4].address).toBe(14);
    expect(writes[5].address).toBe(15);
    expect(writes).toHaveLength(6);
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

  it("round-trips counter value", () => {
    const profile = makeProfile([counterReg("energy", 0, 10)], 0);
    const regs = [0, 0, 0x000F, 0x4240]; // 1000000
    const decoded = decodeRegisters(regs, profile);
    expect(decoded).toHaveLength(1);
    expect(decoded[0].count).toBe(4);
    expect(decoded[0].type).toBe("UInt64 Counter BE");
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
