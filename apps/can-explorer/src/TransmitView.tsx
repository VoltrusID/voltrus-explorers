import { invoke } from "@tauri-apps/api/core";
import { Badge, Button, Callout, Field, IconButton, ProBadge } from "@mactools/ui";
import { Play, Plus, Send, Square, Trash2 } from "lucide-react";
import { useState } from "react";
import { frameInOf, type TxItem } from "./can";
import { card, hint, row, type AddLog, type RequirePro } from "./ui";

interface Props {
  items: TxItem[];
  setItems: (items: TxItem[]) => void;
  /** Keys of the items sending cyclically. */
  running: Set<string>;
  setRunning: (fn: (prev: Set<string>) => Set<string>) => void;
  canSend: boolean;
  requirePro: RequirePro;
  addLog: AddLog;
}

let seq = 0;
const newKey = () => `tx-${Date.now().toString(36)}-${seq++}`;

export default function TransmitView({ items, setItems, running, setRunning, canSend, requirePro, addLog }: Props) {
  const [quick, setQuick] = useState("7DF#02010D0000000000");
  const [error, setError] = useState<string | null>(null);

  const update = (key: string, patch: Partial<TxItem>) => setItems(items.map((t) => (t.key === key ? { ...t, ...patch } : t)));

  const sendOnce = async (t: TxItem) => {
    setError(null);
    const f = frameInOf(t);
    if (typeof f === "string") { setError(`${t.name || t.id}: ${f}`); return; }
    try { await invoke("can_send", { frame: f }); } catch (e) { setError(String(e)); addLog("error", String(e)); }
  };

  const start = async (t: TxItem) => {
    if (!requirePro("Cyclic transmit")) return;
    setError(null);
    const f = frameInOf(t);
    if (typeof f === "string") { setError(`${t.name || t.id}: ${f}`); return; }
    try {
      await invoke("can_periodic_start", { key: t.key, frame: f, periodMs: Math.max(1, Math.round(t.periodMs)), count: null });
      setRunning((prev) => new Set(prev).add(t.key));
      addLog("info", `Sending ${t.id} every ${t.periodMs} ms`);
    } catch (e) {
      setError(String(e));
    }
  };

  const stop = async (key: string) => {
    await invoke("can_periodic_stop", { key });
    setRunning((prev) => { const n = new Set(prev); n.delete(key); return n; });
  };

  const sendQuick = async () => {
    setError(null);
    try { await invoke("can_send_text", { text: quick }); } catch (e) { setError(String(e)); }
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", flex: 1, minHeight: 0, gap: 8, overflow: "auto" }}>
      {!canSend && <Callout tone="warn">The bus is open in listen-only mode: nothing can be sent. Reconnect without listen-only to transmit.</Callout>}
      {error && <Callout tone="danger">{error}</Callout>}
      <div style={{ ...card, display: "flex", flexDirection: "column", gap: 6 }}>
        <strong style={{ fontSize: "var(--fs-sm)" }}>Send one frame</strong>
        <div style={row}>
          <input
            className="vt-input vt-input--mono"
            aria-label="Frame in cansend syntax"
            value={quick}
            spellCheck={false}
            onChange={(e) => setQuick(e.target.value)}
            onKeyDown={(e) => { if (e.key === "Enter") void sendQuick(); }}
            style={{ flex: 1, minWidth: 240 }}
          />
          <Button variant="primary" icon={<Send size={14} strokeWidth={1.75} />} disabled={!canSend || !quick.trim()} onClick={() => void sendQuick()}>Send</Button>
        </div>
        <span style={hint}>
          cansend syntax: <span className="mono">123#DEADBEEF</span> (11-bit), <span className="mono">18FEF100#0102</span> (29-bit, eight digits),{" "}
          <span className="mono">7DF#R</span> (remote), <span className="mono">123##1AABBCC</span> (CAN FD, flag 1 = bit rate switch).
        </span>
      </div>

      <div style={{ ...card, display: "flex", flexDirection: "column", gap: 6, flex: 1, minHeight: 200 }}>
        <div style={row}>
          <strong style={{ fontSize: "var(--fs-sm)" }}>Transmit list</strong>
          <span style={hint}>Send once is free; cyclic sending is <ProBadge /></span>
          <span style={{ flex: 1 }} />
          {running.size > 0 && (
            <Button size="sm" variant="danger" icon={<Square size={14} strokeWidth={1.75} />} onClick={() => { for (const k of running) void stop(k); }}>
              Stop all ({running.size})
            </Button>
          )}
          <Button
            size="sm"
            icon={<Plus size={14} strokeWidth={1.75} />}
            onClick={() => setItems([...items, { key: newKey(), name: "", id: "100", fd: false, brs: false, rtr: false, data: "00 00 00 00 00 00 00 00", periodMs: 100 }])}
          >
            Add frame
          </Button>
        </div>
        <div style={{ overflow: "auto" }}>
          <table className="vt-table" style={{ width: "100%" }}>
            <thead>
              <tr>
                <th>Name</th>
                <th style={{ width: 110 }}>ID (hex)</th>
                <th>Data (hex)</th>
                <th style={{ width: 120 }}>Flags</th>
                <th style={{ width: 96 }}>Period (ms)</th>
                <th style={{ width: 200 }} />
              </tr>
            </thead>
            <tbody>
              {items.map((t) => {
                const on = running.has(t.key);
                const problem = frameInOf(t);
                return (
                  <tr key={t.key}>
                    <td><input className="vt-input" aria-label="Name" value={t.name} placeholder="Name" onChange={(e) => update(t.key, { name: e.target.value })} disabled={on} /></td>
                    <td><input className="vt-input vt-input--mono" aria-label="Identifier" value={t.id} onChange={(e) => update(t.key, { id: e.target.value })} disabled={on} /></td>
                    <td>
                      <input className="vt-input vt-input--mono" aria-label="Data" value={t.data} disabled={on || t.rtr} onChange={(e) => update(t.key, { data: e.target.value })} />
                      {typeof problem === "string" && <div style={{ ...hint, color: "var(--danger-text, var(--danger))" }}>{problem}</div>}
                    </td>
                    <td style={{ fontSize: "var(--fs-xs)" }}>
                      <label style={{ marginRight: 6 }}><input type="checkbox" checked={t.fd} disabled={on} onChange={(e) => update(t.key, { fd: e.target.checked, brs: e.target.checked && t.brs, rtr: false })} /> FD</label>
                      <label style={{ marginRight: 6 }}><input type="checkbox" checked={t.brs} disabled={on || !t.fd} onChange={(e) => update(t.key, { brs: e.target.checked })} /> BRS</label>
                      <label><input type="checkbox" checked={t.rtr} disabled={on || t.fd} onChange={(e) => update(t.key, { rtr: e.target.checked })} /> RTR</label>
                    </td>
                    <td><input className="vt-input vt-input--mono" type="number" min={1} aria-label="Period in milliseconds" value={t.periodMs} disabled={on} onChange={(e) => update(t.key, { periodMs: Number(e.target.value) })} /></td>
                    <td>
                      <div style={{ display: "flex", gap: 4, justifyContent: "flex-end" }}>
                        <Button size="sm" icon={<Send size={13} strokeWidth={1.75} />} disabled={!canSend || typeof problem === "string"} onClick={() => void sendOnce(t)}>Send</Button>
                        {on ? (
                          <Button size="sm" variant="danger" icon={<Square size={13} strokeWidth={1.75} />} onClick={() => void stop(t.key)}>Stop</Button>
                        ) : (
                          <Button size="sm" icon={<Play size={13} strokeWidth={1.75} />} disabled={!canSend || typeof problem === "string"} onClick={() => void start(t)}>Cyclic</Button>
                        )}
                        <IconButton label="Remove" disabled={on} onClick={() => setItems(items.filter((x) => x.key !== t.key))}>
                          <Trash2 size={14} strokeWidth={1.75} />
                        </IconButton>
                      </div>
                      {on && <Badge tone="success">every {t.periodMs} ms</Badge>}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      </div>
    </div>
  );
}
