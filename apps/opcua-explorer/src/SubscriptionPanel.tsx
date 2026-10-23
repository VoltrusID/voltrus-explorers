import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { X } from "lucide-react";
import { Badge, Button, DataTable, Dialog, Field, IconButton, ListGroup, Section, SheetSelect, StatusDot, useIsPhone, type Column } from "@mactools/ui";
import { fmtValue, quality } from "./phone/format";
import { Sparkline } from "./Sparkline";

interface NotificationValue {
  session: string;
  node_id: string;
  value: any;
  source_ts: number;
  server_ts: number;
  status: string;
}

type Dir = "tx" | "rx" | "info" | "error";

const TRIGGER_OPTIONS = ["Status", "StatusValue", "StatusValueTimestamp"].map((t) => ({ value: t, label: t }));
const MODE_OPTIONS = ["Reporting", "Sampling", "Disabled"].map((m) => ({ value: m, label: m }));
const DEADBAND_OPTIONS = [
  { value: 0, label: "None (0)" },
  { value: 1, label: "Absolute (1)" },
  { value: 2, label: "Percent (2)" },
];
const COLORS = ["#3b82f6", "#ef4444", "#22c55e", "#f59e0b", "#a855f7", "#ec4899", "#14b8a6", "#f97316"];

const CAP = 500;

