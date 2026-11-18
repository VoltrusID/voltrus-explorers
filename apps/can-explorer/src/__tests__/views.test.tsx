import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { foldMonitor, type MonitorRow } from "../can";
import type { DbcView, FrameView } from "../types";

// The shared UI package reads `window` when it loads; these tests render to a string under Node.
(globalThis as { window?: unknown }).window ??= globalThis;
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

describe("pro status at launch", () => {
  it("shows the status this device had while the backend answers", async () => {
    const { ProProvider, useProStatus } = await import("@mactools/ui");
    const Probe = () => <span>{useProStatus().pro ? "PRO" : "LOCKED"}</span>;
    const render = () => renderToStaticMarkup(<ProProvider appName="CAN Explorer" features={[]}><Probe /></ProProvider>);
    const items = new Map<string, string>();
    (globalThis as { localStorage?: unknown }).localStorage = {
      getItem: (k: string) => items.get(k) ?? null,
      setItem: (k: string, v: string) => void items.set(k, v),
    };
    try {
      expect(render()).toContain("LOCKED");
      items.set("mt-store-status", JSON.stringify({ store: false, pro: true, reason: "key", price: null, activation: true }));
      expect(render()).toContain("PRO");
      items.set("mt-store-status", "{not json");
      expect(render()).toContain("LOCKED");
      items.set("mt-store-status", JSON.stringify({ pro: "yes" }));
      expect(render()).toContain("LOCKED");
    } finally {
      delete (globalThis as { localStorage?: unknown }).localStorage;
    }
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
