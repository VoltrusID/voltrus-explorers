import { Handle, Position, type NodeProps, type Node } from "@xyflow/react";
import type { BlockNodeData, RegisterType } from "../../../../types/fbd";
import { REGISTER_TYPES, formatRegisterAddress } from "../../../../types/fbd";

function updateNodeData(id: string, data: Partial<BlockNodeData>) {
  window.dispatchEvent(new CustomEvent("fbd:update-node", { detail: { id, data } }));
}

export function OutputPinNode({ data, id }: NodeProps<Node<BlockNodeData>>) {
  const regType = (data.registerType as RegisterType) || "Y";
  const regNum = typeof data.registerNumber === "number" ? data.registerNumber : 0;
  const regAddr = formatRegisterAddress(regType, regNum);

  return (
    <div style={{
      background: "var(--card, #1e1e2e)",
      border: "1px solid #3b82f6",
      borderRadius: 6,
      padding: "4px 8px",
      fontSize: 11,
    }}>
      <Handle
        type="target"
        position={Position.Left}
        id="IN"
        style={{ width: 8, height: 8, background: "#3b82f6" }}
      />
      <div style={{ display: "flex", alignItems: "center", gap: 4 }}>
        <span style={{ fontSize: 9, color: "#3b82f6", fontWeight: 700 }}>OUT</span>
        <select
          value={regType}
          onChange={(e) => {
            const newType = e.target.value as RegisterType;
            const info = REGISTER_TYPES.find((r) => r.id === newType);
            updateNodeData(id, {
              registerType: newType,
              variableName: formatRegisterAddress(newType, regNum),
              variableType: info?.iecType || "BOOL",
            });
          }}
          style={{
            width: 36,
            background: "var(--bg, #11111b)",
            border: "1px solid var(--border, #45475a)",
            borderRadius: 3,
            color: "#3b82f6",
            fontSize: 10,
            fontWeight: 700,
            padding: "0 2px",
            outline: "none",
            cursor: "pointer",
          }}
        >
          {REGISTER_TYPES.map((r) => (
            <option key={r.id} value={r.id}>{r.label}</option>
          ))}
        </select>
        <input
          type="text"
          value={regAddr}
          onChange={(e) => {
            const val = e.target.value;
            updateNodeData(id, {
              variableName: val,
              registerType: undefined,
              registerNumber: undefined,
            });
          }}
          onKeyDown={(e) => {
            if (e.key === "ArrowUp" || e.key === "ArrowDown") {
              e.preventDefault();
              const delta = e.key === "ArrowUp" ? 1 : -1;
              const newNum = Math.max(0, regNum + delta);
              const info = REGISTER_TYPES.find((r) => r.id === regType);
              updateNodeData(id, {
                registerNumber: newNum,
                variableName: formatRegisterAddress(regType, newNum),
                variableType: info?.iecType || "BOOL",
              });
            }
          }}
          placeholder="Y0"
          style={{
            width: 44,
            background: "transparent",
            border: "none",
            color: "var(--text, #cdd6f4)",
            fontSize: 11,
            fontFamily: '"SF Mono", monospace',
            outline: "none",
          }}
        />
      </div>
    </div>
  );
}
