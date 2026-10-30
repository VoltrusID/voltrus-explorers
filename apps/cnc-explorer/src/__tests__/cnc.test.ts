import { describe, it, expect } from "vitest";
import { applyObservations, csvCell, flattenItems, formatValue, liveIdSet, prettyXml, sortConditions, toCsv, trendLabels } from "../mtc";
import { MACHINE_STATES, MAX_WATCHED_MACROS, alarmTypeName, captureStem, fanucTrendRows, parseMacroList, pmcHex } from "../fanuc";
import { needsPro, loadProfiles, loadWatch, normalizeProfile, saveProfiles, saveWatch, PROFILES_KEY, type ConnProfile } from "../storage";
import type { DataItem, Devices, Observation } from "../types";

const item = (id: string, category: DataItem["category"], name: string | null = null): DataItem => ({
  id, name, category, type: "POSITION", subType: null, units: null, representation: null,
});

const devices: Devices = {
  header: { instanceId: 1, version: "2.0", bufferSize: 100, nextSequence: 1 },
  devices: [{
    isAgent: false, id: "d", name: "M1", uuid: null, description: null,
    dataItems: [item("avail", "EVENT")],
    components: [{
      element: "Axes", id: "ax", name: null, dataItems: [],
      components: [{ element: "Linear", id: "x", name: "X", dataItems: [item("Xpos", "SAMPLE", "Xpos"), item("Xpos", "SAMPLE"), item("xc", "CONDITION")], components: [] }],
    }],
  }],
};

const obs = (id: string, sequence: number, value: number, category: Observation["category"] = "SAMPLE"): Observation => ({
  dataItemId: id, name: null, element: "Position", category, dataItemType: null, subType: null,
  timestamp: "2026-01-01T00:00:00Z", sequence, value: { kind: "Number", value }, condition: null,
});

describe("device tree", () => {
  it("flattens in document order with component paths and drops duplicate ids", () => {
    const items = flattenItems(devices);
    expect(items.map((i) => i.id)).toEqual(["avail", "Xpos", "xc"]);
    expect(items[1].path).toBe("M1/ax/X");
  });
  it("limits live ids to non-condition items", () => {
    const items = flattenItems(devices);
    expect([...liveIdSet(items, 2)]).toEqual(["avail", "Xpos"]);
    expect(liveIdSet(items, 10).has("xc")).toBe(false);
  });
  it("disambiguates trend labels that collide", () => {
    const a = flattenItems(devices).map((i) => ({ ...i, name: "Pos" }));
    expect(trendLabels(a).get("Xpos")).toBe("Pos [Xpos]");
  });
});

describe("observations", () => {
  it("applies live updates only to live ids, replace accepts all", () => {
    const base = new Map([["a", obs("a", 1, 1)], ["b", obs("b", 2, 2)]]);
    const live = (id: string) => id === "a";
    const next = applyObservations(base, [obs("a", 3, 10), obs("b", 4, 20)], live, false);
    expect(next.get("a")!.value).toEqual({ kind: "Number", value: 10 });
    expect(next.get("b")!.sequence).toBe(2);
    const all = applyObservations(base, [obs("b", 9, 99)], live, true);
    expect([...all.keys()]).toEqual(["b"]);
  });
  it("keeps the newest sequence", () => {
    const next = applyObservations(new Map([["a", obs("a", 5, 5)]]), [obs("a", 4, 0)], () => true, false);
    expect(next.get("a")!.sequence).toBe(5);
  });
  it("formats every value kind", () => {
    expect(formatValue({ kind: "Unavailable" })).toBe("UNAVAILABLE");
    expect(formatValue({ kind: "TimeSeries", value: [1, 2] })).toBe("1 2");
    expect(formatValue({ kind: "DataSet", value: [{ key: "a", value: "1", removed: false }, { key: "b", value: "2", removed: true }] })).toBe("a=1");
  });
  it("sorts faults first", () => {
    const c = (id: string, level: "Normal" | "Fault" | "Warning"): Observation => ({ ...obs(id, 1, 0, "CONDITION"), condition: { level, nativeCode: null, nativeSeverity: null, qualifier: null, message: null } });
    expect(sortConditions([c("a", "Normal"), c("b", "Fault"), c("c", "Warning")]).map((o) => o.dataItemId)).toEqual(["b", "c", "a"]);
  });
  it("quotes CSV cells", () => {
    expect(csvCell('a,"b"')).toBe('"a,""b"""');
    expect(toCsv(["x", "y"], [[1, "a,b"]])).toBe('x,y\n1,"a,b"\n');
  });
});

