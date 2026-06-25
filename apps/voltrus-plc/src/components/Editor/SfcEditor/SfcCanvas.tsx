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
import { sfcNodeTypes } from "./nodes/NodeTypes";
import { getSfcElementDef } from "../../../types/sfc";
import type { SfcNodeData } from "../../../types/sfc";

interface SfcCanvasProps {
  nodes: Node[];
  edges: Edge[];
  onNodesChange: OnNodesChange<Node>;
  onEdgesChange: OnEdgesChange<Edge>;
  onConnect: OnConnect;
}

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

export default function SfcCanvas({ nodes, edges, onNodesChange, onEdgesChange, onConnect }: SfcCanvasProps) {
  const reactFlowWrapper = useRef<HTMLDivElement>(null);
  const { screenToFlowPosition } = useReactFlow();

  const onDragOver = useCallback((event: React.DragEvent) => {
    event.preventDefault();
    event.dataTransfer.dropEffect = "move";
  }, []);

  const onDrop = useCallback(
    (event: React.DragEvent) => {
      event.preventDefault();
      const rawData = event.dataTransfer.getData("application/sfc-element");
      if (!rawData) return;

      let elementType: string;
      try {
        const parsed = JSON.parse(rawData);
        elementType = parsed.elementType;
      } catch {
        return;
      }

      const def = getSfcElementDef(elementType);
      if (!def) return;

      const position = screenToFlowPosition({ x: event.clientX, y: event.clientY });
      position.x = Math.round(position.x / 120) * 120;
      position.y = Math.round(position.y / 100) * 100;

      const data: Partial<SfcNodeData> & { pinValues: Record<string, string> } = {
        elementType: def.type as SfcNodeData["elementType"],
        pinValues: {},
      };

      if (def.type === "step" || def.type === "initial_step") {
        const stepNum = countSteps(nodes) + 1;
        data.stepName = `Step_${stepNum}`;
        data.actions = [];
      } else if (def.type === "transition") {
        data.condition = "";
      } else if (def.type === "jump") {
        data.targetStepName = "";
      }

      const newNode: Node = {
        id: deriveNextNodeId(nodes),
        type: def.type,
        position,
        data,
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
        nodeTypes={sfcNodeTypes}
        fitView
        snapToGrid={true}
        snapGrid={[120, 100]}
        style={{ background: "var(--bg, #11111b)" }}
        connectionLineStyle={{ stroke: "var(--accent, #7c3aed)", strokeWidth: 2 }}
        defaultEdgeOptions={{ type: "straight", style: { stroke: "var(--text-muted, #6c7086)", strokeWidth: 2 } }}
      >
        <Background gap={100} size={1} color="var(--border, #45475a)" />
        <Controls style={{ background: "var(--card, #1e1e2e)", border: "1px solid var(--border, #45475a)" }} />
        <MiniMap
          style={{ background: "var(--card, #1e1e2e)", border: "1px solid var(--border, #45475a)" }}
          nodeColor={() => "var(--accent, #7c3aed)"}
        />
      </ReactFlow>
    </div>
  );
}
