import { describe, expect, it } from "vitest";
import {
  FREE_PROFILE_LIMIT, IFACE_DEFAULTS, PROFILES_KEY, TX_DEFAULTS, TX_KEY, cansendText, candumpLine, configOf, foldMonitor, frameInOf, frameProblem,
  idText, ifaceProblem, loadProfiles, loadTxList, needsPro, normalizeProfile, parseData, parseFilter, parseId, periodText, pushCapped,
  saveProfiles, sortMonitor, traceCsv, type MonitorRow, type Profile, mergeLatest, MON_ROW_H, MON_LINE_H, MON_SIG_H, countProblem, cyclicText,
  frameFilter, monitorEvicted, monitorLayout, monitorRowHeight, parseDataPattern, parseSearch, periodProblem, sigColumns, visibleRange, busStateBadge, dataRateOptions, rateOptions,
} from "../can";
import type { FrameView } from "../types";

const frame = (id: number, data: number[], ts: number, extra: Partial<FrameView> = {}): FrameView => ({
  ts, dir: "rx", id, ext: id > 0x7ff, rtr: false, fd: false, brs: false, dlc: data.length, data, ...extra,
});

describe("identifiers and data", () => {
  it("reads identifiers like cansend", () => {
    expect(parseId("7df")).toEqual({ id: 0x7df, ext: false });
    expect(parseId("0x123")).toEqual({ id: 0x123, ext: false });
    expect(parseId("00000123")).toEqual({ id: 0x123, ext: true });
    expect(parseId("18FEF100")).toEqual({ id: 0x18fef100, ext: true });
    expect(typeof parseId("2FFFFFFF")).toBe("string");
    expect(typeof parseId("xyz")).toBe("string");
    expect(idText(0x7df, false)).toBe("7DF");
    expect(idText(0x123, true)).toBe("00000123");
  });
  it("reads data bytes in any common spelling", () => {
    expect(parseData("de ad:BE,ef")).toEqual([0xde, 0xad, 0xbe, 0xef]);
    expect(parseData("")).toEqual([]);
    expect(typeof parseData("123")).toBe("string");
    expect(typeof parseData("zz")).toBe("string");
  });
  it("checks frames before they are sent", () => {
    const base = { id: 0x123, ext: false, rtr: false, fd: false, brs: false, data: "", rtrLen: 0 };
    expect(frameProblem({ ...base, data: "00 11" })).toBeNull();
    expect(frameProblem({ ...base, data: "00".repeat(9) })).toMatch(/at most 8/);
    expect(frameProblem({ ...base, fd: true, data: "00".repeat(12) })).toBeNull();
    expect(frameProblem({ ...base, fd: true, data: "00".repeat(13) })).toMatch(/CAN FD/);
    expect(frameProblem({ ...base, brs: true })).toMatch(/bit rate switch/i);
    expect(frameProblem({ ...base, id: 0x800 })).toMatch(/11 bits/);
    expect(frameInOf({ id: "7DF", fd: false, brs: false, rtr: true, data: "ignored" })).toMatchObject({ id: 0x7df, rtr: true, data: "" });
  });
  it("writes cansend and candump syntax", () => {
    expect(cansendText(frame(0x123, [0xde, 0xad], 0))).toBe("123#DEAD");
    expect(cansendText(frame(0x123, [], 0, { rtr: true, dlc: 8 }))).toBe("123#R8");
    expect(cansendText(frame(0x10, [0xaa], 0, { fd: true, brs: true }))).toBe("010##1AA");
    expect(cansendText(frame(0x10, [], 0, { fd: true, brs: true, esi: true }))).toBe("010##3");
    expect(cansendText(frame(0x10, [], 0, { fd: true, esi: true }))).toBe("010##2");
    const fd64 = Array.from({ length: 64 }, (_, i) => i);
    const line = candumpLine(frame(0x201, fd64, 1_762_851_234_000_100, { fd: true, brs: true, dlc: 15 }), "vcan0");
    expect(line).toBe(`(1762851234.000100) vcan0 201##1${fd64.map((b) => b.toString(16).toUpperCase().padStart(2, "0")).join("")}`);
    expect(candumpLine(frame(0x18fef100, [1], 1_699_999_999_000_042), "can0")).toBe("(1699999999.000042) can0 18FEF100#01");
  });
});

