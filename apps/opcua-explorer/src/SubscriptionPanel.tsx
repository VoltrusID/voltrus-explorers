import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { X } from "lucide-react";
import { Button, DataTable, Dialog, Field, StatusDot, type Column } from "@mactools/ui";
import { Sparkline } from "./Sparkline";

interface NotificationValue {
  node_id: string;
  value: any;
  source_ts: number;
  server_ts: number;
  status: string;
}

type Dir = "tx" | "rx" | "info" | "error";

const TRIGGERS = ["Status", "StatusValue", "StatusValueTimestamp"];
const MODES = ["Reporting", "Sampling", "Disabled"];
const COLORS = ["#3b82f6", "#ef4444", "#22c55e", "#f59e0b", "#a855f7", "#ec4899", "#14b8a6", "#f97316"];

const CAP = 500;

export function SubscriptionPanel({
  onClose,
  onLog,
}: {
  onClose: () => void;
  onLog?: (d: Dir, m: string) => void;
}) {
  const [nodes, setNodes] = useState<string[]>([]);
  const [newNode, setNewNode] = useState("");
  const [publishMs, setPublishMs] = useState(500);
  const [samplingMs, setSamplingMs] = useState(500);
  const [queueSize, setQueueSize] = useState(10);
  const [trigger, setTrigger] = useState("StatusValue");
  const [mode, setMode] = useState("Reporting");
  const [deadbandType, setDeadbandType] = useState(0);
  const [deadbandValue, setDeadbandValue] = useState(0);

  const [running, setRunning] = useState(false);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");
  const [live, setLive] = useState<Record<string, NotificationValue>>({});
  const [history, setHistory] = useState<Record<string, number[]>>({});
  const [count, setCount] = useState(0);
  const subIdRef = useRef<number | null>(null);
  const unlistenRef = useRef<UnlistenFn | null>(null);

  // Listen for notifications while running.
  useEffect(() => {
    if (!running) return;
    let active = true;
    listen<NotificationValue>("opcua://notification", (e) => {
      if (!active) return;
      const n = e.payload;
      setLive((prev) => ({ ...prev, [n.node_id]: n }));
      setCount((c) => c + 1);
      const num = typeof n.value === "number" ? n.value : Number(n.value);
      if (!Number.isNaN(num)) {
        setHistory((prev) => {
          const cur = prev[n.node_id] ?? [];
          const next = [...cur, num];
          return { ...prev, [n.node_id]: next.length > CAP ? next.slice(next.length - CAP) : next };
        });
      }
    }).then((fn) => { if (active) unlistenRef.current = fn; });
    return () => { active = false; unlistenRef.current?.(); unlistenRef.current = null; };
  }, [running]);

  const addNode = () => {
    const v = newNode.trim();
    if (!v) return;
    if (!nodes.includes(v)) setNodes((p) => [...p, v]);
    setNewNode("");
  };

  const start = async () => {
    if (nodes.length === 0) { setErr("Add at least one node to monitor"); return; }
    setBusy(true); setErr(""); setLive({}); setHistory({}); setCount(0);
    try {
      const cfg = {
        publishing_interval_ms: publishMs,
        lifetime_count: 60,
        max_keep_alive_count: 10,
        max_notifications_per_publish: 0,
        priority: 0,
        publishing_enabled: true,
      };
      const id = await invoke<number>("create_subscription", { cfg });
      subIdRef.current = id;
      const items = nodes.map((n) => ({
        node_id: n,
        attribute_id: 13,
        sampling_interval_ms: samplingMs,
        queue_size: queueSize,
        discard_oldest: true,
        monitoring_mode: mode,
        trigger,
        deadband_type: deadbandType,
        deadband_value: deadbandValue,
      }));
      const created = await invoke<number>("create_monitored_items", { subscriptionId: id, items });
      setRunning(true);
      onLog?.("tx", `Subscription ${id} created, ${created} monitored items`);
    } catch (e) { setErr(String(e)); onLog?.("error", String(e)); }
    setBusy(false);
  };

  const stop = async () => {
    const id = subIdRef.current;
    if (id !== null) {
      try { await invoke("delete_subscription", { subscriptionId: id }); onLog?.("info", `Subscription ${id} deleted`); }
      catch (e) { onLog?.("error", String(e)); }
    }
    subIdRef.current = null;
    setRunning(false);
  };

  const columns: Column[] = [
    { key: "node_id", label: "Node ID", width: "200px" },
    { key: "value", label: "Value", width: "110px" },
    { key: "status", label: "Status", width: "80px" },
    { key: "trend", label: "Trend" },
  ];
  const rows = nodes.map((n, i) => ({
    node_id: n,
    value: live[n] ? (typeof live[n]!.value === "object" ? JSON.stringify(live[n]!.value) : String(live[n]!.value)) : "—",
    status: live[n]?.status ?? "—",
    trend: live[n]?.value,
    _n: n,
    _i: i,
  }));

  return (
    <Dialog open onClose={onClose} title="Subscriptions and monitored items" width={820}>
      <div style={{ display: "flex", alignItems: "center", gap: 6, minHeight: 20 }}>
        {running && <StatusDot tone="success" label={`Live, ${count} notifications`} />}
        {running && <span style={{ color: "var(--success)", fontSize: "var(--fs-xs)" }}>Live ({count})</span>}
      </div>

      <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr 1fr 1fr", gap: 8 }}>
        <Field label="Publishing (ms)">
          {(id) => <input id={id} className="vt-input vt-input--sm" type="number" disabled={running} value={publishMs} onChange={(e) => setPublishMs(Number(e.target.value))} />}
        </Field>
        <Field label="Sampling (ms)">
          {(id) => <input id={id} className="vt-input vt-input--sm" type="number" disabled={running} value={samplingMs} onChange={(e) => setSamplingMs(Number(e.target.value))} />}
        </Field>
        <Field label="Queue size">
          {(id) => <input id={id} className="vt-input vt-input--sm" type="number" disabled={running} value={queueSize} onChange={(e) => setQueueSize(Number(e.target.value))} />}
        </Field>
        <Field label="Monitoring mode">
          {(id) => (
            <select id={id} className="vt-select vt-select--sm" disabled={running} value={mode} onChange={(e) => setMode(e.target.value)}>
              {MODES.map((m) => <option key={m}>{m}</option>)}
            </select>
          )}
        </Field>
        <Field label="Data change trigger">
          {(id) => (
            <select id={id} className="vt-select vt-select--sm" disabled={running} value={trigger} onChange={(e) => setTrigger(e.target.value)}>
              {TRIGGERS.map((t) => <option key={t}>{t}</option>)}
            </select>
          )}
        </Field>
        <Field label="Deadband type">
          {(id) => (
            <select id={id} className="vt-select vt-select--sm" disabled={running} value={deadbandType} onChange={(e) => setDeadbandType(Number(e.target.value))}>
              <option value={0}>None (0)</option>
              <option value={1}>Absolute (1)</option>
              <option value={2}>Percent (2)</option>
            </select>
          )}
        </Field>
        <Field label="Deadband value">
          {(id) => <input id={id} className="vt-input vt-input--sm" type="number" disabled={running} value={deadbandValue} onChange={(e) => setDeadbandValue(Number(e.target.value))} />}
        </Field>
        <div style={{ display: "flex", alignItems: "flex-end", paddingBottom: 1 }}>
          {!running
            ? <Button variant="primary" disabled={busy} onClick={start}>{busy ? "Starting" : "Start"}</Button>
            : <Button variant="danger" onClick={stop}>Stop</Button>}
        </div>
      </div>

      <div>
        <Field label="Monitored node IDs">
          <div style={{ display: "flex", gap: 6 }}>
            <input
              className="vt-input vt-input--mono"
              style={{ flex: 1 }}
              placeholder="ns=2;s=Reactor.Temperature"
              disabled={running}
              value={newNode}
              aria-label="Node ID to monitor"
              onChange={(e) => setNewNode(e.target.value)}
              onKeyDown={(e) => { if (e.key === "Enter") addNode(); }}
            />
            <Button disabled={running} onClick={addNode}>Add</Button>
          </div>
        </Field>
        <div style={{ display: "flex", flexWrap: "wrap", gap: 4, marginTop: 4 }}>
          {nodes.map((n) => (
            <span key={n} className="mono" style={{ fontSize: "var(--fs-xs)", padding: "2px 6px", borderRadius: "var(--radius-sm)", background: "var(--hover-overlay)", color: "var(--text-muted)", display: "inline-flex", gap: 4, alignItems: "center" }}>
              {n}
              {!running && (
                <button
                  aria-label={`Remove ${n}`}
                  onClick={() => setNodes((p) => p.filter((x) => x !== n))}
                  style={{ background: "none", border: "none", color: "var(--danger)", cursor: "pointer", padding: 0, display: "inline-flex" }}
                >
                  <X size={12} strokeWidth={1.75} />
                </button>
              )}
            </span>
          ))}
        </div>
      </div>

      {err && <div className="vt-hint vt-hint--error" role="alert">{err}</div>}

      <div style={{ flex: 1, minHeight: 200, overflow: "auto" }}>
        <DataTable
          columns={columns}
          data={rows}
          emptyMessage={running ? "Waiting for the first notification" : "Add node IDs, then click Start."}
          renderCell={(key, row) => {
            const r = row as Record<string, unknown>;
            if (key === "node_id") return <span className="mono">{String(r.node_id ?? "")}</span>;
            if (key === "trend") {
              const pts = history[String(r._n)];
              if (!pts || pts.length < 2) return <span style={{ color: "var(--text-dim)" }}>—</span>;
              return <Sparkline points={pts} color={COLORS[Number(r._i) % COLORS.length]!} />;
            }
            return String(r[key] ?? "");
          }}
        />
      </div>
    </Dialog>
  );
}
