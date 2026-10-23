import type { BacnetObjectType, BacnetTag, BacnetTagList } from "@mactools/export";
import type { ObjectListRow } from "./objectCsv";
import type { BacnetConfig } from "./types";

/** What an export needs to know about the device. */
export type ExportDevice = Pick<BacnetConfig, "ip" | "port" | "device_id">;

const SUPPORTED: ReadonlySet<string> = new Set<BacnetObjectType>([
  "analogInput", "analogOutput", "analogValue", "binaryInput", "binaryOutput", "binaryValue",
  "multiStateInput", "multiStateOutput", "multiStateValue",
]);

/** Outputs are commanded; values are usually writable too. Inputs are not. */
const isWritable = (t: string): boolean => t.endsWith("Output") || t.endsWith("Value");

/** Poll interval written into the exports. BACnet points change slowly; the app does not poll on a fixed rate itself. */
export const EXPORT_POLL_MS = 5000;

export interface BacnetTagListResult {
  list: BacnetTagList;
  /** Objects of a type the exporters do not carry (device, schedule, trend log ...). */
  skipped: number;
}

/**
 * The browsed objects as an export tag list. Present value is the property exported for every object. Tag names are the
 * object names the device reported, made unique with the instance number when two objects share one.
 */
export function buildTagList(rows: ObjectListRow[], device: ExportDevice, name: string): BacnetTagListResult {
  const kept = rows.filter((r) => SUPPORTED.has(r.object.object_type));
  const count = new Map<string, number>();
  const names = kept.map((r) => {
    const base = (r.detail?.object_name ?? r.object.object_name ?? "").trim() || `${r.object.object_type}_${r.object.object_instance}`;
    count.set(base, (count.get(base) ?? 0) + 1);
    return base;
  });
  const tags: BacnetTag[] = kept.map((r, i) => {
    const unit = r.detail?.units_symbol ?? r.detail?.units ?? undefined;
    const description = r.detail?.description?.trim() || undefined;
    const base = names[i];
    return {
      name: (count.get(base) ?? 0) > 1 ? `${base}_${r.object.object_instance}` : base,
      objectType: r.object.object_type as BacnetObjectType,
      instance: r.object.object_instance,
      property: "presentValue",
      ...(unit ? { unit } : {}),
      ...(description ? { description } : {}),
      writable: isWritable(r.object.object_type),
    };
  });
  return {
    list: { protocol: "bacnet", source: { name, host: device.ip, port: device.port, deviceId: device.device_id, pollMs: EXPORT_POLL_MS }, tags },
    skipped: rows.length - kept.length,
  };
}
