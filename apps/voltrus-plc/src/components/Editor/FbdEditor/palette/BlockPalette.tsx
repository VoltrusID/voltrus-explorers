import { BLOCK_LIBRARY, type BlockCategory } from "../../../../types/fbd";

const CATEGORIES: { id: BlockCategory | "io"; label: string }[] = [
  { id: "io", label: "I/O" },
  { id: "logic", label: "Logic" },
  { id: "arithmetic", label: "Arithmetic" },
  { id: "comparison", label: "Compare" },
  { id: "timer", label: "Timers" },
  { id: "counter", label: "Counters" },
  { id: "move", label: "Move" },
  { id: "bistable", label: "Bistable" },
  { id: "edge", label: "Edge" },
];

const CATEGORY_COLORS: Record<string, string> = {
  io: "#10b981",
  logic: "#7c3aed",
  arithmetic: "#3b82f6",
  comparison: "#f59e0b",
  timer: "#f59e0b",
  counter: "#8b5cf6",
  move: "#6c7086",
  bistable: "#ef4444",
  edge: "#06b6d4",
};

export default function BlockPalette() {
  const onDragStart = (event: React.DragEvent, blockType: string) => {
    event.dataTransfer.setData("application/fbd-block", blockType);
    event.dataTransfer.effectAllowed = "move";
  };

  return (
    <div style={{
      width: 140,
      borderRight: "1px solid var(--border, #45475a)",
      background: "var(--card, #1e1e2e)",
      overflowY: "auto",
      padding: "6px 4px",
      fontSize: 10,
      flexShrink: 0,
    }}>
      {CATEGORIES.map((cat) => {
        const blocks = BLOCK_LIBRARY.filter((b) =>
          cat.id === "io" ? (b.category === "input" || b.category === "output") : b.category === cat.id,
        );
        if (blocks.length === 0) return null;
        return (
          <div key={cat.id} style={{ marginBottom: 6 }}>
            <div style={{
              color: CATEGORY_COLORS[cat.id],
              fontWeight: 700,
              fontSize: 9,
              marginBottom: 3,
              paddingLeft: 2,
              textTransform: "uppercase",
            }}>
              {cat.label}
            </div>
            <div style={{ display: "flex", flexWrap: "wrap", gap: 3 }}>
              {blocks.map((b) => (
                <div
                  key={b.type}
                  draggable
                  onDragStart={(e) => onDragStart(e, b.type)}
                  style={{
                    padding: "2px 6px",
                    background: `${CATEGORY_COLORS[cat.id]}22`,
                    border: `1px solid ${CATEGORY_COLORS[cat.id]}44`,
                    borderRadius: 3,
                    cursor: "grab",
                    color: CATEGORY_COLORS[cat.id],
                    fontWeight: 500,
                    fontSize: 9,
                  }}
                >
                  {b.label}
                </div>
              ))}
            </div>
          </div>
        );
      })}
    </div>
  );
}
