import { useState, useCallback, useRef, useMemo, useEffect } from "react";
import type { LdProgram, LdCursor, LdElement, LdElementType } from "../../../types/ld";
import { migrateLdProgram } from "./migrate";
import { serializeLd } from "../../../types/canonical";
import { isContactType, isOutputType, MIN_CONTACT_COLS, makeLdElement, makeLdPath, makeLdRung } from "../../../types/ld";
import LdGrid from "./LdGrid";
import LdPalette from "./palette/LdPalette";
import { useLdKeyboard } from "./useLdKeyboard";

interface Props {
  value: string;
  onChange: (value: string) => void;
}

function getNumContactColsForRung(rung: LdProgram["rungs"][number]): number {
  return Math.max(
    MIN_CONTACT_COLS,
    ...rung.paths.map((p) => p.elements.filter((e) => isContactType(e.type)).length),
  );
}

export default function GridLdEditor({ value, onChange }: Props) {
  const [program, setProgram] = useState<LdProgram>(() => {
    try {
      return migrateLdProgram(JSON.parse(value));
    } catch {
      return { version: "2.0", programName: "NewLD", rungs: [] };
    }
  });
  const [cursor, setCursor] = useState<LdCursor | null>(null);
  const [editing, setEditing] = useState(false);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const programRef = useRef(program);
  programRef.current = program;

  const parsedVersion = useMemo(() => {
    try {
      return migrateLdProgram(JSON.parse(value));
    } catch {
      return null;
    }
  }, [value]);

  useEffect(() => {
    if (parsedVersion && parsedVersion !== programRef.current) {
      setProgram(parsedVersion);
    }
  }, [parsedVersion]);

  const scheduleSerialize = useCallback((p: LdProgram) => {
    if (timerRef.current) clearTimeout(timerRef.current);
    timerRef.current = setTimeout(() => {
      onChange(serializeLd(p));
    }, 200);
  }, [onChange]);

  const updateProgram = useCallback((p: LdProgram) => {
    setProgram(p);
    scheduleSerialize(p);
  }, [scheduleSerialize]);

  const handleUpdateElement = useCallback((rungIdx: number, pathIdx: number, colIdx: number, updates: Partial<LdElement>) => {
    const newRungs = program.rungs.map((r, ri) => {
      if (ri !== rungIdx) return r;
      const numContactCols = getNumContactColsForRung(r);
      return {
        ...r,
        paths: r.paths.map((p, pi) => {
          if (pi !== pathIdx) return p;

          const contacts = p.elements.filter((e) => isContactType(e.type));
          const output = p.elements.find((e) => isOutputType(e.type));
          let targetIdx = -1;

          if (colIdx < numContactCols && colIdx < contacts.length) {
            targetIdx = p.elements.indexOf(contacts[colIdx]);
          } else if (colIdx === numContactCols && output) {
            targetIdx = p.elements.indexOf(output);
          }

          if (targetIdx < 0) return p;
          return {
            ...p,
            elements: p.elements.map((el, ei) =>
              ei !== targetIdx ? el : { ...el, ...updates }
            ),
          };
        }),
      };
    });
    updateProgram({ ...program, rungs: newRungs });
  }, [program, updateProgram]);

  const handleStartEdit = useCallback((c: LdCursor) => {
    setCursor(c);
    setEditing(true);
  }, []);

  const handleFinishEdit = useCallback(() => {
    setEditing(false);
  }, []);

  const handleKeyDown = useLdKeyboard({
    program,
    cursor,
    setCursor,
    editing,
    setEditing,
    updateProgram,
  });

  const handleDrop = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    const raw = e.dataTransfer.getData("application/ld-element");
    if (!raw) return;
    try {
      const { elementType, blockType } = JSON.parse(raw);
      const type = elementType as LdElementType;
      const el = makeLdElement(type, { blockType, type });

      if (!cursor || program.rungs.length === 0) {
        const newPath = makeLdPath([el]);
        const newRung = makeLdRung([newPath]);
        const newRungs = [...program.rungs, newRung];
        updateProgram({ ...program, rungs: newRungs });
        setCursor({ rungIndex: newRungs.length - 1, pathIndex: 0, colIndex: 0 });
        return;
      }

      const rungs = program.rungs.map((r) => ({
        ...r,
        paths: r.paths.map((p) => ({ ...p, elements: [...p.elements] })),
      }));
      const path = rungs[cursor.rungIndex].paths[cursor.pathIndex];
      if (!path) return;

      const contacts = path.elements.filter((e) => isContactType(e.type));
      const output = path.elements.find((e) => isOutputType(e.type)) ?? null;

      if (isContactType(type)) {
        const insertAt = Math.min(cursor.colIndex, contacts.length);
        contacts.splice(insertAt, 0, el);
        path.elements = output ? [...contacts, output] : [...contacts];
      } else {
        path.elements = [...contacts, el];
      }

      updateProgram({ ...program, rungs });
    } catch { /* ignore invalid drag data */ }
  }, [cursor, program, updateProgram]);

  const handleDragOver = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.dataTransfer.dropEffect = "move";
  }, []);

  return (
    <div style={{ display: "flex", width: "100%", height: "100%" }}>
      <LdPalette />
      <div
        style={{ flex: 1, display: "flex", flexDirection: "column", overflow: "hidden" }}
        onDrop={handleDrop}
        onDragOver={handleDragOver}
      >
        {/* Toolbar */}
        <div style={{
          display: "flex",
          alignItems: "center",
          gap: 8,
          padding: "4px 12px",
          background: "var(--card)",
          borderBottom: "1px solid var(--border)",
          fontSize: 11,
          color: "var(--text-muted)",
          flexShrink: 0,
        }}>
          <span style={{ fontWeight: 600, color: "var(--text)" }}>{program.programName}</span>
          <span>|</span>
          <span>{program.rungs.length} rung{program.rungs.length !== 1 ? "s" : ""}</span>
          {cursor && (
            <>
              <span>|</span>
              <span>R{cursor.rungIndex + 1}:P{cursor.pathIndex + 1}:C{cursor.colIndex + 1}</span>
            </>
          )}
          <div style={{ flex: 1 }} />
          <span style={{ fontSize: 10, opacity: 0.6 }}>F5-NO F6-NC F7-RE F8-Coil F9-Set F10-Reset F11-FB</span>
        </div>
        <LdGrid
          program={program}
          cursor={cursor}
          editing={editing}
          onKeyDown={handleKeyDown}
          onUpdateElement={handleUpdateElement}
          onSelectCursor={setCursor}
          onStartEdit={handleStartEdit}
          onFinishEdit={handleFinishEdit}
        />
      </div>
    </div>
  );
}