describe("fanuc helpers", () => {
  it("names alarm types", () => {
    expect(alarmTypeName("overtravel")).toBe("overtravel");
    expect(alarmTypeName({ unknown: 7 })).toBe("unknown 7");
  });
  it("pads PMC hex", () => {
    expect(pmcHex(10, 2)).toBe("0x000A");
  });
  it("lists trendable values", () => {
    const rows = fanucTrendRows({
      status: null, feed_rate: 100, spindle_speed: null, alarm_bits: 0, alarm_types: [], errors: [],
      program: { running: 1, main: 1, name: null, sequence: null, block: null },
      axes: [{ name: "X", absolute: 1, machine: 1, relative: 1, distance_to_go: 0, servo_load: 5 }],
      macros: [{ number: 500, value: 12 }, { number: 501, value: null }],
    });
    expect(rows.map((r) => r.name)).toEqual(["Feed rate", "Servo load X", "Absolute X", "Macro #500"]);
  });
  it("parses the watched macro list", () => {
    expect(parseMacroList("500, 501")).toEqual([500, 501]);
    expect(parseMacroList("500-503;510 510")).toEqual([500, 501, 502, 503, 510]);
    expect(parseMacroList("0, -4, abc, 1.5, 7")).toEqual([7]);
    expect(parseMacroList("1-100")).toHaveLength(MAX_WATCHED_MACROS);
    expect(parseMacroList("9-5")).toEqual([]);
    expect(parseMacroList("")).toEqual([]);
  });
  it("derives the capture stem from the chosen path", () => {
    expect(captureStem("/tmp/mill1-idle.capture.json")).toBe("/tmp/mill1-idle");
    expect(captureStem("C:\\field\\mill1.CAPTURE.JSON")).toBe("C:\\field\\mill1");
    expect(captureStem("/tmp/mill1.json")).toBe("/tmp/mill1");
    expect(captureStem("/tmp/mill1")).toBe("/tmp/mill1");
  });
  it("offers the six machine states of the field procedure", () => {
    expect(MACHINE_STATES.map((s) => s.value)).toEqual(["idle", "running", "feed-hold", "alarm", "estop", "mdi"]);
  });
  it("keeps the watched macros", () => {
    const store: Record<string, string> = {};
    expect(loadWatch({ getItem: () => null })).toBe("");
    saveWatch("500-503", { setItem: (k, v) => { store[k] = v; } });
    expect(loadWatch({ getItem: (k) => store[k] ?? null })).toBe("500-503");
  });
});

describe("profiles", () => {
  const p: ConnProfile = { name: "a", type: "fanuc", url: "", device: "", host: "h", port: 8193, timeout: 5000 };
  it("round trips and tolerates garbage", () => {
    const store: Record<string, string> = {};
    saveProfiles([p], { setItem: (k, v) => { store[k] = v; } });
    expect(loadProfiles({ getItem: (k) => store[k] ?? null })).toEqual([p]);
    expect(loadProfiles({ getItem: () => "{nope" })).toEqual([]);
    expect(loadProfiles({ getItem: (k) => (k === PROFILES_KEY ? '[1,{"name":"x"}]' : null) })[0].type).toBe("mtconnect");
    expect(normalizeProfile(null)).toBeNull();
  });
  it("free tier keeps one profile; replacing by name is allowed", () => {
    expect(needsPro([], "a", false)).toBe(false);
    expect(needsPro([p], "b", false)).toBe(true);
    expect(needsPro([p], "a", false)).toBe(false);
    expect(needsPro([p], "b", true)).toBe(false);
  });
});

describe("prettyXml", () => {
  it("indents nested elements and keeps text inline", () => {
    const x = '<a x="1"><b>hi</b><c/><d><e>1</e></d></a>';
    expect(prettyXml(x)).toBe('<a x="1">\n  <b>hi</b>\n  <c/>\n  <d>\n    <e>1</e>\n  </d>\n</a>');
  });
  it("returns plain text unchanged", () => {
    expect(prettyXml("no tags")).toBe("no tags");
  });
});
