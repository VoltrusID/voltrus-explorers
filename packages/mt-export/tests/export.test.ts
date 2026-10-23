import { existsSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { parse as parseYaml } from "yaml";
import { exportTags, FORMATS, formatsFor, groupReads, reference6 } from "../src";
import type { BacnetTagList, FormatId, ModbusTagList, TagList } from "../src";
import { BACNET_LIST, MODBUS_HIGH, MODBUS_LIST, MODBUS_LITTLE, MODBUS_RTU } from "./fixtures";
import { expectGolden } from "./golden";

const MODBUS_CASES: [string, FormatId, TagList][] = [
  ["modbus-tcp", "csv", MODBUS_LIST],
  ["modbus-tcp", "json", MODBUS_LIST],
  ["modbus-tcp", "ignition", MODBUS_LIST],
  ["modbus-tcp", "kepware", MODBUS_LIST],
  ["modbus-tcp", "rustred", MODBUS_LIST],
  ["modbus-tcp", "nodered", MODBUS_LIST],
  ["modbus-tcp", "homeassistant", MODBUS_LIST],
  ["modbus-tcp", "voltrus-scada", MODBUS_LIST],
  ["modbus-tcp", "voltrus-gateway", MODBUS_LIST],
  ["modbus-high", "voltrus-scada", MODBUS_HIGH],
  ["modbus-little", "ignition", MODBUS_LITTLE],
  ["modbus-little", "kepware", MODBUS_LITTLE],
  ["modbus-little", "rustred", MODBUS_LITTLE],
  ["modbus-little", "homeassistant", MODBUS_LITTLE],
  ["modbus-rtu", "homeassistant", MODBUS_RTU],
  ["modbus-rtu", "rustred", MODBUS_RTU],
  ["modbus-rtu", "nodered", MODBUS_RTU],
  ["bacnet", "csv", BACNET_LIST],
  ["bacnet", "json", BACNET_LIST],
  ["bacnet", "ignition", BACNET_LIST],
  ["bacnet", "kepware", BACNET_LIST],
  ["bacnet", "rustred", BACNET_LIST],
  ["bacnet", "voltrus-scada", BACNET_LIST],
  ["bacnet", "voltrus-gateway", BACNET_LIST],
];

describe("golden files", () => {
  for (const [name, id, list] of MODBUS_CASES) {
    it(`${name} as ${id}`, () => {
      const out = exportTags(id, list);
      const ext = FORMATS.find((f) => f.id === id)!.extension;
      expectGolden(`${name}.${id}.${ext}`, out.content);
      expectGolden(`${name}.${id}.warnings.txt`, out.warnings.join("\n") + (out.warnings.length ? "\n" : ""));
    });
  }
});

describe("format registry", () => {
  it("offers Home Assistant for Modbus only and Kepware for both", () => {
    expect(formatsFor("modbus").map((f) => f.id)).toContain("homeassistant");
    expect(formatsFor("modbus").map((f) => f.id)).toContain("kepware");
    expect(formatsFor("bacnet").map((f) => f.id)).toEqual(["csv", "json", "ignition", "kepware", "rustred", "voltrus-scada", "voltrus-gateway"]);
  });
  it("refuses a protocol the format cannot write", () => {
    expect(() => exportTags("homeassistant", BACNET_LIST)).toThrow(/does not support bacnet/);
  });
  it("builds 6-digit references", () => {
    expect(reference6("holding", 0)).toBe("400001");
    expect(reference6("input", 99)).toBe("300100");
    expect(reference6("coil", 9)).toBe("000010");
    expect(reference6("discrete", 65535)).toBe("165536");
  });
});

describe("Ignition schema", () => {
  const tags = (list: TagList) => JSON.parse(exportTags("ignition", list).content).tags[0].tags as Record<string, unknown>[];
  it("writes AtomicTag OPC tags with a Modbus item path inside a folder", () => {
    const root = JSON.parse(exportTags("ignition", MODBUS_LIST).content).tags[0];
    expect(root).toMatchObject({ name: "PM5560 meter", tagType: "Folder" });
    const pattern = /^\[PM5560 meter\]3\.(HR|IR|HRUS|IRUS|HRI|IRI|HRF|IRF|HRD|IRD|C|DI)\d+$/;
    for (const t of tags(MODBUS_LIST)) {
      expect(t).toMatchObject({ tagType: "AtomicTag", valueSource: "opc", opcServer: "Ignition OPC UA Server" });
      expect(String(t.opcItemPath)).toMatch(pattern);
      expect(["Int2", "Int4", "Float4", "Float8", "Boolean"]).toContain(t.dataType);
      expect(String(t.name)).toMatch(/^[A-Za-z_][A-Za-z0-9_ ()'\-:]*$/);
    }
  });
  it("uses 1-based addresses and the right designator per type", () => {
    const paths = Object.fromEntries(tags(MODBUS_LIST).map((t) => [t.name, t.opcItemPath]));
    expect(paths.Voltage_3000).toBe("[PM5560 meter]3.HRF3001");
    expect(paths.Counter_10).toBe("[PM5560 meter]3.HRUS11");
    expect(paths.Temperature_20).toBe("[PM5560 meter]3.HR21");
    expect(paths.Energy_100).toBe("[PM5560 meter]3.IRI101");
    expect(paths.Flow_200).toBe("[PM5560 meter]3.IRD201");
    expect(paths.Run_state_0).toBe("[PM5560 meter]3.C1");
    expect(paths.Door_5).toBe("[PM5560 meter]3.DI6");
  });
  it("turns scale into Linear scaling and marks read-only tags", () => {
    const t = tags(MODBUS_LIST).find((x) => x.name === "Temperature_20")!;
    expect(t).toMatchObject({ scaleMode: "Linear", rawLow: 0, rawHigh: 1, scaledLow: 0, scaledHigh: 0.1, dataType: "Float8" });
    expect(tags(MODBUS_LIST).find((x) => x.name === "Energy_100")).toMatchObject({ readOnly: true });
  });
  it("uses the BACnet item path form", () => {
    expect(tags(BACNET_LIST).map((t) => t.opcItemPath)).toEqual([
      "ns=1;s=[AHU-1]AnalogInput1.Present_Value",
      "ns=1;s=[AHU-1]BinaryOutput2.Present_Value",
      "ns=1;s=[AHU-1]MultiStateValue3.Present_Value",
    ]);
  });
});

describe("Kepware CSV", () => {
  const rows = exportTags("kepware", MODBUS_LIST).content.trimEnd().split("\n");
  it("has the documented header and one record per tag with 17 fields", () => {
    expect(rows[0]).toBe(
      "Tag Name,Address,Data Type,Respect Data Type,Client Access,Scan Rate,Scaling,Raw Low,Raw High,Scaled Low,Scaled High,Scaled Data Type,Clamp Low,Clamp High,Eng Units,Description,Negate Value",
    );
    expect(rows).toHaveLength(MODBUS_LIST.tags.length + 1);
    for (const r of rows) expect(r.match(/("[^"]*(?:""[^"]*)*"|[^,]*)(,|$)/g)!.length).toBeGreaterThanOrEqual(17);
  });
  it("addresses with six digits and Kepware data types", () => {
    const byName = Object.fromEntries(rows.slice(1).map((r) => [r.split(",")[0], r.split(",")]));
    expect(byName.Voltage_3000.slice(1, 6)).toEqual(["403001", "Float", "1", "R/W", "1000"]);
    expect(byName.Counter_10[2]).toBe("Word");
    expect(byName.Temperature_20.slice(2, 3).concat(byName.Temperature_20.slice(6, 12))).toEqual(["Short", "Linear", "0", "1", "0", "0.1", "Double"]);
    expect(byName.Energy_100.slice(1, 5)).toEqual(["300101", "Long", "1", "RO"]);
    expect(byName.Flow_200[2]).toBe("Double");
    expect(byName.Run_state_0.slice(1, 3)).toEqual(["000001", "Boolean"]);
    expect(byName.Door_5.slice(1, 3)).toEqual(["100006", "Boolean"]);
  });
  it("addresses BACnet objects as <object type>.<instance>.<property> with the driver's data types", () => {
    const out = exportTags("kepware", BACNET_LIST);
    const lines = out.content.trimEnd().split("\n");
    expect(lines[0]).toBe(rows[0]);
    const f = lines.slice(1).map((l) => l.split(","));
    expect(f.map((r) => r.slice(0, 6))).toEqual([
      ["Zone temp", "AnalogInput.1.PresentValue", "Float", "1", "RO", "5000"],
      ["Fan cmd", "BinaryOutput.2.PresentValue", "Boolean", "1", "R/W", "5000"],
      ["Mode", "MultistateValue.3.PresentValue", "DWord", "1", "R/W", "5000"],
    ]);
    expect(f[0][14]).toBe("°C");
    expect(out.warnings).toEqual(["Kepware reads the device instance, the IP address and the port from the device, not the tag file: set Device ID 1001 and 192.168.1.60:47808 on the BACnet/IP device."]);
  });
  it("types other properties", () => {
    const list: BacnetTagList = {
      ...BACNET_LIST,
      tags: [
        { name: "n", objectType: "analogInput", instance: 1, property: "objectName", writable: false },
        { name: "u", objectType: "analogInput", instance: 1, property: "units", writable: false },
        { name: "o", objectType: "binaryValue", instance: 9, property: "outOfService", writable: true },
      ],
    };
    const f = exportTags("kepware", list).content.trimEnd().split("\n").slice(1).map((l) => l.split(",").slice(1, 3));
    expect(f).toEqual([["AnalogInput.1.ObjectName", "String"], ["AnalogInput.1.Units", "DWord"], ["BinaryValue.9.OutOfService", "Boolean"]]);
  });
});

describe("flows", () => {
  const flow = (id: "rustred" | "nodered", list: ModbusTagList = MODBUS_LIST) =>
    JSON.parse(exportTags(id, list).content) as Record<string, any>[];

  for (const id of ["rustred", "nodered"] as const) {
    it(`${id}: every wire leads to a node and ids are unique`, () => {
      const nodes = flow(id);
      const ids = new Set(nodes.map((n) => n.id));
      expect(ids.size).toBe(nodes.length);
      for (const n of nodes) for (const out of n.wires ?? []) for (const to of out) expect(ids.has(to)).toBe(true);
      const cfg = nodes.find((n) => n.type === (id === "rustred" ? "modbus-config" : "modbus-client"))!;
      for (const n of nodes.filter((x) => x.type === (id === "rustred" ? "modbus read" : "modbus-read"))) {
        expect(n[id === "rustred" ? "configNode" : "server"]).toBe(cfg.id);
      }
    });
  }

  it("rustred BACnet: reads are wired to the config, the inject node and the naming function", () => {
    const nodes = JSON.parse(exportTags("rustred", BACNET_LIST).content) as Record<string, any>[];
    const ids = new Set(nodes.map((n) => n.id));
    expect(ids.size).toBe(nodes.length);
    for (const n of nodes) for (const out of n.wires ?? []) for (const to of out) expect(ids.has(to)).toBe(true);
    const cfg = nodes.find((n) => n.type === "bacnet-config")!;
    expect(cfg).toMatchObject({ deviceId: 1001, targetHost: "192.168.1.60", targetPort: 47808 });
    const reads = nodes.filter((n) => n.type === "bacnet read");
    expect(reads.map((r) => [r.name, r.objectType, r.objectInstance, r.property, r.configNode])).toEqual([
      ["Zone temp", "analogInput", 1, "presentValue", cfg.id],
      ["Fan cmd", "binaryOutput", 2, "presentValue", cfg.id],
      ["Mode", "multiStateValue", 3, "presentValue", cfg.id],
    ]);
    const inject = nodes.find((n) => n.type === "inject")!;
    expect(inject).toMatchObject({ repeat: "5", once: true });
    expect(inject.wires[0]).toEqual(reads.map((r) => r.id));
  });
  it("rustred BACnet: the function node names a value from msg.bacnet", () => {
    const nodes = JSON.parse(exportTags("rustred", BACNET_LIST).content) as Record<string, any>[];
    const fn = new Function("msg", nodes.find((n) => n.type === "function")!.func);
    const out = fn({ payload: 21.5, bacnet: { objectType: "binaryOutput", objectInstance: 2, property: "presentValue" } });
    expect(out.payload).toEqual({ "Fan cmd": 21.5 });
    expect(out.topic).toBe("AHU-1");
  });
  it("rustred BACnet: warns about writable tags", () => {
    expect(exportTags("rustred", BACNET_LIST).warnings[0]).toMatch(/^2 writable tags are read only/);
  });
  it("has no Node-RED variant for BACnet", () => {
    expect(formatsFor("bacnet").some((f) => f.id === "nodered")).toBe(false);
  });

  it("folds neighbouring tags into one read and splits at the protocol limit", () => {
    expect(groupReads(MODBUS_LIST.tags).map((r) => [r.area, r.address, r.tags.length])).toEqual([
      ["holding", 3000, 2], ["holding", 10, 1], ["holding", 20, 1], ["input", 100, 1], ["input", 200, 1], ["coil", 0, 1], ["discrete", 5, 1],
    ]);
    const many: ModbusTagList = { ...MODBUS_LIST, tags: Array.from({ length: 130 }, (_, i) => ({ name: `T${i}`, area: "holding" as const, address: i, type: "uint16" as const, scale: 1, writable: true })) };
    expect(groupReads(many.tags).map((r) => r.tags.length)).toEqual([125, 5]);
  });

  it("rustred reads typed values for big-endian word order and raw words otherwise", () => {
    const typed = flow("rustred").find((n) => n.type === "modbus read" && n.address === 3000)!;
    expect(typed).toMatchObject({ functionCode: "readHoldingRegisters", quantity: 2, dataType: "float", pollIntervalMs: 1000 });
    const raw = flow("rustred", MODBUS_LITTLE).find((n) => n.type === "modbus read" && n.address === 3000)!;
    expect(raw.dataType).toBeUndefined();
    expect(raw.quantity).toBe(4);
  });

  // Run the generated function nodes on registers a device would send for each word order.
  const run = (node: Record<string, any>, payload: unknown[]) => {
    const msg = { payload } as Record<string, any>;
    return new Function("msg", node.func)(msg).payload as Record<string, number>;
  };
  const words = (type: "float32" | "int32" | "int16" | "uint16" | "float64", value: number, order: "big" | "little" | "swapped") => {
    const buf = new DataView(new ArrayBuffer(8));
    let n = 1;
    if (type === "float32") { buf.setFloat32(0, value); n = 2; }
    else if (type === "int32") { buf.setInt32(0, value); n = 2; }
    else if (type === "int16") buf.setInt16(0, value);
    else if (type === "uint16") buf.setUint16(0, value);
    else { buf.setFloat64(0, value); n = 4; }
    let w = Array.from({ length: n }, (_, i) => buf.getUint16(i * 2));
    if (order === "little") w = w.reverse();
    if (order === "swapped") w = w.map((x) => ((x & 0xff) << 8) | (x >> 8));
    return w;
  };

  for (const order of ["big", "little", "swapped"] as const) {
    it(`node-red function decodes ${order} word order`, () => {
      const list: ModbusTagList = {
        ...MODBUS_LIST,
        source: { ...MODBUS_LIST.source, wordOrder: order },
        tags: [
          { name: "a", area: "holding", address: 0, type: "float32", scale: 1, writable: true },
          { name: "b", area: "holding", address: 2, type: "float32", scale: 2, writable: true },
        ],
      };
      const read = flow("nodered", list);
      const fn = read.find((n) => n.type === "function")!;
      const out = run(fn, [...words("float32", 12.5, order), ...words("float32", -3.25, order)]);
      expect(out).toEqual({ a: 12.5, b: -6.5 });
      expect(fn.func).toContain("msg.topic");
    });
    it(`node-red function decodes int and 64-bit types, ${order}`, () => {
      const list: ModbusTagList = {
        ...MODBUS_LIST,
        source: { ...MODBUS_LIST.source, wordOrder: order },
        tags: [{ name: "x", area: "input", address: 0, type: "int32", scale: 1, writable: false }, { name: "y", area: "input", address: 2, type: "int32", scale: 0.5, writable: false }],
      };
      const fn = flow("nodered", list).find((n) => n.type === "function")!;
      expect(run(fn, [...words("int32", -70000, order), ...words("int32", 100000, order)])).toEqual({ x: -70000, y: 50000 });
      const d: ModbusTagList = { ...list, tags: [{ name: "d", area: "input", address: 0, type: "float64", scale: 1, writable: false }] };
      expect(run(flow("nodered", d).find((n) => n.type === "function")!, words("float64", Math.PI, order))).toEqual({ d: Math.PI });
      const s: ModbusTagList = { ...list, tags: [{ name: "i", area: "input", address: 0, type: "int16", scale: 1, writable: false }, { name: "j", area: "input", address: 1, type: "int16", scale: 1, writable: false }] };
      expect(run(flow("nodered", s).find((n) => n.type === "function")!, [...words("int16", -5, order), ...words("int16", 300, order)])).toEqual({ i: -5, j: 300 });
      const u: ModbusTagList = { ...list, tags: [{ name: "u", area: "input", address: 0, type: "uint16", scale: 1, writable: false }] };
      expect(run(flow("nodered", u).find((n) => n.type === "function")!, words("uint16", 65000, order))).toEqual({ u: 65000 });
    });
  }

  it("bit reads name the booleans", () => {
    const fn = flow("rustred").find((n) => n.type === "function" && String(n.name).includes("Run_state_0"))!;
    expect(run(fn, [true])).toEqual({ Run_state_0: true });
  });

  it("rustred typed function applies the scale", () => {
    const fn = flow("rustred").find((n) => n.type === "function" && String(n.name).includes("Temperature_20"))!;
    expect(run(fn, [215])).toEqual({ Temperature_20: 21.5 });
  });
});

describe("Home Assistant YAML", () => {
  // Parameters and values from https://www.home-assistant.io/integrations/modbus/
  const SENSOR_KEYS = ["name", "unique_id", "address", "device_address", "input_type", "data_type", "swap", "scale", "offset", "precision", "scan_interval", "unit_of_measurement"];
  const BINARY_KEYS = ["name", "unique_id", "address", "device_address", "input_type", "scan_interval"];
  const DATA_TYPES = ["int16", "uint16", "int32", "uint32", "int64", "uint64", "float16", "float32", "float64"];

  function check(list: ModbusTagList) {
    const doc = parseYaml(exportTags("homeassistant", list).content) as { modbus: Record<string, any>[] };
    expect(doc.modbus).toHaveLength(1);
    const hub = doc.modbus[0];
    expect(hub.name).toMatch(/^[a-z0-9_]+$/);
    if (hub.type === "tcp") {
      expect(typeof hub.host).toBe("string");
      expect(Number.isInteger(hub.port)).toBe(true);
    } else {
      expect(hub.type).toBe("serial");
      for (const k of ["port", "baudrate", "bytesize", "method", "parity", "stopbits"]) expect(hub[k]).toBeDefined();
      expect(["E", "O", "N"]).toContain(hub.parity);
      expect([5, 6, 7, 8]).toContain(hub.bytesize);
      expect(["rtu", "ascii"]).toContain(hub.method);
      expect([1, 2]).toContain(hub.stopbits);
    }
    const names = new Set<string>();
    for (const s of hub.sensors ?? []) {
      expect(Object.keys(s).every((k) => SENSOR_KEYS.includes(k))).toBe(true);
      expect(["holding", "input"]).toContain(s.input_type);
      expect(DATA_TYPES).toContain(s.data_type);
      if (s.swap) {
        expect(["byte", "word", "word_byte"]).toContain(s.swap);
        if (s.swap !== "byte") expect(["int16", "uint16"]).not.toContain(s.data_type);
      }
      expect(Number.isInteger(s.address) && s.address >= 0).toBe(true);
      expect(Number.isInteger(s.scan_interval) && s.scan_interval >= 1).toBe(true);
      expect(names.has(s.name)).toBe(false);
      names.add(s.name);
    }
    for (const b of hub.binary_sensors ?? []) {
      expect(Object.keys(b).every((k) => BINARY_KEYS.includes(k))).toBe(true);
      expect(["coil", "discrete_input"]).toContain(b.input_type);
    }
    return hub;
  }

  it("is valid for TCP, word swap and serial hubs", () => {
    const tcp = check(MODBUS_LIST);
    expect(tcp.sensors).toHaveLength(6);
    expect(tcp.binary_sensors).toHaveLength(2);
    expect(tcp.sensors[3]).toMatchObject({ name: "Temperature_20", scale: 0.1, precision: 1, data_type: "int16" });
    expect(tcp.binary_sensors[1].input_type).toBe("discrete_input");
    expect(check(MODBUS_LITTLE).sensors.filter((s: any) => s.swap === "word").length).toBe(3);
    const rtu = check(MODBUS_RTU);
    expect(rtu.parity).toBe("E");
    expect(rtu.sensors.every((s: any) => s.swap === "byte")).toBe(true);
  });
});

describe("Voltrus formats", () => {
  it("SCADA device file carries what create_modbus_device reads", () => {
    const d = JSON.parse(exportTags("voltrus-scada", MODBUS_LIST).content);
    expect(d).toMatchObject({ id: "pm5560_meter", name: "PM5560 meter", ip: "192.168.1.50", port: 502, unit_id: 3, interval_ms: 1000, endianness: "big" });
    expect(d.registers).toHaveLength(8);
    expect(d.registers[6]).toEqual({ name: "Run_state_0", register_area: "coil", offset: 0, register_type: "uint16", scale: 1 });
    expect(d.registers.every((r: Record<string, unknown>) => !("address" in r))).toBe(true);
  });
  it("keeps every address, including those above 30000, as an explicit offset", () => {
    const list: ModbusTagList = {
      ...MODBUS_LIST,
      tags: [
        { name: "far", area: "holding", address: 40000, type: "uint16", scale: 1, writable: true },
        { name: "last", area: "input", address: 65535, type: "uint16", scale: 1, writable: false },
        { name: "edge", area: "holding", address: 30000, type: "uint16", scale: 1, writable: true },
      ],
    };
    const out = exportTags("voltrus-scada", list);
    expect(JSON.parse(out.content).registers.map((r: any) => [r.name, r.register_area, r.offset])).toEqual([
      ["far", "holding", 40000], ["last", "input", 65535], ["edge", "holding", 30000],
    ]);
    expect(out.warnings).toEqual([]);
  });
  it("gateway file is proto3 JSON for ConnectRequest and TagAddress", () => {
    const g = JSON.parse(exportTags("voltrus-gateway", MODBUS_LIST).content);
    expect(g.connect.modbus).toMatchObject({ ip: "192.168.1.50", unitId: 3, endianness: "big" });
    expect(g.tags[0]).toEqual({ modbus: { name: "Voltage_3000", address: 3000, regType: "float32", scale: 1, registerArea: "holding" } });
  });
});

/**
 * The SCADA's own device file parser (voltrus/client/src/deviceFileImport.ts) reads what the SCADA format writes and gets
 * the same tags back. Runs when a Voltrus checkout is next to this repo (or VOLTRUS_CLIENT_SRC points at client/src); the
 * SCADA repo runs the import into its register model against a copy of this export (src/storage/tests/modbus_tests.rs).
 */
const parserPath = resolve(
  process.env.VOLTRUS_CLIENT_SRC ?? resolve(dirname(fileURLToPath(import.meta.url)), "../../../../voltrus/client/src"),
  "deviceFileImport.ts",
);
describe.skipIf(!existsSync(parserPath))("SCADA import round trip", () => {
  for (const [name, list] of [["modbus-tcp", MODBUS_LIST], ["modbus-high", MODBUS_HIGH]] as const) {
    it(`${name}: the SCADA importer returns the exported tags`, async () => {
      const { parseModbusDeviceFile } = await import(/* @vite-ignore */ parserPath);
      const { devices, errors } = parseModbusDeviceFile(exportTags("voltrus-scada", list).content);
      expect(errors).toEqual([]);
      expect(devices).toHaveLength(1);
      const s = list.source;
      expect(devices[0]).toMatchObject({ name: s.name, ip: s.host, port: s.port, unit_id: s.unitId, interval_ms: s.pollMs, endianness: s.wordOrder });
      expect(devices[0].registers).toEqual(
        list.tags.map((t) => ({ name: t.name, offset: t.address, register_type: t.type === "bool" ? "uint16" : t.type, register_area: t.area, scale: t.scale })),
      );
    });
  }
});
