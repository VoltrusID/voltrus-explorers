import type { ExportFormat, ModbusTag, ModbusTagList, TagList } from "../model";
import { fileStem, toJson, typeWidth } from "../util";

/**
 * Node-RED and RustRED flow JSON for a Modbus tag list.
 *
 * Both flows have the same shape: one `modbus-*` configuration node for the device, one read node for every run of
 * neighbouring tags of the same area and type, a function node that names the values (and decodes them, where the read
 * node only returns registers), and a shared debug node.
 *
 * RustRED (https://github.com/dzulfikar08/rust-red, nodes in crates/core/src/runtime/nodes/industrial_nodes):
 * - `modbus-config` (host, port, transport "tcp" or "serial", unitId, timeoutMs, serialPort, baudRate, dataBits, stopBits, parity)
 *   and `modbus read` (configNode, functionCode readCoils|readDiscreteInputs|readHoldingRegisters|readInputRegisters, address,
 *   quantity, dataType uint16|int16|uint32|int32|float|uint64|int64|double, pollIntervalMs). The read node converts register
 *   words to values itself, high word first (AB CD); for the other word orders it reads raw registers and the function decodes.
 *   `quantity` counts values, not registers. Example flow: poc/flows_modbus_to_postgres.json in that repo.
 * Node-RED, node-red-contrib-modbus (https://flows.nodered.org/node/node-red-contrib-modbus): `modbus-client` (clienttype, tcpHost,
 *   tcpPort, unit_id, serial* fields) and `modbus-read` (dataType HoldingRegister|InputRegister|Coil|Input, adr, quantity in
 *   registers or bits, rate and rateUnit, unitid, server). It returns raw words in msg.payload, so the function node decodes
 *   them. RustRED also runs these two node types as exported.
 */

interface Read {
  area: ModbusTag["area"];
  type: ModbusTag["type"];
  address: number;
  tags: ModbusTag[];
}

const MAX_REGISTERS = 125;
const MAX_BITS = 2000;

/** Neighbouring tags of one area and type, in list order, split so one request stays inside the protocol limit. */
export function groupReads(tags: ModbusTag[]): Read[] {
  const reads: Read[] = [];
  for (const t of tags) {
    const last = reads[reads.length - 1];
    const bit = t.area === "coil" || t.area === "discrete";
    const width = bit ? 1 : typeWidth(t.type);
    if (last) {
      const lastTag = last.tags[last.tags.length - 1];
      const lastWidth = bit ? 1 : typeWidth(last.type);
      const used = last.tags.length * lastWidth;
      if (
        last.area === t.area && last.type === t.type && lastTag.address + lastWidth === t.address &&
        used + width <= (bit ? MAX_BITS : MAX_REGISTERS)
      ) {
        last.tags.push(t);
        continue;
      }
    }
    reads.push({ area: t.area, type: t.type, address: t.address, tags: [t] });
  }
  return reads;
}

/** Same 16 hex digits every export, so the file does not change when nothing else did. */
function nodeId(seed: string): string {
  let a = 0x811c9dc5, b = 0x01000193;
  for (let i = 0; i < seed.length; i++) {
    a = Math.imul(a ^ seed.charCodeAt(i), 16777619) >>> 0;
    b = Math.imul(b ^ seed.charCodeAt(i), 2246822519) >>> 0;
  }
  return a.toString(16).padStart(8, "0") + b.toString(16).padStart(8, "0");
}

const RUSTRED_FUNCTION = {
  coil: "readCoils",
  discrete: "readDiscreteInputs",
  holding: "readHoldingRegisters",
  input: "readInputRegisters",
} as const;
const RUSTRED_TYPE: Record<string, string> = { uint16: "uint16", int16: "int16", int32: "int32", float32: "float", float64: "double" };
const CONTRIB_TYPE = { coil: "Coil", discrete: "Input", holding: "HoldingRegister", input: "InputRegister" } as const;

