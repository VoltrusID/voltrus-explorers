import { invoke } from "@tauri-apps/api/core";
import { Badge, Button, Callout, DataTable, Dialog, Field, ProBadge } from "@mactools/ui";
import { Pencil, RefreshCw } from "lucide-react";
import { useCallback, useState } from "react";
import { ackText, rangeProblem } from "./gem";
import type { EcRow, EcWriteView } from "./types";
import { card, hint, mono, row, type AddLog, type RequirePro } from "./ui";

interface Props {
  requirePro: RequirePro;
  addLog: AddLog;
}

export default function ConstantsView({ requirePro, addLog }: Props) {
  const [rows, setRows] = useState<EcRow[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<number | null>(null);
  const [text, setText] = useState("");
  const [confirm, setConfirm] = useState(false);
  const [result, setResult] = useState<{ ok: boolean; message: string } | null>(null);

  const load = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      setRows(await invoke<EcRow[]>("gem_ec_table"));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }, []);

  const ec = rows.find((r) => r.id === selected) ?? null;
  const entered = Number(text);
  const numeric = ec?.number != null;
  const problem = ec && numeric && text.trim() !== "" ? rangeProblem(entered, ec.min, ec.max) : null;

  const askWrite = () => {
    if (!ec) return;
    if (!requirePro("Equipment constant writes")) return;
    setResult(null);
    setConfirm(true);
  };

  const write = async () => {
    if (!ec) return;
    setConfirm(false);
    setBusy(true);
    try {
      const r = await invoke<EcWriteView>("gem_ec_write", { id: ec.id, text });
      setResult({ ok: r.ack.ok, message: r.ack.ok ? `${ec.name} is now ${r.value?.text ?? text}` : `The equipment refused: ${ackText(r.ack)}` });
      addLog(r.ack.ok ? "info" : "error", `Write ${ec.name} = ${text}: ${ackText(r.ack)}`);
      if (r.value) {
        const v = r.value;
        setRows((prev) => prev.map((x) => (x.id === ec.id ? { ...x, ...v } : x)));
      }
    } catch (e) {
      setResult({ ok: false, message: String(e) });
    } finally {
      setBusy(false);
    }
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", flex: 1, minHeight: 0, gap: 8 }}>
      <div style={row}>
        <Button size="sm" icon={<RefreshCw size={14} strokeWidth={1.75} />} onClick={() => void load()} disabled={busy}>
          {rows.length ? "Reload constants" : "Read constants"}
        </Button>
        <span style={hint}>S2F29 names and ranges, S2F13 values. Writing (S2F15) <ProBadge /></span>
      </div>
      {error && <Callout tone="danger">{error}</Callout>}
      <div style={{ flex: 1, minHeight: 140, display: "flex", flexDirection: "column" }}>
        <DataTable
          columns={[
            { key: "id", label: "ECID", width: "80px", align: "right" },
            { key: "name", label: "Name" },
            { key: "value", label: "Value", align: "right" },
            { key: "units", label: "Units", width: "80px" },
            { key: "range", label: "Range", align: "right" },
            { key: "default", label: "Default", align: "right" },
            { key: "kind", label: "Type", width: "60px" },
          ]}
          data={rows.map((r) => ({
            id: r.id,
            name: r.name,
            value: r.text,
            units: r.units,
            range: r.min != null || r.max != null ? `${r.min ?? "-"} to ${r.max ?? "-"}` : "",
            default: r.default,
            kind: r.kind,
          }))}
          emptyMessage={busy ? "Reading" : "Read constants (S2F29, S2F13) to list the equipment constants"}
          onRowClick={(r) => {
            const found = rows.find((x) => x.id === Number(r.id));
            setSelected(Number(r.id));
            setText(found?.text ?? "");
            setResult(null);
          }}
          renderCell={(key, r) => (["id", "value", "range", "default"].includes(key) ? mono(r[key]) : String(r[key] ?? ""))}
        />
      </div>
      <div style={card}>
        {!ec ? (
          <span style={{ fontSize: "var(--fs-sm)", color: "var(--text-muted)" }}>Select a constant to change its value.</span>
        ) : (
          <div style={row}>
            <strong style={{ fontSize: "var(--fs-sm)" }}>{ec.name}</strong>
            <Badge>{ec.kind}</Badge>
            <Field label={`New value${ec.units ? ` (${ec.units})` : ""}`} error={problem ?? undefined}>
              <input
                className="vt-input vt-input--mono"
                style={{ width: 200 }}
                value={text}
                onChange={(e) => setText(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && !problem) askWrite();
                }}
              />
            </Field>
            <Button variant="primary" icon={<Pencil size={14} strokeWidth={1.75} />} disabled={busy || !!problem || text.trim() === ""} onClick={askWrite}>
              Write <ProBadge />
            </Button>
          </div>
        )}
        {result && (
          <div style={{ marginTop: 8 }}>
            <Callout tone={result.ok ? "success" : "danger"}>{result.message}</Callout>
          </div>
        )}
      </div>
      <Dialog
        open={confirm}
        onClose={() => setConfirm(false)}
        title="Write equipment constant"
        width={420}
        footer={
          <>
            <Button variant="ghost" onClick={() => setConfirm(false)}>Cancel</Button>
            <Button variant="danger" onClick={() => void write()}>Write to equipment</Button>
          </>
        }
      >
        <p style={{ margin: 0, fontSize: "var(--fs-sm)", lineHeight: 1.6 }}>
          Set <strong>{ec?.name}</strong> (ECID {ec?.id}) from <span className="mono">{ec?.text}</span> to <span className="mono">{text}</span>
          {ec?.units ? ` ${ec.units}` : ""}? This changes a setting on the equipment.
        </p>
      </Dialog>
    </div>
  );
}
