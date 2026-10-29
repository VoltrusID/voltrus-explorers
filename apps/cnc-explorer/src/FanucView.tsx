import { useCallback, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Badge, Button, Callout, DataTable, Dialog, Field, IconButton, ProBadge, SheetSelect, Tabs } from "@mactools/ui";
import type { Tone } from "@mactools/ui";
import { Pencil, Plus, RefreshCw, X } from "lucide-react";
import { alarmTypeName, fanucTrendRows, fmt, pmcHex } from "./fanuc";
import type { AlarmMessage, FanucPoll, NumberedValues, ProgramEntry, SysInfo } from "./types";

type Sub = "overview" | "axes" | "program" | "alarms" | "macros" | "data";

interface Props {
  info: SysInfo;
  poll: FanucPoll | null;
  trending: Set<string>;
  onToggleTrend: (name: string) => void;
  requirePro: (feature: string) => boolean;
  addLog: (direction: "tx" | "rx" | "info" | "error", message: string) => void;
}

const mono = (v: unknown) => <span className="mono">{String(v ?? "")}</span>;
const card: React.CSSProperties = {
  background: "var(--card)", border: "1px solid var(--border)", borderRadius: "var(--radius-lg)", padding: 12, minWidth: 0,
};
const inputNum: React.CSSProperties = { width: 96 };

function KeyValues({ title, rows }: { title: string; rows: [string, React.ReactNode][] }) {
  return (
    <div style={{ ...card, flex: 1 }}>
      <div style={{ fontSize: "var(--fs-xs)", fontWeight: 600, color: "var(--text-dim)", marginBottom: 6 }}>{title}</div>
      {rows.map(([k, v]) => (
        <div key={k} style={{ display: "flex", justifyContent: "space-between", gap: 8, padding: "3px 0", borderBottom: "1px solid var(--border)", fontSize: "var(--fs-xs)" }}>
          <span style={{ color: "var(--text-dim)" }}>{k}</span>
          <span className="mono" style={{ color: "var(--text)", textAlign: "right" }}>{v}</span>
        </div>
      ))}
    </div>
  );
}

export default function FanucView(props: Props) {
  const [sub, setSub] = useState<Sub>("overview");
  const { poll } = props;
  const alarming = (poll?.alarm_bits ?? 0) !== 0;
  return (
    <div style={{ display: "flex", flexDirection: "column", flex: 1, minHeight: 0, gap: 8 }}>
      <Tabs
        label="FANUC views"
        value={sub}
        onChange={setSub}
        items={[
          { value: "overview", label: "Overview" },
          { value: "axes", label: "Axes" },
          { value: "program", label: "Program" },
          { value: "alarms", label: "Alarms", count: alarming ? poll?.alarm_types.length || 1 : undefined },
          { value: "macros", label: "Macros" },
          { value: "data", label: "Parameters, diagnosis, PMC" },
        ]}
      />
      {sub === "overview" && <Overview {...props} />}
      {sub === "axes" && <Axes poll={poll} />}
      {sub === "program" && <Program poll={poll} addLog={props.addLog} />}
      {sub === "alarms" && <Alarms poll={poll} addLog={props.addLog} />}
      {sub === "macros" && <Macros requirePro={props.requirePro} addLog={props.addLog} />}
      {sub === "data" && <NumberedReads addLog={props.addLog} />}
    </div>
  );
}

function statusTone(label: string, bad: string[]): Tone {
  return bad.includes(label) ? "danger" : "neutral";
}

