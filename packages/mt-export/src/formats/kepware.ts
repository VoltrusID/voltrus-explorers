import type { BacnetObjectType, BacnetTag, ExportFormat, ModbusTag, TagList } from "../model";
import { reference6 } from "../modbusAddress";
import { csvLine, fileStem } from "../util";

/**
 * Kepware (PTC KEPServerEX / Kepware Server) tag CSV for a Modbus TCP/IP Ethernet device or a BACnet/IP device.
 *
 * - Column list and meaning ("Tag Name" ... "Negate Value", Respect Data Type 1/0, Client Access, Scan Rate in ms,
 *   Scaling None/Linear/Square Root): https://support.ptc.com/help/kepware/kepware_server/en/kepware/server/csv-import-export.html
 * - Value spellings in a real export (`Word`, `1`, `R/W`, `100`): the template CSV in
 *   https://github.com/PTCInc/Kepware-Example-Scripts (Datalogger/Powershell/Add Tags from CSV to Log Group/csvs/deviceItems.csv)
 *   and the header string in https://github.com/zhouston86/industrial_iot_demo (kepware/plc_to_kepware.py).
 *   Read-only tags are `RO`.
 * - Addresses `0xxxxx` coils, `1xxxxx` input coils (discrete inputs), `3xxxxx` internal (input) registers, `4xxxxx`
 *   holding registers, 1-based unless the device has Zero-Based Addressing on; six digits are always accepted:
 *   https://support.ptc.com/help/kepware/drivers/en/kepware/drivers/MODBUSETHERNET/Modbus_Addressing.html
 * - Data types Boolean, Word (unsigned 16), Short (signed 16), DWord, Long (signed 32), Float, Double:
 *   https://support.ptc.com/help/kepware/drivers/en/kepware/drivers/MODBUSETHERNET/data-types-description.html
 *   Float and Long take the word order from the device's "First Word Low" setting, which defaults to on (low word first).
 *
 * BACnet/IP driver addresses are `<object type>.<object instance>.<property identifier>`, for example
 * `AnalogValue.100.PresentValue` (Address Descriptions and Addressing Examples):
 * https://support.ptc.com/help/kepware/drivers/en/kepware/drivers/BACNET/address-descriptions.html
 * The object type mnemonics are AnalogInput, AnalogOutput, AnalogValue, BinaryInput, BinaryOutput, BinaryValue,
 * MultistateInput, MultistateOutput and MultistateValue (note the lower case s in Multistate; list in the Object Identifier
 * format of the Schedule object, BACnet/IP driver manual, https://downloads.softwaretoolbox.com/demodnld/prod_docs/topserver_help_pdf/common/bacnet-ip-manual.pdf).
 * Property mnemonics are the BACnet names without spaces (PresentValue, ObjectName, Units, ...). The data type of
 * PresentValue: REAL is Float, Boolean (binary objects) is Boolean, Unsigned Integer (multi-state objects) is DWord or Long
 * (Analog Input, Binary Input and Multi-State Input property tables of the same manual; Data Types Description:
 * https://support.ptc.com/help/kepware/drivers/en/kepware/drivers/BACNET/data-types-description.html).
 * The device instance, IP address and port belong to the device, not to the tag file.
 */

const HEADER = [
  "Tag Name", "Address", "Data Type", "Respect Data Type", "Client Access", "Scan Rate", "Scaling", "Raw Low", "Raw High",
  "Scaled Low", "Scaled High", "Scaled Data Type", "Clamp Low", "Clamp High", "Eng Units", "Description", "Negate Value",
];

function dataType(t: ModbusTag): string {
  if (t.area === "coil" || t.area === "discrete") return "Boolean";
  switch (t.type) {
    case "int16": return "Short";
    case "int32": return "Long";
    case "float32": return "Float";
    case "float64": return "Double";
    default: return "Word";
  }
}

