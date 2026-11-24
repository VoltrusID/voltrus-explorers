import { memo } from "react";
import { Handle, Position, type NodeProps, type Node } from "@xyflow/react";
import type { SfcNodeData } from "../../../../types/sfc";

function SfcBranchNode({ data }: NodeProps<Node<SfcNodeData>>) {
  const isDiverge = data.elementType === "or_diverge";

  return (
    <div className="ed-sfc-branch">
      {isDiverge ? (
        <>
          <Handle type="target" position={Position.Top} id="T" />
          <div className="ed-sfc-branch__card">OR</div>
          <div style={{ display: "flex", gap: 40, marginTop: -4 }}>
            <Handle type="source" position={Position.Bottom} id="B0" style={{ position: "relative", left: -20 }} />
            <Handle type="source" position={Position.Bottom} id="B1" style={{ position: "relative", right: -20 }} />
          </div>
        </>
      ) : (
        <>
          <div style={{ display: "flex", gap: 40, marginBottom: -4 }}>
            <Handle type="target" position={Position.Top} id="T0" style={{ position: "relative", left: -20 }} />
            <Handle type="target" position={Position.Top} id="T1" style={{ position: "relative", right: -20 }} />
          </div>
          <div className="ed-sfc-branch__card">OR</div>
          <Handle type="source" position={Position.Bottom} id="B" />
        </>
      )}
    </div>
  );
}

export default memo(SfcBranchNode);