/** Function node body: names every value of one read, decoding registers when `raw`. */
function functionBody(read: Read, wordOrder: ModbusTagList["source"]["wordOrder"], deviceName: string, raw: boolean): string {
  const bit = read.area === "coil" || read.area === "discrete";
  const lines: string[] = [];
  if (bit) {
    lines.push(`const names = ${JSON.stringify(read.tags.map((t) => t.name))};`);
    lines.push("const values = {};", "names.forEach((n, i) => { values[n] = msg.payload[i]; });");
  } else if (!raw) {
    lines.push(`const specs = ${JSON.stringify(read.tags.map((t) => [t.name, t.scale]))};`);
    lines.push("const values = {};", "specs.forEach(([n, scale], i) => { values[n] = msg.payload[i] * scale; });");
  } else {
    lines.push(`const specs = ${JSON.stringify(read.tags.map((t) => [t.name, t.type, t.scale]))};`);
    lines.push(`const width = { uint16: 1, int16: 1, int32: 2, float32: 2, float64: 4 };`);
    lines.push("const values = {};", "let at = 0;");
    lines.push("for (const [name, type, scale] of specs) {", "  let words = msg.payload.slice(at, at + width[type]);", "  at += width[type];");
    if (wordOrder === "little") lines.push("  words = words.reverse();");
    if (wordOrder === "swapped") lines.push("  words = words.map((w) => ((w & 0xff) << 8) | (w >> 8));");
    lines.push(
      "  const view = new DataView(new ArrayBuffer(8));",
      "  words.forEach((w, i) => view.setUint16(i * 2, w));",
      "  const raw = type === 'uint16' ? view.getUint16(0) : type === 'int16' ? view.getInt16(0) : type === 'int32' ? view.getInt32(0)",
      "    : type === 'float32' ? view.getFloat32(0) : view.getFloat64(0);",
      "  values[name] = raw * scale;",
      "}",
    );
  }
  lines.push("msg.payload = values;", `msg.topic = ${JSON.stringify(deviceName)};`, "return msg;");
  return lines.join("\n");
}

const readName = (r: Read): string => {
  const first = r.tags[0].name;
  return r.tags.length === 1 ? first : `${first} +${r.tags.length - 1}`;
};

interface Built {
  nodes: Record<string, unknown>[];
  warnings: string[];
}