export function SubscriptionPanel({
  session,
  onClose,
  onLog,
}: {
  /** Id of the client session to subscribe on. */
  session: string;
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
      if (n.session !== session) return;
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
  }, [running, session]);

  // Closing the panel (or switching session) must not leave its subscription running on the server.
  useEffect(() => () => {
    const id = subIdRef.current;
    if (id !== null) invoke("delete_subscription", { session, subscriptionId: id }).catch(() => {});
  }, [session]);

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
      const id = await invoke<number>("create_subscription", { session, cfg });
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
      const created = await invoke<number>("create_monitored_items", { session, subscriptionId: id, items });
      setRunning(true);
      onLog?.("tx", `Subscription ${id} created, ${created} monitored items`);
    } catch (e) { setErr(String(e)); onLog?.("error", String(e)); }
    setBusy(false);
  };

  const stop = async () => {
    const id = subIdRef.current;
    if (id !== null) {
      try { await invoke("delete_subscription", { session, subscriptionId: id }); onLog?.("info", `Subscription ${id} deleted`); }
      catch (e) { onLog?.("error", String(e)); }
    }
    subIdRef.current = null;
    setRunning(false);
  };

  const phone = useIsPhone();
  if (phone) {
    const num = (v: number, set: (n: number) => void) => (id: string) => (
      <input id={id} className="vt-input vt-input--mono" type="number" inputMode="decimal" disabled={running} value={v} onChange={(e) => set(Number(e.target.value))} />
    );
    return (
      <Dialog
        open
        onClose={onClose}
        title="Monitored items"
        footer={
          !running ? (
            <Button variant="primary" size="lg" loading={busy} disabled={nodes.length === 0} onClick={start}>Start subscription</Button>
          ) : (
            <Button variant="danger" size="lg" onClick={stop}>Stop subscription</Button>
          )
        }
      >
        {running && (
          <span className="oc-live vt-hint">
            <StatusDot tone="success" />
            Live, {count} notifications so far.
          </span>
        )}
        {err && <div className="vt-hint vt-hint--error" role="alert">{err}</div>}

        <Section title="Nodes" meta={nodes.length > 0 ? nodes.length : undefined}>
          <div style={{ display: "flex", gap: 8 }}>
            <input
              className="vt-input vt-input--mono"
              style={{ flex: 1, minWidth: 0 }}
              placeholder="ns=2;s=Reactor.Temperature"
              disabled={running}
              value={newNode}
              autoCapitalize="off"
              autoCorrect="off"
              spellCheck={false}
              aria-label="Node ID to monitor"
              onChange={(e) => setNewNode(e.target.value)}
              onKeyDown={(e) => { if (e.key === "Enter") addNode(); }}
            />
            <Button disabled={running || !newNode.trim()} onClick={addNode}>Add</Button>
          </div>
          {nodes.length === 0 ? (
            <span className="vt-hint">Type a node ID and tap Add. Browse shows the IDs of every node.</span>
          ) : (
            <ListGroup className="vm-group--flat">
              {nodes.map((n, i) => {
                const l = live[n];
                const q = l ? quality(l.status, l.value) : null;
                const pts = history[n];
                return (
                  <div key={n} className="vm-row oc-watch" style={running ? undefined : { minHeight: 56 }}>
                    <div className="oc-watch__text">
                      <span className="oc-watch__name" style={{ fontFamily: "var(--font-mono)", fontSize: "var(--fs-sm)" }}>{n}</span>
                      {q && <span className="oc-watch__meta"><Badge tone={q.tone}>{q.label}</Badge></span>}
                    </div>
                    {running && (
                      <div className="oc-watch__value">
                        <span className={l ? "oc-value" : "oc-value oc-value--empty"}>{l ? fmtValue(l.value) : "Waiting"}</span>
                        {pts && pts.length > 1 && <Sparkline points={pts} color={COLORS[i % COLORS.length]!} width={72} height={22} />}
                      </div>
                    )}
                    {!running && (
                      <IconButton label={`Remove ${n}`} onClick={() => setNodes((p) => p.filter((x) => x !== n))}>
                        <X size={18} strokeWidth={1.75} />
                      </IconButton>
                    )}
                  </div>
                );
              })}
            </ListGroup>
          )}
        </Section>

        <Section title="Timing">
          <Field label="Publishing (ms)">{num(publishMs, setPublishMs)}</Field>
          <Field label="Sampling (ms)">{num(samplingMs, setSamplingMs)}</Field>
          <Field label="Queue size">{num(queueSize, setQueueSize)}</Field>
        </Section>

        <Section title="Filter">
          <Field label="Monitoring">
            <SheetSelect sheetTitle="Monitoring mode" disabled={running} value={mode} onChange={setMode} options={[
              { value: "Reporting", label: "Reporting", hint: "Sample and report changes" },
              { value: "Sampling", label: "Sampling", hint: "Sample but do not report" },
              { value: "Disabled", label: "Disabled", hint: "Neither sample nor report" },
            ]} />
          </Field>
          <Field label="Trigger">
            <SheetSelect sheetTitle="Data change trigger" disabled={running} value={trigger} onChange={setTrigger} options={[
              { value: "Status", label: "Status", hint: "When the quality changes" },
              { value: "StatusValue", label: "Status and value", hint: "When quality or value changes" },
              { value: "StatusValueTimestamp", label: "Status, value and time", hint: "Also when the timestamp changes" },
            ]} />
          </Field>
          <Field label="Deadband">
            <SheetSelect sheetTitle="Deadband type" disabled={running} value={deadbandType} onChange={setDeadbandType} options={[
              { value: 0, label: "None", hint: "Report every change" },
              { value: 1, label: "Absolute", hint: "Ignore changes below the value" },
              { value: 2, label: "Percent", hint: "Ignore changes below the value, in %" },
            ]} />
          </Field>
          {deadbandType !== 0 && <Field label="Deadband value">{num(deadbandValue, setDeadbandValue)}</Field>}
        </Section>
      </Dialog>
    );
  }

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
            <SheetSelect id={id} size="sm" sheetTitle="Monitoring mode" disabled={running} value={mode} options={MODE_OPTIONS} onChange={setMode} />
          )}
        </Field>
        <Field label="Data change trigger">
          {(id) => (
            <SheetSelect id={id} size="sm" sheetTitle="Data change trigger" disabled={running} value={trigger} options={TRIGGER_OPTIONS} onChange={setTrigger} />
          )}
        </Field>
        <Field label="Deadband type">
          {(id) => (
            <SheetSelect id={id} size="sm" sheetTitle="Deadband type" disabled={running} value={deadbandType} options={DEADBAND_OPTIONS} onChange={setDeadbandType} />
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
