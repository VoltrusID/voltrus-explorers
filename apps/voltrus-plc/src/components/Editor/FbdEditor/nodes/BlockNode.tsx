import { Handle, Position, type NodeProps, type Node } from "@xyflow/react";
import type { BlockNodeData } from "../../../../types/fbd";
import { getBlockDef } from "../../../../types/fbd";

function updateNodeData(id: string, patch: Partial<BlockNodeData>) {
  window.dispatchEvent(new CustomEvent("fbd:update-node", { detail: { id, data: patch } }));
}

export function BlockNode({ id, data }: NodeProps<Node<BlockNodeData>>) {
  const def = getBlockDef(data.blockType);
  if (!def) return <div style={{ color: "red" }}>Unknown: {data.blockType}</div>;

  const inputPins = def.pins.filter((p) => p.direction === "input");
  const outputPins = def.pins.filter((p) => p.direction === "output");

  return (
    <div style={{
      background: "var(--card, #1e1e2e)",
      border: "1px solid var(--border, #45475a)",
      borderRadius: 6,
      minWidth: 80,
      fontSize: 11,
      fontFamily: "system-ui, sans-serif",
    }}>
      <div style={{
        padding: "3px 8px",
        background: "var(--accent, #7c3aed)",
        color: "#fff",
        borderRadius: "5px 5px 0 0",
        fontWeight: 600,
        fontSize: 10,
        textAlign: "center",
      }}>
        {data.label}
      </div>
      {def.isFunctionBlock && (
        <div style={{ padding: "2px 6px", borderTop: "1px solid var(--border, #45475a)" }}>
          <input
            value={data.instanceName ?? ""}
            onChange={(e) => updateNodeData(id, { instanceName: e.target.value })}
            placeholder="instance name"
            style={{
              width: "100%",
              background: "var(--bg, #11111b)",
              border: "1px solid var(--border, #45475a)",
              borderRadius: 3,
              color: "var(--text, #cdd6f4)",
              fontSize: 9,
              padding: "1px 4px",
              outline: "none",
              boxSizing: "border-box",
            }}
          />
        </div>
      )}
      <div style={{ display: "flex", justifyContent: "space-between", padding: "4px 0" }}>
        <div>
          {inputPins.map((pin, i) => (
            <div key={pin.name} style={{ position: "relative", padding: "2px 8px 2px 12px", fontSize: 10 }}>
              <Handle
                type="target"
                position={Position.Left}
                id={pin.name}
                style={{
                  width: 8, height: 8,
                  background: "var(--text-muted, #6c7086)",
                  border: "1px solid var(--border, #45475a)",
                  top: "50%",
                }}
              />
              <span style={{ color: "var(--text-muted, #6c7086)" }}>{pin.name}</span>
            </div>
          ))}
        </div>
        <div>
          {outputPins.map((pin, i) => (
            <div key={pin.name} style={{ position: "relative", padding: "2px 12px 2px 8px", fontSize: 10, textAlign: "right" }}>
              <Handle
                type="source"
                position={Position.Right}
                id={pin.name}
                style={{
                  width: 8, height: 8,
                  background: "var(--text-muted, #6c7086)",
                  border: "1px solid var(--border, #45475a)",
                  top: "50%",
                }}
              />
              <span style={{ color: "var(--text-muted, #6c7086)" }}>{pin.name}</span>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