describe("monitor", () => {
  it("carries the CAN FD flags of the latest frame", () => {
    const rows = new Map<string, MonitorRow>();
    foldMonitor(rows, [frame(0x201, Array(64).fill(0), 10, { fd: true, brs: true, dlc: 15 })]);
    expect(rows.get("s513")).toMatchObject({ fd: true, brs: true, esi: false, dlc: 15 });
    foldMonitor(rows, [frame(0x201, Array(64).fill(1), 20, { fd: true, brs: false, esi: true, dlc: 15 })]);
    expect(rows.get("s513")).toMatchObject({ brs: false, esi: true, count: 2 });
    expect(rows.get("s513")!.changedAt.every((t) => t === 20)).toBe(true);
  });
  it("keeps one row per identifier with period and changed bytes", () => {
    const rows = new Map<string, MonitorRow>();
    foldMonitor(rows, [frame(0x100, [1, 2], 1_000_000), frame(0x100, [1, 3], 1_010_000), frame(0x100, [1, 3], 1_020_000)]);
    foldMonitor(rows, [frame(0x100, [1, 3], 0, { dir: "tx" })]);
    const r = rows.get("s256")!;
    expect(r.count).toBe(3);
    expect(r.periodMs).toBeCloseTo(10);
    expect(r.changedAt).toEqual([1_000_000, 1_010_000]);
    expect(rows.has("s256:tx")).toBe(true);
    // 11-bit 0x100 and 29-bit 0x100 are different rows.
    foldMonitor(rows, [frame(0x100, [], 5, { ext: true })]);
    expect(rows.has("x256")).toBe(true);
    const sorted = sortMonitor([...rows.values()], "id").map((x) => x.key);
    expect(sorted).toEqual(["s256", "s256:tx", "x256"]);
  });
});

describe("monitor period", () => {
  const fold = (rows: Map<string, MonitorRow>, batches: number[][], id = 0x100) => {
    for (const b of batches) foldMonitor(rows, b.map((ts) => frame(id, [1], ts)));
    return rows.get(`s${id}`)!;
  };
  it("is not dragged down by frames read in one batch with one timestamp", () => {
    // A 10 ms ID read every 50 ms: five frames per read, all stamped with the read time.
    const batches = Array.from({ length: 100 }, (_, k) => Array(5).fill(1_000_000 + k * 50_000));
    const r = fold(new Map(), batches);
    expect(r.periodMs).toBeGreaterThan(9);
    expect(r.periodMs).toBeLessThan(11);
    expect(r.count).toBe(500);
  });
  it("reads exact timestamps (kernel stamps)", () => {
    const r = fold(new Map(), [Array.from({ length: 300 }, (_, k) => 5_000_000 + k * 10_000)]);
    expect(r.periodMs).toBeCloseTo(10, 3);
  });
  it("gives a first estimate from the second timestamp and follows a change of rate", () => {
    const rows = new Map<string, MonitorRow>();
    expect(fold(rows, [[0, 0, 0]]).periodMs).toBeNull();
    expect(fold(rows, [[30_000]]).periodMs).toBeCloseTo(10);
    // Quiet for 60 s, then 20 ms again: the long gap does not stick.
    const back = Array.from({ length: 200 }, (_, k) => 60_000_000 + k * 20_000);
    expect(fold(rows, [back]).periodMs).toBeCloseTo(20, 1);
  });
  it("handles slow identifiers and timestamps that go back", () => {
    const rows = new Map<string, MonitorRow>();
    expect(fold(rows, [[0], [5_000_000], [10_000_000], [15_000_000]]).periodMs).toBeCloseTo(5000);
    const r = fold(rows, [[1_000], [2_000_000]]);
    expect(r.periodMs).not.toBeNull();
    expect(r.periodMs!).toBeGreaterThan(0);
  });
});

