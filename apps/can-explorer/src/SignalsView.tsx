import { invoke } from "@tauri-apps/api/core";
import { Badge, Button, Callout, EmptyState, ProBadge } from "@mactools/ui";
import { FileText, Send } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { cansendText, fmtNum, hex, idText } from "./can";
import type { DbcMessageView, DbcView, FrameView } from "./types";
import { card, hint, row, th, type AddLog, type RequirePro } from "./ui";

interface Props {
  dbc: DbcView | null;
  /** Latest decoded values per message name. */
  latest: Map<string, FrameView>;
  canSend: boolean;
  connected: boolean;
  requirePro: RequirePro;
  addLog: AddLog;
  onLoad: () => void;
  onLoadSim: () => void;
}

export default function SignalsView({ dbc, latest, canSend, connected, requirePro, addLog, onLoad, onLoadSim }: Props) {
  const [query, setQuery] = useState("");
  const [sel, setSel] = useState<string | null>(null);
  const messages = useMemo(() => {
    const q = query.trim().toLowerCase();
    const all = dbc?.messages ?? [];
    return q ? all.filter((m) => m.name.toLowerCase().includes(q) || idText(m.id, m.ext).toLowerCase().includes(q) || m.signals.some((s) => s.name.toLowerCase().includes(q))) : all;
  }, [dbc, query]);
  const current = dbc?.messages.find((m) => m.name === sel) ?? messages[0] ?? null;

  if (!dbc) {
    return (
      <EmptyState
        icon={<FileText size={20} strokeWidth={1.75} />}
        title="No DBC file loaded"
        description="A DBC file names the messages and signals on a bus. Load one to decode every frame, or load the simulator's to try it."
        action={
          <div style={{ display: "flex", gap: 8 }}>
            <Button variant="primary" onClick={onLoad}>Open DBC file</Button>
            <Button onClick={onLoadSim}>Use the simulator's DBC</Button>
          </div>
        }
      />
    );
  }

  return (
    <div style={{ display: "flex", flex: 1, minHeight: 0, gap: 8 }}>
      <div style={{ ...card, width: 270, flexShrink: 0, display: "flex", flexDirection: "column", gap: 6, padding: 8 }}>
        <input className="vt-input" placeholder="Find message or signal" aria-label="Find message or signal" value={query} onChange={(e) => setQuery(e.target.value)} />
        <span style={hint}>{dbc.name}: {dbc.messages.length} messages{dbc.version ? `, version ${dbc.version}` : ""}</span>
        <div style={{ overflow: "auto", flex: 1 }}>
          {messages.map((m) => {
            const live = latest.has(m.name);
            return (
              <button
                key={m.name}
                type="button"
                onClick={() => setSel(m.name)}
                className="vt-btn vt-btn--ghost vt-btn--sm"
                style={{ width: "100%", justifyContent: "flex-start", gap: 6, background: current?.name === m.name ? "var(--hover, rgba(127,127,127,.15))" : undefined }}
              >
                <span className="mono" style={{ color: "var(--text-muted)", width: 70, textAlign: "left" }}>{idText(m.id, m.ext)}</span>
                <span style={{ flex: 1, textAlign: "left", overflow: "hidden", textOverflow: "ellipsis" }}>{m.name}</span>
                {live && <span className="vt-dot vt-dot--success" aria-label="Seen on the bus" />}
              </button>
            );
          })}
        </div>
      </div>
      {current && <MessageDetail key={current.name} m={current} frame={latest.get(current.name) ?? null} canSend={canSend && connected} requirePro={requirePro} addLog={addLog} />}
    </div>
  );
}

