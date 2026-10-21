import { quote } from "./csv";
import type { TableRow } from "./utils/rows";

export const CSV_LOG_HEADER = "timestamp,connection,name,area,address,type,value,error\n";

/**
 * One CSV line per polled value (not per register), UTC ISO-8601 timestamps, numeric values
 * (never the on-screen format). Values beyond the free cap were not polled and are left out.
 */
export function csvLogLines(connection: string, timestampMs: number, rows: TableRow[]): string {
  const stamp = new Date(timestampMs).toISOString();
  return rows
    .filter((r) => !r.limited)
    .map((r) =>
      [stamp, quote(connection), quote(r.name), r.area, r.rawAddress, r.type, r.rawValue ?? "", r.error ? quote(r.error) : ""].join(",") + "\n",
    )
    .join("");
}

/** Default file name for a log started at `now`. */
export const csvLogFileName = (now: Date) => `modbus-log-${now.toISOString().slice(0, 19).replace(/:/g, "-")}.csv`;
