import type { ExportFormat, ModbusTag, ModbusTagList, TagList } from "../model";
import { fileStem, slug } from "../util";

/**
 * Home Assistant `modbus:` YAML for configuration.yaml. Reference:
 * https://www.home-assistant.io/integrations/modbus/ (Modbus YAML reference)
 *
 * - Hub: `name`, `type` (tcp or serial), `host` and `port`; serial adds `baudrate`, `bytesize`, `method`, `parity` (E/O/N),
 *   `stopbits`, `port`. `timeout` is in seconds.
 * - Entities: `address` is 0-based, which is what Modbus Poll holds. `device_address` is the unit id (`slave` is the old name).
 *   `scan_interval` is whole seconds. `precision` defaults to 0, so float sensors need it set or they round to integers.
 * - Sensors: `input_type` holding or input; `data_type` int16, uint16, int32, float32, float64 (`count` is worked out for
 *   these); `swap` is `byte` (AB -> BA) or `word` (ABCD -> CDAB, not valid for 16-bit types); `scale`, `offset`.
 * - Binary sensors: `input_type` coil or discrete_input.
 */

const q = (s: string): string => JSON.stringify(s);

function sensor(t: ModbusTag, s: ModbusTagList["source"], hub: string): string[] {
  const lines = [`      - name: ${q(t.name)}`, `        unique_id: ${q(`${hub}_${slug(t.name, "tag")}`)}`, `        address: ${t.address}`];
  lines.push(`        device_address: ${s.unitId}`, `        input_type: ${t.area}`, `        data_type: ${t.type}`);
  const wide = t.type === "int32" || t.type === "float32" || t.type === "float64";
  if (wide && s.wordOrder === "little") lines.push("        swap: word");
  if (s.wordOrder === "swapped") lines.push("        swap: byte");
  if (t.scale !== 1) lines.push(`        scale: ${t.scale}`);
  const decimals = t.type === "float32" || t.type === "float64" ? 3 : Math.max(0, (String(t.scale).split(".")[1] ?? "").length);
  lines.push(`        precision: ${decimals}`);
  lines.push(`        scan_interval: ${Math.max(1, Math.ceil(s.pollMs / 1000))}`);
  if (t.unit) lines.push(`        unit_of_measurement: ${q(t.unit)}`);
  return lines;
}

function binarySensor(t: ModbusTag, s: ModbusTagList["source"], hub: string): string[] {
  return [
    `      - name: ${q(t.name)}`,
    `        unique_id: ${q(`${hub}_${slug(t.name, "tag")}`)}`,
    `        address: ${t.address}`,
    `        device_address: ${s.unitId}`,
    `        input_type: ${t.area === "discrete" ? "discrete_input" : "coil"}`,
    `        scan_interval: ${Math.max(1, Math.ceil(s.pollMs / 1000))}`,
  ];
}

export const homeAssistantFormat: ExportFormat = {
  id: "homeassistant",
  label: "Home Assistant (YAML)",
  hint: "A modbus: hub with sensors and binary sensors for configuration.yaml",
  extension: "yaml",
  protocols: ["modbus"],
  export(list: TagList) {
    if (list.protocol !== "modbus") throw new Error("Home Assistant export is for Modbus tag lists");
    const s = list.source;
    const hub = slug(s.name, "modbus_hub");
    const warnings: string[] = [];
    const out: string[] = [
      `# Modbus Poll tag list for Home Assistant. Merge this into configuration.yaml, then restart Home Assistant.`,
      `# Reference: https://www.home-assistant.io/integrations/modbus/`,
      `modbus:`,
      `  - name: ${q(hub)}`,
    ];
    if (s.transport === "rtu") {
      const ser = s.serial ?? { port: "", baudRate: 9600, dataBits: 8, stopBits: 1, parity: "none" as const };
      out.push(
        `    type: serial`, `    port: ${q(ser.port || "/dev/ttyUSB0")}`, `    baudrate: ${ser.baudRate}`, `    bytesize: ${ser.dataBits}`,
        `    method: rtu`, `    parity: ${ser.parity === "even" ? "E" : ser.parity === "odd" ? "O" : "N"}`, `    stopbits: ${ser.stopBits}`,
      );
      if (!ser.port) warnings.push("The serial port is not set, so /dev/ttyUSB0 is a guess: change it to the port on the Home Assistant host.");
    } else {
      out.push(`    type: tcp`, `    host: ${q(s.host)}`, `    port: ${s.port}`);
    }
    out.push(`    timeout: 5`);

    const sensors = list.tags.filter((t) => t.area === "holding" || t.area === "input");
    const bits = list.tags.filter((t) => t.area === "coil" || t.area === "discrete");
    if (sensors.length > 0) {
      out.push(`    sensors:`);
      for (const t of sensors) out.push(...sensor(t, s, hub));
    }
    if (bits.length > 0) {
      out.push(`    binary_sensors:`);
      for (const t of bits) out.push(...binarySensor(t, s, hub));
    }
    if (list.tags.length === 0) warnings.push("The tag list is empty. Home Assistant refuses a hub with no entities.");
    if (list.tags.some((t) => t.writable)) warnings.push("Writable holding registers and coils are exported as read-only sensors. Add a switch or number entity by hand to write to them.");
    return { fileName: `${fileStem(s.name)}-homeassistant-modbus.yaml`, content: out.join("\n") + "\n", warnings };
  },
};
