import { invoke } from "@tauri-apps/api/core";
import { Badge, Button, Callout, DataTable, Field, ProBadge, SheetSelect, Segmented, saveTextFile } from "@mactools/ui";
import { Copy, Send } from "lucide-react";
import { useMemo, useState } from "react";
import { toCsv } from "./gem";
import type { MsgEntry, RawReply } from "./types";
import { card, clock, hint, mono, row, type AddLog, type RequirePro } from "./ui";

const EXAMPLES = [
  { value: "S1F1 W.", label: "S1F1 Are you there" },
  { value: "S1F3 W <L>.", label: "S1F3 All status variables" },
  { value: "S1F11 W <L>.", label: "S1F11 Status variable names" },
  { value: "S2F13 W <L>.", label: "S2F13 All equipment constants" },
  { value: "S5F5 W <L>.", label: "S5F5 List alarms" },
  { value: 'S2F41 W <L [2] <A "START"> <L [1] <L [2] <A "PPID"> <A "ETCH_SIO2">>>>.', label: "S2F41 Remote command START" },
  { value: "S7F19 W.", label: "S7F19 Process program list" },
];

interface Props {
  entries: MsgEntry[];
  onClear: () => void;
  requirePro: RequirePro;
  addLog: AddLog;
  /** The composer sends as the host; the simulator view has none. */
  canCompose: boolean;
}

export default function MessagesView({ entries, onClear, requirePro, addLog, canCompose }: Props) {
  const [filter, setFilter] = useState<"all" | "data" | "tx" | "rx">("all");
  const [selected, setSelected] = useState<number | null>(null);
  const [sml, setSml] = useState(EXAMPLES[0].value);
  const [reply, setReply] = useState<RawReply | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const shown = useMemo(
    () => entries.filter((e) => (filter === "all" ? true : filter === "data" ? e.data : e.dir === filter)),
    [entries, filter],
  );
  const current = selected != null ? shown[selected] ?? null : null;

  const exportCsv = async () => {
    if (!requirePro("CSV export")) return;
    try {
      const path = await saveTextFile(
        "hsms-messages.csv",
        toCsv(["time", "direction", "message", "system", "sml"], shown.map((e) => [new Date(e.timeMs).toISOString(), e.dir, e.kind, `0x${e.system.toString(16).padStart(8, "0")}`, e.text])),
      );
      if (path) addLog("info", `Exported ${shown.length} messages to ${path}`);
    } catch (e) {
      addLog("error", `CSV export failed: ${String(e)}`);
    }
  };

  const send = async () => {
    if (!requirePro("The SML composer")) return;
    setBusy(true);
    setError(null);
    setReply(null);
    try {
      setReply(await invoke<RawReply>("gem_send_sml", { sml }));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", flex: 1, minHeight: 0, gap: 8 }}>
      <div style={row}>
        <Segmented
          label="Message filter"
          value={filter}
          onChange={setFilter}
          options={[{ value: "all", label: "All" }, { value: "data", label: "Data" }, { value: "tx", label: "TX" }, { value: "rx", label: "RX" }]}
        />
        <Badge>{shown.length}</Badge>
        <span style={{ flex: 1 }} />
        <Button size="sm" onClick={() => void exportCsv()} disabled={shown.length === 0}>Export CSV <ProBadge /></Button>
        <Button size="sm" onClick={() => { onClear(); setSelected(null); }} disabled={entries.length === 0}>Clear</Button>
      </div>
      <div style={{ display: "flex", flex: 1, minHeight: 0, gap: 8 }}>
        <div style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column" }}>
          <DataTable
            columns={[
              { key: "time", label: "Time", width: "110px" },
              { key: "dir", label: "", width: "44px" },
              { key: "kind", label: "Message", width: "130px" },
              { key: "text", label: "SML" },
            ]}
            data={shown.map((e, i) => ({ time: clock(e.timeMs), dir: e.dir === "tx" ? "TX" : "RX", kind: e.kind, text: e.data ? e.text.replace(/^S\d+F\d+( W)?\s*/, "") : e.text, tx: e.dir === "tx", idx: i }))}
            autoScroll
            onRowClick={(r) => setSelected(Number(r.idx))}
            emptyMessage="No messages yet. Everything sent and received appears here, HSMS control messages included."
            renderCell={(key, r) =>
              key === "dir" ? <Badge tone={r.tx ? "accent" : "success"}>{String(r.dir)}</Badge> : key === "time" || key === "text" || key === "kind" ? mono(r[key]) : String(r[key] ?? "")
            }
          />
        </div>
        <div style={{ ...card, width: 340, flexShrink: 0, display: "flex", flexDirection: "column", gap: 6, overflow: "auto" }}>
          <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
            <strong style={{ fontSize: "var(--fs-xs)", color: "var(--text-dim)" }}>{current ? `${current.dir.toUpperCase()} ${current.kind}` : "Message detail"}</strong>
            <span style={{ flex: 1 }} />
            {current && (
              <Button size="sm" icon={<Copy size={14} strokeWidth={1.75} />} onClick={() => void navigator.clipboard.writeText(current.pretty)}>Copy</Button>
            )}
          </div>
          <pre className="mono" style={{ margin: 0, fontSize: "var(--fs-xs)", whiteSpace: "pre-wrap", color: "var(--text)" }}>
            {current ? current.pretty : "Select a message to see it as indented SML."}
          </pre>
        </div>
      </div>
      {canCompose && (
        <div style={{ ...card, display: "flex", flexDirection: "column", gap: 6 }}>
          <div style={row}>
            <strong style={{ fontSize: "var(--fs-sm)" }}>SML composer <ProBadge /></strong>
            <Field label="">
              <SheetSelect aria-label="Example message" sheetTitle="Example message" value="" options={[{ value: "", label: "Examples" }, ...EXAMPLES]} onChange={(v) => { if (v) setSml(v); }} />
            </Field>
            <span style={{ flex: 1 }} />
            <Button variant="primary" icon={<Send size={14} strokeWidth={1.75} />} disabled={busy || !sml.trim()} onClick={() => void send()}>Send</Button>
          </div>
          <textarea
            className="vt-input vt-input--mono"
            rows={3}
            spellCheck={false}
            value={sml}
            onChange={(e) => setSml(e.target.value)}
            aria-label="SML message"
            style={{ resize: "vertical" }}
          />
          <span style={hint}>
            One message in SML, e.g. <span className="mono">S1F13 W &lt;L [2] &lt;A "MDLN"&gt; &lt;A "SOFTREV"&gt;&gt;.</span> With the W-bit the reply is shown below; without it nothing is awaited.
          </span>
          {error && <Callout tone="danger">{error}</Callout>}
          {reply && (
            <pre className="mono" style={{ margin: 0, fontSize: "var(--fs-xs)", whiteSpace: "pre-wrap" }}>
              {reply.name ? `Reply ${reply.name}\n${reply.sml}` : "Sent (no W-bit, no reply awaited)"}
            </pre>
          )}
        </div>
      )}
    </div>
  );
}
