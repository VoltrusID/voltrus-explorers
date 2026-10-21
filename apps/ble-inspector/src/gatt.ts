import type { FormatId } from "./decode";

/** Suffix shared by every 16-bit UUID expanded to 128 bits (Bluetooth Base UUID). */
const BASE_SUFFIX = "-0000-1000-8000-00805f9b34fb";

/** 16-bit assigned numbers (GATT services and characteristics), keyed by lowercase 4-digit hex. */
const ASSIGNED: Record<string, string> = {
  // Services
  "1800": "Generic Access",
  "1801": "Generic Attribute",
  "1802": "Immediate Alert",
  "1803": "Link Loss",
  "1804": "Tx Power",
  "1805": "Current Time",
  "1806": "Reference Time Update",
  "1807": "Next DST Change",
  "1808": "Glucose",
  "1809": "Health Thermometer",
  "180a": "Device Information",
  "180d": "Heart Rate",
  "180e": "Phone Alert Status",
  "180f": "Battery",
  "1810": "Blood Pressure",
  "1811": "Alert Notification",
  "1812": "Human Interface Device",
  "1813": "Scan Parameters",
  "1814": "Running Speed and Cadence",
  "1815": "Automation IO",
  "1816": "Cycling Speed and Cadence",
  "1818": "Cycling Power",
  "1819": "Location and Navigation",
  "181a": "Environmental Sensing",
  "181b": "Body Composition",
  "181c": "User Data",
  "181d": "Weight Scale",
  "181e": "Bond Management",
  "181f": "Continuous Glucose Monitoring",
  "1820": "Internet Protocol Support",
  "1821": "Indoor Positioning",
  "1822": "Pulse Oximeter",
  "1823": "HTTP Proxy",
  "1824": "Transport Discovery",
  "1825": "Object Transfer",
  "1826": "Fitness Machine",
  "1827": "Mesh Provisioning",
  "1828": "Mesh Proxy",
  // Characteristics
  "2a00": "Device Name",
  "2a01": "Appearance",
  "2a02": "Peripheral Privacy Flag",
  "2a03": "Reconnection Address",
  "2a04": "Peripheral Preferred Connection Parameters",
  "2a05": "Service Changed",
  "2a06": "Alert Level",
  "2a07": "Tx Power Level",
  "2a08": "Date Time",
  "2a09": "Day of Week",
  "2a0a": "Day Date Time",
  "2a0f": "Local Time Information",
  "2a18": "Glucose Measurement",
  "2a19": "Battery Level",
  "2a1c": "Temperature Measurement",
  "2a1d": "Temperature Type",
  "2a1e": "Intermediate Temperature",
  "2a23": "System ID",
  "2a24": "Model Number String",
  "2a25": "Serial Number String",
  "2a26": "Firmware Revision String",
  "2a27": "Hardware Revision String",
  "2a28": "Software Revision String",
  "2a29": "Manufacturer Name String",
  "2a2a": "IEEE 11073-20601 Regulatory Certification Data List",
  "2a2b": "Current Time",
  "2a31": "Scan Refresh",
  "2a35": "Blood Pressure Measurement",
  "2a37": "Heart Rate Measurement",
  "2a38": "Body Sensor Location",
  "2a39": "Heart Rate Control Point",
  "2a4a": "HID Information",
  "2a4b": "Report Map",
  "2a4c": "HID Control Point",
  "2a4d": "Report",
  "2a4e": "Protocol Mode",
  "2a4f": "Scan Interval Window",
  "2a50": "PnP ID",
  "2a53": "RSC Measurement",
  "2a55": "SC Control Point",
  "2a5b": "CSC Measurement",
  "2a5c": "CSC Feature",
  "2a5d": "Sensor Location",
  "2a5e": "PLX Spot-Check Measurement",
  "2a5f": "PLX Continuous Measurement",
  "2a63": "Cycling Power Measurement",
  "2a6c": "Elevation",
  "2a6d": "Pressure",
  "2a6e": "Temperature",
  "2a6f": "Humidity",
  "2a76": "UV Index",
  "2a77": "Irradiance",
  "2a98": "Weight",
  "2a9d": "Weight Measurement",
  "2a9e": "Weight Scale Feature",
  "2aa6": "Central Address Resolution",
  "2b29": "Client Supported Features",
  "2b2a": "Database Hash",
  "2b3a": "Server Supported Features",
};

/** Well-known vendor UUIDs that many dev boards expose. Keys are lowercase 128-bit UUIDs. */
const VENDOR: Record<string, string> = {
  "6e400001-b5a3-f393-e0a9-e50e24dcca9e": "Nordic UART Service",
  "6e400002-b5a3-f393-e0a9-e50e24dcca9e": "Nordic UART RX (write)",
  "6e400003-b5a3-f393-e0a9-e50e24dcca9e": "Nordic UART TX (notify)",
};

function shortKey(uuid: string): string | null {
  const u = uuid.toLowerCase();
  if (u.length === 36 && u.startsWith("0000") && u.endsWith(BASE_SUFFIX)) return u.substring(4, 8);
  if (u.length === 4 && /^[0-9a-f]{4}$/.test(u)) return u;
  return null;
}

/** `0x180F` for Bluetooth Base UUIDs, the full UUID otherwise. */
export function shortUuid(uuid: string): string {
  const k = shortKey(uuid);
  return k ? `0x${k.toUpperCase()}` : uuid;
}

/** Assigned or well-known name for a service or characteristic UUID, or `null`. */
export function gattName(uuid: string): string | null {
  const k = shortKey(uuid);
  if (k) return ASSIGNED[k] ?? null;
  return VENDOR[uuid.toLowerCase()] ?? null;
}

export interface DecodeDefault {
  format: FormatId;
  scale: number;
  /** Byte offset of the value inside the payload. */
  offset: number;
}

/** Sensible decode settings for characteristics whose value layout is a single scalar or string. */
const DEFAULTS: Record<string, DecodeDefault> = {
  "2a00": { format: "utf8", scale: 1, offset: 0 },
  "2a19": { format: "uint8", scale: 1, offset: 0 },
  "2a24": { format: "utf8", scale: 1, offset: 0 },
  "2a25": { format: "utf8", scale: 1, offset: 0 },
  "2a26": { format: "utf8", scale: 1, offset: 0 },
  "2a27": { format: "utf8", scale: 1, offset: 0 },
  "2a28": { format: "utf8", scale: 1, offset: 0 },
  "2a29": { format: "utf8", scale: 1, offset: 0 },
  // Heart Rate Measurement: flags byte, then the rate. Assumes the 8-bit rate format (flags bit 0 clear);
  // devices that set bit 0 send a uint16 LE at byte 1, which the format picker can select.
  "2a37": { format: "uint8", scale: 1, offset: 1 },
  "2a6d": { format: "uint32le", scale: 0.1, offset: 0 },
  "2a6e": { format: "int16le", scale: 0.01, offset: 0 },
  "2a6f": { format: "uint16le", scale: 0.01, offset: 0 },
  "2a76": { format: "uint8", scale: 1, offset: 0 },
};

export function defaultDecode(uuid: string): DecodeDefault | null {
  const k = shortKey(uuid);
  return k ? (DEFAULTS[k] ?? null) : null;
}
