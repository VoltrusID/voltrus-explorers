import { useState } from "react";
import { SFC_ELEMENT_DEFS, type SfcElementCategory } from "../../../../types/sfc";

const CATEGORIES: Array<{ id: SfcElementCategory | "all"; label: string }> = [
  { id: "all", label: "All" },
  { id: "step", label: "Steps" },
  { id: "transition", label: "Transitions" },
  { id: "branch", label: "Branches" },
  { id: "jump", label: "Jumps" },
];

// Line drawings of each SFC element, replacing the old text glyphs.
function SfcGlyph({ type }: { type: string }) {
  const c = "currentColor";
  if (type === "step") {
    return (
      <svg viewBox="0 0 30 20">
        <rect x="8" y="3" width="14" height="14" rx="2" fill="none" stroke={c} strokeWidth="1.75" />
      </svg>
    );
  }
  if (type === "initial_step") {
    return (
      <svg viewBox="0 0 30 20">
        <rect x="7" y="2.5" width="16" height="15" rx="2" fill="none" stroke={c} strokeWidth="1.75" />
        <rect x="10" y="5.5" width="10" height="9" rx="1" fill="none" stroke={c} strokeWidth="1.25" />
      </svg>
    );
  }
  if (type === "transition") {
    return (
      <svg viewBox="0 0 30 20">
        <line x1="2" y1="10" x2="28" y2="10" stroke={c} strokeWidth="2" />
      </svg>
    );
  }
  if (type === "or_diverge") {
    return (
      <svg viewBox="0 0 30 20">
        <line x1="15" y1="1" x2="15" y2="7" stroke={c} strokeWidth="1.75" />
        <line x1="15" y1="7" x2="6" y2="13" stroke={c} strokeWidth="1.75" />
        <line x1="15" y1="7" x2="24" y2="13" stroke={c} strokeWidth="1.75" />
        <line x1="6" y1="13" x2="6" y2="19" stroke={c} strokeWidth="1.75" />
        <line x1="24" y1="13" x2="24" y2="19" stroke={c} strokeWidth="1.75" />
      </svg>
    );
  }
  if (type === "or_converge") {
    return (
      <svg viewBox="0 0 30 20">
        <line x1="6" y1="1" x2="6" y2="7" stroke={c} strokeWidth="1.75" />
        <line x1="24" y1="1" x2="24" y2="7" stroke={c} strokeWidth="1.75" />
        <line x1="6" y1="7" x2="15" y2="13" stroke={c} strokeWidth="1.75" />
        <line x1="24" y1="7" x2="15" y2="13" stroke={c} strokeWidth="1.75" />
        <line x1="15" y1="13" x2="15" y2="19" stroke={c} strokeWidth="1.75" />
      </svg>
    );
  }
  if (type === "jump") {
    return (
      <svg viewBox="0 0 30 20">
        <path d="M5 10 H22 M18 5.5 L22.5 10 L18 14.5" fill="none" stroke={c} strokeWidth="1.75" />
      </svg>
    );
  }
  return null;
}

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
    <div className="ed-palette">
      <div className="ed-palette__title">SFC elements</div>

      <div className="ed-chips">
        {CATEGORIES.map((cat) => (
          <button
            key={cat.id}
            type="button"
            className="ed-chip"
            aria-pressed={filter === cat.id}
            onClick={() => setFilter(cat.id)}
          >
            {cat.label}
          </button>
        ))}
      </div>

      <div className="ed-palette__list">
        {elements.map((def) => (
          <div
            key={def.type}
            className="ed-tile"
            draggable
            onDragStart={(e) => onDragStart(e, def)}
          >
            <span className="ed-tile__sym">
              <SfcGlyph type={def.type} />
            </span>
            <span>{def.label}</span>
          </div>
        ))}
      </div>
    </div>
  );
}
