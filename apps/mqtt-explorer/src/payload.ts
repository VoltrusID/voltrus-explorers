// How a received payload is carried through the UI and rendered.

/** Result of the backend's Sparkplug B / protobuf decoding. */
export interface Decoded {
  kind: "sparkplug" | "protobuf";
  /** Decoded JSON tree, absent when decoding failed. */
  value?: unknown;
  error?: string;
}

/** A payload as the backend delivers it. */
export interface PayloadInfo {
  /** Lossy UTF-8 text. Exact when `rawHex` is absent. */
  payload: string;
  /** Exact bytes as hex, only present when the payload is not valid UTF-8. */
  rawHex?: string;
  decoded?: Decoded;
}

export type PayloadFormat = "raw" | "json" | "hex" | "base64" | "decoded";

/** Pull the payload fields out of a backend event or topic row. */
export function payloadInfoFromEvent(ev: {
  payload?: unknown;
  last_payload?: unknown;
  raw_hex?: unknown;
  decoded?: unknown;
}): PayloadInfo {
  const info: PayloadInfo = { payload: String(ev.payload ?? ev.last_payload ?? "") };
  if (typeof ev.raw_hex === "string") info.rawHex = ev.raw_hex;
  if (ev.decoded && typeof ev.decoded === "object") info.decoded = ev.decoded as Decoded;
  return info;
}

export function payloadBytes(p: PayloadInfo): Uint8Array {
  if (p.rawHex !== undefined) {
    const out = new Uint8Array(p.rawHex.length / 2);
    for (let i = 0; i < out.length; i++) out[i] = parseInt(p.rawHex.slice(i * 2, i * 2 + 2), 16);
    return out;
  }
  return new TextEncoder().encode(p.payload);
}

export function toHex(p: PayloadInfo): string {
  return Array.from(payloadBytes(p), (b) => b.toString(16).padStart(2, "0")).join(" ");
}

export function toBase64(p: PayloadInfo): string {
  let bin = "";
  for (const b of payloadBytes(p)) bin += String.fromCharCode(b);
  return btoa(bin);
}

function shortValue(v: unknown): string {
  if (v === null || v === undefined) return "null";
  if (typeof v === "string") return v.length > 24 ? `"${v.slice(0, 24)}…"` : `"${v}"`;
  if (typeof v === "number" || typeof v === "boolean") return String(v);
  if (Array.isArray(v)) return `[${v.length} items]`;
  if (typeof v === "object") {
    if ("rows" in v && "columns" in v && Array.isArray(v.rows) && Array.isArray(v.columns)) {
      return `dataset ${v.rows.length}x${v.columns.length}`;
    }
    if ("metrics" in v && Array.isArray(v.metrics)) return `template, ${v.metrics.length} metrics`;
  }
  return "{…}";
}

interface SparkplugPayload {
  topic?: { message_type?: string };
  seq?: number | string;
  metrics?: SparkplugMetric[];
}

interface SparkplugMetric {
  name?: string;
  alias?: number | string;
  value?: unknown;
}

/** One line for table cells: Sparkplug shows message type, seq and metrics; protobuf shows compact JSON. */
export function summarizeDecoded(d: Decoded): string {
  if (d.error !== undefined) {
    return `${d.kind === "sparkplug" ? "Sparkplug B" : "Protobuf"} decode failed: ${d.error}`;
  }
  if (d.kind === "protobuf") {
    const s = JSON.stringify(d.value);
    return s.length > 300 ? `${s.slice(0, 300)}…` : s;
  }
  // The backend builds this tree in sparkplug.rs, so the shape is known.
  const v = (d.value ?? {}) as SparkplugPayload;
  const metrics = v.metrics ?? [];
  const shown = metrics.slice(0, 8).map((m) => {
    const label = m.name ?? (m.alias !== undefined ? `alias ${m.alias}` : "?");
    return `${label}=${shortValue(m.value)}`;
  });
  const head = `${v.topic?.message_type ?? "Sparkplug"}${v.seq !== undefined ? ` #${v.seq}` : ""}`;
  const more = metrics.length > shown.length ? [`+${metrics.length - shown.length} more`] : [];
  if (shown.length === 0) return `${head}, no metrics`;
  return [head, ...shown, ...more].join(" · ");
}

/** Full decoded tree for the inspector. */
export function decodedPretty(d: Decoded): string {
  if (d.error !== undefined) return d.error;
  return JSON.stringify(d.value, null, 2);
}

/** Text a table cell or tree leaf shows for `fmt`. */
export function formatPayload(p: PayloadInfo, fmt: PayloadFormat): string {
  switch (fmt) {
    case "json":
      try {
        return JSON.stringify(JSON.parse(p.payload), null, 2);
      } catch {
        return p.payload;
      }
    case "hex":
      return toHex(p);
    case "base64":
      return toBase64(p);
    case "decoded":
      return p.decoded ? summarizeDecoded(p.decoded) : p.payload;
    default:
      return p.payload;
  }
}
