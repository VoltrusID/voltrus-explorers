import { describe, expect, it } from "vitest";
import { loadThemeChoice, resolveTheme, saveThemeChoice, THEME_STORAGE_KEY } from "../lib/theme";
import { defaultTypeFor, modeTone } from "../lib/s7";

function memoryStorage(initial: Record<string, string> = {}) {
  const data = { ...initial };
  return {
    data,
    getItem: (k: string) => (k in data ? data[k] : null),
    setItem: (k: string, v: string) => { data[k] = v; },
  };
}

describe("theme choice", () => {
  it("defaults to following the system and ignores junk", () => {
    expect(loadThemeChoice(memoryStorage())).toBe("system");
    expect(loadThemeChoice(memoryStorage({ [THEME_STORAGE_KEY]: "neon" }))).toBe("system");
  });

  it("persists the choice", () => {
    const store = memoryStorage();
    saveThemeChoice("petrol", store);
    expect(store.data[THEME_STORAGE_KEY]).toBe("petrol");
    expect(loadThemeChoice(store)).toBe("petrol");
  });

  it("resolves system to the OS preference and keeps explicit choices", () => {
    expect(resolveTheme("system", true)).toBe("light");
    expect(resolveTheme("system", false)).toBe("dark");
    expect(resolveTheme("petrol", false)).toBe("petrol");
    expect(resolveTheme("dark", true)).toBe("dark");
  });
});

describe("S7 watch helpers", () => {
  it("picks the display type the address width implies", () => {
    expect(defaultTypeFor("M0.0")).toBe("BOOL");
    expect(defaultTypeFor("%DB1.DBX2.3")).toBe("BOOL");
    expect(defaultTypeFor("MB4")).toBe("BYTE");
    expect(defaultTypeFor("db1.dbw2")).toBe("INT");
    expect(defaultTypeFor("QD8")).toBe("DINT");
    expect(defaultTypeFor("T5")).toBe("S5TIME");
    expect(defaultTypeFor("C10")).toBe("COUNTER");
    expect(defaultTypeFor("Motor.Speed")).toBe("");
  });

  it("colours the operating mode", () => {
    expect(modeTone("RUN")).toBe("success");
    expect(modeTone("STOP")).toBe("warn");
    expect(modeTone(null)).toBe("neutral");
  });
});
