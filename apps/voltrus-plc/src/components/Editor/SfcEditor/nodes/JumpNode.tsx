import { memo, type ChangeEvent } from "react";
import { Handle, Position, type NodeProps, type Node } from "@xyflow/react";
import type { SfcNodeData } from "../../../../types/sfc";

function SfcJumpNode({ id, data }: NodeProps<Node<SfcNodeData>>) {
  function updateData(partial: Partial<SfcNodeData>) {
    window.dispatchEvent(new CustomEvent("sfc:update-node", { detail: { id, data: partial } }));
  }

  function handleTargetChange(e: ChangeEvent<HTMLInputElement>) {
    updateData({ targetStepName: e.target.value });
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", alignItems: "center" }}>
      <Handle type="target" position={Position.Top} id="T" style={{ background: "#6c7086", width: 8, height: 8 }} />

      <div style={{
        background: "var(--card, #1e1e2e)",
        border: "1.5px solid #6c7086",
        borderRadius: 4,
        padding: "4px 8px",
        display: "flex", alignItems: "center", gap: 4,
      }}>
        <span style={{ color: "#6c7086", fontSize: 11, fontWeight: 700 }}>→</span>
        <input
          value={data.targetStepName ?? ""}
          onChange={handleTargetChange}
          placeholder="Step name"
          style={{
            border: "none", background: "transparent",
            color: "var(--text)", fontSize: 10, fontWeight: 600,
            outline: "none", width: 70,
          }}
        />
      </div>

      <Handle type="source" position={Position.Bottom} id="B" style={{ background: "#6c7086", width: 8, height: 8 }} />
    </div>
  );
}

export default memo(SfcJumpNode);
