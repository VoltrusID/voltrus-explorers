import { describe, expect, it } from "vitest";
import {
  baseName,
  clampView, cursorDelta, decoderChannels, decoderLabel, defaultDecoder, fitView, fmtRate, fmtSamples, fmtTime, makeSession, patternTrigger, readSession, sampleTime,
  tableCsv, tickStep, viewOf, zoomAt,
} from "../logic";
import type { CaptureInfo, ChannelInfo } from "../types";

const info: CaptureInfo = { sampleRate: 1_000_000, samples: 1_000_000, trigger: 100_000, channels: [], source: "test" };
const chans: ChannelInfo[] = ["RS-485 RO", "Master TX", "Master DE", "Slave TX"].map((name) => ({ name }));

describe("formatting", () => {
  it("times, rates and lengths", () => {
    expect(fmtTime(0.00125)).toBe("1.25 ms");
    expect(fmtTime(4.17e-7)).toBe("417 ns");
    expect(fmtTime(-0.5)).toBe("-500 ms");
    expect(fmtTime(0)).toBe("0 s");
    expect(fmtRate(24_000_000)).toBe("24 MHz");
    expect(fmtRate(19200)).toBe("19.2 kHz");
    expect(fmtSamples(2_000_000)).toBe("2 M");
    expect(fmtSamples(512 * 1024 * 1024)).toBe("512 Mi");
  });

  it("ruler steps are 1, 2 or 5 decades at least 90 px apart", () => {
    expect(tickStep(1e-6)).toBeCloseTo(1e-4);
    expect(tickStep(3e-6)).toBeCloseTo(5e-4);
    expect(tickStep(1e-9)).toBeCloseTo(1e-7);
  });
});

describe("view", () => {
  it("fits, clamps and zooms around the pointer", () => {
    const v = fitView(1_000_000, 1000);
    expect(v).toEqual({ start: 0, spp: 1000 });
    expect(clampView({ start: -50, spp: 1e9 }, 1_000_000, 1000)).toEqual({ start: 0, spp: 1000 });
    const z = zoomAt(v, 2, 250, 1_000_000, 1000);
    expect(z.spp).toBe(500);
    // The sample under x = 250 stays there.
    expect(z.start + 250 * z.spp).toBe(250 * 1000);
    // Down to 64 pixels per sample, no further.
    expect(clampView({ start: 0, spp: 0.0001 }, 1_000_000, 1000).spp).toBe(1 / 64);
    const r = viewOf(1000, 2000, 1_000_000, 1000);
    expect(r.start).toBeLessThan(1000);
    expect(r.start + r.spp * 1000).toBeGreaterThan(2000);
  });

  it("times are relative to the trigger; cursors give delta and frequency", () => {
    expect(sampleTime(info, 100_000)).toBe(0);
    expect(sampleTime(info, 0)).toBeCloseTo(-0.1);
    const d = cursorDelta(info, 3000, 1000);
    expect(d.samples).toBe(2000);
    expect(d.dt).toBeCloseTo(0.002);
    expect(d.freq).toBeCloseTo(500);
    expect(cursorDelta(info, 5, 5).freq).toBeNull();
  });
});

describe("decoders", () => {
  it("defaults are industrial: Modbus RTU 19200 8E1 on two channels", () => {
    const m = defaultDecoder("modbusRtu", 8);
    expect(m).toMatchObject({ kind: "modbusRtu", baud: 19200, parity: "even", stopBits: 1, requestChannel: 0, responseChannel: 1 });
    expect(decoderLabel(m, chans)).toBe("Modbus RTU 19200 8E1 · RS-485 RO / Master TX");
    expect(decoderChannels(m)).toEqual([0, 1]);
    const one = defaultDecoder("modbusRtu", 1);
    expect(one.kind === "modbusRtu" && one.responseChannel).toBeNull();
  });

  it("labels and channels of every kind", () => {
    expect(decoderLabel(defaultDecoder("uart", 8), chans)).toBe("UART 9600 8N1 on RS-485 RO");
    expect(decoderLabel(defaultDecoder("can", 8), chans)).toBe("CAN 500 kbit/s on RS-485 RO");
    expect(decoderChannels(defaultDecoder("spi", 8))).toEqual([1, 2, 3, 0]);
    expect(decoderChannels(defaultDecoder("i2c", 2))).toEqual([0, 1]);
    // Fewer channels than the defaults want: clamped, never out of range.
    expect(Math.max(...decoderChannels(defaultDecoder("spi", 2)))).toBe(1);
  });
});

describe("triggers", () => {
  it("pattern bits become mask and value", () => {
    expect(patternTrigger(["1", "0", "x", "x", "x", "x", "x", "1"])).toEqual({ kind: "pattern", mask: 0b1000_0011, value: 0b1000_0001 });
    expect(patternTrigger(Array(8).fill("x"))).toEqual({ kind: "none" });
  });
});

describe("files", () => {
  it("table CSV quotes cells and adds times", () => {
    const csv = tableCsv({ columns: ["Time", "Detail"], rows: [{ start: 100_500, end: 101_000, level: "data", cells: ["+0.000500 s", 'Exception, "busy"'] }] }, info);
    expect(csv).toBe('Start sample,Start (s),Time,Detail\n100500,0.000500000,+0.000500 s,"Exception, ""busy"""\n');
  });

  it("sessions round-trip and reject junk", () => {
    const s = makeSession([{ id: 1, config: defaultDecoder("modbusRtu", 8) }], [10, null], { start: 0, spp: 2 });
    expect(readSession(JSON.parse(JSON.stringify(s)))).toEqual(s);
    expect(readSession(null)).toBeNull();
    expect(readSession({ version: 2, decoders: [] })).toBeNull();
    const bad = readSession({ version: 1, decoders: [{ id: 1, config: { kind: "evil" } }, { id: 2, config: defaultDecoder("can", 8) }], cursors: ["x", 5] });
    expect(bad?.decoders.map((d) => d.id)).toEqual([2]);
    expect(bad?.cursors).toEqual([null, 5]);
  });
});

describe("CAN Explorer link", () => {
  it("labels DBC and log files by name on macOS, Linux and Windows paths", () => {
    expect(baseName("/Users/a/bus/genset.dbc")).toBe("genset.dbc");
    expect(baseName("C:\\logs\\can0.log")).toBe("can0.log");
    expect(baseName("plain.log")).toBe("plain.log");
  });
  it("new CAN decoders start without a DBC or log and a 2 ms match window", () => {
    const d = defaultDecoder("can", 2);
    expect(d.kind === "can" && d.dbc == null && d.log == null && d.logWindowMs === 2).toBe(true);
  });
});