describe("monitor size", () => {
  it("caps the identifiers, dropping the least recently seen", () => {
    const rows = new Map<string, MonitorRow>();
    foldMonitor(rows, [frame(1, [0], 1), frame(2, [0], 2), frame(3, [0], 3)], 3);
    foldMonitor(rows, [frame(1, [0], 4)], 3); // 1 seen again: 2 is now the oldest
    foldMonitor(rows, [frame(4, [0], 5)], 3);
    expect([...rows.keys()].sort()).toEqual(["s1", "s3", "s4"]);
    expect(monitorEvicted(rows)).toBe(1);
    foldMonitor(rows, Array.from({ length: 10 }, (_, i) => frame(0x100 + i, [0], 10 + i)), 3);
    expect(rows.size).toBe(3);
    expect(monitorEvicted(rows)).toBe(11);
    expect(monitorEvicted(new Map())).toBe(0);
  });
  it("replaces changed rows and keeps unchanged ones, so rendering can skip them", () => {
    const rows = new Map<string, MonitorRow>();
    foldMonitor(rows, [frame(1, [0], 1), frame(2, [0], 1)]);
    const before2 = rows.get("s2");
    const before1 = rows.get("s1");
    foldMonitor(rows, [frame(1, [5], 2)]);
    expect(rows.get("s2")).toBe(before2);
    expect(rows.get("s1")).not.toBe(before1);
    expect(before1!.count).toBe(1);
    expect(rows.get("s1")!.lastChange).toBe(2);
    foldMonitor(rows, [frame(1, [5], 9)]);
    expect(rows.get("s1")!.lastChange).toBe(2);
  });
  it("lays out rows for virtual scrolling", () => {
    const row = (key: string, len: number, sigs = 0) =>
      ({ key, data: Array(len).fill(0), sigs: Array.from({ length: sigs }, (_, i) => [`s${i}`, 0, "", null]) }) as unknown as MonitorRow;
    expect(monitorRowHeight(row("a", 8), false, 3)).toBe(MON_ROW_H);
    expect(monitorRowHeight(row("a", 64), false, 3)).toBe(MON_ROW_H + 3 * MON_LINE_H);
    expect(monitorRowHeight(row("a", 8, 7), false, 3)).toBe(MON_ROW_H);
    expect(monitorRowHeight(row("a", 8, 7), true, 3)).toBe(MON_ROW_H + 3 * MON_SIG_H + 6);
    expect(sigColumns(10)).toBe(1);
    const rows = Array.from({ length: 10_000 }, (_, i) => row(`r${i}`, 8, i === 2 ? 4 : 0));
    const { tops, heights, total } = monitorLayout(rows, (k) => k === "r2", 48 + 240 * 2);
    expect(tops[3]).toBe(3 * MON_ROW_H + 2 * MON_SIG_H + 6);
    expect(total).toBe(10_000 * MON_ROW_H + 2 * MON_SIG_H + 6);
    const [first, last] = visibleRange(tops, heights, 5000 * MON_ROW_H, 20 * MON_ROW_H, 2);
    expect(last - first).toBeLessThan(30);
    expect(tops[first + 2]).toBeLessThanOrEqual(5000 * MON_ROW_H);
    // Row 2 is open (80 px), so nine rows reach into the first 300 px.
    expect(visibleRange(tops, heights, 0, 300, 0)).toEqual([0, 9]);
    expect(visibleRange([], [], 0, 300)).toEqual([0, 0]);
    const [a, b] = visibleRange(tops, heights, total + 1000, 300, 0);
    expect(b).toBe(10_000);
    expect(a).toBe(9_999);
  });
});

