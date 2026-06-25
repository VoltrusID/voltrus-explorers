import { useState } from "react";
import { LD_ELEMENT_DEFS, type LdElementCategory } from "../../../../types/ld";

const CATEGORIES: Array<{ id: LdElementCategory | "all"; label: string }> = [
  { id: "all", label: "All" },
  { id: "contact", label: "Contacts" },
  { id: "coil", label: "Coils" },
  { id: "timer", label: "Timers" },
  { id: "counter", label: "Counters" },
  { id: "function", label: "Functions" },
];

const CATEGORY_COLORS: Record<string, string> = {
  contact: "#10b981",
  coil: "#3b82f6",
  timer: "#f59e0b",
  counter: "#a855f7",
  function: "#6c7086",
};

export default function LdPalette() {
  const [filter, setFilter] = useState<LdElementCategory | "all">("all");

  const elements = filter === "all"
    ? LD_ELEMENT_DEFS
    : LD_ELEMENT_DEFS.filter((d) => d.category === filter);

  const onDragStart = (event: React.DragEvent, def: typeof LD_ELEMENT_DEFS[number]) => {
    event.dataTransfer.setData("application/ld-element", JSON.stringify({
      elementType: def.type,
      blockType: def.blockType,
    }));
    event.dataTransfer.effectAllowed = "move";
  };

  return (
    <div style={{
      width: 140,
      background: "var(--card, #1e1e2e)",
      borderRight: "1px solid var(--border, #45475a)",
      overflowY: "auto",
      fontSize: 10,
      padding: 4,
    }}>
      <div style={{ fontWeight: 700, color: "var(--accent, #7c3aed)", padding: "4px 4px 8px", fontSize: 10 }}>
        LADDER
      </div>

      <div style={{ display: "flex", flexWrap: "wrap", gap: 2, marginBottom: 6 }}>
        {CATEGORIES.map((cat) => (
          <button
            key={cat.id}
            onClick={() => setFilter(cat.id)}
            style={{
              padding: "2px 6px",
              fontSize: 8,
              background: filter === cat.id ? "var(--accent, #7c3aed)" : "var(--bg, #11111b)",
              color: filter === cat.id ? "#fff" : "var(--text-muted, #6c7086)",
              border: "1px solid var(--border, #45475a)",
              borderRadius: 3,
              cursor: "pointer",
            }}
          >
            {cat.label}
          </button>
        ))}
      </div>

      {elements.map((def, i) => (
        <div
          key={`${def.type}-${def.blockType ?? i}`}
          draggable
          onDragStart={(e) => onDragStart(e, def)}
          style={{
            padding: "4px 6px",
            marginBottom: 2,
            background: "var(--bg, #11111b)",
            border: `1px solid var(--border, #45475a)`,
            borderRadius: 4,
            cursor: "grab",
            display: "flex",
            alignItems: "center",
            gap: 6,
          }}
        >
          <span style={{
            color: CATEGORY_COLORS[def.category] || "#6c7086",
            fontFamily: '"SF Mono", monospace',
            fontSize: 10,
            fontWeight: 700,
            minWidth: 28,
            textAlign: "center",
          }}>
            {def.symbol}
          </span>
          <span style={{ color: "var(--text, #cdd6f4)" }}>{def.label}</span>
        </div>
      ))}
    </div>
  );
}
