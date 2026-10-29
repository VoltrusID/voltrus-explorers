import { invoke } from "@tauri-apps/api/core";
import { Button, Callout, DataTable, Field } from "@mactools/ui";
import { Send } from "lucide-react";
import { useState } from "react";
import type { AckView } from "./types";
import { AckBadge, card, clock, hint, mono, row, type AddLog } from "./ui";

export interface TerminalLine {
  time: number;
  dir: "tx" | "rx";
  tid: number;
  text: string;
}

interface Props {
  lines: TerminalLine[];
  onSent: (tid: number, text: string) => void;
  onClear: () => void;
  addLog: AddLog;
}

export default function TerminalView({ lines, onSent, onClear, addLog }: Props) {
  const [tid, setTid] = useState(1);
  const [text, setText] = useState("");
  const [ack, setAck] = useState<AckView | null>(null);
  const [error, setError] = useState<string | null>(null);

  const send = async () => {
    if (!text.trim()) return;
    setError(null);
    try {
      const a = await invoke<AckView>("gem_terminal_send", { tid, text });
      setAck(a);
      onSent(tid, text);
      addLog(a.ok ? "info" : "error", `S10F3 terminal ${tid}: ${a.name}`);
      if (a.ok) setText("");
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", flex: 1, minHeight: 0, gap: 8 }}>
      <div style={{ ...card, display: "flex", flexDirection: "column", gap: 8 }}>
        <div style={row}>
          <Field label="Terminal (TID)">
            <input className="vt-input vt-input--mono" style={{ width: 70 }} type="number" min={0} max={255} value={tid} onChange={(e) => setTid(Math.min(255, Math.max(0, Number(e.target.value) || 0)))} />
          </Field>
          <Field label="Text to display on the equipment">
            <input className="vt-input" style={{ width: 360 }} value={text} onChange={(e) => setText(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter") void send(); }} />
          </Field>
          <Button variant="primary" icon={<Send size={14} strokeWidth={1.75} />} disabled={!text.trim()} onClick={() => void send()}>Send S10F3</Button>
          {ack && <AckBadge ack={ack} />}
        </div>
        <span style={hint}>Messages the operator types on the equipment arrive as S10F1 and are acknowledged automatically.</span>
      </div>
      {error && <Callout tone="danger">{error}</Callout>}
      <div style={{ ...row, alignItems: "center" }}>
        <strong style={{ fontSize: "var(--fs-sm)" }}>Terminal messages</strong>
        <span style={{ flex: 1 }} />
        <Button size="sm" onClick={onClear} disabled={lines.length === 0}>Clear</Button>
      </div>
      <div style={{ flex: 1, minHeight: 100, display: "flex", flexDirection: "column" }}>
        <DataTable
          columns={[
            { key: "time", label: "Time", width: "110px" },
            { key: "dir", label: "", width: "60px" },
            { key: "tid", label: "TID", width: "60px", align: "right" },
            { key: "text", label: "Text" },
          ]}
          data={lines.map((l) => ({ time: clock(l.time), dir: l.dir === "tx" ? "Sent" : "Received", tid: l.tid, text: l.text }))}
          autoScroll
          emptyMessage="No terminal messages"
          renderCell={(key, r) => (key === "tid" ? mono(r[key]) : String(r[key] ?? ""))}
        />
      </div>
    </div>
  );
}
