// Pure helpers for the UI (tested in __tests__/logic.test.ts).

import type { CaptureInfo, ChannelInfo, DecoderConfig, DecoderKind, Level, Session, Table, Trigger } from "./types";

/** Default channel colours: the resistor colour code the sigrok world uses, readable on dark and light. */
export const CHANNEL_COLORS = ["#8a8a8a", "#a0522d", "#e5484d", "#f08c00", "#e5c000", "#30a46c", "#3e8ef7", "#9b5de5"];

export const channelColor = (ch: ChannelInfo | undefined, i: number) => ch?.color || CHANNEL_COLORS[i % CHANNEL_COLORS.length];

export const LEVEL_COLORS: Record<Level, string> = {
  data: "#3e8ef7",
  control: "#30a46c",
  info: "#8e7cc3",
  warning: "#e5a50a",
  error: "#e5484d",
};

export function fmtTime(s: number): string {
  const a = Math.abs(s);
  if (a === 0) return "0 s";
  const [v, unit] = a < 1e-6 ? [s * 1e9, "ns"] : a < 1e-3 ? [s * 1e6, "µs"] : a < 1 ? [s * 1e3, "ms"] : [s, "s"];
  const digits = Math.abs(v) >= 100 ? 1 : Math.abs(v) >= 10 ? 2 : 3;
  return `${Number(v.toFixed(digits))} ${unit}`;
}

export function fmtRate(hz: number): string {
  if (hz >= 1e6) return `${Number((hz / 1e6).toFixed(3))} MHz`;
  if (hz >= 1e3) return `${Number((hz / 1e3).toFixed(3))} kHz`;
  return `${Number(hz.toFixed(3))} Hz`;
}

export function fmtSamples(n: number): string {
  if (n >= 1 << 20 && n % (1 << 20) === 0) return `${n / (1 << 20)} Mi`;
  if (n >= 1e6) return `${Number((n / 1e6).toFixed(2))} M`;
  if (n >= 1e3) return `${Number((n / 1e3).toFixed(1))} k`;
  return String(n);
}

/** Capture lengths offered, in samples. */
export const LENGTHS = [100_000, 500_000, 1_000_000, 2_000_000, 5_000_000, 10_000_000, 20_000_000, 50_000_000, 100_000_000, 200_000_000];
/** Above this the capture is a Pro feature (direct-licence builds). */
export const FREE_SAMPLES = 2_000_000;

/** A "nice" ruler step (1, 2, 5 × 10^n seconds) so that ticks are at least `minPx` apart. */
export function tickStep(secondsPerPixel: number, minPx = 90): number {
  const raw = secondsPerPixel * minPx;
  const p = Math.pow(10, Math.floor(Math.log10(raw)));
  for (const m of [1, 2, 5, 10]) if (m * p >= raw) return m * p;
  return 10 * p;
}

/** The visible window: first sample and samples per pixel (fractional when zoomed past single samples). */
export interface View {
  start: number;
  spp: number;
}

/** Most pixels one sample may take when zoomed in. */
export const MAX_PX_PER_SAMPLE = 64;

export function clampView(v: View, samples: number, width: number): View {
  const maxSpp = Math.max(samples / Math.max(width, 1), 1 / MAX_PX_PER_SAMPLE);
  const spp = Math.min(Math.max(v.spp, 1 / MAX_PX_PER_SAMPLE), maxSpp);
  const visible = spp * width;
  const start = Math.min(Math.max(v.start, 0), Math.max(samples - visible, 0));
  return { start, spp };
}

export function fitView(samples: number, width: number): View {
  return clampView({ start: 0, spp: samples / Math.max(width, 1) }, samples, width);
}

/** Zoom by `factor` (>1 zooms in) keeping the sample under pixel `x` in place. */
export function zoomAt(v: View, factor: number, x: number, samples: number, width: number): View {
  const at = v.start + x * v.spp;
  const spp = v.spp / factor;
  return clampView({ start: at - x * spp, spp }, samples, width);
}

/** A view showing `[a, b)` with a margin. */
export function viewOf(a: number, b: number, samples: number, width: number): View {
  const span = Math.max(b - a, 4);
  const margin = span * 0.15;
  return clampView({ start: a - margin, spp: (span + 2 * margin) / Math.max(width, 1) }, samples, width);
}

export function sampleTime(info: CaptureInfo, sample: number): number {
  return (sample - (info.trigger ?? 0)) / info.sampleRate;
}

/** Cursor readout: the time between two cursors and the frequency it corresponds to. */
export function cursorDelta(info: CaptureInfo, a: number, b: number): { dt: number; freq: number | null; samples: number } {
  const samples = Math.abs(b - a);
  const dt = samples / info.sampleRate;
  return { dt, freq: dt > 0 ? 1 / dt : null, samples };
}

export const DECODER_NAMES: Record<DecoderKind, string> = {
  modbusRtu: "Modbus RTU",
  uart: "UART",
  spi: "SPI",
  i2c: "I2C",
  can: "CAN",
};

