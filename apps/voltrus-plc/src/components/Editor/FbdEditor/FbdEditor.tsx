import { useCallback, useEffect, useMemo, useRef } from "react";
import { ReactFlowProvider, useNodesState, useEdgesState, addEdge, useReactFlow, type Connection } from "@xyflow/react";
import type { Node, Edge } from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import FbdCanvas from "./FbdCanvas";
import BlockPalette from "./palette/BlockPalette";
import { useFbdHotkeys, FBD_HOTKEYS } from "./useFbdHotkeys";
import type { BlockNodeData, FbdProgram } from "../../../types/fbd";
import { parseFbdProgram, serializeFbd } from "../../../types/canonical";

interface FbdEditorProps {
  value: string;
  onChange: (value: string) => void;
}

function FbdEditorInner({ value, onChange }: FbdEditorProps) {
  const programRef = useRef<FbdProgram | null>(null);
  const debounceTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const valueRef = useRef(value);
  valueRef.current = value;

  const initial = useMemo(() => {
    try {
      return parseFbdProgram(value);
    } catch {
      return {
        version: "1.0",
        programName: "NewProgram",
        declarations: [],
        nodes: [],
        edges: [],
        viewport: { x: 0, y: 0, zoom: 1 },
      } satisfies FbdProgram;
    }
  }, [value]);

  programRef.current = initial;

  const [nodes, setNodes, onNodesChange] = useNodesState(initial.nodes);
  const [edges, setEdges, onEdgesChange] = useEdgesState(initial.edges);

  // Debounced serialization — avoids serializing on every pixel of drag
  const scheduleSerialize = useCallback(() => {
    if (debounceTimer.current) clearTimeout(debounceTimer.current);
    debounceTimer.current = setTimeout(() => {
      const program: FbdProgram = {
        ...programRef.current!,
        nodes: nodes as Node<BlockNodeData>[],
        edges: edges,
      };
      onChange(serializeFbd(program));
    }, 150);
  }, [nodes, edges, onChange]);

  useEffect(() => {
    scheduleSerialize();
    return () => {
      if (debounceTimer.current) clearTimeout(debounceTimer.current);
    };
  }, [scheduleSerialize]);

  const handleConnect = useCallback(
    (connection: Connection) => {
      const newEdge = addEdge({
        ...connection,
        data: {
          sourcePin: connection.sourceHandle ?? "",
          targetPin: connection.targetHandle ?? "",
        },
      }, edges);
      setEdges(newEdge);
    },
    [edges, setEdges],
  );

  // Handle node data updates from custom node components
  useEffect(() => {
    const handler = (e: Event) => {
      const { id, data } = (e as CustomEvent).detail;
      setNodes((nds) =>
        nds.map((n) => (n.id === id ? { ...n, data: { ...n.data, ...data } } : n)),
      );
    };
    window.addEventListener("fbd:update-node", handler);
    return () => window.removeEventListener("fbd:update-node", handler);
  }, [setNodes]);

  // Hotkeys
  const { showHelp, setShowHelp } = useFbdHotkeys(
    onNodesChange as (changes: Array<{ type: "add"; item: Node }>) => void,
  );

  return (
    <div style={{ display: "flex", width: "100%", height: "100%", position: "relative" }}>
      <BlockPalette />
      <div style={{ flex: 1 }}>
        <FbdCanvas
          nodes={nodes as Node[]}
          edges={edges as Edge[]}
          onNodesChange={onNodesChange as never}
          onEdgesChange={onEdgesChange as never}
          onConnect={handleConnect}
        />
      </div>
      {/* Hotkey help overlay */}
      {showHelp && (
        <div
          style={{
            position: "absolute",
            top: 10,
            right: 10,
            background: "var(--card, #1e1e2e)",
            border: "1px solid var(--border, #45475a)",
            borderRadius: 8,
            padding: "12px 16px",
            fontSize: 11,
            zIndex: 100,
            minWidth: 200,
            boxShadow: "0 4px 12px rgba(0,0,0,0.4)",
          }}
        >
          <div style={{ fontWeight: 700, marginBottom: 8, color: "var(--accent, #7c3aed)" }}>
            Keyboard Shortcuts
          </div>
          {FBD_HOTKEYS.map((h) => (
            <div key={h.key} style={{ display: "flex", justifyContent: "space-between", padding: "2px 0" }}>
              <span style={{ color: "var(--text-muted, #6c7086)" }}>{h.label}</span>
              <kbd style={{
                background: "var(--bg, #11111b)",
                border: "1px solid var(--border, #45475a)",
                borderRadius: 3,
                padding: "1px 6px",
                fontFamily: '"SF Mono", monospace',
                fontSize: 10,
              }}>
                {h.key}
              </kbd>
            </div>
          ))}
          <div style={{ display: "flex", justifyContent: "space-between", padding: "2px 0" }}>
            <span style={{ color: "var(--text-muted, #6c7086)" }}>Delete selected</span>
            <kbd style={{
              background: "var(--bg, #11111b)",
              border: "1px solid var(--border, #45475a)",
              borderRadius: 3,
              padding: "1px 6px",
              fontFamily: '"SF Mono", monospace',
              fontSize: 10,
            }}>
              Del
            </kbd>
          </div>
          <button
            onClick={() => setShowHelp(false)}
            style={{
              marginTop: 8,
              width: "100%",
              background: "var(--accent, #7c3aed)",
              color: "#fff",
              border: "none",
              borderRadius: 4,
              padding: "4px 0",
              cursor: "pointer",
              fontSize: 10,
            }}
          >
            Close
          </button>
        </div>
      )}
    </div>
  );
}

export default function FbdEditor(props: FbdEditorProps) {
  return (
    <ReactFlowProvider>
      <FbdEditorInner {...props} />
    </ReactFlowProvider>
  );
}
