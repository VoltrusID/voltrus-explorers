import { describe, it, expect } from "vitest";
import {
  ackText, csvCell, eventRow, hostConnectOf, hostFormOf, parseIdList, pushCapped, rangeProblem, sameHost, toCsv,
} from "../gem";
import {
  FREE_LIVE_SVS, FREE_PROFILE_LIMIT, HOST_DEFAULTS, PROFILES_KEY, liveSelectionAllowed, loadProfiles, needsPro, normalizeProfile,
  saveProfiles, type ConnProfile,
} from "../storage";

const profile = (name: string): ConnProfile => ({ ...HOST_DEFAULTS, name });

describe("free and Pro gating", () => {
  it("keeps one saved profile in the free tier; replacing by name is allowed", () => {
    expect(FREE_PROFILE_LIMIT).toBe(1);
    expect(needsPro([], "a", false)).toBe(false);
    expect(needsPro([profile("a")], "b", false)).toBe(true);
    expect(needsPro([profile("a")], "a", false)).toBe(false);
    expect(needsPro([profile("a")], "b", true)).toBe(false);
  });
  it("lets the free tier poll ten status variables live, Pro any number", () => {
    expect(liveSelectionAllowed(FREE_LIVE_SVS, false)).toBe(true);
    expect(liveSelectionAllowed(FREE_LIVE_SVS + 1, false)).toBe(false);
    expect(liveSelectionAllowed(500, true)).toBe(true);
  });
});

describe("profiles", () => {
  it("round-trips through storage", () => {
    const mem: Record<string, string> = {};
    const p: ConnProfile = { ...profile("etch"), mode: "passive", port: 5001, t3: 30 };
    saveProfiles([p], { setItem: (k, v) => { mem[k] = v; } });
    expect(loadProfiles({ getItem: (k) => mem[k] ?? null })).toEqual([p]);
    expect(Object.keys(mem)).toEqual([PROFILES_KEY]);
  });
  it("fills defaults and drops entries without a name", () => {
    expect(normalizeProfile({ name: "x", port: "no", mode: "weird" })).toEqual({ ...HOST_DEFAULTS, name: "x" });
    expect(normalizeProfile({ port: 5000 })).toBeNull();
    expect(normalizeProfile(null)).toBeNull();
    expect(normalizeProfile({ name: "x", port: 70000 })!.port).toBe(HOST_DEFAULTS.port);
  });
  it("survives corrupt storage", () => {
    expect(loadProfiles({ getItem: () => "{not json" })).toEqual([]);
    expect(loadProfiles({ getItem: () => '{"a":1}' })).toEqual([]);
  });
});

describe("connection form", () => {
  it("converts timers to milliseconds", () => {
    const c = hostConnectOf({ ...HOST_DEFAULTS, host: " 10.0.0.5 ", t3: 2.5 });
    expect(c.host).toBe("10.0.0.5");
    expect([c.t3Ms, c.t5Ms, c.t6Ms, c.t7Ms, c.t8Ms]).toEqual([2500, 10000, 5000, 10000, 5000]);
  });
  it("reads form values with defaults for blanks", () => {
    const f = hostFormOf({ mode: "passive", host: "0.0.0.0", port: "5001", deviceId: "", t3: 10 }, HOST_DEFAULTS);
    expect(f).toMatchObject({ mode: "passive", host: "0.0.0.0", port: 5001, deviceId: 0, t3: 10, t5: 10 });
    expect(sameHost(f, f)).toBe(true);
    expect(sameHost(f, { ...f, port: 5002 })).toBe(false);
  });
});

describe("ids and ranges", () => {
  it("parses id lists and ranges", () => {
    expect(parseIdList("1, 2 5-7;9")).toEqual([1, 2, 5, 6, 7, 9]);
    expect(parseIdList("")).toEqual([]);
  });
  it("rejects bad lists", () => {
    expect(() => parseIdList("a")).toThrow(/not an id/);
    expect(() => parseIdList("7-5")).toThrow(/backwards/);
    expect(() => parseIdList("1-100000")).toThrow(/More than/);
  });
  it("reports range errors for equipment constants", () => {
    expect(rangeProblem(25, 20, 400)).toBeNull();
    expect(rangeProblem(10, 20, 400)).toBe("Out of range: 20 to 400");
    expect(rangeProblem(401, 20, 400)).toBe("Out of range: 20 to 400");
    expect(rangeProblem(5, null, null)).toBeNull();
    expect(rangeProblem(NaN, 1, 2)).toBe("Not a number");
    expect(rangeProblem(0, 1, null)).toBe("Below the minimum 1");
  });
  it("names acknowledge codes", () => {
    expect(ackText({ code: 0, name: "Accepted", ok: true })).toBe("Accepted");
    expect(ackText({ code: 3, name: "OutOfRange", ok: false })).toBe("OutOfRange (code 3)");
  });
});

describe("events and csv", () => {
  it("names the values of an event report from the reports defined here", () => {
    const ev = { kind: "event" as const, dataid: 7, ceid: 100, reports: [{ rptid: 1, values: [{ text: "3", number: 3, kind: "U4" }, { text: "ETCH", number: null, kind: "A" }] }] };
    const row = eventRow(1000, ev, [{ rptid: 1, vids: [12, 13], ceids: [100] }], new Map([[12, "WaferCount"]]));
    expect(row).toEqual({ time: 1000, dataid: 7, ceid: 100, summary: "RPTID 1: WaferCount=3, VID 13=ETCH" });
  });
  it("shows values of an unknown report as they are", () => {
    const ev = { kind: "event" as const, dataid: 1, ceid: 5, reports: [{ rptid: 9, values: [{ text: "1", number: 1, kind: "U1" }] }] };
    expect(eventRow(0, ev, [], new Map()).summary).toBe("RPTID 9: 1");
    expect(eventRow(0, { ...ev, reports: [] }, [], new Map()).summary).toBe("(no reports linked)");
  });
  it("quotes csv cells", () => {
    expect(csvCell('a,"b"')).toBe('"a,""b"""');
    expect(toCsv(["x", "y"], [[1, "a b"], [2, "c\nd"]])).toBe('x,y\n1,a b\n2,"c\nd"\n');
  });
  it("caps lists from the front", () => {
    expect(pushCapped([1, 2, 3], 4, 3)).toEqual([2, 3, 4]);
    expect(pushCapped([1], 2, 3)).toEqual([1, 2]);
  });
});
