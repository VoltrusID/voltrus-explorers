import { useMemo, useState } from "react";
import { ChevronDown, ChevronRight } from "lucide-react";
import { Badge } from "@mactools/ui";
import type { PayloadInfo } from "./payload";

// A single latest-value-per-topic entry the tree is built from.
export interface TreeEntry extends PayloadInfo {
  topic: string;
  count: number;
  ts: number;
  retain: boolean;
}

interface TopicTreeProps {
  entries: TreeEntry[];
  formatPayload: (p: PayloadInfo) => string;
  onLeafClick?: (topic: string) => void;
  blinkingTopics?: Set<string>;
}

interface TreeNode {
  name: string;
  path: string;
  count: number; // aggregate messages routed through this node
  children: Map<string, TreeNode>;
  // Leaf-only (set when `path` is a real published topic):
  info?: PayloadInfo;
  ts?: number;
  retain?: boolean;
}


function buildTree(entries: TreeEntry[]): TreeNode | null {
  if (entries.length === 0) return null;
  const root: TreeNode = { name: "", path: "", count: 0, children: new Map() };
  for (const e of entries) {
    const levels = e.topic.split("/").filter((s) => s.length > 0);
    let cur = root;
    let path = "";
    for (const level of levels) {
      path = path ? `${path}/${level}` : level;
      let child = cur.children.get(level);
      if (!child) {
        child = { name: level, path, count: 0, children: new Map() };
        cur.children.set(level, child);
      }
      child.count += e.count;
      cur = child;
    }
    // Keep the newest payload seen at this exact topic.
    if (cur.ts === undefined || e.ts > cur.ts) {
      cur.info = { payload: e.payload, rawHex: e.rawHex, decoded: e.decoded };
      cur.ts = e.ts;
      cur.retain = e.retain;
    }
  }
  return root;
}

function sortNodes(list: TreeNode[]): TreeNode[] {
  return list.sort((a, b) => a.name.localeCompare(b.name, undefined, { numeric: true }));
}

function NodeRow({
  node,
  depth,
  formatPayload,
  onLeafClick,
  blinkingTopics,
}: {
  node: TreeNode;
  depth: number;
  formatPayload: (p: PayloadInfo) => string;
  onLeafClick?: (t: string) => void;
  blinkingTopics?: Set<string>;
}) {
  const [open, setOpen] = useState(true);
  const children = useMemo(() => sortNodes(Array.from(node.children.values())), [node]);
  const hasChildren = children.length > 0;
  const isLeaf = node.info !== undefined;
  const blink = blinkingTopics?.has(node.path);

  return (
    <>
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: 6,
          padding: "2px 6px",
          paddingLeft: 6 + depth * 14,
          borderRadius: "var(--radius-sm)",
          cursor: isLeaf ? "pointer" : "default",
          background: blink ? "var(--accent)" : "transparent",
          color: blink ? "var(--accent-fg)" : "var(--text)",
          transition: "background 0.15s",
        }}
        onClick={() => isLeaf && onLeafClick?.(node.path)}
        title={isLeaf ? `Publish to ${node.path}` : undefined}
      >
        {hasChildren ? (
          <span
            onClick={(e) => {
              e.stopPropagation();
              setOpen((o) => !o);
            }}
            style={{ cursor: "pointer", color: "var(--text-muted)", flexShrink: 0, display: "flex" }}
          >
            {open
              ? <ChevronDown size={12} strokeWidth={1.75} />
              : <ChevronRight size={12} strokeWidth={1.75} />}
          </span>
        ) : (
          <span style={{ width: 12, flexShrink: 0 }} />
        )}
        <span
          style={{
            fontFamily: "var(--font-mono)",
            fontSize: "var(--fs-xs)",
            fontWeight: hasChildren ? 600 : 400,
            flexShrink: 0,
            maxWidth: "40%",
            whiteSpace: "nowrap",
            overflow: "hidden",
            textOverflow: "ellipsis",
          }}
        >
          {node.name}
        </span>
        {isLeaf && node.retain && <Badge tone="info" mono>R</Badge>}
        {isLeaf && (
          <span
            style={{
              flex: 1,
              fontFamily: "var(--font-mono)",
              fontSize: "var(--fs-xs)",
              color: blink ? "var(--accent-fg)" : "var(--text-muted)",
              whiteSpace: "nowrap",
              overflow: "hidden",
              textOverflow: "ellipsis",
            }}
          >
            {node.info ? formatPayload(node.info) : ""}
          </span>
        )}
        <span
          style={{
            flexShrink: 0,
            fontSize: 10,
            fontFamily: "var(--font-mono)",
            fontVariantNumeric: "tabular-nums",
            color: blink ? "var(--accent-fg)" : "var(--text-muted)",
          }}
        >
          {node.count > 0 ? node.count : ""}
        </span>
      </div>
      {hasChildren &&
        open &&
        children.map((c) => (
          <NodeRow
            key={c.path}
            node={c}
            depth={depth + 1}
            formatPayload={formatPayload}
            onLeafClick={onLeafClick}
            blinkingTopics={blinkingTopics}
          />
        ))}
    </>
  );
}

export function TopicTree({ entries, formatPayload, onLeafClick, blinkingTopics }: TopicTreeProps) {
  const root = useMemo(() => buildTree(entries), [entries]);
  if (!root) return null;
  const top = sortNodes(Array.from(root.children.values()));
  return (
    <div style={{ overflow: "auto", flex: 1, minHeight: 0 }}>
      {top.map((n) => (
        <NodeRow
          key={n.path}
          node={n}
          depth={0}
          formatPayload={formatPayload}
          onLeafClick={onLeafClick}
          blinkingTopics={blinkingTopics}
        />
      ))}
    </div>
  );
}
