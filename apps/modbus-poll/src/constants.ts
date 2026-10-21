export const AREAS = ["holding", "input", "coil", "discrete"];
export const TYPES = ["uint16", "int16", "int32", "float32", "float64", "string"];
export const ENDIAN_OPTS: { value: string; label: string }[] = [
  { value: "big", label: "AB CD" },
  { value: "little", label: "CD AB" },
  { value: "swapped", label: "BA DC" },
];

export const sidebarStyle: React.CSSProperties = {
  width: 264,
  flexShrink: 0,
  borderRight: "1px solid var(--border)",
  background: "var(--card)",
  padding: 16,
  overflowY: "auto",
  display: "flex",
  flexDirection: "column",
  gap: 16,
};

export const thStyle: React.CSSProperties = {
  textAlign: "left",
  padding: "6px 12px",
  borderBottom: "1px solid var(--border)",
  color: "var(--text-muted)",
  fontSize: "var(--fs-xs)",
  fontWeight: 600,
  position: "sticky" as const,
  top: 0,
  background: "var(--card)",
  whiteSpace: "nowrap" as const,
};