function Overview({ info, poll, trending, onToggleTrend, requirePro }: Props) {
  const s = poll?.status;
  const rows = fanucTrendRows(poll);
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8, overflow: "auto", flex: 1 }}>
      <div style={{ display: "flex", gap: 8 }}>
        <KeyValues
          title="Identity"
          rows={[
            ["CNC type", info.cnc_type.trim()],
            ["Series", info.series.trim()],
            ["Version", info.version.trim()],
            ["Machine type", info.mt_type.trim()],
            ["Axes", info.axes.trim()],
            ["Max axes", info.max_axis],
          ]}
        />
        <KeyValues
          title="Status"
          rows={
            s
              ? [
                  ["Mode", <>{s.mode.label} ({s.mode.name})</>],
                  ["Run", <>{s.run.label} ({s.run.name})</>],
                  ["Motion", <>{s.motion.label}</>],
                  ["M/S/T/B", <>{s.mstb.label}</>],
                  ["Emergency", <Badge tone={statusTone(s.emergency.label, ["EMG"])}>{s.emergency.label}</Badge>],
                  ["Alarm", <Badge tone={statusTone(s.alarm.label, ["ALM", "PMC"])}>{s.alarm.label}</Badge>],
                  ["Edit", <>{s.edit.label}</>],
                ]
              : [["Status", "\u2014"]]
          }
        />
      </div>
      {poll && poll.errors.length > 0 && (
        <Callout tone="warn">
          The controller refused {poll.errors.length} item(s): {poll.errors.slice(0, 3).join("; ")}
        </Callout>
      )}
      <div style={{ minHeight: 160, display: "flex", flexDirection: "column" }}>
        <DataTable
          columns={[
            { key: "name", label: "Live value" },
            { key: "value", label: "Value", align: "right" },
            { key: "trend", label: "Trend", width: "56px", align: "center" },
          ]}
          data={rows.map((r) => ({ name: r.name, value: fmt(r.value, 2) }))}
          emptyMessage="Waiting for the first poll"
          renderCell={(key, row) => {
            if (key === "trend") {
              const name = String(row.name);
              const active = trending.has(name);
              return (
                <IconButton
                  size="sm"
                  label={active ? `Remove ${name} from trend` : `Add ${name} to trend`}
                  pressed={active}
                  onClick={(e) => { e.stopPropagation(); if (!active && !requirePro("Live trends")) return; onToggleTrend(name); }}
                >
                  {active ? <X size={14} strokeWidth={1.75} /> : <Plus size={14} strokeWidth={1.75} />}
                </IconButton>
              );
            }
            return key === "value" ? mono(row[key]) : String(row[key] ?? "");
          }}
        />
      </div>
    </div>
  );
}

function Axes({ poll }: { poll: FanucPoll | null }) {
  return (
    <DataTable
      columns={[
        { key: "name", label: "Axis", width: "70px" },
        { key: "absolute", label: "Absolute", align: "right" },
        { key: "machine", label: "Machine", align: "right" },
        { key: "relative", label: "Relative", align: "right" },
        { key: "dtg", label: "Distance to go", align: "right" },
        { key: "load", label: "Servo load %", align: "right" },
      ]}
      data={(poll?.axes ?? []).map((a) => ({
        name: a.name, absolute: fmt(a.absolute), machine: fmt(a.machine), relative: fmt(a.relative), dtg: fmt(a.distance_to_go), load: fmt(a.servo_load, 1),
      }))}
      emptyMessage="Waiting for the first poll"
      renderCell={(key, row) => (key === "name" ? String(row[key]) : mono(row[key]))}
    />
  );
}

