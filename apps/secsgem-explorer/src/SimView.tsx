import { invoke } from "@tauri-apps/api/core";
import { Badge, Button, Callout, DataTable, Field } from "@mactools/ui";
import { AlertTriangle, Pause, Play, Square } from "lucide-react";
import { useEffect, useState } from "react";
import type { SimView as View } from "./types";
import { KeyValues, card, hint, mono, row, type AddLog } from "./ui";

interface Props {
  addLog: AddLog;
  onConnect: (port: number) => void;
}

export type SimActionArg =
  | { type: "operator_offline" | "operator_online" | "stop_process" | "pause_process" | "resume_process" }
  | { type: "set_remote"; remote: boolean }
  | { type: "comm_enable"; enabled: boolean }
  | { type: "start_process"; ppid: string | null }
  | { type: "raise_alarm" | "clear_alarm"; alid: number }
  | { type: "set_constant"; id: number; value: number }
  | { type: "set_sv"; id: number; text: string };

export default function SimDashboard({ addLog, onConnect }: Props) {
  const [view, setView] = useState<View | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [edit, setEdit] = useState<{ kind: "sv" | "ec"; id: number; text: string } | null>(null);
  const [ppid, setPpid] = useState("");

  useEffect(() => {
    let alive = true;
    const tick = async () => {
      try {
        const v = await invoke<View>("sim_view");
        if (alive) setView(v);
      } catch (e) {
        if (alive) setError(String(e));
      }
    };
    void tick();
    const t = setInterval(tick, 500);
    return () => { alive = false; clearInterval(t); };
  }, []);

  const act = async (action: SimActionArg) => {
    setError(null);
    try {
      await invoke("sim_action", { action });
      setView(await invoke<View>("sim_view"));
    } catch (e) {
      setError(String(e));
      addLog("error", String(e));
    }
  };

  if (!view) return <div style={hint}>{error ?? "Reading the simulator"}</div>;
  const p = view.process;
  const running = p.state !== "Idle";
  const commTone = view.comm === "Communicating" ? "success" : "warn";

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8, overflow: "auto", flex: 1, minHeight: 0 }}>
      <div style={row}>
        <Badge tone="success">Running on port {view.port || "(active)"}</Badge>
        <Badge tone={view.selected ? "success" : "neutral"}>{view.selected ? "Host selected" : "No host selected"}</Badge>
        <Badge tone={commTone}>{view.comm}</Badge>
        <Badge tone={view.control.online ? "success" : "warn"}>{view.control.name}</Badge>
        <span style={hint}>{view.requests} request(s) served</span>
        <span style={{ flex: 1 }} />
        {view.port > 0 && <Button size="sm" variant="primary" onClick={() => onConnect(view.port)}>Connect client to it</Button>}
      </div>
      {error && <Callout tone="danger">{error}</Callout>}
      <div style={{ display: "flex", gap: 8 }}>
        <div style={{ ...card, flex: 1, display: "flex", flexDirection: "column", gap: 6 }}>
          <div style={{ fontSize: "var(--fs-xs)", fontWeight: 600, color: "var(--text-dim)" }}>Control and communication</div>
          <div style={row}>
            <Button size="sm" onClick={() => void act({ type: "operator_offline" })}>Go offline</Button>
            <Button size="sm" onClick={() => void act({ type: "operator_online" })}>Go online</Button>
            <Button size="sm" variant={view.onlineRemote ? "primary" : undefined} onClick={() => void act({ type: "set_remote", remote: true })}>Remote</Button>
            <Button size="sm" variant={!view.onlineRemote ? "primary" : undefined} onClick={() => void act({ type: "set_remote", remote: false })}>Local</Button>
          </div>
          <div style={row}>
            <Button size="sm" onClick={() => void act({ type: "comm_enable", enabled: !view.commEnabled })}>
              {view.commEnabled ? "Disable communication" : "Enable communication"}
            </Button>
          </div>
          <span style={hint}>Going online needs a communicating host. The host's S1F15 and S1F17 move the state too.</span>
        </div>
        <div style={{ ...card, flex: 1, display: "flex", flexDirection: "column", gap: 6 }}>
          <div style={{ fontSize: "var(--fs-xs)", fontWeight: 600, color: "var(--text-dim)" }}>Process</div>
          <div style={row}>
            <Badge tone={p.state === "Executing" ? "success" : p.state === "Paused" ? "warn" : "neutral"}>{p.state}</Badge>
            <span className="mono" style={{ fontSize: "var(--fs-xs)" }}>{p.ppid || "-"} / lot {p.lotId || "-"} / {p.wafers} wafers / {p.elapsedS.toFixed(0)} of {p.durationS.toFixed(0)} s</span>
          </div>
          <div style={row}>
            <select className="vt-input" aria-label="Process program" value={ppid} onChange={(e) => setPpid(e.target.value)} disabled={running}>
              <option value="">Last program</option>
              {view.ppids.map((x) => <option key={x} value={x}>{x}</option>)}
            </select>
            <Button size="sm" variant="primary" icon={<Play size={14} strokeWidth={1.75} />} disabled={running} onClick={() => void act({ type: "start_process", ppid: ppid || null })}>Start</Button>
            <Button size="sm" icon={<Pause size={14} strokeWidth={1.75} />} disabled={p.state !== "Executing"} onClick={() => void act({ type: "pause_process" })}>Pause</Button>
            <Button size="sm" icon={<Play size={14} strokeWidth={1.75} />} disabled={p.state !== "Paused"} onClick={() => void act({ type: "resume_process" })}>Resume</Button>
            <Button size="sm" variant="danger" icon={<Square size={14} strokeWidth={1.75} />} disabled={!running} onClick={() => void act({ type: "stop_process" })}>Stop</Button>
          </div>
        </div>
      </div>
      <div style={{ display: "flex", gap: 8, minHeight: 200 }}>
        <div style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column" }}>
          <DataTable
            columns={[
              { key: "id", label: "SVID", width: "60px", align: "right" },
              { key: "name", label: "Status variable" },
              { key: "text", label: "Value", align: "right" },
              { key: "units", label: "Units", width: "70px" },
            ]}
            data={view.svs.map((s) => ({ id: s.id, name: s.name, text: s.text, units: s.units, editable: s.editable }))}
            onRowClick={(r) => { if (r.editable) setEdit({ kind: "sv", id: Number(r.id), text: String(r.text) }); }}
            renderCell={(key, r) => (key === "id" || key === "text" ? mono(r[key]) : String(r[key] ?? ""))}
          />
        </div>
        <div style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column" }}>
          <DataTable
            columns={[
              { key: "id", label: "ECID", width: "60px", align: "right" },
              { key: "name", label: "Equipment constant" },
              { key: "value", label: "Value", align: "right" },
              { key: "range", label: "Range", align: "right" },
            ]}
            data={view.constants.map((c) => ({ id: c.id, name: `${c.name}${c.locked ? " (locked)" : ""}`, value: c.value, range: `${c.min} to ${c.max} ${c.units}`, v: c.value }))}
            onRowClick={(r) => setEdit({ kind: "ec", id: Number(r.id), text: String(r.v) })}
            renderCell={(key, r) => (key === "id" || key === "value" || key === "range" ? mono(r[key]) : String(r[key] ?? ""))}
          />
        </div>
      </div>
      {edit && (
        <div style={{ ...card, ...row }}>
          <strong style={{ fontSize: "var(--fs-sm)" }}>{edit.kind === "sv" ? `SVID ${edit.id}` : `ECID ${edit.id}`}</strong>
          <Field label="Value">
            <input
              className="vt-input vt-input--mono"
              style={{ width: 200 }}
              value={edit.text}
              onChange={(e) => setEdit({ ...edit, text: e.target.value })}
              onKeyDown={(e) => { if (e.key === "Enter") document.getElementById("sim-set")?.click(); }}
            />
          </Field>
          <Button
            id="sim-set"
            variant="primary"
            onClick={() => {
              void act(edit.kind === "sv" ? { type: "set_sv", id: edit.id, text: edit.text } : { type: "set_constant", id: edit.id, value: Number(edit.text) });
            }}
          >
            Set
          </Button>
          <Button variant="ghost" onClick={() => setEdit(null)}>Close</Button>
          <span style={hint}>Click a row to edit. Pressure and temperature drift back to the process setpoints.</span>
        </div>
      )}
      <div style={{ display: "flex", gap: 8, minHeight: 160 }}>
        <div style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column" }}>
          <DataTable
            columns={[
              { key: "alid", label: "ALID", width: "60px", align: "right" },
              { key: "text", label: "Alarm" },
              { key: "state", label: "State", width: "80px" },
              { key: "act", label: "", width: "110px" },
            ]}
            data={view.alarms.map((a) => ({ alid: a.alid, text: `${a.text}${a.enabled ? "" : " (reporting off)"}`, state: a.set ? "SET" : "clear", set: a.set }))}
            renderCell={(key, r) => {
              if (key === "state") return <Badge tone={r.set ? "danger" : "neutral"}>{String(r.state)}</Badge>;
              if (key === "act") {
                return r.set ? (
                  <Button size="sm" onClick={() => void act({ type: "clear_alarm", alid: Number(r.alid) })}>Clear</Button>
                ) : (
                  <Button size="sm" variant="danger" icon={<AlertTriangle size={14} strokeWidth={1.75} />} onClick={() => void act({ type: "raise_alarm", alid: Number(r.alid) })}>Raise</Button>
                );
              }
              return key === "alid" ? mono(r[key]) : String(r[key] ?? "");
            }}
          />
        </div>
        <div style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column", gap: 8 }}>
          <KeyValues
            title="Events (enabled by the host with S2F37)"
            rows={view.events.map((e) => [`${e.ceid} ${e.name}`, e.enabled ? "enabled" : "disabled"])}
          />
          <KeyValues title="Terminal messages from the host (S10F3)" rows={view.terminal.length ? view.terminal.map((t, i): [string, string] => [String(i + 1), t]) : [["None", "-"]]} />
        </div>
      </div>
    </div>
  );
}
