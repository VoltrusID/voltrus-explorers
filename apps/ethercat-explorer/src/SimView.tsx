import { Badge, Button, SheetSelect } from "@mactools/ui";
import { invoke } from "@tauri-apps/api/core";
import { Cable, Unplug, Zap } from "lucide-react";
import { useEffect, useState } from "react";
import type { SimAction, SimState } from "./types";
import { type AddLog, StateBadge, card, dim, hint, kv, row } from "./ui";

export default function SimView({ addLog }: { addLog: AddLog }) {
  const [state, setState] = useState<SimState | null>(null);
  const [cut, setCut] = useState("0");
  const [errDev, setErrDev] = useState("1");

  useEffect(() => {
    let alive = true;
    const poll = async () => {
      try {
        const s = await invoke<SimState | null>("sim_state");
        if (alive) setState(s);
      } catch { /* disconnected */ }
    };
    void poll();
    const t = setInterval(() => void poll(), 300);
    return () => { alive = false; clearInterval(t); };
  }, []);

  const act = async (a: SimAction) => {
    try { await invoke("sim_action", { action: a }); } catch (e) { addLog("error", String(e)); }
  };

  if (!state) return null;
  const options = state.devices.map((d) => ({ value: String(d.position), label: `${d.position}: ${d.name}` }));
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8, flex: 1, minHeight: 0, overflow: "auto" }}>
      <div style={card}>
        <div style={row}>
          <strong>Faults to try</strong>
          <span style={dim}>{state.frames.toLocaleString()} frames processed</span>
        </div>
        <div style={{ ...row, marginTop: 8 }}>
          {state.breakAfter === null ? (
            <>
              <span style={{ fontSize: "var(--fs-sm)" }}>Pull the cable after</span>
              <div style={{ width: 170 }}>
                <SheetSelect aria-label="Pull the cable after" sheetTitle="Pull the cable after" value={cut} options={options.slice(0, -1)} onChange={(v) => setCut(String(v))} />
              </div>
              <Button size="sm" icon={<Unplug size={14} strokeWidth={1.75} />} onClick={() => void act({ kind: "breakCable", after: Number(cut) })}>Pull</Button>
            </>
          ) : (
            <>
              <Badge tone="danger">Cable pulled after device {state.breakAfter}</Badge>
              <Button size="sm" icon={<Cable size={14} strokeWidth={1.75} />} onClick={() => void act({ kind: "breakCable", after: null })}>Plug back in</Button>
            </>
          )}
          <span style={{ width: 16 }} />
          <span style={{ fontSize: "var(--fs-sm)" }}>Bad cable into</span>
          <div style={{ width: 170 }}>
            <SheetSelect aria-label="Device with RX errors" sheetTitle="Device" value={errDev} options={options} onChange={(v) => setErrDev(String(v))} />
          </div>
          <Button size="sm" icon={<Zap size={14} strokeWidth={1.75} />} onClick={() => void act({ kind: "rxErrors", device: Number(errDev), port: 0, count: 12 })}>Add RX errors on port 0</Button>
        </div>
        <span style={{ ...hint, display: "block", marginTop: 6 }}>
          A pulled cable makes the devices after it disappear: the working counter drops, the last device closes its port and counts a lost link. RX errors show
          in the Registers view of that device, the way a bad connector does on a real line.
        </span>
      </div>
      <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fill, minmax(280px, 1fr))", gap: 8 }}>
        {state.devices.map((d) => (
          <div key={d.position} style={{ ...card, opacity: d.reachable ? 1 : 0.45 }}>
            <div style={row}>
              <strong>{d.position}: {d.name}</strong>
              <span style={{ flex: 1 }} />
              {d.reachable ? <StateBadge state={d.state} alText={d.error} /> : <Badge tone="danger">unreachable</Badge>}
            </div>
            <div style={{ ...hint, marginBottom: 6 }}>{d.description}</div>
            {d.summary.length > 0 && (
              <div style={kv}>
                {d.summary.map(([k, v]) => [<span key={`k${k}`} style={dim}>{k}</span>, <span key={`v${k}`} className="mono">{v}</span>])}
              </div>
            )}
            {d.actions.length > 0 && (
              <div style={{ ...row, marginTop: 8 }}>
                {d.actions.map(([id, label]) => (
                  <Button key={id} size="sm" onClick={() => void act({ kind: "device", device: d.position, action: id })}>{label}</Button>
                ))}
              </div>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}