describe("filter", () => {
  const f = (id: number, ext = false, msg?: string, label?: string) => ({ id, ext, msg, label });
  it("matches identifiers, ranges and exclusions, never names", () => {
    const keep = parseFilter("7E8, 700-70F 18FEF100").test;
    expect(keep(f(0x7e8))).toBe(true);
    expect(keep(f(0x705))).toBe(true);
    expect(keep(f(0x18fef100, true))).toBe(true);
    expect(keep(f(0x100))).toBe(false);
    // `44` is the identifier 0x044, not text inside "PGN 61444".
    expect(parseFilter("44").test(f(0x18fef100, true, "EEC1", "PGN 61444"))).toBe(false);
    expect(parseFilter("44").test(f(0x44))).toBe(true);
    // `!100` hides 0x100, not labels with 100 in them.
    const hide = parseFilter("!100").test;
    expect(hide(f(0x100))).toBe(false);
    expect(hide(f(0x191, false, undefined, "node 100"))).toBe(true);
    expect(parseFilter("  ").test(f(1))).toBe(true);
  });
  it("keeps 11-bit and 29-bit identifiers apart", () => {
    const std = parseFilter("100-1FF").test;
    expect(std(f(0x150))).toBe(true);
    expect(std(f(0x150, true))).toBe(false);
    const ext = parseFilter("00000100-000001FF").test;
    expect(ext(f(0x150, true))).toBe(true);
    expect(ext(f(0x150))).toBe(false);
    expect(parseFilter("150x").test(f(0x150, true))).toBe(true);
    expect(parseFilter("150x").test(f(0x150))).toBe(false);
    expect(parseFilter("0-FFFx").test(f(0x7ff, true))).toBe(true);
    expect(parseFilter("0x150").test(f(0x150))).toBe(true);
    // Above 7FF is 29-bit whatever the number of digits.
    expect(parseFilter("800").test(f(0x800, true))).toBe(true);
    expect(parseFilter("!7DF").test(f(0x7df, true))).toBe(true);
  });
  it("reports what is not an identifier and ignores it", () => {
    const p = parseFilter("7E8 heartbeat");
    expect(p.error).toMatch(/heartbeat/);
    expect(p.test(f(0x7e8))).toBe(true);
    expect(p.test(f(0x7e9))).toBe(false);
    expect(parseFilter("2FFFFFFF").error).not.toBeNull();
    expect(parseFilter("7E8 !").error).toBeNull();
    expect(parseFilter("heartbeat").test(f(1))).toBe(true);
  });
  it("searches names separately", () => {
    const s = parseSearch("eec1 heartbeat");
    expect(s(f(1, false, "EEC1"))).toBe(true);
    expect(s(f(1, false, undefined, "Heartbeat node 17"))).toBe(true);
    expect(s(f(1, false, "TSC1"))).toBe(false);
    expect(parseSearch("!heartbeat")(f(1, false, undefined, "Heartbeat"))).toBe(false);
    expect(parseSearch("!heartbeat")(f(1))).toBe(true);
    expect(parseSearch(" ")(f(1))).toBe(true);
  });
  it("matches data bytes with wildcards", () => {
    const d = (data: number[]) => ({ data });
    const p = parseDataPattern("xx 12 xx").test;
    expect(p(d([0, 0x12, 0]))).toBe(true);
    expect(p(d([0xff, 0x12, 0x34, 0x56]))).toBe(true);
    expect(p(d([0, 0x13, 0]))).toBe(false);
    expect(p(d([0, 0x12]))).toBe(false);
    expect(parseDataPattern("?f").test(d([0xaf]))).toBe(true);
    expect(parseDataPattern("?f").test(d([0xa0]))).toBe(false);
    expect(parseDataPattern("02x1").test(d([0x02, 0x31]))).toBe(true);
    expect(parseDataPattern("02:41").test(d([0x02, 0x41]))).toBe(true);
    expect(parseDataPattern("").test(d([]))).toBe(true);
    expect(parseDataPattern("1 2").error).toMatch(/not a byte/);
    expect(parseDataPattern("zz").error).not.toBeNull();
  });
  it("combines the three", () => {
    const q = frameFilter({ ids: "100-1FF", text: "eec1", data: "xx 7F" });
    expect(q.test({ id: 0x101, ext: false, msg: "EEC1", data: [0, 0x7f] })).toBe(true);
    expect(q.test({ id: 0x101, ext: false, msg: "EEC2", data: [0, 0x7f] })).toBe(false);
    expect(q.test({ id: 0x101, ext: false, msg: "EEC1", data: [0, 0x7e] })).toBe(false);
    expect(q.test({ id: 0x201, ext: false, msg: "EEC1", data: [0, 0x7f] })).toBe(false);
    expect(q.error).toBeNull();
    expect(frameFilter({ ids: "foo", text: "", data: "q" }).error).toMatch(/foo.*Data pattern/);
  });
});

