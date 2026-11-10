import { invoke } from "@tauri-apps/api/core";
import { Badge, Button, Callout, Field } from "@mactools/ui";
import { AlertTriangle, Power, Zap } from "lucide-react";
import { useEffect, useState } from "react";
import type { SimAction, SimState } from "./types";
import { card, hint, row, type AddLog } from "./ui";

interface Props {
  addLog: AddLog;
  /** Connect the app to the virtual bus (and load the simulator DBC). */
  onConnect: () => void;
  connectedToVirtual: boolean;
}

function Gauge({ label, value, unit, digits = 0 }: { label: string; value: number; unit: string; digits?: number }) {
  return (
    <div style={{ ...card, flex: "1 1 140px", display: "flex", flexDirection: "column", gap: 2 }}>
      <span style={{ fontSize: "var(--fs-xs)", color: "var(--text-dim)" }}>{label}</span>
      <span className="mono" style={{ fontSize: 22, fontWeight: 600 }}>{value.toFixed(digits)} <span style={{ fontSize: "var(--fs-sm)", color: "var(--text-muted)" }}>{unit}</span></span>
    </div>
  );
}

export default function SimDashboard({ addLog, onConnect, connectedToVirtual }: Props) {
  const [s, setS] = useState<SimState | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    const tick = async () => {
      try {
        const v = await invoke<SimState | null>("sim_state");
        if (alive) setS(v);
      } catch (e) {
        if (alive) setError(String(e));
      }
    };
    void tick();
    const t = setInterval(tick, 300);
    return () => { alive = false; clearInterval(t); };
  }, []);

  const act = async (action: SimAction) => {
    setError(null);
    try {
      await invoke("sim_action", { action });
      setS(await invoke<SimState | null>("sim_state"));
    } catch (e) {
      setError(String(e));
      addLog("error", String(e));
    }
  };

  if (!s) return <div style={hint}>{error ?? "Reading the simulator"}</div>;

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8, overflow: "auto", flex: 1, minHeight: 0 }}>
      <div style={row}>
        <Badge tone="success">Running on the virtual bus</Badge>
        <Badge tone={s.running ? "success" : "neutral"}>{s.running ? "Engine running" : "Engine stopped"}</Badge>
        {s.fault && <Badge tone="danger">DM1 fault active</Badge>}
        <span style={hint}>{s.sent.toLocaleString()} frames sent</span>
        <span style={{ flex: 1 }} />
        {!connectedToVirtual && <Button size="sm" variant="primary" onClick={onConnect}>Monitor it</Button>}
      </div>
      {error && <Callout tone="danger">{error}</Callout>}
      <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
        <Gauge label="Engine speed" value={s.rpm} unit="rpm" />
        <Gauge label="Coolant" value={s.coolant} unit="°C" digits={1} />
        <Gauge label="Oil pressure" value={s.oilPressure} unit="kPa" />
        <Gauge label="Frequency" value={s.frequency} unit="Hz" digits={2} />
        <Gauge label="Load" value={s.loadKw} unit="kW" digits={1} />
        <Gauge label="Pump" value={s.pumpSpeed} unit="rpm" />
      </div>
      <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
        <div style={{ ...card, flex: "1 1 320px", display: "flex", flexDirection: "column", gap: 8 }}>
          <div style={{ fontSize: "var(--fs-xs)", fontWeight: 600, color: "var(--text-dim)" }}>Genset</div>
          <div style={row}>
            <Button size="sm" variant={s.running ? "danger" : "primary"} icon={<Power size={14} strokeWidth={1.75} />} onClick={() => void act({ action: "setRunning", running: !s.running })}>
              {s.running ? "Stop engine" : "Start engine"}
            </Button>
            <Button size="sm" icon={<Zap size={14} strokeWidth={1.75} />} onClick={() => void act({ action: "setBreaker", closed: !s.breakerClosed })}>
              {s.breakerClosed ? "Open breaker" : "Close breaker"}
            </Button>
            <Button size="sm" variant={s.fault ? "danger" : undefined} icon={<AlertTriangle size={14} strokeWidth={1.75} />} onClick={() => void act({ action: "setFault", active: !s.fault })}>
              {s.fault ? "Clear fault" : "Raise coolant fault"}
            </Button>
          </div>
          <Field label={`Throttle: ${s.throttle.toFixed(0)} %`}>
            <input type="range" min={0} max={100} step={1} value={s.throttle} onChange={(e) => void act({ action: "setThrottle", value: Number(e.target.value) })} />
          </Field>
          <span style={hint}>The throttle also follows ThrottleCommand (0x511) frames sent from the Signals tab or the transmit list.</span>
        </div>
        <div style={{ ...card, flex: "1 1 320px", display: "flex", flexDirection: "column", gap: 8 }}>
          <div style={{ fontSize: "var(--fs-xs)", fontWeight: 600, color: "var(--text-dim)" }}>Cooling pump and bus</div>
          <div style={row}>
            <label style={{ display: "flex", gap: 6, alignItems: "center", fontSize: "var(--fs-sm)" }}>
              <input type="checkbox" checked={s.pumpEnabled} onChange={(e) => void act({ action: "setPump", enabled: e.target.checked, setpoint: s.pumpSetpoint })} /> Pump enabled
            </label>
          </div>
          <Field label={`Pump setpoint: ${s.pumpSetpoint.toFixed(0)} rpm`}>
            <input type="range" min={0} max={3000} step={10} value={s.pumpSetpoint} onChange={(e) => void act({ action: "setPump", enabled: s.pumpEnabled, setpoint: Number(e.target.value) })} />
          </Field>
          <Field label={`Message rate: ${s.rate.toFixed(1)}x`}>
            <input type="range" min={0.1} max={10} step={0.1} value={s.rate} onChange={(e) => void act({ action: "setRate", rate: Number(e.target.value) })} />
          </Field>
        </div>
      </div>
      <div style={{ ...card, display: "flex", flexDirection: "column", gap: 4 }}>
        <div style={{ fontSize: "var(--fs-xs)", fontWeight: 600, color: "var(--text-dim)" }}>On the bus</div>
        <span style={hint}>
          J1939 engine ECU (SA 00): EEC1 every 20 ms, ET1, EFL/P1, HOURS and DM1. Generator controller 0x321 and 0x322 (Motorola byte order), pump drive 0x410
          (multiplexed), CANopen I/O node 0x11 (heartbeat 0x711, TPDO1 0x191, answers SDO reads of 1000, 1017 and 1018), and OBD-II answers on 0x7E8 for
          PIDs 00, 04, 05, 0C, 0D, 11 and 2F. Pick J1939 or CANopen as the protocol to label them, and use the simulator's DBC to decode the signals.
        </span>
      </div>
    </div>
  );
}
