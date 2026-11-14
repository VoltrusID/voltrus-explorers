import { useState } from "react";
import { ChevronDown, ChevronRight, RefreshCw, Factory } from "lucide-react";
import { Badge, Button, EmptyState } from "@mactools/ui";
import {
  sortedMetrics,
  sortedNodes,
  type SpEdgeNode,
  type SpMetric,
  type SparkplugState,
} from "./sparkplug";

const mono: React.CSSProperties = { fontFamily: "var(--font-mono)", fontSize: "var(--fs-xs)" };
const muted: React.CSSProperties = { color: "var(--text-muted)" };

function valueText(v: unknown): string {
  if (v === null || v === undefined) return "null";
  if (typeof v === "string") return v;
  if (typeof v === "number" || typeof v === "boolean") return String(v);
  return JSON.stringify(v);
}

function timeText(ts?: number): string {
  if (ts === undefined) return "";
  const d = new Date(ts);
  return Number.isNaN(d.getTime()) ? "" : d.toLocaleTimeString();
}

function MetricRows({ metrics, depth }: { metrics: Record<string, SpMetric>; depth: number }) {
  const list = sortedMetrics(metrics);
  if (list.length === 0) {
    return <div style={{ ...mono, ...muted, paddingLeft: 6 + depth * 14 }}>No metrics yet</div>;
  }
  return (
    <>
      {list.map((m) => (
        <div
          key={m.name}
          style={{
            display: "grid",
            gridTemplateColumns: "minmax(120px, 2fr) minmax(80px, 2fr) 70px 46px 80px",
            gap: 8,
            padding: "2px 6px",
            paddingLeft: 6 + depth * 14,
            alignItems: "baseline",
          }}
        >
          <span style={{ ...mono, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }} title={m.name}>
            {m.name}
          </span>
          <span
            style={{ ...mono, fontWeight: 600, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}
            title={valueText(m.value)}
          >
            {valueText(m.value)}
          </span>
          <span style={{ ...mono, ...muted }}>{m.datatype ?? ""}</span>
          <span style={{ ...mono, ...muted }} title="Alias">{m.alias !== undefined ? `#${m.alias}` : ""}</span>
          <span style={{ ...mono, ...muted, fontVariantNumeric: "tabular-nums" }}>{timeText(m.ts)}</span>
        </div>
      ))}
    </>
  );
}

function stateBadge(online: boolean | undefined) {
  if (online === undefined) return <Badge tone="warn">no birth seen</Badge>;
  return online ? <Badge tone="success">online</Badge> : <Badge tone="danger">offline</Badge>;
}

function EdgeNodeRow({
  node,
  onRebirth,
}: {
  node: SpEdgeNode;
  onRebirth?: (group: string, edge: string) => void;
}) {
  const [open, setOpen] = useState(true);
  const [showIssues, setShowIssues] = useState(false);
  const devices = Object.values(node.devices).sort((a, b) => a.id.localeCompare(b.id));
  const Chevron = open ? ChevronDown : ChevronRight;
  return (
    <div style={{ borderBottom: "1px solid var(--border)", padding: "4px 0" }}>
      <div style={{ display: "flex", alignItems: "center", gap: 6, padding: "2px 6px", flexWrap: "wrap" }}>
        <button
          type="button"
          onClick={() => setOpen((o) => !o)}
          aria-label={open ? "Collapse" : "Expand"}
          style={{ background: "none", border: 0, padding: 0, cursor: "pointer", color: "var(--text-muted)", display: "flex" }}
        >
          <Chevron size={12} strokeWidth={1.75} />
        </button>
        <span style={{ ...mono, ...muted }}>{node.group} /</span>
        <span style={{ ...mono, fontWeight: 600 }}>{node.id}</span>
        {stateBadge(node.online)}
        <span style={{ ...mono, ...muted }} title="Birth/death sequence of the current session">
          bdSeq {node.bdSeq ?? "?"}
        </span>
        <span style={{ ...mono, ...muted }} title="Sequence number of the last message">
          seq {node.seq ?? "?"}
        </span>
        {node.issues.length > 0 && (
          <button
            type="button"
            onClick={() => setShowIssues((v) => !v)}
            style={{ background: "none", border: 0, padding: 0, cursor: "pointer" }}
            title="Show what the seq/bdSeq checks found"
          >
            <Badge tone="warn">
              {node.issues.length} issue{node.issues.length !== 1 ? "s" : ""}
            </Badge>
          </button>
        )}
        <span style={{ flex: 1 }} />
        {onRebirth && (
          <Button
            size="sm"
            variant="ghost"
            icon={<RefreshCw size={12} />}
            onClick={() => onRebirth(node.group, node.id)}
            title={`Publish NCMD "Node Control/Rebirth" to spBv1.0/${node.group}/NCMD/${node.id}`}
          >
            Rebirth
          </Button>
        )}
      </div>
      {showIssues && (
        <div style={{ padding: "2px 6px 6px 26px", display: "flex", flexDirection: "column", gap: 2 }}>
          {node.issues
            .slice()
            .reverse()
            .map((i, n) => (
              <span key={n} style={{ ...mono, color: "var(--warn, #b45309)" }}>
                {timeText(i.ts)} {i.topic.split("/")[2]}
                {i.topic.split("/")[4] ? ` ${i.topic.split("/")[4]}` : ""}: {i.text}
              </span>
            ))}
        </div>
      )}
      {open && (
        <>
          <MetricRows metrics={node.metrics} depth={1} />
          {devices.map((d) => (
            <div key={d.id} style={{ marginTop: 4 }}>
              <div style={{ display: "flex", alignItems: "center", gap: 6, padding: "2px 6px", paddingLeft: 20 }}>
                <span style={{ ...mono, fontWeight: 600 }}>{d.id}</span>
                <Badge tone={d.online ? "success" : "danger"}>{d.online ? "online" : "offline"}</Badge>
              </div>
              <MetricRows metrics={d.metrics} depth={2} />
            </div>
          ))}
        </>
      )}
    </div>
  );
}

export function SparkplugPanel({
  state,
  onRebirth,
}: {
  state: SparkplugState;
  /** Absent when not connected. */
  onRebirth?: (group: string, edge: string) => void;
}) {
  const nodes = sortedNodes(state);
  const hosts = Object.values(state.hosts).sort((a, b) => a.id.localeCompare(b.id));
  if (nodes.length === 0 && hosts.length === 0) {
    return (
      <EmptyState
        compact
        icon={<Factory size={18} strokeWidth={1.75} />}
        title="No Sparkplug B traffic yet"
        description="Subscribe to spBv1.0/# to see edge nodes, devices and metrics. Births that happened before you subscribed are not resent: use Rebirth on an edge node to get its metric names."
      />
    );
  }
  return (
    <div style={{ overflow: "auto", flex: 1, minHeight: 0 }}>
      {hosts.length > 0 && (
        <div style={{ display: "flex", gap: 8, alignItems: "center", padding: "4px 6px", flexWrap: "wrap" }}>
          <span style={{ ...mono, ...muted }}>Primary host</span>
          {hosts.map((h) => (
            <span key={h.id} style={{ display: "inline-flex", gap: 4, alignItems: "center" }}>
              <span style={mono}>{h.id}</span>
              <Badge tone={h.online ? "success" : "danger"}>{h.online ? "online" : "offline"}</Badge>
            </span>
          ))}
        </div>
      )}
      {nodes.map((n) => (
        <EdgeNodeRow key={`${n.group}/${n.id}`} node={n} onRebirth={onRebirth} />
      ))}
    </div>
  );
}
