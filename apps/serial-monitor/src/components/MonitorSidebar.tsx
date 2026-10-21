import { RotateCcw, RefreshCw, Play, Square } from "lucide-react";
import type { PortInfo } from "../types";
import { BAUD_PRESETS, PARITY_OPTIONS, DATA_BITS_OPTIONS, STOP_BITS_OPTIONS, TEMPLATES } from "../constants";
import { Button, Field, IconButton, Section } from "@mactools/ui";
import { ConnectionHealth } from "./ConnectionHealth";
import { ProfilesSection } from "./ProfilesSection";
import type { PortProfile } from "../profiles";

interface MonitorSidebarProps {
  // Connection form
  connected: boolean;
  connecting: boolean;
  ports: PortInfo[];
  portName: string;
  setPortName: (v: string) => void;
  baudRate: number;
  setBaudRate: (v: number) => void;
  dataBits: number;
  setDataBits: (v: number) => void;
  stopBits: number;
  setStopBits: (v: number) => void;
  parity: string;
  setParity: (v: string) => void;
  timeoutMs: number;
  setTimeoutMs: (v: number) => void;
  lastDataTime: number | null;
  // Handlers
  refreshPorts: () => Promise<PortInfo[]>;
  handleConnect: () => void;
  handleDisconnect: () => void;
  // Simulation
  simTemplate: string | null;
  setSimTemplate: (v: string | null) => void;
  simRunning: boolean;
  setSimRunning: (v: boolean) => void;
  onSimStart: () => void;
  onSimStop: () => void;
  /** Ports another session has picked; they cannot be chosen here. */
  busyPorts: string[];
  // Saved profiles
  profiles: PortProfile[];
  setProfiles: (list: PortProfile[]) => void;
  applyProfile: (p: PortProfile) => void;
}

