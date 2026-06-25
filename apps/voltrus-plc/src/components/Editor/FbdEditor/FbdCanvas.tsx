import { useCallback, useRef } from "react";
import {
  ReactFlow,
  Background,
  Controls,
  MiniMap,
  useReactFlow,
  type OnNodesChange,
  type OnEdgesChange,
  type OnConnect,
} from "@xyflow/react";
import type { Node, Edge } from "@xyflow/react";
import { fbdNodeTypes } from "./nodes/NodeTypes";
import { getBlockDef, formatRegisterAddress } from "../../../types/fbd";
import type { RegisterType } from "../../../types/fbd";

interface FbdCanvasProps {
  nodes: Node[];
  edges: Edge[];
  onNodesChange: OnNodesChange<Node>;
  onEdgesChange: OnEdgesChange<Edge>;
  onConnect: OnConnect;
}

/** Derive next ID from existing nodes to avoid collisions across file switches */
function deriveNextNodeId(nodes: Node[]): string {
  let max = 0;
  for (const n of nodes) {
    const m = n.id.match(/^node-(\d+)$/);
    if (m) max = Math.max(max, parseInt(m[1]));
  }
  return `node-${max + 1}`;
}

/** Auto-assign register type + next number for input/output pins */
function deriveRegisterDefaults(category: string, nodes: Node[]): Record<string, unknown> {
  if (category !== "input" && category !== "output") return {};
  const regType = (category === "input" ? "X" : "Y") as RegisterType;
  let maxNum = -1;
  for (const n of nodes) {
    const rt = n.data?.registerType as string | undefined;
    const rn = n.data?.registerNumber as number | undefined;
    if (rt === regType && typeof rn === "number") maxNum = Math.max(maxNum, rn);
  }
  const number = maxNum + 1;
  return {
    registerType: regType,
    registerNumber: number,
    variableName: formatRegisterAddress(regType, number),
    variableType: "BOOL" as const,
  };
}

export default function FbdCanvas({ nodes, edges, onNodesChange, onEdgesChange, onConnect }: FbdCanvasProps) {
  const reactFlowWrapper = useRef<HTMLDivElement>(null);
  const { screenToFlowPosition } = useReactFlow();

  const onDragOver = useCallback((event: React.DragEvent) => {
    event.preventDefault();
    event.dataTransfer.dropEffect = "move";
  }, []);

  const onDrop = useCallback(
    (event: React.DragEvent) => {
      event.preventDefault();
      const blockType = event.dataTransfer.getData("application/fbd-block");
      if (!blockType) return;

      const def = getBlockDef(blockType);
      if (!def) return;

      const position = screenToFlowPosition({
        x: event.clientX,
        y: event.clientY,
      });

      const nodeType = def.category === "input" ? "input_pin"
        : def.category === "output" ? "output_pin"
        : def.category === "timer" ? "timer"
        : def.category === "counter" ? "counter"
        : "block";

      const newNode: Node = {
        id: deriveNextNodeId(nodes),
        type: nodeType,
        position,
        data: {
          blockType,
          label: def.label,
          instanceName: def.isFunctionBlock ? `${blockType.toLowerCase()}_${Date.now()}` : undefined,
          pinValues: {},
          ...deriveRegisterDefaults(def.category, nodes),
        },
      };

      onNodesChange([{ type: "add" as const, item: newNode }]);
    },
    [screenToFlowPosition, onNodesChange, nodes],
  );

  return (
    <div ref={reactFlowWrapper} style={{ width: "100%", height: "100%" }}>
      <ReactFlow
        nodes={nodes}
        edges={edges}
        onNodesChange={onNodesChange}
        onEdgesChange={onEdgesChange}
        onConnect={onConnect}
        onDragOver={onDragOver}
        onDrop={onDrop}
        nodeTypes={fbdNodeTypes}
        fitView
        style={{ background: "var(--bg, #11111b)" }}
        connectionLineStyle={{ stroke: "var(--accent, #7c3aed)", strokeWidth: 2 }}
        defaultEdgeOptions={{ type: "smoothstep", style: { stroke: "var(--text-muted, #6c7086)", strokeWidth: 2 } }}
      >
        <Background gap={16} size={1} color="var(--border, #45475a)" />
        <Controls style={{ background: "var(--card, #1e1e2e)", border: "1px solid var(--border, #45475a)" }} />
        <MiniMap
          style={{ background: "var(--card, #1e1e2e)", border: "1px solid var(--border, #45475a)" }}
          nodeColor={() => "var(--accent, #7c3aed)"}
        />
      </ReactFlow>
    </div>
  );
}
