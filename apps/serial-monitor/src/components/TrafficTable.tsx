import { Badge, DataTable } from "@mactools/ui";
import type { TrafficEntry } from "../types";

const COLUMNS = [
  { key: "direction", label: "Dir", width: "56px" },
  { key: "data", label: "Data" },
  { key: "timestamp", label: "Time", width: "120px" },
];

function renderCell(key: string, row: Record<string, unknown>) {
  if (key === "direction") {
    const dir = String(row.direction ?? "");
    return <Badge tone={dir === "TX" ? "info" : "success"}>{dir}</Badge>;
  }
  return String(row[key] ?? "");
}

export function TrafficTable({ data, emptyMessage }: { data: TrafficEntry[]; emptyMessage: string }) {
  return (
    <DataTable columns={COLUMNS} data={data} selectable={false} emptyMessage={emptyMessage} renderCell={renderCell} />
  );
}
