import { Handle, Position, type NodeProps, type Node } from "@xyflow/react";
import { Hash } from "lucide-react";
import type { BlockNodeData } from "../../../../types/fbd";
import { getBlockDef } from "../../../../types/fbd";

export function CounterNode({ data, id }: NodeProps<Node<BlockNodeData>>) {
  const def = getBlockDef(data.blockType);
  const inputPins = def?.pins.filter((p) => p.direction === "input") ?? [];
  const outputPins = def?.pins.filter((p) => p.direction === "output") ?? [];

  const handleInstanceChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    window.dispatchEvent(new CustomEvent("fbd:update-node", {
      detail: { id, data: { ...data, instanceName: e.target.value } },
    }));
  };

  const handlePinValueChange = (pinName: string, value: string) => {
    window.dispatchEvent(new CustomEvent("fbd:update-node", {
      detail: { id, data: { ...data, pinValues: { ...data.pinValues, [pinName]: value } } },
    }));
  };

  return (
    <div className="ed-node" style={{ minWidth: 100 }}>
      <div className="ed-node__head">
        <Hash size={12} strokeWidth={1.75} />
        {data.label}
      </div>
      <div className="ed-node__instance">
        <input
          type="text"
          value={data.instanceName || ""}
          onChange={handleInstanceChange}
          placeholder="Instance"
          aria-label="Counter instance name"
        />
      </div>
      <div className="ed-node__pins">
        <div>
          {inputPins.map((pin) => (
            <div key={pin.name} className="ed-pin">
              <Handle type="target" position={Position.Left} id={pin.name} />
              {pin.name === "PV" ? (
                <input
                  type="text"
                  className="ed-pin__value"
                  value={data.pinValues.PV || ""}
                  onChange={(e) => handlePinValueChange("PV", e.target.value)}
                  placeholder="0"
                  aria-label="Preset count"
                  style={{ width: 36 }}
                />
              ) : (
                <span>{pin.name}</span>
              )}
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
