import type { Device, TagConfig } from "./types";

/** Polled tags allowed without Pro. */
export const FREE_TAG_LIMIT = 10;

const DEVICES_KEY = "s7-explorer-devices";

export function uid(): string {
  return crypto.randomUUID();
}

function defaultTags(): TagConfig[] {
  return [
    { id: uid(), name: "Voltage", address: "DB1.DBD0", data_type: "DWord" },
    { id: uid(), name: "Current", address: "DB1.DBD4", data_type: "DWord" },
    { id: uid(), name: "Status", address: "DB1.DBX8.0", data_type: "Bool" },
    { id: uid(), name: "Counter", address: "DB1.DBW10", data_type: "Word" },
    { id: uid(), name: "Temp", address: "DB1.DBD12", data_type: "Real" },
  ];
}

export function defaultDevices(): Device[] {
  return [
    {
      id: uid(),
      label: "PLC-1",
      host: "192.168.1.100",
      port: 102,
      rack: 0,
      slot: 1,
      tags: defaultTags(),
    },
  ];
}

export function loadDevices(): Device[] {
  const backfill = (d: Device): Device => ({
    ...d,
    // Older builds persisted devices with no tags; seed starters so the tag
    // list is never empty on load.
    tags: Array.isArray(d.tags) && d.tags.length > 0 ? d.tags : defaultTags(),
  });
  try {
    const raw = localStorage.getItem(DEVICES_KEY);
    if (!raw) return defaultDevices();
    const parsed = JSON.parse(raw) as Device[];
    if (!Array.isArray(parsed) || parsed.length === 0) return defaultDevices();
    return parsed.map(backfill);
  } catch {
    return defaultDevices();
  }
}

export function saveDevices(devices: Device[]) {
  localStorage.setItem(DEVICES_KEY, JSON.stringify(devices));
}

export function newDevice(): Device {
  return {
    id: uid(),
    label: `PLC-${Math.floor(Math.random() * 9000) + 1000}`,
    host: "192.168.1.100",
    port: 102,
    rack: 0,
    slot: 1,
    tags: defaultTags(),
  };
}
