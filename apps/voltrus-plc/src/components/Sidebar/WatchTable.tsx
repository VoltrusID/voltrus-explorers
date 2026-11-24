import { useState, useRef, useEffect } from "react";
import { Eye, RotateCcw, X } from "lucide-react";
import { useStore } from "../../store";
import { EmptyState, IconButton } from "@mactools/ui";

export default function WatchTable() {
  const { watchedVariables, removeWatch, forceVariable, unforceVariable } = useStore();
  const executionMode = useStore((s) => s.executionMode);
  const [editingIndex, setEditingIndex] = useState<number | null>(null);
  const [editValue, setEditValue] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (editingIndex !== null && inputRef.current) {
      inputRef.current.focus();
      inputRef.current.select();
    }
  }, [editingIndex]);

  const isBool = (v: typeof watchedVariables[0]) => v.type_name === "BOOL";

  const toggleBool = async (v: typeof watchedVariables[0]) => {
    const current = v.display_value === "TRUE";
    await forceVariable(v.name, current ? "FALSE" : "TRUE");
  };

  const startEdit = (index: number, currentDisplay: string) => {
    setEditingIndex(index);
    setEditValue(currentDisplay);
  };

  const submitEdit = async (name: string) => {
    if (editValue.trim()) {
      await forceVariable(name, editValue);
    }
    setEditingIndex(null);
  };

  const handleKeyDown = (e: React.KeyboardEvent, name: string) => {
    if (e.key === "Enter") {
      submitEdit(name);
    } else if (e.key === "Escape") {
      setEditingIndex(null);
    }
  };

  if (watchedVariables.length === 0) {
    return (
      <EmptyState
        compact
        icon={<Eye size={18} strokeWidth={1.75} />}
        title="Nothing to watch yet"
        description={
          executionMode === "remote"
            ? "Deploy and run the program to see its variables live."
            : "Start the runtime to watch its variables live."
        }
      />
    );
  }

  return (
    <table className="vt-table watch-table">
      <thead>
        <tr>
          <th>Variable</th>
          <th>Type</th>
          <th>Value</th>
          <th></th>
        </tr>
      </thead>
      <tbody>
        {watchedVariables.map((v, i) => (
          <tr key={v.name}>
            <td style={{ fontFamily: "var(--font-mono)", fontSize: "var(--fs-xs)", color: "var(--text)" }}>{v.name}</td>
            <td style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)" }}>{v.type_name}</td>
            <td>
              {isBool(v) ? (
                <button
                  className="vt-btn vt-btn--sm"
                  aria-pressed={v.display_value === "TRUE"}
                  onClick={() => toggleBool(v)}
                  title="Click to force the opposite value"
                  style={{ fontFamily: "var(--font-mono)", height: 20, padding: "0 8px" }}
                >
                  {v.display_value}
                </button>
              ) : editingIndex === i ? (
                <input
                  ref={inputRef}
                  className="vt-input vt-input--sm vt-input--mono"
                  type="text"
                  value={editValue}
                  onChange={(e) => setEditValue(e.target.value)}
                  onKeyDown={(e) => handleKeyDown(e, v.name)}
                  onBlur={() => submitEdit(v.name)}
                  aria-label={`Force value for ${v.name}`}
                />
              ) : (
                <span
                  className="watch-value"
                  onClick={() => startEdit(i, v.display_value)}
                  style={{
                    color:
                      v.quality === "forced"
                        ? "var(--warn)"
                        : v.quality === "bad"
                          ? "var(--danger)"
                          : "var(--accent-text)",
                  }}
                  title="Click to force value"
                >
                  {v.display_value}
                </span>
              )}
            </td>
            <td>
              <div style={{ display: "flex", gap: 2 }}>
                {v.quality === "forced" && (
                  <IconButton
                    label="Unforce"
                    title="Unforce"
                    size="sm"
                    onClick={() => unforceVariable(v.name)}
                  >
                    <RotateCcw size={12} strokeWidth={1.75} style={{ color: "var(--warn)" }} />
                  </IconButton>
                )}
                <IconButton label="Remove watch" title="Remove watch" size="sm" onClick={() => removeWatch(i)}>
                  <X size={12} strokeWidth={1.75} />
                </IconButton>
              </div>
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
