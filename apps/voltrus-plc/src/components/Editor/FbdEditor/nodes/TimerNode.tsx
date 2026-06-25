import { Handle, Position, type NodeProps, type Node } from "@xyflow/react";
import type { BlockNodeData } from "../../../../types/fbd";

export function TimerNode({ data, id }: NodeProps<Node<BlockNodeData>>) {
  const handleInstanceChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    window.dispatchEvent(new CustomEvent("fbd:update-node", {
      detail: { id, data: { ...data, instanceName: e.target.value } },
    }));
  };

  const handlePtChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    window.dispatchEvent(new CustomEvent("fbd:update-node", {
      detail: { id, data: { ...data, pinValues: { ...data.pinValues, PT: e.target.value } } },
    }));
  };

  return (
    <div style={{
      background: "var(--card, #1e1e2e)",
      border: "1px solid #f59e0b",
      borderRadius: 6,
      minWidth: 100,
      fontSize: 11,
    }}>
      <div style={{
        padding: "3px 8px",
        background: "#f59e0b",
        color: "#000",
        borderRadius: "5px 5px 0 0",
        fontWeight: 600,
        fontSize: 10,
        textAlign: "center",
      }}>
        {data.label}
      </div>
      <div style={{ padding: "3px 6px" }}>
        <input
          type="text"
          value={data.instanceName || ""}
          onChange={handleInstanceChange}
          placeholder="instance"
          style={{
            width: "100%",
            background: "transparent",
            border: "none",
            color: "var(--text, #cdd6f4)",
            fontSize: 10,
            fontFamily: '"SF Mono", monospace',
            outline: "none",
            textAlign: "center",
          }}
        />
      </div>
      <div style={{ display: "flex", justifyContent: "space-between", padding: "2px 0" }}>
        <div>
          <div style={{ position: "relative", padding: "1px 8px 1px 12px", fontSize: 10 }}>
            <Handle type="target" position={Position.Left} id="IN" style={{ width: 8, height: 8, background: "#f59e0b", top: "50%" }} />
            <span style={{ color: "var(--text-muted, #6c7086)" }}>IN</span>
          </div>
          <div style={{ position: "relative", padding: "1px 8px 1px 12px", fontSize: 10 }}>
            <Handle type="target" position={Position.Left} id="PT" style={{ width: 8, height: 8, background: "#f59e0b", top: "50%" }} />
            <input
              type="text"
              value={data.pinValues.PT || ""}
              onChange={handlePtChange}
              placeholder="T#0MS"
              style={{
                width: 50,
                background: "transparent",
                border: "none",
                color: "var(--text-muted, #6c7086)",
                fontSize: 10,
                fontFamily: '"SF Mono", monospace',
                outline: "none",
              }}
            />
          </div>
        </div>
        <div>
          <div style={{ position: "relative", padding: "1px 12px 1px 8px", fontSize: 10, textAlign: "right" }}>
            <Handle type="source" position={Position.Right} id="Q" style={{ width: 8, height: 8, background: "#f59e0b", top: "50%" }} />
            <span style={{ color: "var(--text-muted, #6c7086)" }}>Q</span>
          </div>
          <div style={{ position: "relative", padding: "1px 12px 1px 8px", fontSize: 10, textAlign: "right" }}>
            <Handle type="source" position={Position.Right} id="ET" style={{ width: 8, height: 8, background: "#f59e0b", top: "50%" }} />
            <span style={{ color: "var(--text-muted, #6c7086)" }}>ET</span>
          </div>
        </div>
      </div>
    </div>
  );
}