/** Decoders that need Pro in licence-key builds. */
/** `CanConfig.dbc` value for CAN Explorer's simulator DBC (matches the CAN scenario). */
export const SIM_DBC = "builtin:can-explorer-sim";

/** The file name of a path, for labels. */
export function baseName(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}

export const PRO_DECODERS: DecoderKind[] = ["can"];

export function defaultDecoder(kind: DecoderKind, channels: number): DecoderConfig {
  const ch = (i: number) => Math.min(i, Math.max(channels - 1, 0));
  switch (kind) {
    case "uart":
      return { kind, channel: 0, baud: 9600, dataBits: 8, parity: "none", stopBits: 1, inverted: false, msbFirst: false };
    case "modbusRtu":
      return { kind, requestChannel: 0, responseChannel: channels > 1 ? 1 : null, baud: 19200, parity: "even", stopBits: 1, inverted: false, timeoutMs: 1000 };
    case "spi":
      return { kind, cs: ch(0), clk: ch(1), mosi: ch(2), miso: ch(3), mode: 0, wordBits: 8, lsbFirst: false, csActiveHigh: false, device: "none" };
    case "i2c":
      return { kind, scl: ch(0), sda: ch(1), minPulse: 0 };
    case "can":
      return { kind, channel: 0, bitrate: 500_000, samplePoint: 0.75, inverted: false, logWindowMs: 2 };
  }
}

const chName = (channels: ChannelInfo[], i: number | null) => (i == null ? "—" : channels[i]?.name || `D${i}`);

/** One-line description of a decoder for the stack list. */
export function decoderLabel(d: DecoderConfig, channels: ChannelInfo[]): string {
  const parity = (p: string) => p[0].toUpperCase();
  switch (d.kind) {
    case "uart":
      return `UART ${d.baud} ${d.dataBits}${parity(d.parity)}${d.stopBits} on ${chName(channels, d.channel)}`;
    case "modbusRtu":
      return `Modbus RTU ${d.baud} 8${parity(d.parity)}${d.stopBits} · ${chName(channels, d.requestChannel)}${d.responseChannel != null ? ` / ${chName(channels, d.responseChannel)}` : ""}`;
    case "spi":
      return `SPI mode ${d.mode}${d.device === "mcp2515" ? " · MCP2515" : ""}`;
    case "i2c":
      return `I2C on ${chName(channels, d.scl)} / ${chName(channels, d.sda)}`;
    case "can":
      return `CAN ${d.bitrate / 1000} kbit/s on ${chName(channels, d.channel)}`;
  }
}

/** Channels a decoder reads. */
export function decoderChannels(d: DecoderConfig): number[] {
  switch (d.kind) {
    case "uart":
    case "can":
      return [d.channel];
    case "modbusRtu":
      return [d.requestChannel, d.responseChannel].filter((x): x is number => x != null);
    case "spi":
      return [d.clk, d.mosi, d.miso, d.cs].filter((x): x is number => x != null);
    case "i2c":
      return [d.scl, d.sda];
  }
}

/** Per channel: "0", "1" or "x" (don't care), as a trigger pattern. */
export type PatternBit = "0" | "1" | "x";

export function patternTrigger(bits: PatternBit[]): Trigger {
  let mask = 0;
  let value = 0;
  bits.forEach((b, i) => {
    if (b === "x") return;
    mask |= 1 << i;
    if (b === "1") value |= 1 << i;
  });
  return mask === 0 ? { kind: "none" } : { kind: "pattern", mask, value };
}

function csvCell(s: string): string {
  return /[",\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
}

export function tableCsv(t: Table, info: CaptureInfo): string {
  const head = ["Start sample", "Start (s)", ...t.columns].map(csvCell).join(",");
  const rows = t.rows.map((r) => [String(r.start), sampleTime(info, r.start).toFixed(9), ...r.cells].map(csvCell).join(","));
  return [head, ...rows].join("\n") + "\n";
}

export function makeSession(decoders: { id: number; config: DecoderConfig; hidden?: boolean }[], cursors: [number | null, number | null], view?: View): Session {
  return { version: 1, decoders, cursors, view };
}

/** Accepts a session from a file only if it has the expected shape. */
export function readSession(v: unknown): Session | null {
  if (!v || typeof v !== "object") return null;
  const s = v as Partial<Session>;
  if (s.version !== 1 || !Array.isArray(s.decoders)) return null;
  const decoders = s.decoders.filter((d) => d && typeof d.id === "number" && d.config && typeof d.config.kind === "string" && d.config.kind in DECODER_NAMES);
  const c = Array.isArray(s.cursors) ? s.cursors : [null, null];
  const num = (x: unknown) => (typeof x === "number" && Number.isFinite(x) ? x : null);
  return { version: 1, decoders, cursors: [num(c[0]), num(c[1])], view: s.view };
}