export function MonitorSidebar({
  connected,
  connecting,
  ports,
  portName,
  setPortName,
  baudRate,
  setBaudRate,
  dataBits,
  setDataBits,
  stopBits,
  setStopBits,
  parity,
  setParity,
  timeoutMs,
  setTimeoutMs,
  lastDataTime,
  refreshPorts,
  handleConnect,
  handleDisconnect,
  simTemplate,
  setSimTemplate,
  simRunning,
  setSimRunning,
  onSimStart,
  onSimStop,
  busyPorts,
  profiles,
  setProfiles,
  applyProfile,
}: MonitorSidebarProps) {
  return (
    <>
      <Section title="Connection">
        <Field label="Port">
          {(id) => (
            <div style={{ display: "flex", gap: 6 }}>
              <select
                id={id}
                className="vt-select"
                value={portName}
                onChange={(e) => setPortName(e.target.value)}
                disabled={connected}
              >
                <option value="">Select port...</option>
                {ports.map((p) => (
                  <option key={p.port_name} value={p.port_name} disabled={busyPorts.includes(p.port_name)}>
                    {p.port_name}
                    {p.product ? ` (${p.product})` : ""}
                    {p.manufacturer ? ` - ${p.manufacturer}` : ""}
                    {busyPorts.includes(p.port_name) ? " (in use)" : ""}
                  </option>
                ))}
              </select>
              <IconButton
                label="Refresh ports"
                onClick={() => refreshPorts()}
                disabled={connected}
              >
                <RefreshCw size={16} strokeWidth={1.75} />
              </IconButton>
            </div>
          )}
        </Field>

        <div className="vt-field-row">
          <Field label="Baud rate">
            {(id) =>
              !BAUD_PRESETS.includes(baudRate) ? (
                <div style={{ display: "flex", gap: 6 }}>
                  <input
                    id={id}
                    type="number"
                    className="vt-input"
                    value={baudRate}
                    onChange={(e) => {
                      const v = parseInt(e.target.value, 10);
                      if (!isNaN(v) && v > 0) setBaudRate(v);
                    }}
                    disabled={connected}
                    min={1}
                  />
                  <IconButton
                    label="Back to baud presets"
                    size="sm"
                    onClick={() => setBaudRate(9600)}
                    disabled={connected}
                  >
                    <RotateCcw size={14} strokeWidth={1.75} />
                  </IconButton>
                </div>
              ) : (
                <select
                  id={id}
                  className="vt-select"
                  value={baudRate}
                  onChange={(e) => {
                    const v = e.target.value;
                    setBaudRate(v === "custom" ? 0 : Number(v));
                  }}
                  disabled={connected}
                >
                  {BAUD_PRESETS.map((b) => (
                    <option key={b} value={b}>{b}</option>
                  ))}
                  <option value="custom">Custom...</option>
                </select>
              )
            }
          </Field>
          <Field label="Parity">
            {(id) => (
              <select
                id={id}
                className="vt-select"
                value={parity}
                onChange={(e) => setParity(e.target.value)}
                disabled={connected}
              >
                {PARITY_OPTIONS.map((p) => (
                  <option key={p} value={p}>{p.charAt(0).toUpperCase() + p.slice(1)}</option>
                ))}
              </select>
            )}
          </Field>
        </div>

        <div className="vt-field-row">
          <Field label="Data bits">
            {(id) => (
              <select
                id={id}
                className="vt-select"
                value={dataBits}
                onChange={(e) => setDataBits(Number(e.target.value))}
                disabled={connected}
              >
                {DATA_BITS_OPTIONS.map((d) => (
                  <option key={d} value={d}>{d}</option>
                ))}
              </select>
            )}
          </Field>
          <Field label="Stop bits">
            {(id) => (
              <select
                id={id}
                className="vt-select"
                value={stopBits}
                onChange={(e) => setStopBits(Number(e.target.value))}
                disabled={connected}
              >
                {STOP_BITS_OPTIONS.map((s) => (
                  <option key={s} value={s}>{s}</option>
                ))}
              </select>
            )}
          </Field>
        </div>

        <Field label="Read timeout (ms)">
          {(id) => (
            <input
              id={id}
              type="number"
              className="vt-input"
              value={timeoutMs}
              onChange={(e) => {
                const v = parseInt(e.target.value, 10);
                if (!isNaN(v) && v >= 100 && v <= 30000) setTimeoutMs(v);
              }}
              disabled={connected}
              min={100}
              max={30000}
              step={100}
            />
          )}
        </Field>

        {connected && <ConnectionHealth lastDataTime={lastDataTime} />}

        <Button
          block
          variant={connected ? "danger" : "primary"}
          loading={connecting}
          disabled={(!connected && !portName) || baudRate <= 0}
          onClick={connected ? handleDisconnect : handleConnect}
        >
          {connecting ? "Connecting..." : connected ? "Disconnect" : "Connect"}
        </Button>
      </Section>

      <ProfilesSection
        profiles={profiles}
        setProfiles={setProfiles}
        onApply={applyProfile}
        disabled={connected}
        current={{ port_name: portName, baud_rate: baudRate, data_bits: dataBits, stop_bits: stopBits, parity, timeout_ms: timeoutMs }}
      />

      <Section title="Frame simulation">
        <Field label="Template">
          {(id) => (
            <select
              id={id}
              className="vt-select"
              value={simTemplate ?? ""}
              onChange={(e) => {
                const val = e.target.value || null;
                setSimTemplate(val);
                if (simRunning && !val) setSimRunning(false);
              }}
            >
              <option value="">Select template...</option>
              {Object.entries(TEMPLATES).map(([key, tmpl]) => (
                <option key={key} value={key}>{tmpl.name}</option>
              ))}
            </select>
          )}
        </Field>
        {simTemplate && (
          <div style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)", lineHeight: 1.45 }}>
            {TEMPLATES[simTemplate].description}
          </div>
        )}
        <Button
          block
          variant={simRunning ? "danger" : "default"}
          icon={
            simRunning ? (
              <Square size={14} strokeWidth={1.75} />
            ) : (
              <Play size={14} strokeWidth={1.75} />
            )
          }
          onClick={() => {
            if (simRunning) onSimStop();
            else if (simTemplate) onSimStart();
          }}
        >
          {simRunning ? "Stop simulation" : "Start simulation"}
        </Button>
      </Section>
    </>
  );
}
