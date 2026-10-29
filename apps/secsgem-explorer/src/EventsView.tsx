import { invoke } from "@tauri-apps/api/core";
import { Badge, Button, Callout, DataTable, Field, Section } from "@mactools/ui";
import { Trash2, Wand2 } from "lucide-react";
import { useMemo, useState } from "react";
import { ackText, parseIdList, toCsv, type EventRow } from "./gem";
import type { AckView, LocalReport, SvRow, WizardStep } from "./types";
import { AckBadge, Locked, card, clock, hint, mono, row, type AddLog, type RequirePro } from "./ui";
import { saveTextFile } from "@mactools/ui";

interface Props {
  pro: boolean;
  requirePro: RequirePro;
  addLog: AddLog;
  names: SvRow[];
  reports: LocalReport[];
  setReports: (f: (prev: LocalReport[]) => LocalReport[]) => void;
  rows: EventRow[];
  onClear: () => void;
}

export default function EventsView({ pro, requirePro, addLog, names, reports, setReports, rows, onClear }: Props) {
  const [rptid, setRptid] = useState("1");
  const [vidText, setVidText] = useState("");
  const [ceidText, setCeidText] = useState("");
  const [enable, setEnable] = useState(true);
  const [steps, setSteps] = useState<WizardStep[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<AckView | null>(null);

  const vids = useMemo(() => {
    try { return parseIdList(vidText); } catch { return null; }
  }, [vidText]);

  const addVid = (id: number) => {
    const cur = vids ?? [];
    setVidText((cur.includes(id) ? cur.filter((v) => v !== id) : [...cur, id]).join(", "));
  };

  const run = async () => {
    if (!requirePro("Report wizard and live events")) return;
    setError(null);
    setSteps(null);
    let ceids: number[];
    let vs: number[];
    const id = Number(rptid);
    try {
      vs = parseIdList(vidText);
      ceids = parseIdList(ceidText);
      if (!Number.isInteger(id) || id < 0) throw new Error("The report id must be a whole number");
      if (vs.length === 0) throw new Error("Pick at least one variable (VID) for the report");
      if (ceids.length === 0) throw new Error("Enter at least one event id (CEID) to link");
    } catch (e) {
      setError(String((e as Error).message ?? e));
      return;
    }
    setBusy(true);
    try {
      const s = await invoke<WizardStep[]>("gem_report_wizard", { request: { rptid: id, vids: vs, ceids, enable } });
      setSteps(s);
      s.forEach((x) => addLog(x.ack.ok ? "info" : "error", `${x.step}: ${ackText(x.ack)}`));
      if (s.length >= 2 && s[1].ack.ok) {
        setReports((prev) => [...prev.filter((r) => r.rptid !== id), { rptid: id, vids: vs, ceids }]);
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const deleteReport = async (id: number) => {
    if (!requirePro("Report wizard and live events")) return;
    try {
      const a = await invoke<AckView>("gem_report_delete", { rptid: id });
      setNote(a);
      if (a.ok) setReports((prev) => prev.filter((r) => r.rptid !== id));
    } catch (e) {
      setError(String(e));
    }
  };

  const disableAll = async () => {
    if (!requirePro("Report wizard and live events")) return;
    try {
      setNote(await invoke<AckView>("gem_events_enable", { enable: false, ceids: [] }));
    } catch (e) {
      setError(String(e));
    }
  };

  const exportCsv = async () => {
    if (!requirePro("CSV export")) return;
    try {
      const path = await saveTextFile("event-reports.csv", toCsv(["time", "dataid", "ceid", "values"], rows.map((r) => [new Date(r.time).toISOString(), r.dataid, r.ceid, r.summary])));
      if (path) addLog("info", `Exported ${rows.length} events to ${path}`);
    } catch (e) {
      addLog("error", `CSV export failed: ${String(e)}`);
    }
  };

  if (!pro) return <Locked feature="The report wizard and live event reports" requirePro={requirePro} />;

  return (
    <div style={{ display: "flex", flex: 1, minHeight: 0, gap: 8 }}>
      <div style={{ ...card, width: 330, flexShrink: 0, overflow: "auto", display: "flex", flexDirection: "column", gap: 8 }}>
        <Section title="Report wizard">
          <span style={hint}>Defines a report (S2F33), links it to events (S2F35) and enables them (S2F37). The equipment then sends an S6F11 each time one fires.</span>
          <Field label="1. Report id (RPTID)">
            <input className="vt-input vt-input--mono" value={rptid} onChange={(e) => setRptid(e.target.value)} />
          </Field>
          <Field label="2. Variables (VIDs)" hint="Ids or ranges, e.g. 10, 11, 12-13. Click names below to add.">
            <input className="vt-input vt-input--mono" value={vidText} onChange={(e) => setVidText(e.target.value)} placeholder="10, 11, 12" />
          </Field>
          {names.length > 0 && (
            <div style={{ display: "flex", flexWrap: "wrap", gap: 4, maxHeight: 96, overflow: "auto" }}>
              {names.map((n) => (
                <button
                  key={n.id}
                  type="button"
                  onClick={() => addVid(n.id)}
                  className="vt-btn vt-btn--sm"
                  aria-pressed={vids?.includes(n.id) ?? false}
                  style={vids?.includes(n.id) ? { borderColor: "var(--accent)" } : undefined}
                >
                  {n.name}
                </button>
              ))}
            </div>
          )}
          <Field label="3. Events to link (CEIDs)" hint="Event ids are equipment specific. The built-in simulator uses 100-104, 201-204, 300, 301.">
            <input className="vt-input vt-input--mono" value={ceidText} onChange={(e) => setCeidText(e.target.value)} placeholder="100, 101" />
          </Field>
          <label style={{ display: "flex", gap: 6, alignItems: "center", fontSize: "var(--fs-sm)" }}>
            <input type="checkbox" checked={enable} onChange={(e) => setEnable(e.target.checked)} /> 4. Enable the events
          </label>
          <Button variant="primary" icon={<Wand2 size={14} strokeWidth={1.75} />} disabled={busy} onClick={() => void run()}>
            Define, link{enable ? " and enable" : ""}
          </Button>
          {error && <Callout tone="danger">{error}</Callout>}
          {steps && (
            <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
              {steps.map((s) => (
                <div key={s.step} style={{ display: "flex", justifyContent: "space-between", gap: 6, fontSize: "var(--fs-xs)", alignItems: "center" }}>
                  <span>{s.step}</span>
                  <AckBadge ack={s.ack} />
                </div>
              ))}
            </div>
          )}
        </Section>
        <Section title="Reports defined here">
          {reports.length === 0 && <span style={hint}>None yet.</span>}
          {reports.map((r) => (
            <div key={r.rptid} style={{ display: "flex", alignItems: "center", gap: 6, fontSize: "var(--fs-xs)" }}>
              <Badge>RPTID {r.rptid}</Badge>
              <span className="mono" style={{ flex: 1 }}>VIDs {r.vids.join(",")} on CEIDs {r.ceids.join(",")}</span>
              <Button size="sm" variant="danger" icon={<Trash2 size={14} strokeWidth={1.75} />} onClick={() => void deleteReport(r.rptid)}>Delete</Button>
            </div>
          ))}
          <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
            <Button size="sm" onClick={() => void disableAll()}>Disable all events</Button>
            {note && <AckBadge ack={note} />}
          </div>
        </Section>
      </div>
      <div style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column", gap: 8 }}>
        <div style={row}>
          <strong style={{ fontSize: "var(--fs-sm)" }}>Live event reports (S6F11)</strong>
          <Badge>{rows.length}</Badge>
          <span style={{ flex: 1 }} />
          <Button size="sm" onClick={() => void exportCsv()} disabled={rows.length === 0}>Export CSV</Button>
          <Button size="sm" onClick={onClear} disabled={rows.length === 0}>Clear</Button>
        </div>
        <DataTable
          columns={[
            { key: "time", label: "Time", width: "110px" },
            { key: "ceid", label: "CEID", width: "70px", align: "right" },
            { key: "dataid", label: "DATAID", width: "80px", align: "right" },
            { key: "summary", label: "Reports and values" },
          ]}
          data={rows.map((r) => ({ time: clock(r.time), ceid: r.ceid, dataid: r.dataid, summary: r.summary }))}
          autoScroll
          emptyMessage="No event reports yet. Define a report and link it to an event; the equipment sends one when the event fires."
          renderCell={(key, r) => (key === "summary" ? String(r[key]) : mono(r[key]))}
        />
      </div>
    </div>
  );
}
