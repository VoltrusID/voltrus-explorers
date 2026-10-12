import { useState, useCallback, useRef } from "react";
import { ArrowRight, Plus, Plug, Unplug, X } from "lucide-react";
import { Button, IconButton, StatusDot } from "@mactools/ui";
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
    // Suppress the browser menu; deletion happens via the delete button
    // (never delete instantly on right-click — users lose tags by accident).
    e.preventDefault();
  }, []);

  return (
    <div className="acc-body">
      {/* PLC connection */}
      <div className="sub-block">
        <div className="sub-title">PLC connection</div>
        <div className="sub-actions">
          <input
            type="text"
            value={plcIp}
            onChange={(e) => setPlcIp(e.target.value)}
            disabled={plcConnected || plcConnecting}
            placeholder="192.168.1.100"
            aria-label="PLC IP address"
            className="vt-input vt-input--mono"
            style={{ flex: 1, minWidth: 0 }}
          />
          <Button
            size="sm"
            variant={plcConnected ? "danger" : "primary"}
            icon={plcConnected ? <Unplug size={14} strokeWidth={1.75} /> : <Plug size={14} strokeWidth={1.75} />}
            onClick={handleConnect}
            disabled={plcConnecting}
            style={{ flexShrink: 0 }}
          >
            {plcConnecting ? "…" : plcConnected ? "Disconnect" : "Connect"}
          </Button>
        </div>
        <div className="status-line">
          <StatusDot
            tone={plcConnected ? "success" : plcConnecting ? "warn" : "danger"}
            label={plcConnected ? "Connected" : plcConnecting ? "Connecting" : "Disconnected"}
          />
          <span>
            {plcConnected ? "Connected" : plcConnecting ? "Connecting…" : "Disconnected"}
          </span>
        </div>
      </div>

      {/* Variables */}
      <div className="sub-block" style={{ flex: 1, minHeight: 0, overflow: "hidden" }}>
        <div className="sub-title">
          <span>Variables</span>
          <span className="row-meta">{plcVariables.length}</span>
        </div>
        {/* Capped so a large PLC program does not push every other sidebar
            section off-screen; the list scrolls instead. */}
        <div className="sub-list" style={{ maxHeight: 260 }}>
          {plcVariables.length === 0 ? (
            <div className="sub-empty">
              {plcConnected ? "Waiting for data…" : "Connect to a PLC to browse variables."}
            </div>
          ) : (
            plcVariables.map((v) => (
              <div
                key={v.name}
                draggable
                onDragStart={(e) => handleVarDragStart(e, v)}
                onClick={() => setSelectedVar(selectedVar === v.name ? null : v.name)}
                className={`list-row${selectedVar === v.name ? " is-selected" : ""}`}
              >
                <span className="row-main" title={v.name}>
                  {v.name}
                </span>
                <span className="vt-badge vt-badge--mono">{v.type}</span>
                <span className="row-meta" style={{ minWidth: 30, textAlign: "right" }}>
                  {formatVarValue(v.value, v.type)}
                </span>
              </div>
            ))
          )}
        </div>
        {plcConnected && plcVariables.length > 0 && (
          <div style={{ padding: "0 10px 2px" }}>
            <Button
              size="sm"
              variant="primary"
              block
              icon={<Plus size={14} strokeWidth={1.75} />}
              onClick={handleAddTag}
              disabled={!selectedVar}
            >
              Add as tag
            </Button>
          </div>
        )}
      </div>

      {/* Tags */}
      <div className="sub-block" style={{ flex: 1, minHeight: 0, overflow: "hidden" }}>
        <div className="sub-title">
          <span>Tags</span>
          <span className="row-meta">{tags.length}</span>
        </div>
        <div className="sub-list" style={{ maxHeight: 220 }}>
          {tags.length === 0 ? (
            <div className="sub-empty">
              No tags yet. Select a variable and click "Add as tag".
            </div>
          ) : (
            tags.map((tag) => (
              <div
                key={tag.id}
                draggable
                onDragStart={(e) => handleTagDragStart(e, tag)}
                onDoubleClick={() => handleTagDoubleClick(tag.id, tag.name)}
                onContextMenu={(e) => handleTagContextMenu(e)}
                className="list-row is-card"
              >
                {renamingTagId === tag.id ? (
                  <input
                    ref={renameInputRef}
                    type="text"
                    value={renameValue}
                    onChange={(e) => setRenameValue(e.target.value)}
                    onBlur={() => handleRenameSubmit(tag.id)}
                    onKeyDown={(e) => handleRenameKeyDown(e, tag.id)}
                    className="row-name-input vt-input"
                    aria-label="Tag name"
                  />
                ) : (
                  <>
                    <span className="row-main" style={{ fontWeight: 500 }} title={tag.name}>
                      {tag.name}
                    </span>
                    <span className="row-arrow" aria-hidden="true">
                      <ArrowRight size={12} strokeWidth={1.75} />
                    </span>
                    <span className="row-target" title={tag.plcVariable}>
                      {tag.plcVariable}
                    </span>
                    <IconButton
                      label="Delete tag"
                      size="sm"
                      danger
                      onClick={(e) => {
                        e.stopPropagation();
                        removeTag(tag.id);
                      }}
                    >
                      <X size={12} strokeWidth={1.75} />
                    </IconButton>
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
