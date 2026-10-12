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
    <div className="ed-sfc-transition">
      <Handle type="target" position={Position.Top} id="T" />

      <div className="ed-sfc-transition__bar" />

      <input
        className="ed-sfc-transition__cond"
        value={data.condition ?? ""}
        onChange={handleConditionChange}
        placeholder="Condition (ST expression)"
        aria-label="Transition condition"
      />

      <Handle type="source" position={Position.Bottom} id="B" />
    </div>
  );
}

export default memo(SfcTransitionNode);
