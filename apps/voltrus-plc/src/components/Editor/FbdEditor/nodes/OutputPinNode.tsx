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
    <div className="ed-node ed-io">
      <Handle type="target" position={Position.Left} id="IN" />
      <span className="ed-io__tag">OUT</span>
      <select
        className="ed-io__select"
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
        aria-label="Register type"
      >
        {REGISTER_TYPES.map((r) => (
          <option key={r.id} value={r.id}>{r.label}</option>
        ))}
      </select>
      <input
        type="text"
        className="ed-io__addr"
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
        aria-label="Output address"
      />
    </div>
  );
}