function Program({ poll, addLog }: { poll: FanucPoll | null; addLog: Props["addLog"] }) {
  const p = poll?.program;
  const [list, setList] = useState<ProgramEntry[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const load = useCallback(async () => {
    setError(null);
    try { setList(await invoke<ProgramEntry[]>("fanuc_programs")); } catch (e) { setError(String(e)); addLog("error", `Program list failed: ${String(e)}`); }
  }, [addLog]);
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8, overflow: "auto", flex: 1 }}>
      <div style={{ display: "flex", gap: 8 }}>
        <KeyValues
          title="Running program"
          rows={[
            ["Running (O)", p?.running ?? "\u2014"],
            ["Main (O)", p?.main ?? "\u2014"],
            ["Name", p?.name || "\u2014"],
            ["Sequence (N)", p?.sequence ?? "\u2014"],
          ]}
        />
        <div style={{ ...card, flex: 1 }}>
          <div style={{ fontSize: "var(--fs-xs)", fontWeight: 600, color: "var(--text-dim)", marginBottom: 6 }}>Executing block</div>
          <pre className="mono" style={{ margin: 0, fontSize: "var(--fs-sm)", whiteSpace: "pre-wrap", color: "var(--text)" }}>{p?.block || "\u2014"}</pre>
        </div>
      </div>
      <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
        <Button size="sm" icon={<RefreshCw size={14} strokeWidth={1.75} />} onClick={load}>List programs</Button>
        {list && <span style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)" }}>{list.length} program(s)</span>}
      </div>
      {error && <Callout tone="danger">{error}</Callout>}
      {list && (
        <div style={{ minHeight: 160, display: "flex", flexDirection: "column" }}>
          <DataTable
            columns={[
              { key: "number", label: "O number", width: "100px", align: "right" },
              { key: "size", label: "Size (bytes)", width: "120px", align: "right" },
              { key: "comment", label: "Comment" },
            ]}
            data={list.map((e) => ({ number: e.number, size: e.size, comment: e.comment }))}
            emptyMessage="No programs"
          />
        </div>
      )}
    </div>
  );
}

function Alarms({ poll, addLog }: { poll: FanucPoll | null; addLog: Props["addLog"] }) {
  const [messages, setMessages] = useState<AlarmMessage[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const load = useCallback(async () => {
    setError(null);
    try { setMessages(await invoke<AlarmMessage[]>("fanuc_alarms")); } catch (e) { setError(String(e)); addLog("error", `Alarm read failed: ${String(e)}`); }
  }, [addLog]);
  const active = (poll?.alarm_bits ?? 0) !== 0;
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8, flex: 1, minHeight: 0 }}>
      <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
        <Badge tone={active ? "danger" : "success"}>{active ? "Alarm active" : "No alarm"}</Badge>
        {poll && poll.alarm_types.length > 0 && <span style={{ fontSize: "var(--fs-xs)", color: "var(--text-dim)" }}>{poll.alarm_types.join(", ")}</span>}
        <span style={{ flex: 1 }} />
        <Button size="sm" icon={<RefreshCw size={14} strokeWidth={1.75} />} onClick={load}>Read alarm messages</Button>
      </div>
      {error && <Callout tone="danger">{error}</Callout>}
      <DataTable
        columns={[
          { key: "number", label: "Number", width: "100px", align: "right" },
          { key: "type", label: "Type", width: "160px" },
          { key: "axis", label: "Axis", width: "70px", align: "right" },
          { key: "text", label: "Message" },
        ]}
        data={(messages ?? []).map((m) => ({ number: m.number, type: alarmTypeName(m.alarm_type), axis: m.axis || "", text: m.text ?? "" }))}
        emptyMessage={messages ? "No alarm messages" : "Read alarm messages to list them"}
      />
    </div>
  );
}

