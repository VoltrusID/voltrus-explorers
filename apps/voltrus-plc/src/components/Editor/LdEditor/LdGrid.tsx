import { EmptyState } from "@mactools/ui";
import type { LdProgram, LdCursor, LdElement } from "../../../types/ld";
import RungRow from "./RungRow";

interface Props {
  program: LdProgram;
  cursor: LdCursor | null;
  editing: boolean;
  onKeyDown: (e: React.KeyboardEvent) => void;
  onUpdateElement: (rungIdx: number, pathIdx: number, colIdx: number, updates: Partial<LdElement>) => void;
  onSelectCursor: (cursor: LdCursor) => void;
  onStartEdit: (cursor: LdCursor) => void;
  onFinishEdit: () => void;
}

const RAIL_W = 24;
const RAIL_BAR = 4;

export default function LdGrid({ program, cursor, editing, onKeyDown, onUpdateElement, onSelectCursor, onStartEdit, onFinishEdit }: Props) {
  return (
    <div style={{ display: "flex", flex: 1, overflow: "hidden", position: "relative" }}>
      {/* Left power rail */}
      <div style={{ width: RAIL_W, flexShrink: 0, position: "relative" }}>
        <div style={{
          position: "absolute",
          left: (RAIL_W - RAIL_BAR) / 2,
          top: 0,
          bottom: 0,
          width: RAIL_BAR,
          background: "var(--accent)",
          borderRadius: 2,
        }} />
      </div>

      {/* Scrollable content */}
      <div
        tabIndex={0}
        onKeyDown={onKeyDown}
        style={{
          flex: 1,
          overflow: "auto",
          outline: "none",
          background: "var(--bg)",
          padding: "8px 0",
          minWidth: 0,
          display: "flex",
          flexDirection: "column",
        }}
      >
        {program.rungs.length === 0 ? (
          <div className="ed-ld-empty">
            <EmptyState
              compact
              title="Empty program"
              description="Drag an element from the palette, press F5 to add a contact, or Ctrl+Enter to add a rung."
            />
          </div>
        ) : (
          program.rungs.map((rung, ri) => (
            <RungRow
              key={rung.id}
              rung={rung}
              rungIndex={ri}
              cursor={cursor}
              editing={editing}
              onUpdateElement={onUpdateElement}
              onSelect={onSelectCursor}
              onStartEdit={onStartEdit}
              onFinishEdit={onFinishEdit}
            />
          ))
        )}
      </div>

      {/* Right power rail */}
      <div style={{ width: RAIL_W, flexShrink: 0, position: "relative" }}>
        <div style={{
          position: "absolute",
          right: (RAIL_W - RAIL_BAR) / 2,
          top: 0,
          bottom: 0,
          width: RAIL_BAR,
          background: "var(--border-light)",
          borderRadius: 2,
        }} />
      </div>
    </div>
  );
}
