import type { BacnetObjectDetail, BacnetObjectInfo } from "./types";
import { formatObjectType } from "./types";

export interface ObjectListRow {
  object: BacnetObjectInfo;
  /** `null` when the device did not answer for this object. */
  detail: BacnetObjectDetail | null;
  error?: string;
}

const HEADER = [
  "Object Type",
  "Instance",
  "Object Name",
  "Description",
  "Present Value",
  "Units",
  "Status Flags",
  "Reliability",
  "Out Of Service",
  "Read Error",
];

/** Quote a cell when it holds a separator, quote or line break. */
function cell(value: string): string {
  return /[",\r\n]/.test(value) ? `"${value.replace(/"/g, '""')}"` : value;
}

/** Text that came from a device must not start a spreadsheet formula. */
function textCell(value: string | null | undefined): string {
  const text = value ?? "";
  return cell(/^[=+\-@\t\r]/.test(text) ? `'${text}` : text);
}

export function buildObjectListCsv(rows: ObjectListRow[]): string {
  const lines = [HEADER.join(",")];
  for (const { object, detail, error } of rows) {
    lines.push(
      [
        cell(formatObjectType(object.object_type)),
        String(object.object_instance),
        textCell(detail?.object_name ?? object.object_name),
        textCell(detail?.description),
        cell(detail?.present_value ?? ""),
        textCell(detail?.units_symbol ?? detail?.units),
        cell(detail?.status_flags ?? ""),
        cell(detail?.reliability ?? ""),
        detail?.out_of_service == null ? "" : String(detail.out_of_service),
        textCell(error),
      ].join(","),
    );
  }
  return lines.join("\n") + "\n";
}