describe("transmit", () => {
  it("validates the cyclic period and count", () => {
    expect(periodProblem(0)).toMatch(/at least 1 ms/);
    expect(periodProblem(Number.NaN)).toMatch(/at least 1 ms/);
    expect(periodProblem(-5)).toMatch(/at least 1 ms/);
    expect(periodProblem(0.5)).toMatch(/at least 1 ms/);
    expect(periodProblem(1.5)).toMatch(/whole/);
    expect(periodProblem(1)).toBeNull();
    expect(periodProblem(3_600_001)).toMatch(/at most/);
    expect(countProblem(undefined)).toBeNull();
    expect(countProblem(0)).toBeNull();
    expect(countProblem(10)).toBeNull();
    expect(countProblem(2.5)).not.toBeNull();
    expect(countProblem(-1)).not.toBeNull();
    expect(cyclicText({ periodMs: 100 })).toBe("every 100 ms");
    expect(cyclicText({ periodMs: 100, count: 1000 })).toBe("every 100 ms, 1,000 frames");
  });
  it("keeps the requested length of a remote frame", () => {
    expect(frameInOf({ id: "7DF", fd: false, brs: false, rtr: true, data: "", rtrLen: 8 })).toMatchObject({ rtr: true, rtrLen: 8, data: "" });
    // Without a DLC set yet, the length of the data typed before ticking RTR.
    expect(frameInOf({ id: "7DF", fd: false, brs: false, rtr: true, data: "01 02 03" })).toMatchObject({ rtrLen: 3 });
    expect(frameInOf({ id: "7DF", fd: false, brs: false, rtr: true, data: "", rtrLen: 9 })).toMatch(/0 to 8/);
    expect(frameInOf({ id: "7DF", fd: false, brs: false, rtr: false, data: "01", rtrLen: 8 })).toMatchObject({ rtrLen: 0, data: "01" });
  });
  it("loads the count and DLC, repairing bad values", () => {
    const load = (x: object) => loadTxList({ getItem: () => JSON.stringify([{ key: "a", id: "123", periodMs: 10, ...x }]) })[0];
    expect(load({ rtrLen: 4, count: 50 })).toMatchObject({ rtrLen: 4, count: 50 });
    expect(load({ rtrLen: 12, count: -3 })).not.toHaveProperty("rtrLen");
    expect(load({ count: 1.5 })).not.toHaveProperty("count");
    expect(load({ periodMs: null }).periodMs).toBe(100);
    expect(load({ periodMs: 2.5 }).periodMs).toBe(100);
  });
});

