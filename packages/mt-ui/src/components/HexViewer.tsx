import { EmptyState } from "../primitives";

interface HexViewerProps {
  data: Uint8Array;
  bytesPerRow?: number;
}

const thStyle: React.CSSProperties = {
  padding: "4px 10px",
  fontSize: "var(--fs-xs)",
  fontWeight: 600,
  color: "var(--text-muted)",
  textAlign: "left",
  borderBottom: "1px solid var(--border)",
  background: "var(--card)",
  position: "sticky",
  top: 0,
};

const tdStyle: React.CSSProperties = {
  padding: "2px 10px",
  borderBottom: "1px solid var(--border)",
};

export function HexViewer({ data, bytesPerRow = 16 }: HexViewerProps) {
  const rows: { offset: string; hex: string; ascii: string }[] = [];
  for (let i = 0; i < data.length; i += bytesPerRow) {
    const slice = data.slice(i, Math.min(i + bytesPerRow, data.length));
    const hex = Array.from(slice)
      .map((b) => b.toString(16).padStart(2, "0"))
      .join(" ");
    const ascii = Array.from(slice)
      .map((b) => (b >= 32 && b <= 126 ? String.fromCharCode(b) : "\u00B7"))
      .join("");
    rows.push({
      offset: i.toString(16).padStart(8, "0").toUpperCase(),
      hex: hex.padEnd(bytesPerRow * 3 - 1, " "),
      ascii,
    });
  }

  return (
    <div
      style={{
        overflow: "auto",
        fontFamily: "var(--font-mono)",
        fontVariantNumeric: "tabular-nums",
        fontSize: "var(--fs-xs)",
      }}
    >
      <table style={{ width: "100%", borderCollapse: "collapse" }}>
        <thead>
          <tr>
            <th style={{ ...thStyle, width: 80 }}>Offset</th>
            <th style={thStyle}>Hex</th>
            <th style={{ ...thStyle, width: 140 }}>ASCII</th>
          </tr>
        </thead>
        <tbody>
          {rows.length === 0 ? (
            <tr>
              <td colSpan={3} style={{ padding: 0 }}>
                <EmptyState
                  compact
                  title="No data"
                  description="Payload bytes appear here after a message is captured."
                />
              </td>
            </tr>
          ) : (
            rows.map((row, i) => (
              <tr key={i}>
                <td style={{ ...tdStyle, color: "var(--text-muted)" }}>{row.offset}</td>
                <td style={{ ...tdStyle, color: "var(--text)" }}>{row.hex}</td>
                <td style={{ ...tdStyle, color: "var(--text-dim)" }}>{row.ascii}</td>
              </tr>
            ))
          )}
        </tbody>
      </table>
    </div>
  );
}