function Macros({ requirePro, addLog }: Pick<Props, "requirePro" | "addLog">) {
  const [first, setFirst] = useState(500);
  const [last, setLast] = useState(520);
  const [values, setValues] = useState<{ number: number; value: number | null }[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [writing, setWriting] = useState<{ number: string; value: string } | null>(null);
  const [writeError, setWriteError] = useState<string | null>(null);

  const read = useCallback(async () => {
    setError(null);
    try { setValues(await invoke("fanuc_read_macro", { first, last })); } catch (e) { setError(String(e)); }
  }, [first, last]);

  const openWrite = (number: string, value: string) => {
    if (!requirePro("Macro variable writes")) return;
    setWriteError(null);
    setWriting({ number, value });
  };

  const confirmWrite = async () => {
    if (!writing) return;
    const number = Number(writing.number);
    const value = Number(writing.value);
    if (!Number.isInteger(number) || number < 0 || writing.value.trim() === "" || !Number.isFinite(value)) {
      setWriteError("Enter a macro number and a numeric value.");
      return;
    }
    try {
      await invoke("fanuc_write_macro", { number, value });
      setWriting(null);
      addLog("info", `Macro #${number} written, reading back`);
      setFirst(number);
      setLast(number);
      setValues(await invoke("fanuc_read_macro", { first: number, last: number }));
    } catch (e) {
      setWriteError(String(e));
    }
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8, flex: 1, minHeight: 0 }}>
      <div style={{ display: "flex", gap: 8, alignItems: "flex-end" }}>
        <Field label="From #"><input className="vt-input vt-input--mono" style={inputNum} type="number" min={0} value={first} onChange={(e) => setFirst(Number(e.target.value))} /></Field>
        <Field label="To #"><input className="vt-input vt-input--mono" style={inputNum} type="number" min={0} value={last} onChange={(e) => setLast(Number(e.target.value))} /></Field>
        <Button variant="primary" onClick={read}>Read</Button>
        <span style={{ flex: 1 }} />
        <Button icon={<Pencil size={14} strokeWidth={1.75} />} onClick={() => openWrite(String(first), "")}>Write variable <ProBadge /></Button>
      </div>
      {error && <Callout tone="danger">{error}</Callout>}
      <DataTable
        columns={[
          { key: "number", label: "Variable", width: "120px" },
          { key: "value", label: "Value", align: "right" },
        ]}
        data={(values ?? []).map((v) => ({ number: v.number, label: `#${v.number}`, value: v.value == null ? "\u2014" : String(v.value) }))}
        emptyMessage={values ? "No values" : "Read a range of macro variables (1 to 200)"}
        onRowClick={(r) => openWrite(String(r.number), String(r.value) === "\u2014" ? "" : String(r.value))}
        renderCell={(key, row) => (key === "number" ? mono(`#${row.number}`) : mono(row[key]))}
      />
      <Dialog
        open={writing != null}
        onClose={() => setWriting(null)}
        title="Write macro variable"
        width={380}
        footer={(
          <>
            <Button variant="ghost" onClick={() => setWriting(null)}>Cancel</Button>
            <Button variant="danger" onClick={confirmWrite}>Write to CNC</Button>
          </>
        )}
      >
        {writing && (
          <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
            <Callout tone="warn">This changes data in the controller. Make sure the variable is not in use by a running program.</Callout>
            <div style={{ display: "flex", gap: 8 }}>
              <Field label="Variable #"><input className="vt-input vt-input--mono" type="number" min={0} value={writing.number} onChange={(e) => setWriting({ ...writing, number: e.target.value })} /></Field>
              <Field label="Value"><input className="vt-input vt-input--mono" type="number" step="any" value={writing.value} onChange={(e) => setWriting({ ...writing, value: e.target.value })} autoFocus /></Field>
            </div>
            <span style={{ fontSize: "var(--fs-sm)", color: "var(--text-dim)" }}>
              Write <strong>#{writing.number || "?"} = {writing.value || "?"}</strong> to the connected CNC?
            </span>
            {writeError && <Callout tone="danger">{writeError}</Callout>}
          </div>
        )}
      </Dialog>
    </div>
  );
}

const PMC_AREAS = ["G", "F", "Y", "X", "A", "R", "T", "K", "C", "D"].map((a) => ({ value: a.toLowerCase(), label: `${a} area` }));
const PMC_TYPES = [{ value: "byte", label: "Byte" }, { value: "word", label: "Word" }, { value: "d_word", label: "Double word" }];
const PMC_BYTES: Record<string, number> = { byte: 1, word: 2, d_word: 4 };

type Kind = "param" | "diag" | "pmc";

