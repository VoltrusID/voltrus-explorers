import { Handle, Position, type NodeProps, type Node } from "@xyflow/react";
import type { BlockNodeData } from "../../../../types/fbd";
import { getBlockDef } from "../../../../types/fbd";

function updateNodeData(id: string, patch: Partial<BlockNodeData>) {
  window.dispatchEvent(new CustomEvent("fbd:update-node", { detail: { id, data: patch } }));
}

export function BlockNode({ id, data }: NodeProps<Node<BlockNodeData>>) {
  const def = getBlockDef(data.blockType);
  if (!def) return <div className="ed-node" style={{ color: "var(--danger)", padding: 8 }}>Unknown: {data.blockType}</div>;

  const inputPins = def.pins.filter((p) => p.direction === "input");
  const outputPins = def.pins.filter((p) => p.direction === "output");

  return (
    <div className="ed-node" style={{ minWidth: 80 }}>
      <div className="ed-node__head">{data.label}</div>
      {def.isFunctionBlock && (
        <div className="ed-node__instance">
          <input
            value={data.instanceName ?? ""}
            onChange={(e) => updateNodeData(id, { instanceName: e.target.value })}
            placeholder="Instance name"
            aria-label="Instance name"
          />
        </div>
      )}
      <div className="ed-node__pins">
        <div>
          {inputPins.map((pin) => (
            <div key={pin.name} className="ed-pin">
              <Handle type="target" position={Position.Left} id={pin.name} />
              <span>{pin.name}</span>
            </div>
          ))}
        </div>
        <div>
          {outputPins.map((pin) => (
            <div key={pin.name} className="ed-pin ed-pin--out">
              <Handle type="source" position={Position.Right} id={pin.name} />
              <span>{pin.name}</span>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
