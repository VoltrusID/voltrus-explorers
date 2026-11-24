import { memo, type ChangeEvent } from "react";
import { Handle, Position, type NodeProps, type Node } from "@xyflow/react";
import { Plus, X } from "lucide-react";
import type { SfcNodeData, SfcAction, ActionQualifier } from "../../../../types/sfc";

const QUALIFIERS: ActionQualifier[] = ["N", "S", "R", "P1", "P0"];

function SfcStepNode({ id, data }: NodeProps<Node<SfcNodeData>>) {
  const isInitial = data.elementType === "initial_step";
  const actions = (data.actions ?? []) as SfcAction[];

  function updateData(partial: Partial<SfcNodeData>) {
    window.dispatchEvent(new CustomEvent("sfc:update-node", { detail: { id, data: partial } }));
  }

  function handleNameChange(e: ChangeEvent<HTMLInputElement>) {
    updateData({ stepName: e.target.value });
  }

  function handleActionChange(idx: number, field: keyof SfcAction, value: string) {
    const next = actions.map((a, i) => (i === idx ? { ...a, [field]: value } : a));
    updateData({ actions: next });
  }

  function addAction() {
    updateData({ actions: [...actions, { qualifier: "N", variableName: "" }] });
  }

  function removeAction(idx: number) {
    updateData({ actions: actions.filter((_, i) => i !== idx) });
  }

  return (
    <div className={`ed-node ed-sfc-step${isInitial ? " ed-sfc-step--initial" : ""}`}>
      {isInitial && <span className="ed-sfc-step__init">Initial</span>}

      <Handle type="target" position={Position.Top} id="T" />

      <input
        className="ed-sfc-step__name"
        value={data.stepName ?? ""}
        onChange={handleNameChange}
        placeholder="Step name"
        aria-label="Step name"
      />

      {actions.map((act, i) => (
        <div key={i} className="ed-sfc-action">
          <select
            className="ed-sfc-qualifier"
            value={act.qualifier}
            onChange={(e) => handleActionChange(i, "qualifier", e.target.value)}
            aria-label="Action qualifier"
          >
            {QUALIFIERS.map((q) => <option key={q} value={q}>{q}</option>)}
          </select>
          <input
            value={act.variableName}
            onChange={(e) => handleActionChange(i, "variableName", e.target.value)}
            placeholder="Variable"
            aria-label={`Action ${i + 1} variable`}
          />
          <button
            type="button"
            className="ed-sfc-remove"
            onClick={() => removeAction(i)}
            aria-label={`Remove action ${i + 1}`}
          >
            <X size={12} strokeWidth={1.75} />
          </button>
        </div>
      ))}

      <button type="button" className="ed-sfc-add" onClick={addAction}>
        <Plus size={12} strokeWidth={1.75} />
        Action
      </button>

      <Handle type="source" position={Position.Bottom} id="B" />
    </div>
  );
}

export default memo(SfcStepNode);
