import { appendTextFile } from "@mactools/ui";
import type { TagConfig } from "./types";
import type { S7Value } from "./useS7Device";
import { csvField } from "./tagCsv";

/** Rows are batched so a fast poll rate does not mean a file write per poll. */
const FLUSH_MS = 1000;

/**
 * Header line for a value log: `timestamp` plus one column per tag. Tags that share a name get
 * their address appended so every column is distinct.
 */
export function valueLogHeader(tags: TagConfig[]): string {
  const counts: Record<string, number> = {};
  for (const t of tags) counts[t.name] = (counts[t.name] ?? 0) + 1;
  const cols = tags.map((t) => csvField(counts[t.name] > 1 ? `${t.name} [${t.address}]` : t.name));
  return ["timestamp", ...cols].join(",");
}

/** One log line: an ISO 8601 UTC timestamp, then each value (booleans as 1 or 0). */
export function valueLogRow(time: Date, values: S7Value[]): string {
  const cells = values.map((v) => {
    if (typeof v === "boolean") return v ? "1" : "0";
    if (typeof v === "number") return String(v);
    return csvField(v);
  });
  return [time.toISOString(), ...cells].join(",");
}

/**
 * Appends log rows to a file the user picked. Rows are buffered and written in order at most once
 * per second. After a failed write the writer is closed and ignores further rows.
 */
export class ValueLogWriter {
  rows = 0;
  closed = false;
  private pending = "";
  private timer: ReturnType<typeof setTimeout> | null = null;
  private chain: Promise<void> = Promise.resolve();

  constructor(
    readonly path: string,
    /** The tag columns the file was started with. A log never changes its columns. */
    readonly signature: string,
    private readonly onError: (e: unknown) => void,
  ) {}

  push(line: string) {
    if (this.closed) return;
    this.pending += line + "\n";
    this.rows++;
    this.timer ??= setTimeout(() => void this.flush(), FLUSH_MS);
  }

  /** Write everything buffered so far. Resolves once the file is up to date. */
  flush(): Promise<void> {
    if (this.timer) {
      clearTimeout(this.timer);
      this.timer = null;
    }
    const text = this.pending;
    if (text === "") return this.chain;
    this.pending = "";
    this.chain = this.chain.then(async () => {
      if (this.closed) return;
      try {
        await appendTextFile(this.path, text);
      } catch (e) {
        this.closed = true;
        this.onError(e);
      }
    });
    return this.chain;
  }

  /** Flush and stop accepting rows. */
  async close() {
    await this.flush();
    this.closed = true;
  }
}
