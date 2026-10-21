export interface MessageRow {
  timestamp: number;
  topic: string;
  payload: string;
  /** `null` when the source has no per-message QoS (broker stream). */
  qos: number | null;
  retain: boolean;
}

function field(v: string): string {
  // Quote when needed, and neutralise spreadsheet formula injection from untrusted payloads.
  let t = v;
  if (/^([=@\t\r]|[+\-][^0-9.])/.test(t)) t = `'${t}`;
  return /[",\r\n]/.test(t) ? `"${t.replace(/"/g, '""')}"` : t;
}

export function messagesToCsv(rows: MessageRow[]): string {
  const lines = ["timestamp,topic,payload,qos,retain"];
  for (const r of rows) {
    lines.push(
      [
        new Date(r.timestamp).toISOString(),
        field(r.topic),
        field(r.payload),
        r.qos === null ? "" : String(r.qos),
        r.retain ? "true" : "false",
      ].join(","),
    );
  }
  return lines.join("\r\n") + "\r\n";
}
