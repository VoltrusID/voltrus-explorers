import { useCallback, useEffect, useMemo, useRef } from "react";
import { ReactFlowProvider, useNodesState, useEdgesState, addEdge, type Connection } from "@xyflow/react";
import type { Node, Edge } from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import SfcCanvas from "./SfcCanvas";
import SfcPalette from "./palette/SfcPalette";
import { useSfcHotkeys, SFC_HOTKEYS } from "./useSfcHotkeys";
import type { SfcNodeData, SfcProgram } from "../../../types/sfc";
import { parseSfcProgram, serializeSfc } from "../../../types/canonical";

interface SfcEditorProps {
  value: string;
  onChange: (value: string) => void;
}

function SfcEditorInner({ value, onChange }: SfcEditorProps) {
  const programRef = useRef<SfcProgram | null>(null);
  const debounceTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const valueRef = useRef(value);
  valueRef.current = value;

  const initial = useMemo(() => {
    try {
      return parseSfcProgram(value);
    } catch {
      return {
        version: "1.0",
        programName: "NewSFC",
        nodes: [],
        edges: [],
        viewport: { x: 0, y: 0, zoom: 1 },
      } satisfies SfcProgram;
    }
  }, [value]);

  programRef.current = initial;

  const [nodes, setNodes, onNodesChange] = useNodesState(initial.nodes);
  const [edges, setEdges, onEdgesChange] = useEdgesState(initial.edges);

  const scheduleSerialize = useCallback(() => {
    if (debounceTimer.current) clearTimeout(debounceTimer.current);
    debounceTimer.current = setTimeout(() => {
      const program: SfcProgram = {
        ...programRef.current!,
        nodes: nodes as Node<SfcNodeData>[],
        edges,
      };
      onChange(serializeSfc(program));
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
      const newEdge = addEdge(connection, edges);
      setEdges(newEdge);
    },
    [edges, setEdges],
  );

  useEffect(() => {
    const handler = (e: Event) => {
      const { id, data } = (e as CustomEvent).detail;
      setNodes((nds) =>
        nds.map((n) => (n.id === id ? { ...n, data: { ...n.data, ...data } } : n)),
      );
    };
    window.addEventListener("sfc:update-node", handler);
    return () => window.removeEventListener("sfc:update-node", handler);
  }, [setNodes]);

  const { showHelp, setShowHelp } = useSfcHotkeys(
    onNodesChange as (changes: Array<{ type: "add"; item: Node }>) => void,
  );

  return (
    <div style={{ display: "flex", width: "100%", height: "100%", position: "relative" }}>
      <SfcPalette />
      <div style={{ flex: 1 }}>
        <SfcCanvas
          nodes={nodes as Node[]}
          edges={edges as Edge[]}
          onNodesChange={onNodesChange as never}
          onEdgesChange={onEdgesChange as never}
          onConnect={handleConnect}
        />
      </div>
      {showHelp && (
        <div style={{
          position: "absolute", top: 10, right: 10,
          background: "var(--card, #1e1e2e)", border: "1px solid var(--border, #45475a)",
          borderRadius: 8, padding: "12px 16px", fontSize: 11, zIndex: 100,
          minWidth: 200, boxShadow: "0 4px 12px rgba(0,0,0,0.4)",
        }}>
          <div style={{ fontWeight: 700, marginBottom: 8, color: "var(--accent, #7c3aed)" }}>Keyboard Shortcuts</div>
          {SFC_HOTKEYS.map((h) => (
            <div key={h.key} style={{ display: "flex", justifyContent: "space-between", padding: "2px 0" }}>
              <span style={{ color: "var(--text-muted)" }}>{h.label}</span>
              <kbd style={{ background: "var(--bg)", border: "1px solid var(--border)", borderRadius: 3, padding: "1px 6px", fontFamily: '"SF Mono", monospace', fontSize: 10 }}>{h.key}</kbd>
            </div>
          ))}
          <button onClick={() => setShowHelp(false)} style={{ marginTop: 8, width: "100%", background: "var(--accent)", color: "#fff", border: "none", borderRadius: 4, padding: "4px 0", cursor: "pointer", fontSize: 10 }}>Close</button>
        </div>
      )}
    </div>
  );
}

export default function SfcEditor(props: SfcEditorProps) {
  return (
    <ReactFlowProvider>
      <SfcEditorInner {...props} />
    </ReactFlowProvider>
  );
}
