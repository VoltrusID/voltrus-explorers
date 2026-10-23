import { RotateCcw, RefreshCw, Play, Square } from "lucide-react";
import type { PortInfo } from "../types";
import { BAUD_PRESETS, PARITY_OPTIONS, DATA_BITS_OPTIONS, STOP_BITS_OPTIONS, TEMPLATES } from "../constants";
import { Button, Field, IconButton, Section, SheetSelect } from "@mactools/ui";
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

const BAUD_SELECT = [...BAUD_PRESETS.map((b) => ({ value: b, label: String(b) })), { value: 0, label: "Custom..." }];
const PARITY_SELECT = PARITY_OPTIONS.map((p) => ({ value: p, label: p.charAt(0).toUpperCase() + p.slice(1) }));
const DATA_BITS_SELECT = DATA_BITS_OPTIONS.map((d) => ({ value: d, label: String(d) }));
const STOP_BITS_SELECT = STOP_BITS_OPTIONS.map((s) => ({ value: s, label: String(s) }));

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
              <SheetSelect
                id={id}
                sheetTitle="Port"
                value={portName}
                options={ports.map((p) => ({
                  value: p.port_name,
                  label: p.port_name,
                  hint: [p.product, p.manufacturer, busyPorts.includes(p.port_name) ? "in use" : ""].filter(Boolean).join(" - ") || undefined,
                  disabled: busyPorts.includes(p.port_name),
                }))}
                placeholder="Select port..."
                onChange={(v) => setPortName(v)}
                disabled={connected}
              />
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
                <SheetSelect
                  id={id}
                  sheetTitle="Baud rate"
                  value={baudRate}
                  options={BAUD_SELECT}
                  onChange={(v) => setBaudRate(v)}
                  disabled={connected}
                />
              )
            }
          </Field>
          <Field label="Parity">
            {(id) => (
              <SheetSelect
                id={id}
                sheetTitle="Parity"
                value={parity}
                options={PARITY_SELECT}
                onChange={(v) => setParity(v)}
                disabled={connected}
              />
            )}
          </Field>
        </div>

        <div className="vt-field-row">
          <Field label="Data bits">
            {(id) => (
              <SheetSelect
                id={id}
                sheetTitle="Data bits"
                value={dataBits}
                options={DATA_BITS_SELECT}
                onChange={(v) => setDataBits(v)}
                disabled={connected}
              />
            )}
          </Field>
          <Field label="Stop bits">
            {(id) => (
              <SheetSelect
                id={id}
                sheetTitle="Stop bits"
                value={stopBits}
                options={STOP_BITS_SELECT}
                onChange={(v) => setStopBits(v)}
                disabled={connected}
              />
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
            <SheetSelect
              id={id}
              sheetTitle="Template"
              value={simTemplate ?? ""}
              options={Object.entries(TEMPLATES).map(([key, tmpl]) => ({ value: key, label: tmpl.name }))}
              placeholder="Select template..."
              onChange={(v) => {
                setSimTemplate(v || null);
                if (simRunning && !v) setSimRunning(false);
              }}
            />
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
