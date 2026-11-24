import type { SymbolDoc, SymbolInstance } from "../src/model";

export function symbol(overrides: Partial<SymbolInstance> = {}): SymbolInstance {
  return {
    id: "s1",
    type: "gauge",
    x: 100,
    y: 100,
    w: 60,
    h: 40,
    rotation: 0,
    zIndex: 0,
    config: {},
    ...overrides,
  };
}

export function doc(symbols: SymbolInstance[]): SymbolDoc {
  return {
    id: "screen-1",
    name: "Screen 1",
    width: 1024,
    height: 600,
    symbols,
  };
}

/** a(z0), b(z1), c(z2) at distinct positions. */
export function abcDoc(): SymbolDoc {
  return doc([
    symbol({ id: "a", x: 0, y: 0, zIndex: 0 }),
    symbol({ id: "b", x: 10, y: 10, zIndex: 1 }),
    symbol({ id: "c", x: 20, y: 20, zIndex: 2 }),
  ]);
}