const BACNET_OBJECT: Record<BacnetObjectType, string> = {
  analogInput: "AnalogInput",
  analogOutput: "AnalogOutput",
  analogValue: "AnalogValue",
  binaryInput: "BinaryInput",
  binaryOutput: "BinaryOutput",
  binaryValue: "BinaryValue",
  multiStateInput: "MultistateInput",
  multiStateOutput: "MultistateOutput",
  multiStateValue: "MultistateValue",
};

/** `presentValue` becomes `PresentValue`. */
const bacnetProperty = (property: string): string => property.charAt(0).toUpperCase() + property.slice(1);

/** Data type of a BACnet property as the driver presents it. Only PresentValue depends on the object type. */
function bacnetDataType(t: BacnetTag): string {
  switch (t.property) {
    case "objectName":
    case "description":
      return "String";
    case "units":
    case "numberOfStates":
      return "DWord";
    case "outOfService":
      return "Boolean";
    default:
      if (t.objectType.startsWith("analog")) return "Float";
      if (t.objectType.startsWith("binary")) return "Boolean";
      return "DWord";
  }
}

/** A tag name may not hold a period (that is the group separator). */
const tagName = (name: string): string => name.replace(/\./g, "_");

export const kepwareFormat: ExportFormat = {
  id: "kepware",
  label: "Kepware tags (CSV)",
  hint: "Static tags for a Modbus TCP/IP Ethernet or BACnet/IP device, for Import CSV",
  extension: "csv",
  protocols: ["modbus", "bacnet"],
  export(list: TagList) {
    const warnings: string[] = [];
    if (list.protocol === "bacnet") {
      const s = list.source;
      const rows = list.tags.map((t) =>
        csvLine([
          tagName(t.name), `${BACNET_OBJECT[t.objectType]}.${t.instance}.${bacnetProperty(t.property)}`, bacnetDataType(t), 1,
          t.writable ? "R/W" : "RO", Math.max(10, s.pollMs), "None", "", "", "", "", "", "", "", t.unit ?? "", t.description ?? "", "",
        ]),
      );
      warnings.push(`Kepware reads the device instance, the IP address and the port from the device, not the tag file: set Device ID ${s.deviceId} and ${s.host}:${s.port} on the BACnet/IP device.`);
      return { fileName: `${fileStem(s.name)}-kepware-tags.csv`, content: [csvLine(HEADER), ...rows].join("\n") + "\n", warnings };
    }
    const s = list.source;
    const rows = list.tags.map((t) => {
      const scaled = t.scale !== 1 && t.area !== "coil" && t.area !== "discrete";
      return csvLine([
        tagName(t.name), reference6(t.area, t.address), dataType(t), 1, t.writable ? "R/W" : "RO", Math.max(10, s.pollMs),
        scaled ? "Linear" : "None", scaled ? 0 : "", scaled ? 1 : "", scaled ? 0 : "", scaled ? t.scale : "",
        scaled ? "Double" : "", scaled ? 0 : "", scaled ? 0 : "", t.unit ?? "", t.description ?? "", scaled ? 0 : "",
      ]);
    });
    if (s.transport === "rtu") warnings.push("Import these tags under a Modbus RTU Serial device, or use a Modbus TCP gateway.");
    const wide = list.tags.some((t) => t.type === "int32" || t.type === "float32" || t.type === "float64");
    if (wide && s.wordOrder === "big") {
      warnings.push("Device settings: switch off First Word Low in 32-Bit Data Types (and First DWord Low in 64-Bit Data Types). This device sends the high word first (AB CD).");
    } else if (wide && s.wordOrder === "swapped") {
      warnings.push("Device settings: switch off First Word Low in 32-Bit Data Types (and First DWord Low in 64-Bit Data Types) and switch off Modbus Byte Order. This device sends the high word first with the bytes swapped inside each register (BA DC).");
    }
    warnings.push("Kepware reads the unit id and the port from the device, not the tag file: set Unit ID " + s.unitId + " and Port " + s.port + " on the device.");
    return { fileName: `${fileStem(s.name)}-kepware-tags.csv`, content: [csvLine(HEADER), ...rows].join("\n") + "\n", warnings };
  },
};