function NumberedReads({ addLog }: Pick<Props, "addLog">) {
  const [kind, setKind] = useState<Kind>("param");
  const [first, setFirst] = useState(1020);
  const [last, setLast] = useState(1020);
  const [axis, setAxis] = useState(-1);
  const [area, setArea] = useState("d");
  const [dtype, setDtype] = useState("word");
  const [count, setCount] = useState(8);
  const [rows, setRows] = useState<Record<string, unknown>[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const read = useCallback(async () => {
    setError(null);
    try {
      if (kind === "pmc") {
        const out = await invoke<{ address: number; value: number }[]>("fanuc_read_pmc", { area, dataType: dtype, first, count });
        const bytes = PMC_BYTES[dtype];
        setRows(out.map((v) => ({ address: `${area.toUpperCase()}${v.address}`, dec: v.value, hex: pmcHex(v.value, bytes) })));
      } else {
        const out = await invoke<NumberedValues[]>(kind === "param" ? "fanuc_read_params" : "fanuc_read_diag", { first, last, axis });
        setRows(out.map((v) => ({ number: v.number, axis: v.axis, values: v.values.join(", ") })));
      }
    } catch (e) {
      setError(String(e));
      addLog("error", String(e));
    }
  }, [kind, first, last, axis, area, dtype, count, addLog]);

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8, flex: 1, minHeight: 0 }}>
      <div style={{ display: "flex", gap: 8, alignItems: "flex-end", flexWrap: "wrap" }}>
        <Field label="Read">
          <SheetSelect aria-label="Read kind" sheetTitle="Read" value={kind} onChange={(v) => {
            setKind(v as Kind);
            setRows(null);
            if (v === "param") { setFirst(1020); setLast(1020); }
            else if (v === "diag") { setFirst(308); setLast(309); }
            else setFirst(0);
          }}
            options={[{ value: "param", label: "Parameters" }, { value: "diag", label: "Diagnosis" }, { value: "pmc", label: "PMC" }]} />
        </Field>
        {kind === "pmc" ? (
          <>
            <Field label="Area"><SheetSelect aria-label="PMC area" sheetTitle="PMC area" value={area} onChange={setArea} options={PMC_AREAS} /></Field>
            <Field label="Type"><SheetSelect aria-label="PMC data type" sheetTitle="Data type" value={dtype} onChange={setDtype} options={PMC_TYPES} /></Field>
            <Field label="Address"><input className="vt-input vt-input--mono" style={inputNum} type="number" min={0} value={first} onChange={(e) => setFirst(Number(e.target.value))} /></Field>
            <Field label="Count"><input className="vt-input vt-input--mono" style={inputNum} type="number" min={1} value={count} onChange={(e) => setCount(Number(e.target.value))} /></Field>
          </>
        ) : (
          <>
            <Field label="From"><input className="vt-input vt-input--mono" style={inputNum} type="number" min={0} value={first} onChange={(e) => setFirst(Number(e.target.value))} /></Field>
            <Field label="To"><input className="vt-input vt-input--mono" style={inputNum} type="number" min={0} value={last} onChange={(e) => setLast(Number(e.target.value))} /></Field>
            <Field label="Axis (-1 all, 0 none)"><input className="vt-input vt-input--mono" style={inputNum} type="number" min={-1} value={axis} onChange={(e) => setAxis(Number(e.target.value))} /></Field>
          </>
        )}
        <Button variant="primary" onClick={read}>Read</Button>
      </div>
      {error && <Callout tone="danger">{error}</Callout>}
      {kind === "pmc" ? (
        <DataTable
          columns={[{ key: "address", label: "Address", width: "120px" }, { key: "dec", label: "Decimal", align: "right" }, { key: "hex", label: "Hex", align: "right" }]}
          data={rows ?? []}
          emptyMessage="Choose an area and address, then Read"
          renderCell={(key, row) => mono(row[key])}
        />
      ) : (
        <DataTable
          columns={[{ key: "number", label: "Number", width: "110px", align: "right" }, { key: "axis", label: "Axis", width: "70px", align: "right" }, { key: "values", label: "Value(s), raw", align: "right" }]}
          data={rows ?? []}
          emptyMessage="Choose a range, then Read"
          renderCell={(key, row) => mono(row[key])}
        />
      )}
    </div>
  );
}
