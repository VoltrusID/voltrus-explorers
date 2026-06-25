import { memo, type ChangeEvent } from "react";
import { Handle, Position, type NodeProps, type Node } from "@xyflow/react";
import type { SfcNodeData } from "../../../../types/sfc";

function SfcTransitionNode({ id, data }: NodeProps<Node<SfcNodeData>>) {
  function updateData(partial: Partial<SfcNodeData>) {
    window.dispatchEvent(new CustomEvent("sfc:update-node", { detail: { id, data: partial } }));
  }

  function handleConditionChange(e: ChangeEvent<HTMLInputElement>) {
    updateData({ condition: e.target.value });
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", alignItems: "center" }}>
      <Handle type="target" position={Position.Top} id="T" style={{ background: "#f59e0b", width: 8, height: 8 }} />

      <div style={{
        width: 180, height: 3,
        background: "#f59e0b",
        borderRadius: 1,
      }} />

      <input
        value={data.condition ?? ""}
        onChange={handleConditionChange}
        placeholder="Condition (ST expression)"
        style={{
          width: 180, border: "1px solid #f59e0b40",
          borderRadius: 3, padding: "2px 6px", marginTop: 2,
          background: "var(--card, #1e1e2e)",
          color: "var(--text)", fontSize: 10,
          textAlign: "center", outline: "none",
        }}
      />

      <Handle type="source" position={Position.Bottom} id="B" style={{ background: "#f59e0b", width: 8, height: 8 }} />
    </div>
  );
}

export default memo(SfcTransitionNode);
