import { useCallback, useEffect, useState } from "react";
import { useReactFlow, type Node } from "@xyflow/react";
import { getBlockDef, formatRegisterAddress } from "../../../types/fbd";
import type { RegisterType } from "../../../types/fbd";

interface HotkeyAction {
  label: string;
  blockType?: string;
}

const FKEY_MAP: Record<string, HotkeyAction> = {
  F5:  { label: "LD (Input)", blockType: "INPUT_PIN" },
  F6:  { label: "AND", blockType: "AND" },
  F7:  { label: "OR", blockType: "OR" },
  F8:  { label: "OUT (Output)", blockType: "OUTPUT_PIN" },
  F9:  { label: "NOT", blockType: "NOT" },
  F10: { label: "TON", blockType: "TON" },
  F11: { label: "MOVE", blockType: "MOVE" },
  F12: { label: "XOR", blockType: "XOR" },
};

export const FBD_HOTKEYS: Array<{ key: string; label: string }> = [
  ...Object.entries(FKEY_MAP).map(([key, action]) => ({ key, label: action.label })),
  { key: "Delete", label: "Delete selected" },
  { key: "F1", label: "Toggle help" },
];

let nextRegCounter = 100;

function deriveNextNodeId(nodes: Node[]): string {
  let max = 0;
  for (const n of nodes) {
    const m = n.id.match(/^node-(\d+)$/);
    if (m) max = Math.max(max, parseInt(m[1]));
  }
  return `node-${max + 1}`;
}

export function useFbdHotkeys(
  onNodesChange: (changes: Array<{ type: "add"; item: Node }>) => void,
) {
  const { screenToFlowPosition, getNodes } = useReactFlow();
  const [showHelp, setShowHelp] = useState(false);

  const addBlock = useCallback((blockType: string) => {
    const def = getBlockDef(blockType);
    if (!def) return;

    const nodes = getNodes();
    const cx = window.innerWidth / 2;
    const cy = window.innerHeight / 2;
    const position = screenToFlowPosition({ x: cx, y: cy });

    // Slight random offset to avoid stacking
    position.x += (Math.random() - 0.5) * 40;
    position.y += (Math.random() - 0.5) * 40;

    const nodeType = def.category === "input" ? "input_pin"
      : def.category === "output" ? "output_pin"
      : def.category === "timer" ? "timer"
      : def.category === "counter" ? "counter"
      : "block";

    let extraData: Record<string, unknown> = {};
    if (def.category === "input" || def.category === "output") {
      const regType = (def.category === "input" ? "X" : "Y") as RegisterType;
      nextRegCounter++;
      const num = nextRegCounter;
      extraData = {
        registerType: regType,
        registerNumber: num,
        variableName: formatRegisterAddress(regType, num),
        variableType: "BOOL",
      };
    }

    const newNode: Node = {
      id: deriveNextNodeId(nodes),
      type: nodeType,
      position,
      data: {
        blockType,
        label: def.label,
        instanceName: def.isFunctionBlock ? `${blockType.toLowerCase()}_${Date.now()}` : undefined,
        pinValues: {},
        ...extraData,
      },
    };

    onNodesChange([{ type: "add" as const, item: newNode }]);
  }, [screenToFlowPosition, getNodes, onNodesChange]);

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      // Ignore if typing in an input/textarea
      const tag = (e.target as HTMLElement)?.tagName;
      if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;

      // F1 = toggle help
      if (e.key === "F1") {
        e.preventDefault();
        setShowHelp((v) => !v);
        return;
      }

      // F5–F12 = insert block
      const action = FKEY_MAP[e.key];
      if (action?.blockType) {
        e.preventDefault();
        addBlock(action.blockType);
      }
    };

    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [addBlock]);

  return { showHelp, setShowHelp };
}