describe("signals tab", () => {
  it("keeps the other pages of a multiplexed message", () => {
    const p0 = frame(0x4a0, [0], 1, { msg: "BatteryPackFD", sigs: [["CellPage", 0, "", null], ["SOC", 70, "%", null], ["Cell01", 3.9, "V", null]] });
    const p1 = frame(0x4a0, [1], 2, { msg: "BatteryPackFD", sigs: [["CellPage", 1, "", null], ["SOC", 71, "%", null], ["Cell17", 3.8, "V", null]] });
    const m = mergeLatest(p0, p1);
    expect(m.ts).toBe(2);
    expect(Object.fromEntries(m.sigs!.map(([n, v]) => [n, v]))).toEqual({ CellPage: 1, SOC: 71, Cell17: 3.8, Cell01: 3.9 });
    expect(mergeLatest(undefined, p1)).toBe(p1);
  });
});

describe("export", () => {
  it("writes CSV with relative times and decoded signals", () => {
    const csv = traceCsv([
      frame(0x321, [0x0f, 0xa0], 2_000_000, { msg: "GenStatus", sigs: [["Voltage", 400, "V", null], ["Breaker", 1, "", "Closed"]] }),
      frame(0x7e8, [4, 0x41], 2_500_000, { dir: "tx", label: "OBD-II response, x" }),
    ]);
    const lines = csv.trim().split("\n");
    expect(lines[0]).toBe("time_s,direction,id,extended,fd,brs,rtr,dlc,data,message,signals,label");
    expect(lines[1]).toBe("0.000000,rx,321,0,0,0,0,2,0F A0,GenStatus,Voltage=400 V; Breaker=Closed,");
    expect(lines[2]).toBe('0.500000,tx,7E8,0,0,0,0,2,04 41,,,"OBD-II response, x"');
  });
  it("caps lists from the end", () => {
    expect(pushCapped([1, 2, 3], [4, 5], 4)).toEqual([2, 3, 4, 5]);
    expect(periodText(null)).toBe("");
    expect(periodText(1500)).toBe("1.50 s");
    expect(periodText(20)).toBe("20.0 ms");
  });
});

