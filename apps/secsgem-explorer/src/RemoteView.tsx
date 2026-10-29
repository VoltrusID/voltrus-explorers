import { invoke } from "@tauri-apps/api/core";
import { Badge, Button, Callout, Field, IconButton, SheetSelect } from "@mactools/ui";
import { Plus, Send, X } from "lucide-react";
import { useState } from "react";
import type { CommandView, ParamIn } from "./types";
import { AckBadge, Locked, card, hint, row, type AddLog, type RequirePro } from "./ui";

const KINDS = ["ascii", "u1", "u2", "u4", "i1", "i2", "i4", "f4", "f8", "bool", "binary", "sml"].map((k) => ({ value: k, label: k.toUpperCase() }));

interface Props {
  pro: boolean;
  requirePro: RequirePro;
  addLog: AddLog;
}

export default function RemoteView({ pro, requirePro, addLog }: Props) {
  const [rcmd, setRcmd] = useState("START");
  const [params, setParams] = useState<ParamIn[]>([{ name: "PPID", kind: "ascii", value: "ETCH_SIO2" }]);
  const [result, setResult] = useState<{ rcmd: string; view: CommandView }[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  if (!pro) return <Locked feature="Remote commands" requirePro={requirePro} />;

  const patch = (i: number, p: Partial<ParamIn>) => setParams((prev) => prev.map((x, j) => (j === i ? { ...x, ...p } : x)));

  const send = async (cmd: string, ps: ParamIn[]) => {
    if (!requirePro("Remote commands")) return;
    setBusy(true);
    setError(null);
    try {
      const view = await invoke<CommandView>("gem_remote_command", { rcmd: cmd, params: ps.filter((p) => p.name.trim() !== "") });
      setResult((prev) => [{ rcmd: cmd, view }, ...prev].slice(0, 50));
      addLog(view.hcack.ok ? "info" : "error", `S2F41 ${cmd}: HCACK ${view.hcack.name}`);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", flex: 1, minHeight: 0, gap: 8, overflow: "auto" }}>
      <div style={{ ...card, display: "flex", flexDirection: "column", gap: 8 }}>
        <div style={row}>
          <Field label="Command (RCMD)">
            <input className="vt-input vt-input--mono" style={{ width: 200 }} value={rcmd} onChange={(e) => setRcmd(e.target.value)} />
          </Field>
          <Button size="sm" onClick={() => { setRcmd("START"); setParams([{ name: "PPID", kind: "ascii", value: "ETCH_SIO2" }]); }}>START</Button>
          <Button size="sm" onClick={() => { setRcmd("STOP"); setParams([]); }}>STOP</Button>
          <Button size="sm" onClick={() => { setRcmd("PAUSE"); setParams([]); }}>PAUSE</Button>
          <Button size="sm" onClick={() => { setRcmd("RESUME"); setParams([]); }}>RESUME</Button>
        </div>
        {params.map((p, i) => (
          <div key={i} style={row}>
            <Field label={i === 0 ? "Parameter (CPNAME)" : ""}>
              <input className="vt-input vt-input--mono" style={{ width: 160 }} value={p.name} onChange={(e) => patch(i, { name: e.target.value })} />
            </Field>
            <Field label={i === 0 ? "Type" : ""}>
              <SheetSelect aria-label="Parameter type" sheetTitle="Parameter type" value={p.kind} options={KINDS} onChange={(v) => patch(i, { kind: v })} />
            </Field>
            <Field label={i === 0 ? "Value (CPVAL)" : ""}>
              <input className="vt-input vt-input--mono" style={{ width: 220 }} value={p.value} onChange={(e) => patch(i, { value: e.target.value })} />
            </Field>
            <IconButton size="sm" label="Remove parameter" onClick={() => setParams((prev) => prev.filter((_, j) => j !== i))}>
              <X size={14} strokeWidth={1.75} />
            </IconButton>
          </div>
        ))}
        <div style={row}>
          <Button size="sm" icon={<Plus size={14} strokeWidth={1.75} />} onClick={() => setParams((prev) => [...prev, { name: "", kind: "ascii", value: "" }])}>Add parameter</Button>
          <span style={{ flex: 1 }} />
          <Button variant="primary" icon={<Send size={14} strokeWidth={1.75} />} disabled={busy || rcmd.trim() === ""} onClick={() => void send(rcmd.trim(), params)}>
            Send S2F41
          </Button>
        </div>
        <span style={hint}>The equipment answers with HCACK and, per parameter, CPACK. A command that finishes later reports completion through an event.</span>
      </div>
      {error && <Callout tone="danger">{error}</Callout>}
      {result.map((r, i) => (
        <div key={i} style={{ ...card, display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap" }}>
          <strong className="mono" style={{ fontSize: "var(--fs-sm)" }}>{r.rcmd}</strong>
          <span style={hint}>HCACK</span>
          <AckBadge ack={r.view.hcack} />
          {r.view.params.map((p) => (
            <span key={p.name} style={{ display: "inline-flex", gap: 4, alignItems: "center" }}>
              <Badge>{p.name}</Badge>
              <AckBadge ack={p.cpack} />
            </span>
          ))}
        </div>
      ))}
    </div>
  );
}
