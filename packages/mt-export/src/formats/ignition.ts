import type { BacnetObjectType, ExportFormat, ModbusTag, TagList } from "../model";
import { fileStem, toJson } from "../util";

/**
 * Ignition (Inductive Automation) tag import JSON, 8.x.
 *
 * Sources, all Ignition 8.1 manual pages:
 * - File shape: https://www.docs.inductiveautomation.com/docs/8.1/platform/tags/exporting-and-importing-tags
 *   A tag file is `{"tags": [...]}`; every OPC tag has `tagType: "AtomicTag"`, `valueSource: "opc"`, `opcServer` and
 *   `opcItemPath`, `name`. A folder is a tag with `tagType: "Folder"` and its own `tags` array.
 * - Property names (`dataType`, `readOnly`, `scaleMode`, `rawLow` ... `engUnit`, `documentation`):
 *   https://www.docs.inductiveautomation.com/docs/8.1/platform/tags/tag-properties
 * - `dataType` strings Int2, Int4, Float4, Float8, Boolean:
 *   https://www.docs.inductiveautomation.com/docs/8.1/platform/tags/tag-data-types
 * - Modbus item paths `[Device]<unit>.<designator><address>`: designators HR/IR (signed 16-bit), HRUS/IRUS (unsigned 16),
 *   HRI/IRI (32-bit), HRF/IRF (float), HRD/IRD (double), C (coil), DI (discrete input). Addresses are 1-based (HR1 is
 *   holding register 0, "equivalent to 40001") unless the device has Zero-based Addressing ticked. The unit id goes in front.
 *   https://www.docs.inductiveautomation.com/docs/8.1/ignition-modules/opc-ua/opc-ua-drivers/modbus/modbus-addressing
 * - Device settings "Reverse Word Order" and "Zero-based Addressing" live on the device, not the tag:
 *   https://www.docs.inductiveautomation.com/docs/8.1/ignition-modules/opc-ua/opc-ua-drivers/modbus/connecting-to-modbus-device
 * - BACnet item paths `ns=1;s=[RemoteDevice]AnalogInput100.Present_Value`:
 *   https://docs.inductiveautomation.com/docs/8.1/ignition-modules/opc-ua/opc-ua-drivers/bacnet
 */

const OPC_SERVER = "Ignition OPC UA Server";

function modbusDesignator(t: ModbusTag): { designator: string; dataType: string } {
  const reg = t.area === "input" ? "IR" : "HR";
  switch (t.area) {
    case "coil":
      return { designator: "C", dataType: "Boolean" };
    case "discrete":
      return { designator: "DI", dataType: "Boolean" };
    default:
      break;
  }
  switch (t.type) {
    case "int16":
      return { designator: reg, dataType: "Int2" };
    case "uint16":
    case "bool":
      // Int2 holds -32768..32767, so an unsigned 16-bit value needs the next size up.
      return { designator: `${reg}US`, dataType: "Int4" };
    case "int32":
      return { designator: `${reg}I`, dataType: "Int4" };
    case "float32":
      return { designator: `${reg}F`, dataType: "Float4" };
    case "float64":
      return { designator: `${reg}D`, dataType: "Float8" };
  }
}

const BACNET_PATH_TYPE: Record<BacnetObjectType, string> = {
  analogInput: "AnalogInput",
  analogOutput: "AnalogOutput",
  analogValue: "AnalogValue",
  binaryInput: "BinaryInput",
  binaryOutput: "BinaryOutput",
  binaryValue: "BinaryValue",
  multiStateInput: "MultiStateInput",
  multiStateOutput: "MultiStateOutput",
  multiStateValue: "MultiStateValue",
};

function bacnetDataType(t: BacnetObjectType): string {
  if (t.startsWith("analog")) return "Float4";
  if (t.startsWith("binary")) return "Boolean";
  return "Int4";
}

/** Ignition's name rule: starts with a letter or underscore, then letters, digits, spaces, `_ ( ) ' - :`. */
function tagName(name: string): string {
  const cleaned = name.replace(/[^A-Za-z0-9_ ()'\-:]/g, "_");
  return /^[A-Za-z_]/.test(cleaned) ? cleaned : `_${cleaned}`;
}

export const ignitionFormat: ExportFormat = {
  id: "ignition",
  label: "Ignition tags (JSON)",
  hint: "OPC tags on the Ignition Modbus or BACnet driver, for Import Tags",
  extension: "json",
  protocols: ["modbus", "bacnet"],
  export(list: TagList) {
    const warnings: string[] = [];
    const folderName = tagName(list.source.name);
    // The `[Device]` in every path is the name of the device in Ignition's Connected Devices list.
    const device = list.source.name;

    if (list.protocol === "bacnet") {
      const tags = list.tags.map((t) => ({
        name: tagName(t.name),
        tagType: "AtomicTag",
        valueSource: "opc",
        dataType: bacnetDataType(t.objectType),
        opcServer: OPC_SERVER,
        opcItemPath: `ns=1;s=[${device}]${BACNET_PATH_TYPE[t.objectType]}${t.instance}.${t.property === "presentValue" ? "Present_Value" : t.property}`,
        ...(t.writable ? {} : { readOnly: true }),
        ...(t.unit ? { engUnit: t.unit } : {}),
        ...(t.description ? { documentation: t.description } : {}),
      }));
      warnings.push(`Name the BACnet remote device "${device}" in Ignition, or edit the [${device}] part of each OPC item path.`);
      return {
        fileName: `${fileStem(list.source.name)}-ignition-tags.json`,
        content: toJson({ tags: [{ name: folderName, tagType: "Folder", tags }] }),
        warnings,
      };
    }

    const s = list.source;
    const tags = list.tags.map((t) => {
      const { designator, dataType } = modbusDesignator(t);
      const scaled = t.scale !== 1 && t.area !== "coil" && t.area !== "discrete";
      return {
        name: tagName(t.name),
        tagType: "AtomicTag",
        valueSource: "opc",
        dataType: scaled && dataType.startsWith("Int") ? "Float8" : dataType,
        opcServer: OPC_SERVER,
        opcItemPath: `[${device}]${s.unitId}.${designator}${t.address + 1}`,
        ...(t.writable ? {} : { readOnly: true }),
        // Linear scaling from 0..1 onto 0..scale multiplies the raw value by `scale`.
        ...(scaled ? { scaleMode: "Linear", rawLow: 0, rawHigh: 1, scaledLow: 0, scaledHigh: t.scale } : {}),
        ...(t.unit ? { engUnit: t.unit } : {}),
        ...(t.description ? { documentation: t.description } : {}),
      };
    });
    if (s.transport === "rtu") warnings.push("Ignition's Modbus driver talks Modbus TCP. Reach an RTU device through a serial gateway and point the Ignition device at the gateway.");
    if (s.wordOrder !== "big" && list.tags.some((t) => t.type === "int32" || t.type === "float32" || t.type === "float64")) {
      warnings.push(
        s.wordOrder === "little"
          ? "Tick Reverse Word Order on the Ignition device: this device sends the low word first (CD AB)."
          : "Ignition has no setting for bytes swapped inside each word (BA DC); 32 and 64-bit values will read wrong.",
      );
    }
    warnings.push(`Name the Ignition device "${device}", or edit the [${device}] part of each OPC item path. Leave Zero-based Addressing unticked.`);
    return {
      fileName: `${fileStem(s.name)}-ignition-tags.json`,
      content: toJson({ tags: [{ name: folderName, tagType: "Folder", tags }] }),
      warnings,
    };
  },
};
