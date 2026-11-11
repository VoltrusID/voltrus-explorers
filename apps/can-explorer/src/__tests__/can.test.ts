import { describe, expect, it } from "vitest";
import {
  FREE_PROFILE_LIMIT, IFACE_DEFAULTS, PROFILES_KEY, TX_DEFAULTS, TX_KEY, cansendText, candumpLine, configOf, foldMonitor, frameInOf, frameProblem,
  idText, ifaceProblem, loadProfiles, loadTxList, needsPro, normalizeProfile, parseData, parseFilter, parseId, periodText, pushCapped,
  saveProfiles, sortMonitor, traceCsv, type MonitorRow, type Profile,
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

describe("filter", () => {
  it("matches ids, ranges, names and exclusions", () => {
    const f = (id: number, msg?: string, label?: string) => ({ id, ext: false, msg, label });
    const keep = parseFilter("7E8, 700-70F EEC1");
    expect(keep(f(0x7e8))).toBe(true);
    expect(keep(f(0x705))).toBe(true);
    expect(keep(f(0x100, "EEC1"))).toBe(true);
    expect(keep(f(0x100))).toBe(false);
    const hide = parseFilter("!heartbeat");
    expect(hide(f(0x711, undefined, "Heartbeat node 17: operational"))).toBe(false);
    expect(hide(f(0x191))).toBe(true);
    expect(parseFilter("  ")(f(1))).toBe(true);
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
    expect(normalizeProfile({ name: "x", bitrate: 83333, kind: "weird" })).toEqual(p("x"));
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
  it("loads the transmit list with defaults", () => {
    expect(loadTxList({ getItem: () => null })).toEqual(TX_DEFAULTS);
    expect(loadTxList({ getItem: (k) => (k === TX_KEY ? JSON.stringify([{ key: "a", id: "123", periodMs: 0 }]) : null) })).toEqual([
      { key: "a", name: "", id: "123", fd: false, brs: false, rtr: false, data: "", periodMs: 100 },
    ]);
    for (const t of TX_DEFAULTS) expect(typeof frameInOf(t)).toBe("object");
  });
});
