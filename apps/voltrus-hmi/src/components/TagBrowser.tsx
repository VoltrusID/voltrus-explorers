import { useState, useCallback, useRef } from "react";
import { useStore } from "../store";
import type { PlcVariable } from "../types";

/** Compact value for the variable list (raw float32 values are long). */
function formatVarValue(v: unknown, type: string): string {
  if (typeof v === "number" && (type === "REAL" || !Number.isInteger(v))) {
    return Number.isFinite(v) ? v.toFixed(2) : String(v);
  }
  return String(v);
}

export function TagBrowser() {
  const plcIp = useStore((s) => s.plcIp);
  const plcConnected = useStore((s) => s.plcConnected);
  const plcConnecting = useStore((s) => s.plcConnecting);
  const plcVariables = useStore((s) => s.plcVariables);
  const setPlcIp = useStore((s) => s.setPlcIp);
  const connectPlc = useStore((s) => s.connectPlc);
  const disconnectPlc = useStore((s) => s.disconnectPlc);
  const tags = useStore((s) => s.tags);
  const addTag = useStore((s) => s.addTag);
  const removeTag = useStore((s) => s.removeTag);
  const renameTag = useStore((s) => s.renameTag);

  const [selectedVar, setSelectedVar] = useState<string | null>(null);
  const [renamingTagId, setRenamingTagId] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState("");
  const renameInputRef = useRef<HTMLInputElement>(null);

  const handleConnect = useCallback(() => {
    if (plcConnected) {
      disconnectPlc();
    } else {
      connectPlc();
    }
  }, [plcConnected, connectPlc, disconnectPlc]);

  const handleAddTag = useCallback(() => {
    if (!selectedVar) return;
    const variable = plcVariables.find((v) => v.name === selectedVar);
    if (!variable) return;

    // Auto-generate friendly name: capitalize first letter, replace underscores with spaces
    const friendlyName = variable.name
      .split("_")
      .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
      .join(" ");

    addTag({
      name: friendlyName,
      plcVariable: variable.name,
      plcIp,
      dataType: variable.type,
      access: "readwrite",
    });
    setSelectedVar(null);
  }, [selectedVar, plcVariables, plcIp, addTag]);

  const handleVarDragStart = useCallback(
    (e: React.DragEvent, variable: PlcVariable) => {
      e.dataTransfer.setData(
        "application/variable",
        JSON.stringify({ name: variable.name, type: variable.type })
      );
      e.dataTransfer.effectAllowed = "copy";
    },
    []
  );

  const handleTagDragStart = useCallback(
    (e: React.DragEvent, tag: (typeof tags)[number]) => {
      e.dataTransfer.setData("application/tag", JSON.stringify(tag));
      e.dataTransfer.effectAllowed = "copy";
    },
    []
  );

  const handleTagDoubleClick = useCallback(
    (tagId: string, currentName: string) => {
      setRenamingTagId(tagId);
      setRenameValue(currentName);
      setTimeout(() => renameInputRef.current?.select(), 0);
    },
    []
  );

  const handleRenameSubmit = useCallback(
    (tagId: string) => {
      const trimmed = renameValue.trim();
      if (trimmed) {
        renameTag(tagId, trimmed);
      }
      setRenamingTagId(null);
    },
    [renameValue, renameTag]
  );

  const handleRenameKeyDown = useCallback(
    (e: React.KeyboardEvent, tagId: string) => {
      if (e.key === "Enter") {
        handleRenameSubmit(tagId);
      } else if (e.key === "Escape") {
        setRenamingTagId(null);
      }
    },
    [handleRenameSubmit]
  );

  const handleTagContextMenu = useCallback((e: React.MouseEvent) => {
    // Suppress the browser menu; deletion happens via the × button
    // (never delete instantly on right-click — users lose tags by accident).
    e.preventDefault();
  }, []);

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        height: "100%",
        fontSize: 12,
      }}
    >
      {/* PLC Connection Section */}
      <div
        style={{
          padding: "10px 12px",
          borderBottom: "1px solid var(--border)",
        }}
      >
        <div
          style={{
            fontSize: 11,
            fontWeight: 600,
            textTransform: "uppercase",
            letterSpacing: 0.5,
            color: "var(--text-muted)",
            marginBottom: 8,
          }}
        >
          PLC Connection
        </div>
        <div style={{ display: "flex", gap: 6, marginBottom: 6 }}>
          <input
            type="text"
            value={plcIp}
            onChange={(e) => setPlcIp(e.target.value)}
            disabled={plcConnected || plcConnecting}
            placeholder="192.168.1.100"
            style={{
              flex: 1,
              minWidth: 0,
              padding: "4px 8px",
              border: "1px solid var(--border)",
              borderRadius: 4,
              background: "var(--input-bg)",
              color: "var(--text)",
              fontSize: 12,
              outline: "none",
            }}
          />
          <button
            onClick={handleConnect}
            disabled={plcConnecting}
            style={{
              padding: "4px 10px",
              border: "1px solid var(--border)",
              borderRadius: 4,
              background: plcConnected ? "var(--danger)" : "var(--accent)",
              color: plcConnected ? "#fff" : "#000",
              fontSize: 11,
              cursor: plcConnecting ? "wait" : "pointer",
              whiteSpace: "nowrap",
              flexShrink: 0,
            }}
          >
            {plcConnecting
              ? "…"
              : plcConnected
                ? "Disconnect"
                : "Connect"}
          </button>
        </div>
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: 6,
            fontSize: 11,
            fontWeight: 500,
            color: plcConnected ? "#22c55e" : plcConnecting ? "#eab308" : "#ef4444",
          }}
        >
          <span
            style={{
              width: 10,
              height: 10,
              borderRadius: "50%",
              background: plcConnected
                ? "#22c55e"
                : plcConnecting
                  ? "#eab308"
                  : "#ef4444",
              display: "inline-block",
              flexShrink: 0,
              boxShadow: plcConnected
                ? "0 0 6px #22c55e"
                : plcConnecting
                  ? "0 0 6px #eab308"
                  : "none",
            }}
          />
          <span>
            {plcConnected
              ? "Connected"
              : plcConnecting
                ? "Connecting…"
                : "Disconnected"}
          </span>
        </div>
      </div>

      {/* Variables Section */}
      <div
        style={{
          flex: 1,
          overflow: "hidden",
          display: "flex",
          flexDirection: "column",
          borderBottom: "1px solid var(--border)",
        }}
      >
        <div
          style={{
            padding: "10px 12px 6px",
            fontSize: 11,
            fontWeight: 600,
            textTransform: "uppercase",
            letterSpacing: 0.5,
            color: "var(--text-muted)",
          }}
        >
          Variables ({plcVariables.length})
        </div>
        {/* Capped so a large PLC program does not push every other sidebar
            section off-screen; the list scrolls instead. */}
        <div style={{ flex: 1, maxHeight: 260, overflowY: "auto", padding: "0 12px 8px" }}>
          {plcVariables.length === 0 ? (
            <div
              style={{
                color: "var(--text-muted)",
                fontSize: 11,
                textAlign: "center",
                padding: "12px 0",
                opacity: 0.6,
              }}
            >
              {plcConnected
                ? "Waiting for data…"
                : "Connect to a PLC to browse variables"}
            </div>
          ) : (
            plcVariables.map((v) => (
              <div
                key={v.name}
                draggable
                onDragStart={(e) => handleVarDragStart(e, v)}
                onClick={() =>
                  setSelectedVar(selectedVar === v.name ? null : v.name)
                }
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: 6,
                  padding: "4px 6px",
                  borderRadius: 4,
                  cursor: "grab",
                  marginBottom: 2,
                  background:
                    selectedVar === v.name ? "var(--hover)" : "transparent",
                  border:
                    selectedVar === v.name
                      ? "1px solid var(--accent)"
                      : "1px solid transparent",
                  transition: "background 0.1s",
                  userSelect: "none",
                }}
              >
                <span
                  style={{
                    flex: 1,
                    overflow: "hidden",
                    textOverflow: "ellipsis",
                    whiteSpace: "nowrap",
                    color: "var(--text)",
                  }}
                  title={v.name}
                >
                  {v.name}
                </span>
                <span
                  style={{
                    fontSize: 10,
                    color: "var(--text-muted)",
                    background: "var(--surface)",
                    padding: "1px 4px",
                    borderRadius: 3,
                    flexShrink: 0,
                  }}
                >
                  {v.type}
                </span>
                <span
                  style={{
                    fontSize: 10,
                    color: "var(--text-muted)",
                    flexShrink: 0,
                    minWidth: 30,
                    textAlign: "right",
                    fontVariantNumeric: "tabular-nums",
                  }}
                >
                  {formatVarValue(v.value, v.type)}
                </span>
              </div>
            ))
          )}
        </div>
        {plcConnected && plcVariables.length > 0 && (
          <div style={{ padding: "0 12px 10px" }}>
            <button
              onClick={handleAddTag}
              disabled={!selectedVar}
              style={{
                width: "100%",
                padding: "4px 10px",
                border: "1px solid var(--border)",
                borderRadius: 4,
                background: selectedVar ? "var(--accent)" : "var(--surface)",
                color: selectedVar ? "#000" : "var(--text-muted)",
                fontSize: 11,
                cursor: selectedVar ? "pointer" : "default",
                opacity: selectedVar ? 1 : 0.5,
              }}
            >
              + Add as Tag
            </button>
          </div>
        )}
      </div>

      {/* Tags Section */}
      <div
        style={{
          flex: 1,
          overflow: "hidden",
          display: "flex",
          flexDirection: "column",
        }}
      >
        <div
          style={{
            padding: "10px 12px 6px",
            fontSize: 11,
            fontWeight: 600,
            textTransform: "uppercase",
            letterSpacing: 0.5,
            color: "var(--text-muted)",
          }}
        >
          Tags ({tags.length})
        </div>
        <div style={{ flex: 1, maxHeight: 220, overflowY: "auto", padding: "0 12px 8px" }}>
          {tags.length === 0 ? (
            <div
              style={{
                color: "var(--text-muted)",
                fontSize: 11,
                textAlign: "center",
                padding: "12px 0",
                opacity: 0.6,
              }}
            >
              No tags yet. Select a variable and click "Add as Tag".
            </div>
          ) : (
            tags.map((tag) => (
              <div
                key={tag.id}
                draggable
                onDragStart={(e) => handleTagDragStart(e, tag)}
                onDoubleClick={() =>
                  handleTagDoubleClick(tag.id, tag.name)
                }
                onContextMenu={(e) => handleTagContextMenu(e)}
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: 6,
                  padding: "5px 6px",
                  borderRadius: 4,
                  cursor: "grab",
                  marginBottom: 2,
                  background: "var(--surface)",
                  border: "1px solid var(--border)",
                  userSelect: "none",
                }}
              >
                {renamingTagId === tag.id ? (
                  <input
                    ref={renameInputRef}
                    type="text"
                    value={renameValue}
                    onChange={(e) => setRenameValue(e.target.value)}
                    onBlur={() => handleRenameSubmit(tag.id)}
                    onKeyDown={(e) => handleRenameKeyDown(e, tag.id)}
                    style={{
                      flex: 1,
                      padding: "1px 4px",
                      border: "1px solid var(--accent)",
                      borderRadius: 3,
                      background: "var(--bg)",
                      color: "var(--text)",
                      fontSize: 12,
                      outline: "none",
                    }}
                  />
                ) : (
                  <>
                    <span
                      style={{
                        flex: 1,
                        overflow: "hidden",
                        textOverflow: "ellipsis",
                        whiteSpace: "nowrap",
                        color: "var(--text)",
                        fontWeight: 500,
                      }}
                      title={tag.name}
                    >
                      {tag.name}
                    </span>
                    <span
                      style={{
                        color: "var(--text-muted)",
                        fontSize: 10,
                        flexShrink: 0,
                      }}
                    >
                      &rarr;
                    </span>
                    <span
                      style={{
                        fontSize: 10,
                        color: "var(--accent)",
                        overflow: "hidden",
                        textOverflow: "ellipsis",
                        whiteSpace: "nowrap",
                        flexShrink: 0,
                        maxWidth: 80,
                      }}
                      title={tag.plcVariable}
                    >
                      {tag.plcVariable}
                    </span>
                    <button
                      onClick={(e) => {
                        e.stopPropagation();
                        removeTag(tag.id);
                      }}
                      title="Delete tag"
                      style={{
                        border: "none",
                        background: "transparent",
                        color: "var(--text-muted)",
                        cursor: "pointer",
                        padding: "0 2px",
                        fontSize: 14,
                        lineHeight: 1,
                        flexShrink: 0,
                      }}
                    >
                      &times;
                    </button>
                  </>
                )}
              </div>
            ))
          )}
        </div>
      </div>
    </div>
  );
}
