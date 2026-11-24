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

export default function BlockPalette() {
  const onDragStart = (event: React.DragEvent, blockType: string) => {
    event.dataTransfer.setData("application/fbd-block", blockType);
    event.dataTransfer.effectAllowed = "move";
  };

  return (
    <div className="ed-palette">
      {CATEGORIES.map((cat) => {
        const blocks = BLOCK_LIBRARY.filter((b) =>
          cat.id === "io" ? (b.category === "input" || b.category === "output") : b.category === cat.id,
        );
        if (blocks.length === 0) return null;
        return (
          <div key={cat.id} className="ed-palette__group">
            <div className="ed-palette__group-title">{cat.label}</div>
            <div className="ed-palette__items">
              {blocks.map((b) => (
                <div
                  key={b.type}
                  className="ed-tile"
                  draggable
                  onDragStart={(e) => onDragStart(e, b.type)}
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
