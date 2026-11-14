import { useState } from "react";
import { Plug, Unplug } from "lucide-react";
import { Button, Field, Segmented } from "@mactools/ui";
import type { Device, Protocol } from "./types";
import { DEFAULT_PORT } from "./types";
import { defaultTags } from "./helpers";

interface Props {
  device: Device;
  connected: boolean;
  loading: boolean;
  pollMs: number;
  onPollMs: (ms: number) => void;
  onChange: (patch: Partial<Device>) => void;
  onConnect: () => void;
  onDisconnect: () => void;
}

const num = (v: string, fallback: number) => (v.trim() === "" || Number.isNaN(Number(v)) ? fallback : Number(v));
const hex = (v: string, fallback: number) => {
  const n = parseInt(v.replace(/^0x/i, ""), 16);
  return Number.isNaN(n) ? fallback : n;
};

/** Connection settings of one PLC: protocol, address, and the MC frame or FINS node options. */
export default function ConnectionPanel({ device, connected, loading, pollMs, onPollMs, onChange, onConnect, onDisconnect }: Props) {
  const [advanced, setAdvanced] = useState(false);
  const locked = connected || loading;
  const { mc, fins } = device;

  const setProtocol = (p: Protocol) => {
    if (p === device.protocol) return;
    // Swap the port to the protocol's default only when it was still the other protocol's default.
    const port = device.port === DEFAULT_PORT[device.protocol] ? DEFAULT_PORT[p] : device.port;
    const untouched = JSON.stringify(device.tags.map((t) => t.address)) === JSON.stringify(defaultTags(device.protocol).map((t) => t.address));
    onChange({ protocol: p, port, ...(untouched ? { tags: defaultTags(p) } : {}) });
  };

  const input = (id: string, value: string | number, set: (v: string) => void, opts: { width?: number; mono?: boolean } = {}) => (
    <input
      id={id}
      className={opts.mono ? "vt-input mono" : "vt-input"}
      value={value}
      disabled={locked}
      onChange={(e) => set(e.target.value)}
      style={opts.width ? { width: opts.width } : undefined}
    />
  );

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 10, padding: 14 }}>
      <Segmented<Protocol>
        label="Protocol"
        block
        value={device.protocol}
        onChange={(p) => !locked && setProtocol(p)}
        options={[
          { value: "mc", label: "Mitsubishi MC", disabled: locked && device.protocol !== "mc" },
          { value: "fins", label: "Omron FINS", disabled: locked && device.protocol !== "fins" },
        ]}
      />
      <Field label="Name">{(id) => input(id, device.label, (v) => onChange({ label: v }))}</Field>
      <div style={{ display: "flex", gap: 8 }}>
        <div style={{ flex: 1 }}>
          <Field label="IP address">{(id) => input(id, device.host, (v) => onChange({ host: v.trim() }))}</Field>
        </div>
        <div style={{ width: 76 }}>
          <Field label="Port">{(id) => input(id, device.port, (v) => onChange({ port: num(v, device.port) }))}</Field>
        </div>
      </div>

      {device.protocol === "mc" ? (
        <>
          <Field label="Transport">
            {() => (
              <Segmented
                label="Transport"
                size="sm"
                block
                value={mc.transport}
                onChange={(v) => !locked && onChange({ mc: { ...mc, transport: v } })}
                options={[{ value: "tcp", label: "TCP" }, { value: "udp", label: "UDP" }]}
              />
            )}
          </Field>
          <div style={{ display: "flex", gap: 8 }}>
            <div style={{ flex: 1 }}>
              <Field label="Frame">
                {() => (
                  <Segmented
                    label="Frame"
                    size="sm"
                    block
                    value={mc.frame}
                    onChange={(v) => !locked && onChange({ mc: { ...mc, frame: v } })}
                    options={[{ value: "3E", label: "3E" }, { value: "4E", label: "4E" }]}
                  />
                )}
              </Field>
            </div>
            <div style={{ flex: 1.4 }}>
              <Field label="Code">
                {() => (
                  <Segmented
                    label="Communication data code"
                    size="sm"
                    block
                    value={mc.encoding}
                    onChange={(v) => !locked && onChange({ mc: { ...mc, encoding: v } })}
                    options={[{ value: "binary", label: "Binary" }, { value: "ascii", label: "ASCII" }]}
                  />
                )}
              </Field>
            </div>
          </div>
          <label style={{ display: "flex", alignItems: "center", gap: 8, fontSize: "var(--fs-sm)", color: "var(--text-dim)" }}>
            <input type="checkbox" checked={mc.xyOctal} disabled={locked} onChange={(e) => onChange({ mc: { ...mc, xyOctal: e.target.checked } })} />
            X and Y in octal (iQ-F / FX5)
          </label>
        </>
      ) : (
        <>
          <Field label="Transport">
            {() => (
              <Segmented
                label="Transport"
                size="sm"
                block
                value={fins.transport}
                onChange={(v) => !locked && onChange({ fins: { ...fins, transport: v } })}
                options={[{ value: "udp", label: "FINS/UDP" }, { value: "tcp", label: "FINS/TCP" }]}
              />
            )}
          </Field>
          <div style={{ display: "flex", gap: 8 }}>
            <div style={{ flex: 1 }}>
              <Field label="PLC node (DA1)" hint={fins.transport === "tcp" ? "From the handshake" : "0 = last IP octet"}>
                {(id) => input(id, fins.destNode, (v) => onChange({ fins: { ...fins, destNode: num(v, 0) } }))}
              </Field>
            </div>
            <div style={{ flex: 1 }}>
              <Field label="Own node (SA1)" hint={fins.transport === "tcp" ? "0 = assigned" : "0 = own last octet"}>
                {(id) => input(id, fins.srcNode, (v) => onChange({ fins: { ...fins, srcNode: num(v, 0) } }))}
              </Field>
            </div>
          </div>
        </>
      )}

      <button
        type="button"
        onClick={() => setAdvanced((a) => !a)}
        style={{ alignSelf: "flex-start", background: "none", border: 0, padding: 0, color: "var(--accent)", cursor: "pointer", font: "inherit", fontSize: "var(--fs-sm)" }}
        aria-expanded={advanced}
      >
        {advanced ? "Hide routing and timeout" : "Routing and timeout"}
      </button>
      {advanced && (
        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 8 }}>
          {device.protocol === "mc" ? (
            <>
              <Field label="Network">{(id) => input(id, mc.network, (v) => onChange({ mc: { ...mc, network: num(v, 0) } }))}</Field>
              <Field label="PC no. (hex)">{(id) => input(id, mc.pc.toString(16).toUpperCase(), (v) => onChange({ mc: { ...mc, pc: hex(v, 0xff) } }), { mono: true })}</Field>
              <Field label="Module I/O (hex)">{(id) => input(id, mc.io.toString(16).toUpperCase().padStart(4, "0"), (v) => onChange({ mc: { ...mc, io: hex(v, 0x3ff) } }), { mono: true })}</Field>
              <Field label="Station">{(id) => input(id, mc.station, (v) => onChange({ mc: { ...mc, station: num(v, 0) } }))}</Field>
            </>
          ) : (
            <>
              <Field label="PLC network (DNA)">{(id) => input(id, fins.destNetwork, (v) => onChange({ fins: { ...fins, destNetwork: num(v, 0) } }))}</Field>
              <Field label="PLC unit (DA2)" hint="0 = CPU">{(id) => input(id, fins.destUnit, (v) => onChange({ fins: { ...fins, destUnit: num(v, 0) } }))}</Field>
              <Field label="Own network (SNA)">{(id) => input(id, fins.srcNetwork, (v) => onChange({ fins: { ...fins, srcNetwork: num(v, 0) } }))}</Field>
              <Field label="Local UDP port" hint="0 = any; some units reply to 9600 only">
                {(id) => input(id, fins.localPort, (v) => onChange({ fins: { ...fins, localPort: num(v, 0) } }))}
              </Field>
            </>
          )}
          <Field label="Timeout (ms)">{(id) => input(id, device.timeoutMs, (v) => onChange({ timeoutMs: Math.max(100, num(v, 3000)) }))}</Field>
        </div>
      )}

      {connected ? (
        <Button variant="danger" block icon={<Unplug size={14} strokeWidth={1.75} />} onClick={onDisconnect}>
          Disconnect
        </Button>
      ) : (
        <Button variant="primary" block loading={loading} icon={<Plug size={14} strokeWidth={1.75} />} disabled={!device.host || loading} onClick={onConnect}>
          {loading ? "Connecting…" : "Connect"}
        </Button>
      )}

      <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
        <label className="vt-label" htmlFor={`poll-${device.id}`} style={{ marginBottom: 0 }}>Poll interval (ms)</label>
        <input
          id={`poll-${device.id}`}
          type="number"
          min={100}
          step={100}
          value={pollMs}
          onChange={(e) => onPollMs(Math.max(100, Number(e.target.value)))}
          className="vt-input"
          style={{ width: 84 }}
        />
      </div>
    </div>
  );
}
