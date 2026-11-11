import { type ReactNode, useMemo, useRef, useState } from "react";
import { Button, EmptyState, ProBadge } from "@mactools/ui";
import { Download, TableProperties } from "lucide-react";
import type { Level, Table, TableRow } from "./types";

const ROW_H = 24;

const levelColor: Record<Level, string | undefined> = {
  data: undefined,
  control: undefined,
  info: "var(--text-dim)",
  warning: "var(--warn)",
  error: "var(--danger)",
};

interface Props {
  table: Table | null;
  selected: number | null;
  onSelect: (index: number, row: TableRow) => void;
  onExport: () => void;
  pro: boolean;
  /** More buttons next to CSV (the CAN table's candump export). */
  actions?: ReactNode;
}

/** A protocol table (Modbus transactions, CAN frames...), windowed so tens of thousands of rows stay smooth. */
export default function ProtocolTable({ table, selected, onSelect, onExport, pro, actions }: Props) {
  const [filter, setFilter] = useState("");
  const [scroll, setScroll] = useState(0);
  const [height, setHeight] = useState(200);
  const ref = useRef<HTMLDivElement>(null);

  const rows = useMemo(() => {
    if (!table) return [];
    const all = table.rows.map((r, i) => ({ r, i }));
    const f = filter.trim().toLowerCase();
    if (!f) return all;
    return all.filter(({ r }) => r.cells.some((c) => c.toLowerCase().includes(f)));
  }, [table, filter]);

  if (!table) {
    return <EmptyState compact icon={<TableProperties size={18} strokeWidth={1.75} />} title="No table" description="Modbus RTU, CAN, SPI and I2C decoders fill this table. Add one in the sidebar." />;
  }
  const cols = table.columns;
  const first = Math.max(0, Math.floor(scroll / ROW_H) - 5);
  const last = Math.min(rows.length, first + Math.ceil(height / ROW_H) + 10);
  const grid = cols.map((c) => (c === "Time" ? "120px" : c === "Request" || c === "Meaning" || c === "Data" || c === "Values" || c === "Signals" ? "minmax(160px, 2fr)" : "minmax(70px, 1fr)")).join(" ");

  return (
    <div style={{ display: "flex", flexDirection: "column", flex: 1, minHeight: 0, gap: 6 }}>
      <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
        <input className="vt-input" aria-label="Filter rows" placeholder="Filter: slave 2, Exception, 0x123, CRC" value={filter} onChange={(e) => setFilter(e.target.value)} style={{ maxWidth: 320 }} />
        <span style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)" }}>{rows.length.toLocaleString()} of {table.rows.length.toLocaleString()}</span>
        <span style={{ flex: 1 }} />
        {actions}
        <Button size="sm" icon={<Download size={14} strokeWidth={1.75} />} onClick={onExport}>CSV {!pro && <ProBadge />}</Button>
      </div>
      <div style={{ display: "grid", gridTemplateColumns: grid, gap: 8, padding: "0 8px", fontSize: "var(--fs-xs)", color: "var(--text-muted)", fontWeight: 600 }}>
        {cols.map((c) => <span key={c}>{c}</span>)}
      </div>
      <div
        ref={(el) => { ref.current = el; if (el && el.clientHeight !== height) setHeight(el.clientHeight); }}
        onScroll={(e) => setScroll(e.currentTarget.scrollTop)}
        style={{ flex: 1, minHeight: 0, overflowY: "auto", border: "1px solid var(--border)", borderRadius: "var(--radius)", background: "var(--card)" }}
      >
        <div style={{ height: rows.length * ROW_H, position: "relative" }}>
          {rows.slice(first, last).map(({ r, i }, k) => (
            <div
              key={i}
              role="button"
              tabIndex={0}
              onClick={() => onSelect(i, r)}
              onKeyDown={(e) => { if (e.key === "Enter") onSelect(i, r); }}
              className="mono"
              style={{
                position: "absolute",
                top: (first + k) * ROW_H,
                left: 0,
                right: 0,
                height: ROW_H,
                display: "grid",
                gridTemplateColumns: grid,
                gap: 8,
                alignItems: "center",
                padding: "0 8px",
                fontSize: 11.5,
                cursor: "pointer",
                background: selected === i ? "var(--selection)" : undefined,
                color: levelColor[r.level],
                borderBottom: "1px solid var(--border)",
              }}
            >
              {r.cells.map((c, j) => (
                <span key={j} title={c} style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{c}</span>
              ))}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
