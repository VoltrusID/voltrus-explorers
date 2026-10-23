import { EmptyState, IconButton, ListGroup, ListRow } from "@mactools/ui";
import { ChartLine } from "lucide-react";
import { isBitArea } from "../helpers";
import type { TableRow } from "../utils/rows";

interface Props {
  rows: TableRow[];
  trendNames: { has: (name: string) => boolean };
  onRowClick: (row: TableRow) => void;
  onToggleTrend: (name: string) => void;
}

function valueCell(r: TableRow) {
  if (r.error) {
    const code = /\(0x([0-9A-Fa-f]{2})\)/.exec(r.error)?.[1];
    return <span style={{ color: "var(--danger)" }}>{code ? `EXC 0x${code}` : "ERR"}</span>;
  }
  if (isBitArea(r.area) && r.rawValue !== null) {
    const on = r.value === "ON";
    return <span className={on ? "vt-badge vt-badge--success" : "vt-badge"}>{r.value}</span>;
  }
  return r.value;
}

/** The Data tab on a phone: one row per value (name over address, hex and type; the value at the right) in a grouped card. */
export function PhoneDataList({ rows, trendNames, onRowClick, onToggleTrend }: Props) {
  if (rows.length === 0) {
    return <EmptyState title="No values" description="Add a read block on the Blocks tab to start polling registers." />;
  }
  return (
    <ListGroup>
      {rows.map((r) => {
        const active = trendNames.has(r.name);
        return (
          <ListRow
            key={`${r._blockIdx}:${r.rawAddress}:${r.name}`}
            title={r.name}
            subtitle={`${r.address} · ${r.hex} · ${r.type}`}
            value={valueCell(r)}
            onClick={() => onRowClick(r)}
            trailing={
              <IconButton
                outline
                pressed={active}
                label={active ? `Remove ${r.name} from trend` : `Add ${r.name} to trend`}
                onClick={(e) => {
                  e.stopPropagation();
                  onToggleTrend(r.name);
                }}
              >
                <ChartLine size={18} strokeWidth={1.75} />
              </IconButton>
            }
          />
        );
      })}
    </ListGroup>
  );
}
