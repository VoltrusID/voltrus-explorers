import { useState } from "react";
import { SFC_ELEMENT_DEFS, type SfcElementCategory } from "../../../../types/sfc";

const CATEGORIES: Array<{ id: SfcElementCategory | "all"; label: string }> = [
  { id: "all", label: "All" },
  { id: "step", label: "Steps" },
  { id: "transition", label: "Trans." },
  { id: "branch", label: "Branch" },
  { id: "jump", label: "Jumps" },
];

const CATEGORY_COLORS: Record<string, string> = {
  step: "#10b981",
  transition: "#f59e0b",
  branch: "#a855f7",
  jump: "#6c7086",
};

export default function SfcPalette() {
  const [filter, setFilter] = useState<SfcElementCategory | "all">("all");

  const elements = filter === "all"
    ? SFC_ELEMENT_DEFS
    : SFC_ELEMENT_DEFS.filter((d) => d.category === filter);

  const onDragStart = (event: React.DragEvent, def: typeof SFC_ELEMENT_DEFS[number]) => {
    event.dataTransfer.setData("application/sfc-element", JSON.stringify({
      elementType: def.type,
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
        SFC
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

      {elements.map((def) => (
        <div
          key={def.type}
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
