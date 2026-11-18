import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { foldMonitor, type MonitorRow } from "../can";
import type { DbcView, FrameView } from "../types";

// The shared UI package reads `window` when it loads; these tests render to a string under Node.
(globalThis as { window?: unknown }).window ??= globalThis;
const { default: BusStatus } = await import("../BusStatus");
const { default: MonitorView } = await import("../MonitorView");
const { dbcNotes } = await import("../SignalsView");

const frame = (id: number, ts: number): FrameView => ({ ts, dir: "rx", id, ext: false, rtr: false, fd: false, brs: false, dlc: 2, data: [1, 2] });

describe("monitor view", () => {
  it("renders only the rows in view", () => {
    const rows = new Map<string, MonitorRow>();
    foldMonitor(rows, Array.from({ length: 2000 }, (_, i) => frame(i % 0x800, i)));
    const html = renderToStaticMarkup(<MonitorView rows={[...rows.values()]} filter="" onClear={() => {}} trending={new Set()} onToggleTrend={() => {}} />);
    const rendered = html.match(/class="can-mon-row"/g)?.length ?? 0;
    expect(rendered).toBeGreaterThan(5);
    expect(rendered).toBeLessThan(40);
    expect(html).toContain("2000 of 2000 IDs");
    expect(html).not.toContain("IDs not shown");
  });
  it("says how many identifiers were dropped", () => {
    const html = renderToStaticMarkup(<MonitorView rows={[]} filter="" onClear={() => {}} trending={new Set()} onToggleTrend={() => {}} evicted={0} />);
    expect(html).toContain("No frames yet");
    const rows = new Map<string, MonitorRow>();
    foldMonitor(rows, [frame(1, 1)]);
    const dropped = renderToStaticMarkup(<MonitorView rows={[...rows.values()]} filter="zz" onClear={() => {}} trending={new Set()} onToggleTrend={() => {}} evicted={1234} />);
    expect(dropped).toContain("1,234 IDs not shown");
    expect(dropped).toContain("Not an identifier: zz");
  });
});

describe("bus status", () => {
  it("shows nothing until the driver reports health", () => {
    expect(renderToStaticMarkup(<BusStatus stats={null} />)).toBe("");
    expect(renderToStaticMarkup(<BusStatus stats={{ dropped: 0 }} />)).toBe("");
  });
  it("shows the bus state and error counters", () => {
    const html = renderToStaticMarkup(<BusStatus stats={{ busState: "passive", txErr: 130, rxErr: 0, dropped: 0 }} />);
    expect(html).toContain("Error passive");
    expect(html).toContain("TEC 130 · REC 0");
    expect(renderToStaticMarkup(<BusStatus stats={{ busState: "bus-off" }} />)).toContain("Bus off");
    expect(renderToStaticMarkup(<BusStatus stats={{ busState: "weird" as never, rxErr: 5 }} />)).toContain("TEC – · REC 5");
  });
  it("shows a notShown count when the core reports it under that name", () => {
    expect(renderToStaticMarkup(<BusStatus stats={{ notShown: 4200 }} />)).toContain("4,200 not shown");
  });
});

describe("signals tab", () => {
  const dbc = (extra: object, messages = 1): DbcView =>
    ({ name: "x.dbc", version: "", nodes: [], messages: Array(messages).fill({}), skipped: 0, ...extra }) as DbcView;
  it("shows errors and warnings from dbc_load", () => {
    expect(dbcNotes(dbc({}))).toEqual([]);
    expect(dbcNotes(dbc({ warnings: ["line 3: unknown keyword"], errors: ["line 9: bad BO_"] }))).toEqual([
      { level: "error", text: "line 9: bad BO_" },
      { level: "warning", text: "line 3: unknown keyword" },
    ]);
    expect(dbcNotes(dbc({ warnings: "not a list" }))).toEqual([]);
  });
  it("warns about a file without messages", () => {
    expect(dbcNotes(dbc({}, 0))[0].text).toMatch(/no messages.*Is it a DBC file/);
  });
});
