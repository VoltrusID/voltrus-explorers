import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ChevronDown, ChevronRight, Loader2, Network } from "lucide-react";
import { Button, DataTable, Dialog, EmptyState, Tabs, type Column } from "@mactools/ui";

export interface BrowseChild {
  node_id: string;
  browse_name: string;
  display_name: string;
  node_class: string;
  data_type: string;
  is_variable: boolean;
}
export interface AttributeRead {
  attribute: string;
  value: any;
  status: string;
}
export interface ReferenceInfo {
  reference_type: string;
  is_forward: boolean;
  node_id: string;
  browse_name: string;
  display_name: string;
  node_class: string;
}

type Dir = "tx" | "rx" | "info" | "error";

const classColor = (nc: string): string =>
  nc === "Variable" ? "var(--accent)" : nc === "Object" ? "var(--success)" : "var(--text-dim)";

export function BrowserPanel({
  onClose,
  onLog,
}: {
  onClose: () => void;
  onLog?: (d: Dir, m: string) => void;
}) {
  const [children, setChildren] = useState<Record<string, BrowseChild[]>>({});
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [selected, setSelected] = useState<string | null>(null);
  const [attrs, setAttrs] = useState<AttributeRead[]>([]);
  const [refs, setRefs] = useState<ReferenceInfo[]>([]);
  const [tab, setTab] = useState<"attributes" | "references">("attributes");
  const [busy, setBusy] = useState<string | null>(null);

  const loadChildren = useCallback(async (parentId: string) => {
    setBusy(parentId);
    try {
      const kids = await invoke<BrowseChild[]>("browse_children", { nodeId: parentId });
      setChildren((prev) => ({ ...prev, [parentId]: kids }));
      onLog?.("rx", `Browsed ${kids.length} children under ${parentId || "ObjectsFolder"}`);
    } catch (e) { onLog?.("error", `Browse failed: ${String(e)}`); }
    setBusy(null);
  }, [onLog]);

  useEffect(() => { loadChildren(""); /* eslint-disable-next-line */ }, [loadChildren]);

  const toggle = (id: string) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else { next.add(id); if (children[id] === undefined) loadChildren(id); }
      return next;
    });
  };

  const selectNode = async (id: string) => {
    setSelected(id);
    try {
      const [a, r] = await Promise.all([
        invoke<AttributeRead[]>("read_attributes", { nodeId: id }).catch((e) => { onLog?.("error", `read_attributes: ${e}`); return []; }),
        invoke<ReferenceInfo[]>("browse_references", { nodeId: id }).catch((e) => { onLog?.("error", `browse_references: ${e}`); return []; }),
      ]);
      setAttrs(a); setRefs(r);
    } catch (e) { onLog?.("error", String(e)); }
  };

  const renderNode = (node: BrowseChild, depth: number): React.ReactNode => {
    const kids = children[node.node_id];
    const isOpen = expanded.has(node.node_id);
    const isSel = selected === node.node_id;
    return (
      <div key={node.node_id}>
        <div
          style={{
            display: "flex", alignItems: "center", gap: 6, padding: "3px 4px", cursor: "pointer",
            paddingLeft: 4 + depth * 14, background: isSel ? "var(--hover-overlay)" : "transparent",
            borderRadius: "var(--radius-sm)",
          }}
          onClick={() => selectNode(node.node_id)}
        >
          <span
            role="button"
            aria-label={isOpen ? `Collapse ${node.display_name}` : `Expand ${node.display_name}`}
            tabIndex={-1}
            onClick={(e) => { e.stopPropagation(); toggle(node.node_id); }}
            style={{ width: 14, height: 14, display: "inline-flex", alignItems: "center", justifyContent: "center", color: "var(--text-muted)" }}
          >
            {busy === node.node_id
              ? <Loader2 size={14} strokeWidth={1.75} />
              : isOpen ? <ChevronDown size={14} strokeWidth={1.75} /> : <ChevronRight size={14} strokeWidth={1.75} />}
          </span>
          <span style={{ flex: 1, color: "var(--text)", fontSize: "var(--fs-sm)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
            {node.display_name || node.browse_name || node.node_id}
          </span>
          <span style={{ fontSize: "var(--fs-xs)", color: classColor(node.node_class), padding: "1px 5px", borderRadius: "var(--radius-sm)", background: "var(--hover-overlay)" }}>
            {node.node_class}{node.data_type ? ` · ${node.data_type}` : ""}
          </span>
        </div>
        {isOpen && kids?.map((k) => renderNode(k, depth + 1))}
      </div>
    );
  };

  const refColumns: Column[] = [
    { key: "reference_type", label: "Reference", width: "150px" },
    { key: "dir", label: "Dir", width: "60px" },
    { key: "node_class", label: "Class", width: "90px" },
    { key: "display_name", label: "Target" },
    { key: "node_id", label: "NodeId", width: "180px" },
  ];
  const refRows = refs.map((r) => ({
    reference_type: r.reference_type,
    dir: r.is_forward ? "fwd" : "inv",
    node_class: r.node_class,
    display_name: r.display_name || r.browse_name,
    node_id: r.node_id,
    _id: r.node_id,
  }));

  const tree = children[""];
  const treeEmpty = !tree || tree.length === 0;

  return (
    <Dialog open onClose={onClose} title="Namespace browser" width={920}>
      <div style={{ display: "flex", justifyContent: "flex-end" }}>
        <Button size="sm" onClick={() => loadChildren("")}>Reload root</Button>
      </div>

      <div style={{ display: "flex", flex: 1, minHeight: 320, gap: 8 }}>
        {/* Tree */}
        <div
          style={{
            flex: "0 0 48%", overflow: "auto", border: "1px solid var(--border)",
            borderRadius: "var(--radius-lg)", background: "var(--bg)", padding: 8,
          }}
        >
          {treeEmpty ? (
            <EmptyState
              compact
              icon={<Network size={18} strokeWidth={1.75} />}
              title="Nothing to browse"
              description="Connect to a server first, then reopen the browser to walk its address space."
            />
          ) : tree.map((n) => renderNode(n, 0))}
        </div>

        {/* Detail */}
        <div style={{ flex: 1, display: "flex", flexDirection: "column", minWidth: 0 }}>
          <div style={{ marginBottom: 6 }}>
            <Tabs
              label="Node detail view"
              value={tab}
              onChange={setTab}
              items={[
                { value: "attributes", label: "Attributes" },
                { value: "references", label: "References", count: refs.length > 0 ? refs.length : undefined },
              ]}
            />
          </div>

          {!selected ? (
            <div
              style={{ ...{ flex: 1, overflow: "auto", border: "1px solid var(--border)", borderRadius: "var(--radius-lg)", background: "var(--bg)", padding: 8 } }}
            >
              <EmptyState compact title="No node selected" description="Select a node in the tree to view its attributes and references." />
            </div>
          ) : tab === "attributes" ? (
            <div style={{ flex: 1, overflow: "auto", border: "1px solid var(--border)", borderRadius: "var(--radius-lg)", background: "var(--bg)", padding: 8 }}>
              <div className="mono" style={{ color: "var(--text-muted)", marginBottom: 8 }}>{selected}</div>
              {attrs.map((a) => (
                <div key={a.attribute} style={{ display: "flex", gap: 10, padding: "4px 0", borderBottom: "1px solid var(--border)" }}>
                  <span style={{ width: 120, color: "var(--text-muted)", fontSize: "var(--fs-xs)" }}>{a.attribute}</span>
                  <span className="mono" style={{ flex: 1, color: a.status === "Good" ? "var(--text)" : "var(--danger)", wordBreak: "break-all" }}>
                    {a.value === null ? "—" : typeof a.value === "object" ? JSON.stringify(a.value) : String(a.value)}
                  </span>
                </div>
              ))}
              {attrs.length === 0 && <span className="vt-hint">No attributes readable on this node.</span>}
            </div>
          ) : (
            <div style={{ flex: 1, minHeight: 0 }}>
              <DataTable
                columns={refColumns}
                data={refRows}
                emptyMessage="No references"
                renderCell={(key, row) => (key === "node_id" ? <span className="mono">{String(row[key] ?? "")}</span> : String(row[key] ?? ""))}
                onRowClick={(row) => selectNode(String((row as Record<string, unknown>)._id))}
              />
            </div>
          )}
        </div>
      </div>
    </Dialog>
  );
}
