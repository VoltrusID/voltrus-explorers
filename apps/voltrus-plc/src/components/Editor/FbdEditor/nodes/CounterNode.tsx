import { Handle, Position, type NodeProps, type Node } from "@xyflow/react";
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
    <div style={{
      background: "var(--card, #1e1e2e)",
      border: "1px solid #8b5cf6",
      borderRadius: 6,
      minWidth: 100,
      fontSize: 11,
    }}>
      <div style={{
        padding: "3px 8px",
        background: "#8b5cf6",
        color: "#fff",
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
          {inputPins.map((pin) => (
            <div key={pin.name} style={{ position: "relative", padding: "1px 8px 1px 12px", fontSize: 10 }}>
              <Handle type="target" position={Position.Left} id={pin.name} style={{ width: 8, height: 8, background: "#8b5cf6", top: "50%" }} />
              {pin.name === "PV" ? (
                <input
                  type="text"
                  value={data.pinValues.PV || ""}
                  onChange={(e) => handlePinValueChange("PV", e.target.value)}
                  placeholder="0"
                  style={{
                    width: 30,
                    background: "transparent",
                    border: "none",
                    color: "var(--text-muted, #6c7086)",
                    fontSize: 10,
                    fontFamily: '"SF Mono", monospace',
                    outline: "none",
                  }}
                />
              ) : (
                <span style={{ color: "var(--text-muted, #6c7086)" }}>{pin.name}</span>
              )}
            </div>
          ))}
        </div>
        <div>
          {outputPins.map((pin) => (
            <div key={pin.name} style={{ position: "relative", padding: "1px 12px 1px 8px", fontSize: 10, textAlign: "right" }}>
              <Handle type="source" position={Position.Right} id={pin.name} style={{ width: 8, height: 8, background: "#8b5cf6", top: "50%" }} />
              <span style={{ color: "var(--text-muted, #6c7086)" }}>{pin.name}</span>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
