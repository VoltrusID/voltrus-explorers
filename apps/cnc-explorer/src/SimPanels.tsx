import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Badge, Button, Callout, DataTable, Field, SheetSelect } from "@mactools/ui";
import type { Tone } from "@mactools/ui";
import { Pause, Play, RotateCcw, Square, TriangleAlert } from "lucide-react";
import { alarmTypeName, fmt } from "./fanuc";
import { formatValue } from "./mtc";
import type { FocasSimView, MtcSimSnapshot, SimItemState } from "./types";

const mono = (v: unknown) => <span className="mono">{String(v ?? "")}</span>;
const card: React.CSSProperties = {
  background: "var(--card)", border: "1px solid var(--border)", borderRadius: "var(--radius-lg)", padding: 12, minWidth: 0,
};
const levelTone: Record<string, Tone> = { Normal: "success", Warning: "warn", Fault: "danger", Unavailable: "neutral" };

function usePolled<T>(command: string, ms = 500): { data: T | null; error: string | null } {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let alive = true;
    const tick = async () => {
      try {
        const v = await invoke<T>(command);
        if (alive) { setData(v); setError(null); }
      } catch (e) {
        if (alive) setError(String(e));
      }
    };
    void tick();
    const t = setInterval(tick, ms);
    return () => { alive = false; clearInterval(t); };
  }, [command, ms]);
  return { data, error };
}

function conditionLevel(i: SimItemState): string {
  return i.condition?.level ?? "Unavailable";
}