function MessageDetail({ m, frame, canSend, requirePro, addLog }: { m: DbcMessageView; frame: FrameView | null; canSend: boolean; requirePro: RequirePro; addLog: AddLog }) {
  const values = new Map((frame?.sigs ?? []).map(([n, v, u, l]) => [n, { v, u, l }]));
  const [draft, setDraft] = useState<Record<string, string>>({});
  const [preview, setPreview] = useState<FrameView | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Start the editor from the live values (or the signal minimum).
  useEffect(() => {
    setDraft(Object.fromEntries(m.signals.map((s) => [s.name, String(values.get(s.name)?.v ?? Math.max(s.min, Math.min(s.max, 0)))])));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [m.name]);

  const encode = async (): Promise<FrameView | null> => {
    setError(null);
    const vals: Record<string, number> = {};
    for (const [k, v] of Object.entries(draft)) {
      const n = Number(v);
      if (!Number.isFinite(n)) { setError(`${k}: "${v}" is not a number`); return null; }
      vals[k] = n;
    }
    try {
      const f = await invoke<FrameView>("dbc_encode", { message: m.name, values: vals });
      setPreview(f);
      return f;
    } catch (e) {
      setError(String(e));
      return null;
    }
  };

  const send = async () => {
    if (!requirePro("Sending by signal values")) return;
    const f = await encode();
    if (!f) return;
    try {
      await invoke("can_send", { frame: { id: f.id, ext: f.ext, rtr: false, fd: f.fd, brs: f.brs, data: hex(f.data), rtrLen: 0 } });
      addLog("tx", `${m.name}: ${hex(f.data)}`);
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column", gap: 8, overflow: "auto" }}>
      <div style={{ ...card, display: "flex", flexDirection: "column", gap: 4 }}>
        <div style={row}>
          <strong>{m.name}</strong>
          <Badge mono>{idText(m.id, m.ext)}</Badge>
          <Badge>{m.ext ? "29-bit" : "11-bit"}</Badge>
          <Badge>{m.size} bytes</Badge>
          {m.fd && <Badge tone="info">CAN FD{m.brs ? " · BRS" : ""}</Badge>}
          {m.cycleMs != null && <Badge>every {m.cycleMs} ms</Badge>}
          {m.transmitter && <span style={hint}>from {m.transmitter}</span>}
          <span style={{ flex: 1 }} />
          {frame ? (
            // CAN FD payloads wrap instead of running off the card.
            <Badge tone="success" mono style={frame.data.length > 8 ? { whiteSpace: "normal", flexBasis: "100%", lineHeight: 1.5 } : undefined}>{hex(frame.data)}</Badge>
          ) : <Badge tone="neutral">not seen on the bus</Badge>}
        </div>
        {m.comment && <span style={hint}>{m.comment}</span>}
      </div>
      <div style={{ ...card, padding: 0, overflow: "auto" }}>
        <table className="vt-table" style={{ width: "100%" }}>
          <thead>
            <tr>
              <th style={th}>Signal</th>
              <th style={{ ...th, textAlign: "right" }}>Value</th>
              <th style={th}>Bits</th>
              <th style={th}>Scale</th>
              <th style={th}>Range</th>
              <th style={{ ...th, width: 150 }}>Send value <ProBadge /></th>
            </tr>
          </thead>
          <tbody>
            {m.signals.map((s) => {
              const v = values.get(s.name);
              return (
                <tr key={s.name} title={s.comment || undefined}>
                  <td>
                    {s.name} {s.multiplex && <Badge tone="neutral">{s.multiplex}</Badge>}
                    {s.comment && <div style={hint}>{s.comment}</div>}
                  </td>
                  <td className="mono" style={{ textAlign: "right" }}>{v ? `${v.l ?? fmtNum(v.v)}${s.unit ? ` ${s.unit}` : ""}` : ""}</td>
                  <td className="mono" style={{ fontSize: "var(--fs-xs)" }}>
                    {s.startBit}|{s.length} {s.byteOrder === "intel" ? "LE" : "BE"} {s.valueType === "unsigned" ? "u" : s.valueType === "signed" ? "s" : s.valueType}
                  </td>
                  <td className="mono" style={{ fontSize: "var(--fs-xs)" }}>×{fmtNum(s.factor)} {s.offset ? `${s.offset > 0 ? "+" : ""}${fmtNum(s.offset)}` : ""}</td>
                  <td className="mono" style={{ fontSize: "var(--fs-xs)" }}>{s.min !== s.max ? `${fmtNum(s.min)} to ${fmtNum(s.max)}` : ""}</td>
                  <td>
                    {s.values.length > 0 ? (
                      <select className="vt-input" aria-label={`${s.name} value`} value={draft[s.name] ?? ""} onChange={(e) => setDraft({ ...draft, [s.name]: e.target.value })}>
                        {s.values.map(([raw, label]) => <option key={raw} value={String(raw * s.factor + s.offset)}>{label}</option>)}
                      </select>
                    ) : (
                      <input className="vt-input vt-input--mono" aria-label={`${s.name} value`} value={draft[s.name] ?? ""} onChange={(e) => setDraft({ ...draft, [s.name]: e.target.value })} />
                    )}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
      <div style={row}>
        <Button onClick={() => void encode()}>Preview frame</Button>
        <Button variant="primary" icon={<Send size={14} strokeWidth={1.75} />} disabled={!canSend} onClick={() => void send()}>Send {m.name} <ProBadge /></Button>
        {preview && <span className="mono" style={{ fontSize: "var(--fs-xs)", wordBreak: "break-all" }}>{cansendText(preview)}</span>}
      </div>
      {error && <Callout tone="danger">{error}</Callout>}
      <span style={hint}>Values are physical (after factor and offset), rounded and clamped to what each signal can hold. For a multiplexed message, only the signals of the selected multiplexor value are written.</span>
    </div>
  );
}
