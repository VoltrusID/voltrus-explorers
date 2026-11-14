// EtherCAT helpers for the UI: number formats, process data entry decoding, state tones, pcap and CSV export.

import type { Tone } from "@mactools/ui";
import type { AlStateName, DeviceInfo, FrameView, NetInterface, PdoEntryView } from "./types";

export const hex = (v: number, digits: number) => `0x${(v >>> 0).toString(16).toUpperCase().padStart(digits, "0")}`;

export const hexBytes = (b: number[]) => b.map((x) => x.toString(16).toUpperCase().padStart(2, "0")).join(" ");

export function stateTone(state: AlStateName | string, error?: boolean): Tone {
  if (error) return "danger";
  switch (state) {
    case "OP":
      return "success";
    case "SAFE-OP":
      return "info";
    case "PRE-OP":
      return "warn";
    default:
      return "neutral";
  }
}

/** Little-endian bits from a byte image at a bit offset, as a bigint (entries can be 64 bits). */
export function extractBits(img: number[], bitOffset: number, bits: number): bigint {
  let v = 0n;
  for (let b = 0; b < bits; b++) {
    const at = bitOffset + b;
    const byte = img[at >> 3];
    if (byte !== undefined && (byte >> (at & 7)) & 1) v |= 1n << BigInt(b);
  }
  return v;
}

const SIGNED = new Set([0x0002, 0x0003, 0x0004, 0x0010, 0x0012, 0x0013, 0x0014, 0x0015]);

/** The value of a process data entry, for display. */
export function entryValue(e: Pick<PdoEntryView, "dataType" | "bits">, raw: bigint): string {
  const bits = e.bits;
  if (e.dataType === 0x0001 || bits === 1) return raw ? "TRUE" : "FALSE";
  if (e.dataType === 0x0008 && bits === 32) {
    const dv = new DataView(new ArrayBuffer(4));
    dv.setUint32(0, Number(raw), true);
    return String(Number(dv.getFloat32(0, true).toPrecision(7)));
  }
  if (e.dataType === 0x0011 && bits === 64) {
    const dv = new DataView(new ArrayBuffer(8));
    dv.setBigUint64(0, raw, true);
    return String(dv.getFloat64(0, true));
  }
  if (SIGNED.has(e.dataType)) {
    const sign = 1n << BigInt(bits - 1);
    return String(raw & sign ? raw - (1n << BigInt(bits)) : raw);
  }
  if (e.dataType >= 0x0030 && e.dataType <= 0x0037) return `0b${raw.toString(2).padStart(bits, "0")}`;
  if (e.dataType === 0x001e || e.dataType === 0x001f || e.dataType === 0x0020) return hex(Number(raw), Math.ceil(bits / 4));
  return raw.toString();
}

/** A numeric reading of an entry for trends, or null when it is not a number. */
export function entryNumber(e: Pick<PdoEntryView, "dataType" | "bits">, raw: bigint): number | null {
  const s = entryValue(e, raw);
  if (s === "TRUE") return 1;
  if (s === "FALSE") return 0;
  const n = Number(s);
  return Number.isFinite(n) ? n : null;
}

/** CiA 402 statusword in words: the state machine state from bits 0-3, 5 and 6. */
export function ds402State(sw: number): string {
  const s = sw & 0x6f;
  if ((sw & 0x4f) === 0x00) return "Not ready to switch on";
  if ((sw & 0x4f) === 0x40) return "Switch on disabled";
  if (s === 0x21) return "Ready to switch on";
  if (s === 0x23) return "Switched on";
  if (s === 0x27) return "Operation enabled";
  if (s === 0x07) return "Quick stop active";
  if ((sw & 0x4f) === 0x0f) return "Fault reaction active";
  if ((sw & 0x4f) === 0x08) return "Fault";
  return "?";
}

export function interfaceLabel(i: NetInterface): string {
  const parts = [i.name];
  if (i.label) parts.push(i.label);
  if (i.mac) parts.push(i.mac);
  if (!i.running) parts.push("no link");
  if (i.wireless) parts.push("Wi-Fi, cannot carry EtherCAT");
  return parts.join(" · ");
}

/** A pcap file (link type Ethernet) of captured frames; Wireshark decodes EtherCAT in it. */
export function pcap(frames: FrameView[], startUnixMs: number): Uint8Array {
  const total = 24 + frames.reduce((n, f) => n + 16 + f.bytes.length, 0);
  const buf = new ArrayBuffer(total);
  const dv = new DataView(buf);
  const u8 = new Uint8Array(buf);
  dv.setUint32(0, 0xa1b2c3d4, true);
  dv.setUint16(4, 2, true);
  dv.setUint16(6, 4, true);
  dv.setInt32(8, 0, true);
  dv.setUint32(12, 0, true);
  dv.setUint32(16, 65535, true);
  dv.setUint32(20, 1, true);
  let at = 24;
  for (const f of frames) {
    const us = startUnixMs * 1000 + f.tsUs;
    dv.setUint32(at, Math.floor(us / 1e6), true);
    dv.setUint32(at + 4, Math.floor(us % 1e6), true);
    dv.setUint32(at + 8, f.bytes.length, true);
    dv.setUint32(at + 12, f.bytes.length, true);
    u8.set(f.bytes, at + 16);
    at += 16 + f.bytes.length;
  }
  return u8;
}

const csvCell = (v: unknown) => {
  const s = String(v ?? "");
  return /[",\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
};

export function devicesCsv(devices: DeviceInfo[]): string {
  const head = ["Position", "Address", "Alias", "Name", "Description", "Vendor ID", "Vendor", "Product code", "Revision", "Serial", "ESC", "DC", "Mailbox", "State", "AL status"];
  const rows = devices.map((d) => [
    d.position, hex(d.address, 4), d.alias, d.name, d.description, hex(d.vendorId, 8), d.vendorName ?? "", hex(d.productCode, 8), hex(d.revision, 8), d.serial,
    d.escName || hex(d.escType, 2), d.dc, d.mailbox.join(" "), d.state, d.alText ?? "",
  ]);
  return [head, ...rows].map((r) => r.map(csvCell).join(",")).join("\n") + "\n";
}

/** Parses a register address typed as hex (`0x0130`, `130`). */
export function parseAddr(text: string): number | null {
  const t = text.trim().replace(/^0x/i, "");
  if (!/^[0-9a-f]{1,4}$/i.test(t)) return null;
  return parseInt(t, 16);
}

export const STATES: AlStateName[] = ["INIT", "PRE-OP", "SAFE-OP", "OP"];
