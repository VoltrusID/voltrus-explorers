import type { BacnetTag, BacnetTagList, ExportOptions } from "../model";
import { nodeId } from "../util";

/**
 * RustRED flow for a BACnet tag list (https://github.com/dzulfikar08/rust-red, nodes in
 * crates/core/src/runtime/nodes/industrial_nodes/bacnet_config.rs and bacnet_read.rs, working from RustRED 0.3.3).
 *
 * - `bacnet-config`: `deviceId`, `targetHost`, `targetPort`, `apduTimeoutMs`, `retries`. The device is addressed by IP and UDP port.
 * - `bacnet read`: `configNode`, `objectType` (analogInput, binaryValue, multiStateValue, ...), `objectInstance` and `property`
 *   (presentValue, objectName, units, ...). The node reads when a message arrives and puts the value in `msg.payload`
 *   (numbers, booleans and strings; binary and multi-state present values are numbers) and the object and property in `msg.bacnet`.
 * - `bacnet write` (opt in, `bacnetWriteNodes`): `configNode`, `objectType`, `objectInstance`, `property`, `priority` (1-16, empty for the
 *   device default), `valueType` (auto: Real for analog, Enumerated for binary, Unsigned for multi-state present values) and
 *   `relinquish`. It writes `msg.payload` (null relinquishes at the priority; `msg.priority` overrides) and passes `true` on.
 * - `inject` (Node-RED core): `repeat` in seconds triggers every read node; `once` starts the first round right after deploy.
 *   Each write node gets an inject of its own, with a placeholder payload and no trigger of its own: nothing is written until it is clicked.
 *
 * One function node names each value from `msg.bacnet`, so the debug node shows `{ "<tag name>": value }` with the device name as topic.
 * The node-red-contrib-bacnet nodes have no documented flow schema (only source and an example flow), so there is no Node-RED variant.
 */
export function buildBacnetRustredFlow(list: BacnetTagList, options: ExportOptions = {}): { nodes: Record<string, unknown>[]; warnings: string[] } {
  const s = list.source;
  const warnings: string[] = [];
  const tab = nodeId(`tab:${s.name}`);
  const config = nodeId(`config:${s.name}`);
  const inject = nodeId(`inject:${s.name}`);
  const func = nodeId(`func:${s.name}`);
  const debug = nodeId(`debug:${s.name}`);
  const reads = list.tags.map((t, i) => ({ tag: t, id: nodeId(`read:${s.name}:${i}`) }));
  const seconds = s.pollMs / 1000;

  const nodes: Record<string, unknown>[] = [
    { id: tab, type: "tab", label: s.name, disabled: false, info: "Exported from BACnet Explorer" },
    {
      id: config, type: "bacnet-config", name: s.name, deviceId: s.deviceId, targetHost: s.host, targetPort: s.port,
      apduTimeoutMs: 3000, retries: 1,
    },
    {
      id: inject, type: "inject", z: tab, name: `every ${seconds} s`, props: [{ p: "payload" }], repeat: String(seconds), crontab: "",
      once: true, onceDelay: 0.5, topic: "", payload: "", payloadType: "date", x: 150, y: 80, wires: [reads.map((r) => r.id)],
    },
  ];
  reads.forEach(({ tag, id }, i) => {
    nodes.push({
      id, type: "bacnet read", z: tab, name: tag.name, configNode: config, objectType: tag.objectType, objectInstance: tag.instance,
      property: tag.property, x: 400, y: 80 + i * 50, wires: [[func]],
    });
  });
  const writables = options.bacnetWriteNodes ? list.tags.filter((t) => t.writable) : [];
  if (writables.length > 0) {
    const writeDebug = nodeId(`debug-write:${s.name}`);
    writables.forEach((tag, i) => {
      const y = 80 + (list.tags.length + 1 + i) * 50;
      const injectId = nodeId(`write-inject:${s.name}:${tag.name}`);
      const writeId = nodeId(`write:${s.name}:${tag.name}`);
      const [payload, payloadType] = placeholder(tag);
      nodes.push(
        {
          id: injectId, type: "inject", z: tab, name: `write ${tag.name}`, props: [{ p: "payload" }], repeat: "", crontab: "", once: false,
          onceDelay: 0.1, topic: "", payload, payloadType, x: 150, y, wires: [[writeId]],
        },
        {
          id: writeId, type: "bacnet write", z: tab, name: tag.name, configNode: config, objectType: tag.objectType, objectInstance: tag.instance,
          property: tag.property, priority: "", valueType: "auto", relinquish: false, x: 400, y, wires: [[writeDebug]],
        },
      );
    });
    nodes.push({
      id: writeDebug, type: "debug", z: tab, name: "write results", active: true, tosidebar: true, console: true, tostatus: false, complete: "true",
      targetType: "full", statusVal: "", statusType: "auto", x: 650, y: 80 + (list.tags.length + 1) * 50, wires: [],
    });
  }

  const names = Object.fromEntries(list.tags.map((t) => [`${t.objectType}:${t.instance}:${t.property}`, t.name]));
  nodes.push(
    {
      id: func, type: "function", z: tab, name: "name the value",
      func: [
        `const names = ${JSON.stringify(names)};`,
        "const b = msg.bacnet;",
        "msg.payload = { [names[`${b.objectType}:${b.objectInstance}:${b.property}`]]: msg.payload };",
        `msg.topic = ${JSON.stringify(s.name)};`,
        "return msg;",
      ].join("\n"),
      outputs: 1, timeout: 0, noerr: 0, initialize: "", finalize: "", libs: [], x: 650, y: 80, wires: [[debug]],
    },
    {
      id: debug, type: "debug", z: tab, name: "values", active: true, tosidebar: true, console: true, tostatus: false, complete: "payload",
      targetType: "msg", statusVal: "", statusType: "auto", x: 880, y: 80, wires: [],
    },
  );

  if (list.tags.length === 0) warnings.push("The tag list is empty, so the flow has no read nodes.");
  if (s.pollMs < 1000) warnings.push(`The poll interval is ${s.pollMs} ms; RustRED's inject node repeats at that rate.`);
  const readOnly = options.bacnetWriteNodes ? 0 : list.tags.filter((t) => t.writable).length;
  if (readOnly > 0) {
    warnings.push(`${readOnly} writable ${readOnly === 1 ? "tag is" : "tags are"} read only in this flow. Turn on write nodes to add a bacnet write node for ${readOnly === 1 ? "it" : "each"}.`);
  }
  if (writables.length > 0) warnings.push("Each write node has an inject node with a placeholder payload. Set the payload before you click it; a null payload relinquishes the priority slot.");
  if (new Set(Object.keys(names)).size < list.tags.length) {
    warnings.push("Some tags share an object and property; the debug output shows only the last name for them.");
  }
  return { nodes, warnings };
}

/** Placeholder payload for a write inject node: a value the object type accepts. */
function placeholder(tag: BacnetTag): [string, string] {
  if (tag.property !== "presentValue") return ["", "str"];
  if (tag.objectType.startsWith("binary")) return ["false", "bool"];
  if (tag.objectType.startsWith("multiState")) return ["1", "num"];
  return ["0", "num"];
}
