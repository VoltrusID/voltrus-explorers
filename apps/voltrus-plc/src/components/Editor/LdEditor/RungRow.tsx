import type { LdRung, LdCursor, LdElement, LdElementType } from "../../../types/ld";
import { isContactType, isOutputType, MIN_CONTACT_COLS } from "../../../types/ld";
import ElementCell, { CELL_W, CELL_H } from "./ElementCell";

interface Props {
  rung: LdRung;
  rungIndex: number;
  cursor: LdCursor | null;
  editing: boolean;
  onUpdateElement: (rungIdx: number, pathIdx: number, colIdx: number, updates: Partial<LdElement>) => void;
  onSelect: (cursor: LdCursor) => void;
  onStartEdit: (cursor: LdCursor) => void;
  onFinishEdit: () => void;
}

const ROW_H = CELL_H + 10;
const WIRE_H = 2;

function getNumContactCols(rung: LdRung): number {
  const maxContacts = Math.max(
    ...rung.paths.map((p) => p.elements.filter((e) => isContactType(e.type)).length),
    0,
  );
  return Math.max(MIN_CONTACT_COLS, maxContacts);
}

function getGridColumn(
  path: LdRung["paths"][number],
  numContactCols: number,
  colIdx: number,
): LdElement | null {
  const contacts = path.elements.filter((e) => isContactType(e.type));
  const output = path.elements.find((e) => isOutputType(e.type)) ?? null;

  if (colIdx < numContactCols) return contacts[colIdx] ?? null;
  if (colIdx === numContactCols) return output;
  return null;
}

function EmptyCell({
  selected,
  onClick,
}: {
  selected: boolean;
  onClick: () => void;
}) {
  return (
    <div
      onClick={onClick}
      style={{
        width: CELL_W,
        height: CELL_H,
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        border: selected ? "2px dashed var(--accent)" : "1px dashed transparent",
        borderRadius: 4,
        cursor: "pointer",
        flexShrink: 0,
        background: selected ? "rgba(22, 163, 74, 0.04)" : "transparent",
        transition: "border-color 0.15s",
      }}
    >
      {selected && (
        <span style={{ fontSize: 12, color: "var(--accent)", opacity: 0.6, fontWeight: 700 }}>
          +
        </span>
      )}
    </div>
  );
}

export default function RungRow({
  rung,
  rungIndex,
  cursor,
  editing,
  onUpdateElement,
  onSelect,
  onStartEdit,
  onFinishEdit,
}: Props) {
  const numContactCols = getNumContactCols(rung);
  const totalCols = numContactCols + 1;
  const hasMultiplePaths = rung.paths.length > 1;
  const totalH = rung.paths.length * ROW_H;

  return (
    <div style={{ position: "relative", marginBottom: 6 }}>
      {/* OR junction SVG for parallel paths */}
      {hasMultiplePaths && (
        <svg
          style={{
            position: "absolute",
            left: -8,
            top: 0,
            width: 16,
            height: totalH,
            zIndex: 3,
            pointerEvents: "none",
          }}
          viewBox={`0 0 16 ${totalH}`}
        >
          {/* Vertical bar */}
          <line
            x1="8"
            y1={ROW_H / 2}
            x2="8"
            y2={totalH - ROW_H / 2}
            stroke="var(--accent)"
            strokeWidth="2"
          />
          {/* Horizontal branches to each path */}
          {rung.paths.map((_, pi) => (
            <line
              key={pi}
              x1="8"
              y1={pi * ROW_H + ROW_H / 2}
              x2="16"
              y2={pi * ROW_H + ROW_H / 2}
              stroke="var(--text-muted)"
              strokeWidth={WIRE_H}
            />
          ))}
        </svg>
      )}

      {rung.paths.map((path, pathIdx) => {
        const isCurPath =
          cursor?.rungIndex === rungIndex && cursor?.pathIndex === pathIdx;

        return (
          <div
            key={path.id}
            style={{
              display: "flex",
              alignItems: "center",
              height: ROW_H,
              position: "relative",
            }}
          >
            {/* Continuous background wire */}
            <div
              style={{
                position: "absolute",
                top: ROW_H / 2 - WIRE_H / 2,
                left: hasMultiplePaths ? 12 : 8,
                right: 8,
                height: WIRE_H,
                background: "var(--text-muted)",
                zIndex: 0,
              }}
            />

            {/* Left wire stub from rail to grid */}
            <div
              style={{
                width: hasMultiplePaths ? 12 : 8,
                flexShrink: 0,
              }}
            />

            {/* Grid columns */}
            {Array.from({ length: totalCols }, (_, colIdx) => {
              const elem = getGridColumn(path, numContactCols, colIdx);
              const isOutputCol = colIdx === numContactCols;
              const isCur = isCurPath && cursor.colIndex === colIdx;

              return (
                <div key={colIdx} style={{ position: "relative", zIndex: 1, flexShrink: 0 }}>
                  {elem ? (
                    <ElementCell
                      element={elem}
                      selected={isCur}
                      editing={isCur && editing}
                      isOutputColumn={isOutputCol}
                      onVariableChange={(v) =>
                        onUpdateElement(rungIndex, pathIdx, colIdx, { variable: v })
                      }
                      onSelect={() =>
                        onSelect({ rungIndex, pathIndex: pathIdx, colIndex: colIdx })
                      }
                      onStartEdit={() =>
                        onStartEdit({ rungIndex, pathIndex: pathIdx, colIndex: colIdx })
                      }
                      onFinishEdit={onFinishEdit}
                    />
                  ) : (
                    <EmptyCell
                      selected={isCur}
                      onClick={() =>
                        onSelect({ rungIndex, pathIndex: pathIdx, colIndex: colIdx })
                      }
                    />
                  )}
                </div>
              );
            })}

            {/* Right wire stub to rail */}
            <div style={{ width: 8, flexShrink: 0 }} />
          </div>
        );
      })}
    </div>
  );
}
