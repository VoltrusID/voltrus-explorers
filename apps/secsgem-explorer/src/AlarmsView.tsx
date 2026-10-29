import { invoke } from "@tauri-apps/api/core";
import { Badge, Button, Callout, DataTable } from "@mactools/ui";
import { RefreshCw } from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import type { AckView, AlarmView } from "./types";
import { AckBadge, clock, hint, mono, row, type AddLog } from "./ui";

export interface AlarmEvent {
  time: number;
  alid: number;
  set: boolean;
  category: number;
  text: string;
}

interface Props {
  events: AlarmEvent[];
  onClear: () => void;
  addLog: AddLog;
}

export default function AlarmsView({ events, onClear, addLog }: Props) {
  const [alarms, setAlarms] = useState<AlarmView[]>([]);
  const [loadedAt, setLoadedAt] = useState(0);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [ack, setAck] = useState<AckView | null>(null);

  const load = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      setAlarms(await invoke<AlarmView[]>("gem_alarm_list"));
      setLoadedAt(Date.now());
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }, []);

  const enable = async (on: boolean, alid: number | null) => {
    setError(null);
    try {
      const a = await invoke<AckView>("gem_alarm_enable", { enable: on, alid });
      setAck(a);
      addLog(a.ok ? "info" : "error", `${on ? "Enable" : "Disable"} ${alid ?? "all"} alarms: ${a.name}`);
    } catch (e) {
      setError(String(e));
    }
  };

  // An S5F1 after the list was read is newer than the list.
  const shown = useMemo(
    () =>
      alarms.map((a) => {
        const last = [...events].reverse().find((e) => e.alid === a.alid && e.time >= loadedAt);
        return { ...a, set: last ? last.set : a.set };
      }),
    [alarms, events, loadedAt],
  );

  return (
    <div style={{ display: "flex", flexDirection: "column", flex: 1, minHeight: 0, gap: 8 }}>
      <div style={row}>
        <Button size="sm" icon={<RefreshCw size={14} strokeWidth={1.75} />} onClick={() => void load()} disabled={busy}>
          {alarms.length ? "Reload alarms" : "List alarms"}
        </Button>
        <Button size="sm" onClick={() => void enable(true, null)}>Enable all</Button>
        <Button size="sm" onClick={() => void enable(false, null)}>Disable all</Button>
        {ack && <AckBadge ack={ack} />}
        <span style={{ flex: 1 }} />
        <span style={hint}>S5F5 list, S5F3 enable. Reports arrive as S5F1 while enabled.</span>
      </div>
      {error && <Callout tone="danger">{error}</Callout>}
      <div style={{ flex: 1, minHeight: 120, display: "flex", flexDirection: "column" }}>
        <DataTable
          columns={[
            { key: "alid", label: "ALID", width: "80px", align: "right" },
            { key: "state", label: "State", width: "90px" },
            { key: "category", label: "Category", width: "90px", align: "right" },
            { key: "text", label: "Text" },
            { key: "act", label: "Reporting", width: "180px" },
          ]}
          data={shown.map((a) => ({ alid: a.alid, state: a.set ? "SET" : "clear", category: a.category, text: a.text, set: a.set }))}
          emptyMessage={busy ? "Reading" : "List alarms (S5F5) to see every alarm the equipment defines"}
          renderCell={(key, r) => {
            if (key === "state") return <Badge tone={r.set ? "danger" : "neutral"}>{String(r.state)}</Badge>;
            if (key === "act") {
              return (
                <span style={{ display: "flex", gap: 4 }}>
                  <Button size="sm" onClick={() => void enable(true, Number(r.alid))}>Enable</Button>
                  <Button size="sm" onClick={() => void enable(false, Number(r.alid))}>Disable</Button>
                </span>
              );
            }
            return key === "alid" || key === "category" ? mono(r[key]) : String(r[key] ?? "");
          }}
        />
      </div>
      <div style={row}>
        <strong style={{ fontSize: "var(--fs-sm)" }}>Live alarm reports (S5F1)</strong>
        <Badge>{events.length}</Badge>
        <span style={{ flex: 1 }} />
        <Button size="sm" onClick={onClear} disabled={events.length === 0}>Clear</Button>
      </div>
      <div style={{ height: 150, display: "flex", flexDirection: "column" }}>
        <DataTable
          columns={[
            { key: "time", label: "Time", width: "110px" },
            { key: "alid", label: "ALID", width: "80px", align: "right" },
            { key: "state", label: "State", width: "90px" },
            { key: "category", label: "Category", width: "90px", align: "right" },
            { key: "text", label: "Text" },
          ]}
          data={events.map((e) => ({ time: clock(e.time), alid: e.alid, state: e.set ? "SET" : "CLEARED", category: e.category, text: e.text, set: e.set }))}
          autoScroll
          emptyMessage="No alarm reports received"
          renderCell={(key, r) => (key === "state" ? <Badge tone={r.set ? "danger" : "success"}>{String(r.state)}</Badge> : key === "alid" || key === "category" ? mono(r[key]) : String(r[key] ?? ""))}
        />
      </div>
    </div>
  );
}
