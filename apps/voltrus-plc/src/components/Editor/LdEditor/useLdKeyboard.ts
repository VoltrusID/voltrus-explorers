import { useCallback } from "react";
import type { LdProgram, LdCursor, LdElementType, LdElement, LdRung } from "../../../types/ld";
import { makeLdElement, makeLdPath, makeLdRung, isContactType, isOutputType, MIN_CONTACT_COLS } from "../../../types/ld";

interface Props {
  program: LdProgram;
  cursor: LdCursor | null;
  setCursor: (c: LdCursor | null) => void;
  editing: boolean;
  setEditing: (e: boolean) => void;
  updateProgram: (p: LdProgram) => void;
}

function getNumContactCols(rung: LdRung): number {
  return Math.max(
    MIN_CONTACT_COLS,
    ...rung.paths.map((p) => p.elements.filter((e) => isContactType(e.type)).length),
  );
}

function deepCloneRungs(rungs: LdProgram["rungs"]) {
  return rungs.map((r) => ({
    ...r,
    paths: r.paths.map((p) => ({ ...p, elements: [...p.elements] })),
  }));
}

export function useLdKeyboard({ program, cursor, setCursor, editing, setEditing, updateProgram }: Props) {
  return useCallback((e: React.KeyboardEvent) => {
    if (editing) return;

    const { rungs } = program;

    // Ctrl/Cmd + Enter: add new rung
    if (e.key === "Enter" && (e.ctrlKey || e.metaKey) && !e.shiftKey) {
      e.preventDefault();
      addRung();
      return;
    }
    // Ctrl/Cmd + Shift + Enter: add parallel path
    if (e.key === "Enter" && (e.ctrlKey || e.metaKey) && e.shiftKey) {
      e.preventDefault();
      addParallelPath();
      return;
    }

    switch (e.key) {
      case "ArrowLeft": {
        e.preventDefault();
        if (!cursor) { selectFirst(); break; }
        if (cursor.colIndex > 0) {
          setCursor({ ...cursor, colIndex: cursor.colIndex - 1 });
        }
        break;
      }
      case "ArrowRight": {
        e.preventDefault();
        if (!cursor) { selectFirst(); break; }
        const numCols = getNumContactCols(rungs[cursor.rungIndex]);
        if (cursor.colIndex < numCols) {
          setCursor({ ...cursor, colIndex: cursor.colIndex + 1 });
        }
        break;
      }
      case "ArrowUp": {
        e.preventDefault();
        if (!cursor) { selectFirst(); break; }
        if (cursor.pathIndex > 0) {
          setCursor({ ...cursor, pathIndex: cursor.pathIndex - 1 });
        } else if (cursor.rungIndex > 0) {
          const prevRung = rungs[cursor.rungIndex - 1];
          const maxCol = getNumContactCols(prevRung);
          setCursor({
            rungIndex: cursor.rungIndex - 1,
            pathIndex: prevRung.paths.length - 1,
            colIndex: Math.min(cursor.colIndex, maxCol),
          });
        }
        break;
      }
      case "ArrowDown": {
        e.preventDefault();
        if (!cursor) { selectFirst(); break; }
        const rung = rungs[cursor.rungIndex];
        if (cursor.pathIndex < rung.paths.length - 1) {
          setCursor({ ...cursor, pathIndex: cursor.pathIndex + 1 });
        } else if (cursor.rungIndex < rungs.length - 1) {
          const nextRung = rungs[cursor.rungIndex + 1];
          const maxCol = getNumContactCols(nextRung);
          setCursor({
            rungIndex: cursor.rungIndex + 1,
            pathIndex: 0,
            colIndex: Math.min(cursor.colIndex, maxCol),
          });
        }
        break;
      }
      case "Enter": {
        e.preventDefault();
        if (cursor) setEditing(true);
        break;
      }
      case "Escape": {
        e.preventDefault();
        setCursor(null);
        break;
      }
      case "Delete":
      case "Backspace": {
        e.preventDefault();
        if (cursor) deleteAtCursor();
        break;
      }
      case "F5": { e.preventDefault(); insertElement("contact_no"); break; }
      case "F6": { e.preventDefault(); insertElement("contact_nc"); break; }
      case "F7": { e.preventDefault(); insertElement("contact_re"); break; }
      case "F8": { e.preventDefault(); insertElement("coil"); break; }
      case "F9": { e.preventDefault(); insertElement("coil_set"); break; }
      case "F10": { e.preventDefault(); insertElement("coil_reset"); break; }
      case "F11": { e.preventDefault(); insertElement("fb_block"); break; }
    }
  }, [program, cursor, editing, setCursor, setEditing, updateProgram]);

  function selectFirst() {
    if (program.rungs.length > 0) {
      setCursor({ rungIndex: 0, pathIndex: 0, colIndex: 0 });
    }
  }

  function deleteAtCursor() {
    if (!cursor) return;
    const rungs = deepCloneRungs(program.rungs);
    const rung = rungs[cursor.rungIndex];
    if (!rung) return;
    const path = rung.paths[cursor.pathIndex];
    if (!path) return;

    const numContactCols = getNumContactCols(rung);
    const contacts = path.elements.filter((e) => isContactType(e.type));
    const output = path.elements.find((e) => isOutputType(e.type)) ?? null;

    if (cursor.colIndex === numContactCols && output) {
      // Delete output
      path.elements = [...contacts];
    } else if (cursor.colIndex < contacts.length) {
      // Delete a contact
      contacts.splice(cursor.colIndex, 1);
      path.elements = output ? [...contacts, output] : [...contacts];
    } else {
      // Empty column — nothing to delete
      return;
    }

    // If path is now empty, remove it
    if (path.elements.length === 0) {
      rung.paths.splice(cursor.pathIndex, 1);
      if (rung.paths.length === 0) {
        rungs.splice(cursor.rungIndex, 1);
        updateProgram({ ...program, rungs });
        if (rungs.length > 0) {
          const ri = Math.min(cursor.rungIndex, rungs.length - 1);
          setCursor({ rungIndex: ri, pathIndex: 0, colIndex: 0 });
        } else {
          setCursor(null);
        }
        return;
      }
    }

    updateProgram({ ...program, rungs });
  }

  function insertElement(type: LdElementType) {
    const el = makeLdElement(type, type === "fb_block" ? { blockType: "TON" } : {});

    if (!cursor || program.rungs.length === 0) {
      const newPath = makeLdPath([el]);
      const newRung = makeLdRung([newPath]);
      const newRungs = [...program.rungs, newRung];
      updateProgram({ ...program, rungs: newRungs });
      const ri = newRungs.length - 1;
      const col = isOutputType(type) ? getNumContactCols(newRungs[ri]) : 0;
      setCursor({ rungIndex: ri, pathIndex: 0, colIndex: col });
      return;
    }

    const rungs = deepCloneRungs(program.rungs);
    const path = rungs[cursor.rungIndex].paths[cursor.pathIndex];
    if (!path) return;

    const contacts = path.elements.filter((e) => isContactType(e.type));
    const output = path.elements.find((e) => isOutputType(e.type)) ?? null;

    if (isContactType(type)) {
      const insertAt = Math.min(cursor.colIndex, contacts.length);
      contacts.splice(insertAt, 0, el);
      path.elements = output ? [...contacts, output] : [...contacts];
      setCursor({ ...cursor, colIndex: insertAt });
    } else {
      path.elements = [...contacts, el];
      setCursor({ ...cursor, colIndex: getNumContactCols(rungs[cursor.rungIndex]) });
    }

    updateProgram({ ...program, rungs });
  }

  function addRung() {
    const newRung = makeLdRung([makeLdPath([])]);
    const newRungs = [...program.rungs];
    const insertAt = cursor ? cursor.rungIndex + 1 : newRungs.length;
    newRungs.splice(insertAt, 0, newRung);
    updateProgram({ ...program, rungs: newRungs });
    setCursor({ rungIndex: insertAt, pathIndex: 0, colIndex: 0 });
  }

  function addParallelPath() {
    if (!cursor) return;
    const rungs = deepCloneRungs(program.rungs);
    const rung = rungs[cursor.rungIndex];
    const newPath = makeLdPath([]);
    rung.paths.splice(cursor.pathIndex + 1, 0, newPath);
    updateProgram({ ...program, rungs });
    setCursor({ ...cursor, pathIndex: cursor.pathIndex + 1, colIndex: 0 });
  }
}
