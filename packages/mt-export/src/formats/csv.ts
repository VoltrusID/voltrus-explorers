import type { BacnetTag, ExportFormat, ModbusTag, TagList } from "../model";
import { READ_FUNCTION, reference6 } from "../modbusAddress";
import { csvLine, fileStem } from "../util";

const MODBUS_HEADER = [
  "name", "device", "host", "port", "unit_id", "area", "function_code", "address", "reference",
  "registers", "type", "word_order", "scale", "unit", "access", "description",
];
const BACNET_HEADER = [
  "name", "device", "host", "port", "device_instance", "object_type", "object_instance", "property",
  "unit", "access", "description",
];

/**
 * Plain CSV, one row per tag, for spreadsheets and for scripts. `address` is the 0-based address on the wire;
 * `reference` is the same register as the 1-based 6-digit number most vendor tools show (400001 = holding register 0).
 */
export const csvFormat: ExportFormat = {
  id: "csv",
  label: "Generic CSV",
  hint: "One row per tag with host, unit, area, address and type",
  extension: "csv",
  protocols: ["modbus", "bacnet"],
  export(list: TagList) {
    const stem = fileStem(list.source.name);
    if (list.protocol === "modbus") {
      const s = list.source;
      const rows = list.tags.map((t: ModbusTag) =>
        csvLine([
          t.name, s.name, s.host, s.port, s.unitId, t.area, READ_FUNCTION[t.area], t.address, reference6(t.area, t.address),
          t.type === "float64" ? 4 : t.type === "float32" || t.type === "int32" ? 2 : 1, t.type, s.wordOrder, t.scale,
          t.unit ?? "", t.writable ? "read-write" : "read-only", t.description ?? "",
        ]),
      );
      return { fileName: `${stem}-tags.csv`, content: [csvLine(MODBUS_HEADER), ...rows].join("\n") + "\n", warnings: [] };
    }
    const s = list.source;
    const rows = list.tags.map((t: BacnetTag) =>
      csvLine([
        t.name, s.name, s.host, s.port, s.deviceId, t.objectType, t.instance, t.property,
        t.unit ?? "", t.writable ? "read-write" : "read-only", t.description ?? "",
      ]),
    );
    return { fileName: `${stem}-tags.csv`, content: [csvLine(BACNET_HEADER), ...rows].join("\n") + "\n", warnings: [] };
  },
};
