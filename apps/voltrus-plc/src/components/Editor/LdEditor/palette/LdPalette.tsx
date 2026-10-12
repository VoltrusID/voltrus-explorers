import { useState } from "react";
import { LD_ELEMENT_DEFS, type LdElementCategory } from "../../../../types/ld";
import { SvgSymbol } from "../ElementCell";

const CATEGORIES: Array<{ id: LdElementCategory | "all"; label: string }> = [
  { id: "all", label: "All" },
  { id: "contact", label: "Contacts" },
  { id: "coil", label: "Coils" },
  { id: "timer", label: "Timers" },
  { id: "counter", label: "Counters" },
  { id: "function", label: "Functions" },
];

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
    <div className="ed-palette">
      <div className="ed-palette__title">Ladder elements</div>

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
        {elements.map((def, i) => (
          <div
            key={`${def.type}-${def.blockType ?? i}`}
            className="ed-tile"
            draggable
            onDragStart={(e) => onDragStart(e, def)}
          >
            <span className="ed-tile__sym">
              <SvgSymbol type={def.type} blockType={def.blockType} />
            </span>
            <span>{def.label}</span>
          </div>
        ))}
      </div>
    </div>
  );
}
