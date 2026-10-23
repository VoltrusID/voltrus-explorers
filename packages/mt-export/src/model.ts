/**
 * The tag list every exporter reads. An app turns what the user is polling into one of these; each format module turns
 * it into a file. Nothing here knows about a particular SCADA product.
 */

export type Protocol = "modbus" | "bacnet";

export type ModbusArea = "holding" | "input" | "coil" | "discrete";
export type ModbusType = "bool" | "uint16" | "int16" | "int32" | "float32" | "float64";
/** How a 32/64-bit value is laid out over registers, as in Modbus Poll: AB CD, CD AB, BA DC. */
export type WordOrder = "big" | "little" | "swapped";

export interface ModbusSource {
  /** Device name, used for the device node, folder or file name. */
  name: string;
  transport: "tcp" | "rtu";
  host: string;
  port: number;
  unitId: number;
  pollMs: number;
  wordOrder: WordOrder;
  /** Only read when `transport` is "rtu". */
  serial?: { port: string; baudRate: number; dataBits: number; stopBits: number; parity: "none" | "even" | "odd" };
}

export interface ModbusTag {
  /** Unique within the list. */
  name: string;
  area: ModbusArea;
  /** 0-based address on the wire (the one in the request PDU). */
  address: number;
  type: ModbusType;
  /** Multiplier applied to the raw value. 1 when the value is used as read. */
  scale: number;
  unit?: string;
  description?: string;
  writable: boolean;
}

export interface ModbusTagList {
  protocol: "modbus";
  source: ModbusSource;
  tags: ModbusTag[];
}

export type BacnetObjectType =
  | "analogInput"
  | "analogOutput"
  | "analogValue"
  | "binaryInput"
  | "binaryOutput"
  | "binaryValue"
  | "multiStateInput"
  | "multiStateOutput"
  | "multiStateValue";

export interface BacnetSource {
  name: string;
  host: string;
  port: number;
  /** BACnet device instance number. */
  deviceId: number;
  pollMs: number;
}

export interface BacnetTag {
  name: string;
  objectType: BacnetObjectType;
  instance: number;
  /** Property to read, "presentValue" unless the app says otherwise. */
  property: string;
  unit?: string;
  description?: string;
  writable: boolean;
}

export interface BacnetTagList {
  protocol: "bacnet";
  source: BacnetSource;
  tags: BacnetTag[];
}

export type TagList = ModbusTagList | BacnetTagList;

export type FormatId =
  | "csv"
  | "json"
  | "ignition"
  | "kepware"
  | "rustred"
  | "nodered"
  | "homeassistant"
  | "voltrus-scada"
  | "voltrus-gateway";

export interface ExportResult {
  fileName: string;
  content: string;
  /** Things the format cannot say. Shown to the user before they save; each one names the tag or setting affected. */
  warnings: string[];
}

export interface ExportFormat {
  id: FormatId;
  label: string;
  /** One line for the picker. */
  hint: string;
  /** File extension without the dot. */
  extension: string;
  protocols: Protocol[];
  export(list: TagList): ExportResult;
}
