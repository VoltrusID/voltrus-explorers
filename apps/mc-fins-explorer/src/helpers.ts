import type { Device, FinsSettings, McSettings, Protocol, TagConfig } from "./types";
import { DEFAULT_PORT } from "./types";

/** Polled tags allowed without Pro. */
export const FREE_TAG_LIMIT = 10;

const DEVICES_KEY = "mc-fins-explorer-devices";

export function uid(): string {
  return crypto.randomUUID();
}

export const DEFAULT_MC: McSettings = {
  transport: "tcp",
  frame: "3E",
  encoding: "binary",
  network: 0,
  pc: 0xff,
  io: 0x03ff,
  station: 0,
  xyOctal: false,
};

export const DEFAULT_FINS: FinsSettings = {
  transport: "udp",
  destNetwork: 0,
  destNode: 0,
  destUnit: 0,
  srcNetwork: 0,
  srcNode: 0,
  localPort: 0,
};

export function defaultTags(protocol: Protocol): TagConfig[] {
  if (protocol === "fins") {
    return [
      { id: uid(), name: "Speed", address: "D100", data_type: "Int16" },
      { id: uid(), name: "Count", address: "D101", data_type: "Int32" },
      { id: uid(), name: "Running", address: "CIO0.00", data_type: "Bool" },
    ];
  }
  return [
    { id: uid(), name: "Speed", address: "D100", data_type: "Int16" },
    { id: uid(), name: "Count", address: "D101", data_type: "Int32" },
    { id: uid(), name: "Running", address: "M0", data_type: "Bool" },
  ];
}

export function newDevice(protocol: Protocol = "mc", n = 1): Device {
  return {
    id: uid(),
    label: `PLC-${n}`,
    protocol,
    host: protocol === "mc" ? "192.168.3.39" : "192.168.250.1",
    port: DEFAULT_PORT[protocol],
    timeoutMs: 3000,
    mc: { ...DEFAULT_MC },
    fins: { ...DEFAULT_FINS },
    tags: defaultTags(protocol),
  };
}

/** Fill fields a saved device from an older build may miss. */
function backfill(d: Partial<Device>, i: number): Device {
  const base = newDevice(d.protocol === "fins" ? "fins" : "mc", i + 1);
  return {
    ...base,
    ...d,
    mc: { ...DEFAULT_MC, ...(d.mc ?? {}) },
    fins: { ...DEFAULT_FINS, ...(d.fins ?? {}) },
    tags: Array.isArray(d.tags) ? d.tags : base.tags,
  } as Device;
}

export function loadDevices(): Device[] {
  try {
    const raw = localStorage.getItem(DEVICES_KEY);
    if (!raw) return [newDevice()];
    const parsed = JSON.parse(raw) as Partial<Device>[];
    if (!Array.isArray(parsed) || parsed.length === 0) return [newDevice()];
    return parsed.map(backfill);
  } catch {
    return [newDevice()];
  }
}

export function saveDevices(devices: Device[]) {
  localStorage.setItem(DEVICES_KEY, JSON.stringify(devices));
}

/** The connect payload the Rust side expects (`ConnectConfig`). */
export function connectConfig(d: Device): Record<string, unknown> {
  if (d.protocol === "mc") {
    return {
      protocol: "mc",
      host: d.host,
      port: d.port,
      transport: d.mc.transport,
      frame: d.mc.frame,
      encoding: d.mc.encoding,
      route: { network: d.mc.network, pc: d.mc.pc, io: d.mc.io, station: d.mc.station },
      timeout_ms: d.timeoutMs,
      xy_octal: d.mc.xyOctal,
    };
  }
  return {
    protocol: "fins",
    host: d.host,
    port: d.port,
    transport: d.fins.transport,
    dest_network: d.fins.destNetwork,
    dest_node: d.fins.destNode,
    dest_unit: d.fins.destUnit,
    src_network: d.fins.srcNetwork,
    src_node: d.fins.srcNode,
    local_port: d.fins.localPort,
    timeout_ms: d.timeoutMs,
  };
}

/** Bool for bit addresses, Int16 for words: the type a new tag starts with. */
export function guessType(protocol: Protocol, address: string, current: string): string {
  const a = address.trim().toUpperCase();
  const isBit =
    a.includes(".") ||
    (protocol === "mc" && /^(SM|TS|TC|SS|SC|CS|CC|SB|DX|DY|X|Y|M|L|F|V|B)[0-9A-F]/.test(a) && !/^(SD|SW|SN)/.test(a));
  if (isBit) return "Bool";
  return current === "Bool" ? "Int16" : current;
}

/** A word as the device monitor shows it. */
export function formatWord(w: number, fmt: "dec" | "sdec" | "hex" | "bin"): string {
  switch (fmt) {
    case "sdec":
      return String(w >= 0x8000 ? w - 0x10000 : w);
    case "hex":
      return w.toString(16).toUpperCase().padStart(4, "0");
    case "bin":
      return w.toString(2).padStart(16, "0").replace(/(.{4})(?=.)/g, "$1 ");
    default:
      return String(w);
  }
}

/** The address of word `i` after a monitor head: bit devices step 16 numbers per word, hex-numbered ones in hex. */
export function offsetAddress(protocol: Protocol, head: string, i: number): string {
  const a = head.trim().toUpperCase();
  if (protocol === "fins") {
    const m = /^(CIO|DM|HR|AR|WR|E[0-9A-F]+_|[DWHATC]|)(\d+)$/.exec(a);
    if (!m) return `${a}+${i}`;
    return `${m[1]}${Number(m[2]) + i}`;
  }
  const m = /^(SM|SD|TS|TC|TN|SS|SC|SN|CS|CC|CN|SB|SW|DX|DY|ZR|X|Y|M|L|F|V|B|D|W|Z|R)([0-9A-F]+)$/.exec(a);
  if (!m) return `${a}+${i}`;
  const dev = m[1];
  const hex = ["X", "Y", "B", "W", "SB", "SW", "DX", "DY"].includes(dev);
  const bitDev = ["SM", "X", "Y", "M", "L", "F", "V", "B", "TS", "TC", "SS", "SC", "CS", "CC", "SB", "DX", "DY"].includes(dev);
  const n = parseInt(m[2], hex ? 16 : 10) + i * (bitDev ? 16 : 1);
  return dev + (hex ? n.toString(16).toUpperCase() : String(n));
}
