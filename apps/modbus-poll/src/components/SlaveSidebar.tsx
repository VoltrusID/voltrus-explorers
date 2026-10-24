import { Button, Field, IconButton, Section, Segmented, SheetSelect } from "@mactools/ui";
import { Copy, X } from "lucide-react";
import { useStore } from "../store";

const BAUD_PRESETS = [1200, 2400, 4800, 9600, 19200, 38400, 57600, 115200];

interface Props {
  onSlaveStart: () => void;
  onSlaveStop: () => void;
}

export function SlaveSidebar({ onSlaveStart, onSlaveStop }: Props) {
  const {
    slaveRunning, slavePort, setSlavePort, slaveUnitId, setSlaveUnitId,
    slaveRegCount, setSlaveRegCount, slavePollMs, setSlavePollMs,
    slaveTransport, setSlaveTransport,
    slaveRtuPort, setSlaveRtuPort, slaveRtuBaud, setSlaveRtuBaud,
    slaveRtuDataBits, setSlaveRtuDataBits, slaveRtuStopBits, setSlaveRtuStopBits,
    slaveRtuParity, setSlaveRtuParity,
    slaveRtuVirtualPort, setSlaveRtuVirtualPort,
    slaveVirtualPortPath,
    serialPorts,
    platform,
  } = useStore();

  // iOS has no serial ports — Modbus TCP only.
  const isIos = platform === "ios";
  // The virtual RTU port is a pty pair: macOS only.
  const hasVirtualPort = platform === "macos";

  return (
    <>
      {/* Mode toggle */}
      <Segmented
        label="Mode"
        block
        value="slave"
        onChange={() => useStore.getState().setMode("master")}
        options={[
          { value: "master", label: "Master" },
          { value: "slave", label: "Slave", title: slaveRunning ? "Slave server running" : undefined },
        ]}
      />

      {/* Slave transport toggle (RTU requires serial hardware — hidden on iOS) */}
      {!isIos && (
        <Segmented
          label="Transport"
          block
          size="sm"
          value={slaveTransport}
          onChange={(v) => setSlaveTransport(v)}
          options={[
            { value: "tcp", label: "TCP" },
            { value: "rtu", label: "RTU" },
          ]}
        />
      )}

      {slaveTransport === "tcp" || isIos ? (
        <Section title="Slave server">
          <div className="vt-rows" style={{ display: "flex", flexDirection: "column", gap: 8 }}>
            <Field label="TCP port">
              <input className="vt-input" type="number" value={slavePort} onChange={(e) => setSlavePort(Number(e.target.value))} disabled={slaveRunning} />
            </Field>
            <Field label="Unit ID">
              <input className="vt-input" type="number" value={slaveUnitId} onChange={(e) => setSlaveUnitId(Number(e.target.value))} disabled={slaveRunning} />
            </Field>
            <Field label="Registers">
              <input className="vt-input" type="number" value={slaveRegCount} onChange={(e) => setSlaveRegCount(Number(e.target.value))} disabled={slaveRunning} />
            </Field>
            <Field label="Refresh (ms)">
              <input className="vt-input" type="number" value={slavePollMs} onChange={(e) => setSlavePollMs(Number(e.target.value))} min={100} step={100} />
            </Field>
          </div>
        </Section>
      ) : (
        <Section title="Slave RTU">
          <div className="vt-rows" style={{ display: "flex", flexDirection: "column", gap: 8 }}>
            <Field label="Serial port">
              <SheetSelect
                sheetTitle="Serial port"
                value={slaveRtuVirtualPort ? "__virtual__" : slaveRtuPort}
                onChange={(port) => {
                  if (port === "__virtual__") {
                    setSlaveRtuVirtualPort(true);
                    setSlaveRtuPort("");
                  } else {
                    setSlaveRtuVirtualPort(false);
                    setSlaveRtuPort(port);
                  }
                }}
                disabled={slaveRunning}
                options={[
                  ...(hasVirtualPort ? [{ value: "__virtual__", label: "Virtual port (PTY)" }] : []),
                  { value: "", label: "Select port…" },
                  ...serialPorts.map((p) => ({ value: p, label: p })),
                ]}
              />
            </Field>
            {slaveRtuVirtualPort && slaveRunning && slaveVirtualPortPath && (
              <div style={{ display: "flex", gap: 4, alignItems: "center" }}>
                <code style={{ flex: 1, fontSize: 11, padding: "4px 8px", background: "var(--bg)", border: "1px solid var(--border)", borderRadius: 4, userSelect: "all", wordBreak: "break-all" }}>
                  {slaveVirtualPortPath}
                </code>
                <Button size="sm" variant="ghost" onClick={() => navigator.clipboard.writeText(slaveVirtualPortPath)} icon={<Copy size={14} strokeWidth={1.75} />}>Copy</Button>
              </div>
            )}
            <div style={{ opacity: slaveRtuVirtualPort ? 0.4 : 1 }}><Field label="Baud rate">
              <div style={{ display: "flex", gap: 4 }}>
                {!BAUD_PRESETS.includes(slaveRtuBaud) ? (
                  <input
                    type="number"
                    className="vt-input vt-input--mono" style={{ flex: 1 }}
                    value={slaveRtuBaud || ""}
                    placeholder="Custom baud"
                    onChange={(e) => setSlaveRtuBaud(Number(e.target.value))}
                    disabled={slaveRunning || slaveRtuVirtualPort}
                  />
                ) : (
                  <SheetSelect
                    sheetTitle="Baud rate"
                    style={{ flex: 1 }}
                    value={slaveRtuBaud}
                    onChange={(v) => setSlaveRtuBaud(v === "custom" ? 0 : Number(v))}
                    disabled={slaveRunning || slaveRtuVirtualPort}
                    options={[...BAUD_PRESETS.map((b) => ({ value: b as string | number, label: String(b) })), { value: "custom", label: "Custom…" }]}
                  />
                )}
                {!BAUD_PRESETS.includes(slaveRtuBaud) && (
                  <IconButton label="Back to preset baud rates" size="sm" onClick={() => setSlaveRtuBaud(9600)} disabled={slaveRunning || slaveRtuVirtualPort}>
                    <X size={16} strokeWidth={1.75} />
                  </IconButton>
                )}
              </div>
            </Field></div>
            <div className="vt-field-grid" style={{ display: "flex", gap: 4, opacity: slaveRtuVirtualPort ? 0.4 : 1 }}>
              <div style={{ flex: 1 }}><Field stacked label="Data bits">
                <SheetSelect
                  sheetTitle="Data bits"
                  value={slaveRtuDataBits}
                  onChange={setSlaveRtuDataBits}
                  disabled={slaveRunning || slaveRtuVirtualPort}
                  options={[{ value: 7, label: "7" }, { value: 8, label: "8" }]}
                />
              </Field></div>
              <div style={{ flex: 1 }}><Field stacked label="Stop bits">
                <SheetSelect
                  sheetTitle="Stop bits"
                  value={slaveRtuStopBits}
                  onChange={setSlaveRtuStopBits}
                  disabled={slaveRunning || slaveRtuVirtualPort}
                  options={[{ value: 1, label: "1" }, { value: 2, label: "2" }]}
                />
              </Field></div>
              <div style={{ flex: 1 }}><Field stacked label="Parity">
                <SheetSelect
                  sheetTitle="Parity"
                  value={slaveRtuParity}
                  onChange={setSlaveRtuParity}
                  disabled={slaveRunning || slaveRtuVirtualPort}
                  options={[{ value: "none", label: "None" }, { value: "even", label: "Even" }, { value: "odd", label: "Odd" }]}
                />
              </Field></div>
            </div>
            <Field label="Unit ID (1-247)">
              <input className="vt-input" type="number" min={1} max={247} value={slaveUnitId} onChange={(e) => setSlaveUnitId(Number(e.target.value))} disabled={slaveRunning} />
            </Field>
            <Field label="Registers">
              <input className="vt-input" type="number" value={slaveRegCount} onChange={(e) => setSlaveRegCount(Number(e.target.value))} disabled={slaveRunning} />
            </Field>
            <Field label="Refresh (ms)">
              <input className="vt-input" type="number" value={slavePollMs} onChange={(e) => setSlavePollMs(Number(e.target.value))} min={100} step={100} />
            </Field>
          </div>
        </Section>
      )}
      {!slaveRunning ? (
        <Button variant="primary" onClick={onSlaveStart}>Start server</Button>
      ) : (
        <Button variant="danger" onClick={onSlaveStop}>Stop server</Button>
      )}

      {slaveRunning && (
        <div>
          <div style={{ fontSize: "var(--fs-xs)", fontWeight: 600, color: "var(--text)", marginBottom: 6 }}>Registers ({useStore.getState().slaveRegs.length})</div>
          <div style={{ fontSize: "var(--fs-sm)", color: "var(--text-dim)", lineHeight: 1.5 }}>
            Tap any register to edit its value. External masters can read/write via FC03/FC06/FC16.
          </div>
        </div>
      )}
    </>
  );
}
