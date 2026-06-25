import { useState, useRef, useEffect } from "react";
import { useStore } from "../../store";

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

  return (
    <table className="watch-table">
      <thead>
        <tr>
          <th>Variable</th>
          <th>Type</th>
          <th>Value</th>
          <th></th>
        </tr>
      </thead>
      <tbody>
        {watchedVariables.length === 0 ? (
          <tr>
            <td colSpan={4} style={{ color: "var(--text-muted)", fontSize: 11 }}>
              {executionMode === "remote"
                ? "Deploy & run program to see live variables"
                : "Start runtime to watch variables"}
            </td>
          </tr>
        ) : (
          watchedVariables.map((v, i) => (
            <tr key={v.name}>
              <td style={{ color: "var(--text)", fontSize: 12 }}>{v.name}</td>
              <td style={{ color: "var(--text-muted)", fontSize: 11 }}>{v.type_name}</td>
              <td>
                {isBool(v) ? (
                  <button
                    className="watch-toggle"
                    onClick={() => toggleBool(v)}
                    style={{
                      background: v.display_value === "TRUE" ? "var(--accent)" : "var(--surface)",
                      border: `1px solid ${v.display_value === "TRUE" ? "var(--accent)" : "var(--border)"}`,
                      color: v.display_value === "TRUE" ? "#fff" : "var(--text-muted)",
                      borderRadius: 3,
                      padding: "1px 8px",
                      fontSize: 11,
                      cursor: "pointer",
                      fontFamily: '"SF Mono", monospace',
                    }}
                  >
                    {v.display_value}
                  </button>
                ) : editingIndex === i ? (
                  <input
                    ref={inputRef}
                    className="watch-edit-input"
                    type="text"
                    value={editValue}
                    onChange={(e) => setEditValue(e.target.value)}
                    onKeyDown={(e) => handleKeyDown(e, v.name)}
                    onBlur={() => submitEdit(v.name)}
                  />
                ) : (
                  <span
                    className="watch-value"
                    onClick={() => startEdit(i, v.display_value)}
                    style={{
                      color:
                        v.quality === "forced"
                          ? "#f59e0b"
                          : v.quality === "bad"
                            ? "var(--danger)"
                            : "var(--accent)",
                      fontFamily: '"SF Mono", monospace',
                      fontSize: 12,
                      cursor: "pointer",
                      padding: "1px 4px",
                      borderRadius: 3,
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
                    <button
                      onClick={() => unforceVariable(v.name)}
                      title="Unforce"
                      style={{
                        background: "none",
                        border: "none",
                        color: "#f59e0b",
                        cursor: "pointer",
                        fontSize: 10,
                        padding: "0 3px",
                      }}
                    >
                      &#x21BA;
                    </button>
                  )}
                  <button
                    onClick={() => removeWatch(i)}
                    style={{
                      background: "none",
                      border: "none",
                      color: "var(--text-muted)",
                      cursor: "pointer",
                      fontSize: 12,
                      padding: "0 4px",
                    }}
                  >
                    x
                  </button>
                </div>
              </td>
            </tr>
          ))
        )}
      </tbody>
    </table>
  );
}
