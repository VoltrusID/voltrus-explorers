import { memo, type ChangeEvent } from "react";
import { Handle, Position, type NodeProps, type Node } from "@xyflow/react";
import { ArrowRight } from "lucide-react";
import type { SfcNodeData } from "../../../../types/sfc";

function SfcJumpNode({ id, data }: NodeProps<Node<SfcNodeData>>) {
  function updateData(partial: Partial<SfcNodeData>) {
    window.dispatchEvent(new CustomEvent("sfc:update-node", { detail: { id, data: partial } }));
  }

  function handleTargetChange(e: ChangeEvent<HTMLInputElement>) {
    updateData({ targetStepName: e.target.value });
  }

  return (
    <div className="ed-sfc-jump">
      <Handle type="target" position={Position.Top} id="T" />

      <div className="ed-sfc-jump__card">
        <ArrowRight size={12} strokeWidth={1.75} />
        <input
          value={data.targetStepName ?? ""}
          onChange={handleTargetChange}
          placeholder="Step name"
          aria-label="Jump target step"
        />
      </div>

      <Handle type="source" position={Position.Bottom} id="B" />
    </div>
  );
}

export default memo(SfcJumpNode);
