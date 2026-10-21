import { useState, useMemo, useRef, useEffect } from "react";
import { ChevronDown, ChevronUp } from "lucide-react";
import { EmptyState } from "../primitives";

export interface Column {
  key: string;
  label: string;
  width?: string;
  align?: "left" | "right" | "center";
}

const wrapperStyle: React.CSSProperties = {
  flex: 1,
  display: "flex",
  flexDirection: "column",
  minHeight: 0,
  overflow: "auto",
  background: "var(--card)",
  border: "1px solid var(--border)",
  borderRadius: "var(--radius-lg)",
};

interface DataTableProps {
  columns: Column[];
  data: Record<string, unknown>[];
  onRowClick?: (row: Record<string, unknown>, index: number) => void;
  selectable?: boolean;
  emptyMessage?: string;
  getRowStyle?: (row: Record<string, unknown>, index: number) => React.CSSProperties | undefined;
  autoScroll?: boolean;
  renderCell?: (key: string, row: Record<string, unknown>, index: number) => React.ReactNode;
}

export function DataTable({
  columns,
  data,
  onRowClick,
  selectable = false,
  emptyMessage = "No data",
  getRowStyle,
  autoScroll = false,
  renderCell,
}: DataTableProps) {
  const [sortKey, setSortKey] = useState<string | null>(null);
  const [sortDir, setSortDir] = useState<"asc" | "desc">("asc");
  const [selected, setSelected] = useState<number | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (autoScroll && scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [data, autoScroll]);

  const sorted = useMemo(() => {
    if (!sortKey) return data;
    return [...data].sort((a, b) => {
      const va = String(a[sortKey] ?? "");
      const vb = String(b[sortKey] ?? "");
      const cmp = va.localeCompare(vb, undefined, { numeric: true });
      return sortDir === "asc" ? cmp : -cmp;
    });
  }, [data, sortKey, sortDir]);

  const handleSort = (key: string) => {
    if (sortKey === key) {
      setSortDir(sortDir === "asc" ? "desc" : "asc");
    } else {
      setSortKey(key);
      setSortDir("asc");
    }
  };

  const handleCopy = (text: string) => {
    navigator.clipboard.writeText(text);
  };

  return (
    <div ref={scrollRef} style={wrapperStyle}>
      {sorted.length === 0 ? (
        <EmptyState compact title={emptyMessage} />
      ) : (
        <table className="vt-table" style={{ width: "100%" }}>
          <thead>
            <tr>
              {columns.map((col) => (
                <th
                  key={col.key}
                  onClick={() => handleSort(col.key)}
                  aria-sort={sortKey === col.key ? (sortDir === "asc" ? "ascending" : "descending") : undefined}
                  style={{
                    width: col.width,
                    textAlign: col.align === "right" ? "right" : col.align === "center" ? "center" : "left",
                    cursor: "pointer",
                  }}
                >
                  {col.label}
                  {sortKey === col.key && (sortDir === "asc"
                    ? <ChevronUp size={12} strokeWidth={1.75} color="var(--accent)" style={{ marginLeft: 2, verticalAlign: "-2px" }} />
                    : <ChevronDown size={12} strokeWidth={1.75} color="var(--accent)" style={{ marginLeft: 2, verticalAlign: "-2px" }} />)}
                </th>
              ))}
            </tr>
          </thead>
          <tbody className="mono">
            {sorted.map((row, i) => {
              const isSelected = selectable && selected === i;
              const extraStyle = getRowStyle?.(row, i);
              return (
                <tr
                  key={i}
                  aria-selected={selectable ? isSelected : undefined}
                  onClick={() => {
                    if (selectable) setSelected(i);
                    onRowClick?.(row, i);
                  }}
                  style={extraStyle}
                >
                  {columns.map((col) => (
                    <td
                      key={col.key}
                      onDoubleClick={() =>
                        handleCopy(String(row[col.key] ?? ""))
                      }
                      title="Double-click to copy"
                      className={col.align === "right" ? "is-num" : undefined}
                      style={col.align === "center" ? { textAlign: "center" } : undefined}
                    >
                      {renderCell ? renderCell(col.key, row, i) : String(row[col.key] ?? "")}
                    </td>
                  ))}
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
    </div>
  );
}