function build(list: ModbusTagList, variant: "rustred" | "nodered"): Built {
  const s = list.source;
  const warnings: string[] = [];
  const tab = nodeId(`tab:${s.name}`);
  const config = nodeId(`config:${s.name}`);
  const debug = nodeId(`debug:${s.name}`);
  const nodes: Record<string, unknown>[] = [{ id: tab, type: "tab", label: s.name, disabled: false, info: "Exported from Modbus Poll" }];

  if (variant === "rustred") {
    const cfg: Record<string, unknown> = { id: config, type: "modbus-config", name: s.name, transport: s.transport === "rtu" ? "serial" : "tcp", host: s.host, port: s.port, unitId: s.unitId, timeoutMs: 5000 };
    if (s.transport === "rtu" && s.serial) Object.assign(cfg, { serialPort: s.serial.port, baudRate: s.serial.baudRate, dataBits: String(s.serial.dataBits), stopBits: String(s.serial.stopBits), parity: s.serial.parity });
    nodes.push(cfg);
    if (s.transport === "rtu") warnings.push("RustRED opens serial ports only in builds with the nodes_modbus_serial feature.");
  } else {
    const rtu = s.transport === "rtu";
    const ser = s.serial;
    nodes.push({
      id: config, type: "modbus-client", name: s.name, clienttype: rtu ? "serial" : "tcp", bufferCommands: true, stateLogEnabled: false,
      queueLogEnabled: false, failureLogEnabled: true, tcpHost: s.host, tcpPort: String(s.port), tcpType: "DEFAULT",
      serialPort: ser?.port ?? "/dev/ttyUSB0", serialType: "RTU-BUFFERD", serialBaudrate: String(ser?.baudRate ?? 9600),
      serialDatabits: String(ser?.dataBits ?? 8), serialStopbits: String(ser?.stopBits ?? 1), serialParity: ser?.parity ?? "none",
      serialConnectionDelay: "100", serialAsciiResponseStartDelimiter: "0x3A", unit_id: s.unitId, commandDelay: "1",
      clientTimeout: "5000", reconnectOnTimeout: true, reconnectTimeout: "2000", parallelUnitIdsAllowed: true, showErrors: false,
      showWarnings: true, showLogs: true,
    });
  }

  const reads = groupReads(list.tags);
  reads.forEach((r, i) => {
    const bit = r.area === "coil" || r.area === "discrete";
    const readId = nodeId(`read:${s.name}:${i}`);
    const funcId = nodeId(`func:${s.name}:${i}`);
    const y = 80 + i * 60;
    const typed = variant === "rustred" && s.wordOrder === "big" && !bit;
    const width = bit ? 1 : typeWidth(r.type);
    if (variant === "rustred") {
      nodes.push({
        id: readId, type: "modbus read", z: tab, name: readName(r), configNode: config, functionCode: RUSTRED_FUNCTION[r.area],
        address: r.address, quantity: typed || bit ? r.tags.length : r.tags.length * width, ...(typed ? { dataType: RUSTRED_TYPE[r.type] } : {}),
        pollIntervalMs: s.pollMs, x: 200, y, wires: [[funcId]],
      });
    } else {
      const seconds = s.pollMs % 1000 === 0;
      nodes.push({
        id: readId, type: "modbus-read", z: tab, name: readName(r), topic: "", showStatusActivities: false, logIOActivities: false,
        showErrors: false, showWarnings: true, unitid: String(s.unitId), dataType: CONTRIB_TYPE[r.area], adr: String(r.address),
        quantity: String(bit ? r.tags.length : r.tags.length * width), rate: String(seconds ? s.pollMs / 1000 : s.pollMs),
        rateUnit: seconds ? "s" : "ms", delayOnStart: false, enableDeformedMessages: false, startDelayTime: "", server: config,
        useIOFile: false, ioFile: "", useIOForPayload: false, emptyMsgOnFail: false, x: 200, y, wires: [[funcId], []],
      });
    }
    nodes.push({
      id: funcId, type: "function", z: tab, name: `name ${readName(r)}`, func: functionBody(r, s.wordOrder, s.name, !typed && !bit),
      outputs: 1, timeout: 0, noerr: 0, initialize: "", finalize: "", libs: [], x: 480, y, wires: [[debug]],
    });
  });
  nodes.push({
    id: debug, type: "debug", z: tab, name: "values", active: true, tosidebar: true, console: true, tostatus: false, complete: "payload",
    targetType: "msg", statusVal: "", statusType: "auto", x: 740, y: 80, wires: [],
  });

  if (list.tags.length === 0) warnings.push("The tag list is empty, so the flow has no read nodes.");
  if (s.pollMs < 100) warnings.push(`The poll interval is ${s.pollMs} ms; every read node polls that fast.`);
  return { nodes, warnings };
}

export const rustredFormat: ExportFormat = {
  id: "rustred",
  label: "RustRED flow (JSON)",
  hint: "Modbus read nodes and a Modbus config for RustRED's editor",
  extension: "json",
  protocols: ["modbus"],
  export(list: TagList) {
    if (list.protocol !== "modbus") throw new Error("Flow export is for Modbus tag lists");
    const { nodes, warnings } = build(list, "rustred");
    return { fileName: `${fileStem(list.source.name)}-rustred-flow.json`, content: toJson(nodes), warnings };
  },
};

export const noderedFormat: ExportFormat = {
  id: "nodered",
  label: "Node-RED flow (JSON)",
  hint: "node-red-contrib-modbus read nodes; RustRED runs it too",
  extension: "json",
  protocols: ["modbus"],
  export(list: TagList) {
    if (list.protocol !== "modbus") throw new Error("Flow export is for Modbus tag lists");
    const { nodes, warnings } = build(list, "nodered");
    warnings.push("Install node-red-contrib-modbus in Node-RED before importing the flow.");
    return { fileName: `${fileStem(list.source.name)}-nodered-flow.json`, content: toJson(nodes), warnings };
  },
};