export function MtcSimPanel({ baseUrl, onConnect }: { baseUrl: string; onConnect: () => void }) {
  const { data: snap, error } = usePolled<MtcSimSnapshot>("mtc_sim_snapshot");
  const [selected, setSelected] = useState<string | null>(null);
  const [text, setText] = useState("");
  const [level, setLevel] = useState("Fault");
  const [code, setCode] = useState("E42");
  const [message, setMessage] = useState("Servo overload");
  const [actionError, setActionError] = useState<string | null>(null);

  const run = useCallback(async (command: string, args?: Record<string, unknown>) => {
    setActionError(null);
    try { await invoke(command, args); } catch (e) { setActionError(String(e)); }
  }, []);

  const item = snap?.items.find((i) => i.id === selected) ?? null;
  const isCondition = item?.category === "CONDITION";

  return (
    <div style={{ display: "flex", flexDirection: "column", flex: 1, minHeight: 0, gap: 8 }}>
      <div style={{ display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap" }}>
        <Badge tone="success">Running</Badge>
        <span className="mono" style={{ fontSize: "var(--fs-xs)", color: "var(--text-dim)" }}>{baseUrl}/current</span>
        <span style={{ flex: 1 }} />
        {snap && (
          snap.paused ? (
            <Button size="sm" variant="primary" icon={<Play size={14} strokeWidth={1.75} />} onClick={() => run("mtc_sim_set_paused", { paused: false })}>Resume program</Button>
          ) : (
            <Button size="sm" icon={<Pause size={14} strokeWidth={1.75} />} onClick={() => run("mtc_sim_set_paused", { paused: true })}>Pause program</Button>
          )
        )}
        <Button size="sm" icon={<RotateCcw size={14} strokeWidth={1.75} />} onClick={() => run("mtc_sim_restart_agent")}>Restart agent</Button>
        <Button size="sm" variant="primary" onClick={onConnect}>Connect client to it</Button>
      </div>
      {(error || actionError) && <Callout tone="danger">{actionError ?? error}</Callout>}
      <div style={{ flex: 1, minHeight: 120, display: "flex", flexDirection: "column" }}>
        <DataTable
          columns={[
            { key: "id", label: "Id", width: "150px" },
            { key: "type", label: "Type", width: "170px" },
            { key: "category", label: "Category", width: "90px" },
            { key: "value", label: "Value", align: "right" },
            { key: "pinned", label: "", width: "70px" },
          ]}
          data={(snap?.items ?? []).map((i) => ({
            id: i.id,
            type: i.dataType,
            category: i.category,
            value: i.category === "CONDITION" ? conditionLevel(i) : formatValue(i.value),
            pinned: i.pinned ? "pinned" : "",
          }))}
          emptyMessage="Waiting for the simulator"
          onRowClick={(r) => {
            const it = snap?.items.find((x) => x.id === r.id);
            setSelected(String(r.id));
            if (it && it.category !== "CONDITION") setText(it.value.kind === "Unavailable" ? "" : formatValue(it.value));
          }}
          renderCell={(key, row) => (key === "value" ? mono(row[key]) : String(row[key] ?? ""))}
        />
      </div>
      <div style={{ ...card }}>
        {!item ? (
          <span style={{ fontSize: "var(--fs-sm)", color: "var(--text-muted)" }}>Select a data item to change its value or raise a condition.</span>
        ) : isCondition ? (
          <div style={{ display: "flex", gap: 8, alignItems: "flex-end", flexWrap: "wrap" }}>
            <strong style={{ fontSize: "var(--fs-sm)" }}>{item.id}</strong>
            <Badge tone={levelTone[conditionLevel(item)]}>{conditionLevel(item)}</Badge>
            <Field label="Level">
              <SheetSelect aria-label="Condition level" sheetTitle="Level" value={level} onChange={setLevel} options={[{ value: "Warning", label: "Warning" }, { value: "Fault", label: "Fault" }]} />
            </Field>
            <Field label="Native code"><input className="vt-input vt-input--mono" style={{ width: 90 }} value={code} onChange={(e) => setCode(e.target.value)} /></Field>
            <Field label="Message"><input className="vt-input" style={{ width: 220 }} value={message} onChange={(e) => setMessage(e.target.value)} /></Field>
            <Button variant="danger" icon={<TriangleAlert size={14} strokeWidth={1.75} />} onClick={() => run("mtc_sim_raise", { id: item.id, level, code, message })}>Raise</Button>
            <Button onClick={() => run("mtc_sim_clear", { id: item.id })}>Clear</Button>
          </div>
        ) : (
          <div style={{ display: "flex", gap: 8, alignItems: "flex-end", flexWrap: "wrap" }}>
            <strong style={{ fontSize: "var(--fs-sm)" }}>{item.id}</strong>
            <Field label={item.category === "SAMPLE" ? `Value (number${item.units ? `, ${item.units}` : ""})` : "Value (text)"}>
              <input className="vt-input vt-input--mono" style={{ width: 200 }} value={text} onChange={(e) => setText(e.target.value)}
                onKeyDown={(e) => { if (e.key === "Enter") void run("mtc_sim_set_value", { id: item.id, text }); }} />
            </Field>
            <Button variant="primary" onClick={() => run("mtc_sim_set_value", { id: item.id, text })}>Set (pins the item)</Button>
            <Button disabled={!item.pinned} onClick={() => run("mtc_sim_release", { id: item.id })}>Release</Button>
          </div>
        )}
      </div>
    </div>
  );
}

const ALARM_TYPES = [
  { value: "servo", label: "Servo" },
  { value: "overtravel", label: "Overtravel" },
  { value: "macro", label: "Macro" },
  { value: "spindle", label: "Spindle" },
  { value: "overheat", label: "Overheat" },
  { value: "foreground_ps", label: "Program (PS)" },
  { value: "external_message", label: "External message" },
];

export function FocasSimPanel({ onConnect }: { onConnect: () => void }) {
  const { data: v, error } = usePolled<FocasSimView>("focas_sim_view");
  const [number, setNumber] = useState(1000);
  const [type, setType] = useState("servo");
  const [message, setMessage] = useState("SERVO ALARM: SIMULATED");
  const [macroNo, setMacroNo] = useState(500);
  const [macroVal, setMacroVal] = useState("0");
  const [actionError, setActionError] = useState<string | null>(null);

  const run = useCallback(async (command: string, args?: Record<string, unknown>) => {
    setActionError(null);
    try { await invoke(command, args); } catch (e) { setActionError(String(e)); }
  }, []);

  const emergency = v?.status.emergency.label === "EMG";
  return (
    <div style={{ display: "flex", flexDirection: "column", flex: 1, minHeight: 0, gap: 8, overflow: "auto" }}>
      <div style={{ display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap" }}>
        <Badge tone="success">Running</Badge>
        {v && <span style={{ fontSize: "var(--fs-xs)", color: "var(--text-dim)" }}>{v.profile} on port {v.port}, {v.sessions} client(s), {v.requests} requests</span>}
        <span style={{ flex: 1 }} />
        <Button size="sm" variant="primary" onClick={onConnect}>Connect client to it</Button>
      </div>
      {(error || actionError) && <Callout tone="danger">{actionError ?? error}</Callout>}
      {v && (
        <>
          <div style={{ ...card, display: "flex", gap: 24, flexWrap: "wrap", fontSize: "var(--fs-sm)" }}>
            <span>Mode <strong>{v.status.mode.label}</strong></span>
            <span>Run <strong>{v.status.run.label}</strong></span>
            <span>Emergency <Badge tone={emergency ? "danger" : "neutral"}>{v.status.emergency.label}</Badge></span>
            <span>Alarm <Badge tone={v.status.alarm.label === "ALM" ? "danger" : "neutral"}>{v.status.alarm.label}</Badge></span>
            <span>O{v.program} N{v.sequence}</span>
            <span>Feed <span className="mono">{fmt(v.feed_rate, 0)}</span></span>
            <span>Spindle <span className="mono">{fmt(v.spindle_speed, 0)}</span></span>
            <span>Parts <span className="mono">{v.part_count}</span></span>
            {v.axes.map(([n, p]) => <span key={n}>{n} <span className="mono">{fmt(p)}</span></span>)}
          </div>
          <div style={{ ...card, fontSize: "var(--fs-xs)", color: "var(--text-dim)" }}>
            Block: <span className="mono" style={{ color: "var(--text)" }}>{v.block || "\u2014"}</span>
          </div>
          <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
            <Button variant="primary" icon={<Play size={14} strokeWidth={1.75} />} onClick={() => run("focas_sim_action", { action: "cycle_start" })}>Cycle start</Button>
            <Button icon={<Pause size={14} strokeWidth={1.75} />} onClick={() => run("focas_sim_action", { action: "feed_hold" })}>Feed hold</Button>
            <Button icon={<RotateCcw size={14} strokeWidth={1.75} />} onClick={() => run("focas_sim_action", { action: "reset" })}>Reset</Button>
            <Button variant={emergency ? "default" : "danger"} icon={<Square size={14} strokeWidth={1.75} />} onClick={() => run("focas_sim_action", { action: emergency ? "emergency_off" : "emergency_on" })}>
              {emergency ? "Release emergency stop" : "Emergency stop"}
            </Button>
          </div>
          <div style={{ ...card, display: "flex", flexDirection: "column", gap: 8 }}>
            <div style={{ display: "flex", gap: 8, alignItems: "flex-end", flexWrap: "wrap" }}>
              <Field label="Alarm number"><input className="vt-input vt-input--mono" style={{ width: 90 }} type="number" value={number} onChange={(e) => setNumber(Number(e.target.value))} /></Field>
              <Field label="Type"><SheetSelect aria-label="Alarm type" sheetTitle="Alarm type" value={type} onChange={setType} options={ALARM_TYPES} /></Field>
              <Field label="Message"><input className="vt-input" style={{ width: 240 }} value={message} onChange={(e) => setMessage(e.target.value)} /></Field>
              <Button variant="danger" icon={<TriangleAlert size={14} strokeWidth={1.75} />} onClick={() => run("focas_sim_raise_alarm", { number, alarmType: type, message })}>Raise alarm</Button>
              <Button onClick={() => run("focas_sim_clear_alarm", { number: null })}>Clear all</Button>
            </div>
            {v.alarms.map((a) => (
              <div key={a.number} style={{ display: "flex", gap: 8, alignItems: "center", fontSize: "var(--fs-xs)" }}>
                <Badge tone="danger">{a.number}</Badge>
                <span style={{ color: "var(--text-dim)" }}>{alarmTypeName(a.alarm_type)}</span>
                <span style={{ flex: 1 }}>{a.message}</span>
                <Button size="sm" onClick={() => run("focas_sim_clear_alarm", { number: a.number })}>Clear</Button>
              </div>
            ))}
          </div>
          <div style={{ ...card, display: "flex", flexDirection: "column", gap: 8 }}>
            <div style={{ display: "flex", gap: 8, alignItems: "flex-end" }}>
              <Field label="Macro #"><input className="vt-input vt-input--mono" style={{ width: 90 }} type="number" min={0} value={macroNo} onChange={(e) => setMacroNo(Number(e.target.value))} /></Field>
              <Field label="Value"><input className="vt-input vt-input--mono" style={{ width: 120 }} type="number" step="any" value={macroVal} onChange={(e) => setMacroVal(e.target.value)} /></Field>
              <Button onClick={() => run("focas_sim_set_macro", { number: macroNo, value: Number(macroVal) })}>Set macro</Button>
            </div>
            <div style={{ display: "flex", flexWrap: "wrap", gap: 12, fontSize: "var(--fs-xs)" }}>
              {v.macros.map(([n, val]) => (
                <span key={n} className="mono" style={{ cursor: "pointer" }} onClick={() => { setMacroNo(n); setMacroVal(String(val)); }}>#{n}={val}</span>
              ))}
              {v.macros.length === 0 && <span style={{ color: "var(--text-muted)" }}>No macro variables set.</span>}
            </div>
          </div>
        </>
      )}
    </div>
  );
}
