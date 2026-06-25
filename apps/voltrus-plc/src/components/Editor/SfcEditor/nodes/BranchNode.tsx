import { memo } from "react";
import { Handle, Position, type NodeProps, type Node } from "@xyflow/react";
import type { SfcNodeData } from "../../../../types/sfc";

function SfcBranchNode({ data }: NodeProps<Node<SfcNodeData>>) {
  const isDiverge = data.elementType === "or_diverge";
  const label = isDiverge ? "OR" : "OR";

  return (
    <div style={{ display: "flex", flexDirection: "column", alignItems: "center" }}>
      {isDiverge ? (
        <>
          <Handle type="target" position={Position.Top} id="T" style={{ background: "#a855f7", width: 8, height: 8 }} />
          <div style={{
            width: 32, height: 32,
            background: "var(--card, #1e1e2e)",
            border: "2px solid #a855f7",
            borderRadius: 4,
            display: "flex", alignItems: "center", justifyContent: "center",
            fontSize: 10, fontWeight: 700, color: "#a855f7",
          }}>
            {label}
          </div>
          <div style={{ display: "flex", gap: 40, marginTop: -4 }}>
            <Handle type="source" position={Position.Bottom} id="B0" style={{ background: "#a855f7", width: 8, height: 8, position: "relative", left: -20 }} />
            <Handle type="source" position={Position.Bottom} id="B1" style={{ background: "#a855f7", width: 8, height: 8, position: "relative", right: -20 }} />
          </div>
        </>
      ) : (
        <>
          <div style={{ display: "flex", gap: 40, marginBottom: -4 }}>
            <Handle type="target" position={Position.Top} id="T0" style={{ background: "#a855f7", width: 8, height: 8, position: "relative", left: -20 }} />
            <Handle type="target" position={Position.Top} id="T1" style={{ background: "#a855f7", width: 8, height: 8, position: "relative", right: -20 }} />
          </div>
          <div style={{
            width: 32, height: 32,
            background: "var(--card, #1e1e2e)",
            border: "2px solid #a855f7",
            borderRadius: 4,
            display: "flex", alignItems: "center", justifyContent: "center",
            fontSize: 10, fontWeight: 700, color: "#a855f7",
          }}>
            {label}
          </div>
          <Handle type="source" position={Position.Bottom} id="B" style={{ background: "#a855f7", width: 8, height: 8 }} />
        </>
      )}
    </div>
  );
}

export default memo(SfcBranchNode);