describe("profiles and tiers", () => {
  const p = (name: string): Profile => ({ ...IFACE_DEFAULTS, name });
  it("keeps one saved profile in the free tier", () => {
    expect(FREE_PROFILE_LIMIT).toBe(1);
    expect(needsPro([], "a", false)).toBe(false);
    expect(needsPro([p("a")], "b", false)).toBe(true);
    expect(needsPro([p("a")], "a", false)).toBe(false);
    expect(needsPro([p("a")], "b", true)).toBe(false);
  });
  it("round-trips through storage and repairs bad entries", () => {
    const mem: Record<string, string> = {};
    const saved: Profile = { ...p("bench"), kind: "socketcan", ifname: "can1", bitrate: 250000 };
    saveProfiles([saved], { setItem: (k, v) => { mem[k] = v; } });
    expect(loadProfiles({ getItem: (k) => mem[k] ?? null })).toEqual([saved]);
    expect(Object.keys(mem)).toEqual([PROFILES_KEY]);
    expect(normalizeProfile({ name: "x", bitrate: 83334, kind: "weird" })).toEqual(p("x"));
    expect(normalizeProfile({ bitrate: 500000 })).toBeNull();
    expect(loadProfiles({ getItem: () => "{oops" })).toEqual([]);
  });
  it("builds interface configs", () => {
    expect(configOf({ ...IFACE_DEFAULTS, port: "/dev/cu.usbmodem1", dataBitrate: 2000000 })).toEqual({
      kind: "slcan", port: "/dev/cu.usbmodem1", bitrate: 500000, dataBitrate: 2000000, serialBaud: 115200, listenOnly: false,
    });
    expect(configOf({ ...IFACE_DEFAULTS, kind: "virtual" })).toEqual({ kind: "virtual" });
    expect(configOf({ ...IFACE_DEFAULTS, kind: "gsusb", gsDevice: "0039001A", channel: 1, listenOnly: true })).toEqual({
      kind: "gsusb", device: "0039001A", channel: 1, bitrate: 500000, dataBitrate: null, listenOnly: true,
    });
    expect(configOf({ ...IFACE_DEFAULTS, kind: "gsusb" })).toMatchObject({ device: null, channel: 0 });
    expect(ifaceProblem({ ...IFACE_DEFAULTS, kind: "gsusb" })).toBeNull();
    expect(normalizeProfile({ name: "g", kind: "gsusb", gsDevice: "AB", channel: 9 })).toMatchObject({ kind: "gsusb", gsDevice: "AB", channel: 0 });
    expect(ifaceProblem(IFACE_DEFAULTS)).toMatch(/serial port/);
    expect(ifaceProblem({ ...IFACE_DEFAULTS, kind: "socketcan", ifname: "can0" })).toBeNull();
    expect(ifaceProblem({ ...IFACE_DEFAULTS, kind: "socketcan", ifname: "bad name" })).not.toBeNull();
  });
  it("offers each adapter kind only the rates it has", () => {
    const slcan = { ...IFACE_DEFAULTS, port: "/dev/cu.usbmodem1" };
    // CANable 2 slcan firmware: 2 and 5 Mbit/s data rates only.
    expect(ifaceProblem({ ...slcan, dataBitrate: 2000000 })).toBeNull();
    expect(ifaceProblem({ ...slcan, dataBitrate: 4000000 })).toMatch(/2 and 5 Mbit\/s/);
    expect(ifaceProblem({ ...slcan, kind: "gsusb", dataBitrate: 8000000 })).toBeNull();
    expect(dataRateOptions("slcan", 0).map((o) => o.value)).toEqual(["0", "2000000", "5000000"]);
    const odd = dataRateOptions("slcan", 8000000);
    expect(odd[odd.length - 1].label).toMatch(/not available/);
    // 750k / 83.3k / 800k are labelled with the adapters that have them.
    const labels = rateOptions("slcan", 500000).map((o) => o.label);
    expect(labels).toContain("750 kbit/s (CANable only)");
    expect(labels).toContain("83.3 kbit/s (CANable 2 only)");
    expect(labels.find((l) => l.startsWith("800 kbit/s"))).toMatch(/not CANable/);
    expect(rateOptions("gsusb", 500000).every((o) => !o.label.includes("("))).toBe(true);
    expect(normalizeProfile({ name: "c", bitrate: 750000 })?.bitrate).toBe(750000);
  });
  it("shows the controller state", () => {
    expect(busStateBadge(null)).toBeNull();
    expect(busStateBadge({ state: "error-active", txErrors: null, rxErrors: null })).toBeNull();
    expect(busStateBadge({ state: "error-active", txErrors: 0, rxErrors: 0 })).toBeNull();
    expect(busStateBadge({ state: "error-active", txErrors: 8, rxErrors: 0 })?.text).toBe("TEC 8 REC 0");
    expect(busStateBadge({ state: "error-passive", txErrors: 128, rxErrors: 0 })).toEqual({
      text: "Error passive", tone: "danger", title: "Controller state: error passive; TX errors 128, RX errors 0",
    });
    expect(busStateBadge({ state: "bus-off", txErrors: null, rxErrors: null })?.tone).toBe("danger");
    expect(busStateBadge({ state: "error-warning", txErrors: 96, rxErrors: 0 })?.tone).toBe("warn");
  });
  it("loads the transmit list with defaults", () => {
    expect(loadTxList({ getItem: () => null })).toEqual(TX_DEFAULTS);
    expect(loadTxList({ getItem: (k) => (k === TX_KEY ? JSON.stringify([{ key: "a", id: "123", periodMs: 0 }]) : null) })).toEqual([
      { key: "a", name: "", id: "123", fd: false, brs: false, rtr: false, data: "", periodMs: 100 },
    ]);
    for (const t of TX_DEFAULTS) expect(typeof frameInOf(t)).toBe("object");
  });
});
