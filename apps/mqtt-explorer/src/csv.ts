export interface MessageRow {
  timestamp: number;
  topic: string;
  /** Text for UTF-8 payloads, hex for binary ones (see `encoding`). */
  payload: string;
  /** `hex` when the payload is not valid UTF-8 and `payload` holds its bytes as hex. */
  encoding: "utf8" | "hex";
  /** `null` when the source has no per-message QoS (broker stream). */
  qos: number | null;
  retain: boolean;
  /** Sparkplug B / protobuf result as compact JSON, empty when the message was not decoded. */
  decoded: string;
}

function field(v: string): string {
  // Quote when needed, and neutralise spreadsheet formula injection from untrusted payloads.
  let t = v;
  if (/^([=@\t\r]|[+\-][^0-9.])/.test(t)) t = `'${t}`;
  return /[",\r\n]/.test(t) ? `"${t.replace(/"/g, '""')}"` : t;
}

export function messagesToCsv(rows: MessageRow[]): string {
  const lines = ["timestamp,topic,payload,qos,retain,encoding,decoded"];
  for (const r of rows) {
    lines.push(
      [
        new Date(r.timestamp).toISOString(),
        field(r.topic),
        field(r.payload),
        r.qos === null ? "" : String(r.qos),
        r.retain ? "true" : "false",
        r.encoding,
        field(r.decoded),
      ].join(","),
    );
  }
  return lines.join("\r\n") + "\r\n";
}
