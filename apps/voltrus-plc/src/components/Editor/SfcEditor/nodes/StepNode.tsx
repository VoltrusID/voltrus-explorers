import { memo, type ChangeEvent } from "react";
import { Handle, Position, type NodeProps, type Node } from "@xyflow/react";
import type { SfcNodeData, SfcAction, ActionQualifier } from "../../../../types/sfc";

const QUALIFIERS: ActionQualifier[] = ["N", "S", "R", "P1", "P0"];
const QUALIFIER_COLORS: Record<ActionQualifier, string> = {
  N: "#10b981",
  S: "#3b82f6",
  R: "#ef4444",
  P1: "#f59e0b",
  P0: "#a855f7",
};

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

  const borderColor = isInitial ? "#3b82f6" : "#10b981";
  const borderWidth = isInitial ? 3 : 1.5;

  return (
    <div style={{
      background: "var(--card, #1e1e2e)",
      border: `${borderWidth}px solid ${borderColor}`,
      borderRadius: 6,
      minWidth: 160,
      maxWidth: 220,
      padding: "8px 10px",
      boxShadow: isInitial ? `0 0 0 1.5px ${borderColor}40` : "none",
      position: "relative",
    }}>
      {isInitial && <div style={{
        position: "absolute", top: 2, right: 6,
        fontSize: 8, color: "#3b82f6", fontWeight: 700,
      }}>INIT</div>}

      <Handle type="target" position={Position.Top} id="T" style={{ background: borderColor, width: 8, height: 8 }} />

      <input
        value={data.stepName ?? ""}
        onChange={handleNameChange}
        placeholder="Step name"
        style={{
          width: "100%", border: "none", background: "transparent",
          color: "var(--text)", fontSize: 12, fontWeight: 600,
          outline: "none", marginBottom: 4,
        }}
      />

      {actions.map((act, i) => (
        <div key={i} style={{ display: "flex", gap: 3, alignItems: "center", marginBottom: 2 }}>
          <select
            value={act.qualifier}
            onChange={(e) => handleActionChange(i, "qualifier", e.target.value)}
            style={{
              width: 32, fontSize: 9, padding: "1px 2px",
              background: QUALIFIER_COLORS[act.qualifier],
              color: "#fff", border: "none", borderRadius: 3, fontWeight: 700,
            }}
          >
            {QUALIFIERS.map((q) => <option key={q} value={q}>{q}</option>)}
          </select>
          <input
            value={act.variableName}
            onChange={(e) => handleActionChange(i, "variableName", e.target.value)}
            placeholder="variable"
            style={{
              flex: 1, fontSize: 9, border: "1px solid var(--border, #45475a)",
              borderRadius: 3, padding: "1px 4px", background: "var(--bg, #11111b)",
              color: "var(--text)", minWidth: 0,
            }}
          />
          <button onClick={() => removeAction(i)} style={{
            background: "none", border: "none", color: "#ef4444",
            cursor: "pointer", fontSize: 10, padding: 0, lineHeight: 1,
          }}>×</button>
        </div>
      ))}

      <button onClick={addAction} style={{
        width: "100%", fontSize: 9, padding: "2px 0",
        background: "var(--bg, #11111b)", border: "1px dashed var(--border, #45475a)",
        borderRadius: 3, color: "var(--text-muted)", cursor: "pointer",
        marginTop: 2,
      }}>+ action</button>

      <Handle type="source" position={Position.Bottom} id="B" style={{ background: borderColor, width: 8, height: 8 }} />
    </div>
  );
}

export default memo(SfcStepNode);
