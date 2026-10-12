import { Handle, Position, type NodeProps, type Node } from "@xyflow/react";
import { Timer } from "lucide-react";
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
    <div className="ed-node" style={{ minWidth: 100 }}>
      <div className="ed-node__head">
        <Timer size={12} strokeWidth={1.75} />
        {data.label}
      </div>
      <div className="ed-node__instance">
        <input
          type="text"
          value={data.instanceName || ""}
          onChange={handleInstanceChange}
          placeholder="Instance"
          aria-label="Timer instance name"
        />
      </div>
      <div className="ed-node__pins">
        <div>
          <div className="ed-pin">
            <Handle type="target" position={Position.Left} id="IN" />
            <span>IN</span>
          </div>
          <div className="ed-pin">
            <Handle type="target" position={Position.Left} id="PT" />
            <input
              type="text"
              className="ed-pin__value"
              value={data.pinValues.PT || ""}
              onChange={handlePtChange}
              placeholder="T#0MS"
              aria-label="Preset time"
            />
          </div>
        </div>
        <div>
          <div className="ed-pin ed-pin--out">
            <Handle type="source" position={Position.Right} id="Q" />
            <span>Q</span>
          </div>
          <div className="ed-pin ed-pin--out">
            <Handle type="source" position={Position.Right} id="ET" />
            <span>ET</span>
          </div>
        </div>
      </div>
    </div>
  );
}
