import { useCallback, useEffect, useState } from "react";
import { useReactFlow, type Node } from "@xyflow/react";
import type { SfcNodeData } from "../../../types/sfc";

interface HotkeyAction {
  label: string;
  elementType: SfcNodeData["elementType"];
  nodeType: string;
}

const FKEY_MAP: Record<string, HotkeyAction> = {
  F5:  { label: "Step",              elementType: "step",         nodeType: "step" },
  F6:  { label: "Initial Step",      elementType: "initial_step", nodeType: "initial_step" },
  F7:  { label: "Transition",        elementType: "transition",   nodeType: "transition" },
  F8:  { label: "OR Diverge",        elementType: "or_diverge",   nodeType: "or_diverge" },
  F9:  { label: "OR Converge",       elementType: "or_converge",  nodeType: "or_converge" },
  F10: { label: "Jump",             elementType: "jump",         nodeType: "jump" },
  F11: { label: "AND Diverge (*)",  elementType: "and_diverge",  nodeType: "and_diverge" },
  F12: { label: "AND Converge (*)", elementType: "and_converge", nodeType: "and_converge" },
};

export const SFC_HOTKEYS: Array<{ key: string; label: string }> = [
  ...Object.entries(FKEY_MAP).map(([key, action]) => ({ key, label: action.label })),
  { key: "Delete", label: "Delete selected" },
  { key: "F1", label: "Toggle help" },
];

function deriveNextNodeId(nodes: Node[]): string {
  let max = 0;
  for (const n of nodes) {
    const m = n.id.match(/^node-(\d+)$/);
    if (m) max = Math.max(max, parseInt(m[1]));
  }
  return `node-${max + 1}`;
}

function countSteps(nodes: Node[]): number {
  let count = 0;
  for (const n of nodes) {
    if (n.data?.elementType === "step" || n.data?.elementType === "initial_step") count++;
  }
  return count;
}

export function useSfcHotkeys(
  onNodesChange: (changes: Array<{ type: "add"; item: Node }>) => void,
) {
  const { screenToFlowPosition, getNodes } = useReactFlow();
  const [showHelp, setShowHelp] = useState(false);

  const addElement = useCallback((action: HotkeyAction) => {
    const nodes = getNodes();
    const cx = window.innerWidth / 2;
    const cy = window.innerHeight / 2;
    const position = screenToFlowPosition({ x: cx, y: cy });
    position.x = Math.round(position.x / 120) * 120;
    position.y = Math.round(position.y / 100) * 100;

    const data: Record<string, unknown> = {
      elementType: action.elementType,
      pinValues: {},
    };

    if (action.elementType === "step" || action.elementType === "initial_step") {
      const stepNum = countSteps(nodes) + 1;
      data.stepName = `Step_${stepNum}`;
      data.actions = [];
    } else if (action.elementType === "transition") {
      data.condition = "";
    } else if (action.elementType === "jump") {
      data.targetStepName = "";
    }

    const newNode: Node = {
      id: deriveNextNodeId(nodes),
      type: action.nodeType,
      position,
      data,
    };

    onNodesChange([{ type: "add" as const, item: newNode }]);
  }, [screenToFlowPosition, getNodes, onNodesChange]);

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      const tag = (e.target as HTMLElement)?.tagName;
      if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;

      if (e.key === "F1") {
        e.preventDefault();
        setShowHelp((v) => !v);
        return;
      }

      const action = FKEY_MAP[e.key];
      if (action) {
        e.preventDefault();
        addElement(action);
      }
    };

    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [addElement]);

  return { showHelp, setShowHelp };
}
