import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Button, Dialog, Field, ProBadge, Segmented, SheetSelect } from "@mactools/ui";
import type { ChannelInfo, DecoderConfig, DecoderKind, Parity } from "./types";
import { DECODER_NAMES, PRO_DECODERS, defaultDecoder } from "./logic";
import { hint } from "./ui";

const BAUDS = [1200, 2400, 4800, 9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600];
const CAN_RATES = [10_000, 20_000, 50_000, 100_000, 125_000, 250_000, 500_000, 800_000, 1_000_000];
const PARITIES: { value: Parity; label: string }[] = [
  { value: "none", label: "None" },
  { value: "even", label: "Even" },
  { value: "odd", label: "Odd" },
  { value: "mark", label: "Mark" },
  { value: "space", label: "Space" },
];

interface Props {
  open: boolean;
  initial: DecoderConfig | null;
  channels: ChannelInfo[];
  pro: boolean;
  onClose: () => void;
  onSave: (d: DecoderConfig) => void;
}

export default function DecoderDialog({ open, initial, channels, pro, onClose, onSave }: Props) {
  const [d, setD] = useState<DecoderConfig>(initial ?? defaultDecoder("modbusRtu", channels.length));
  const [estimating, setEstimating] = useState(false);
  useEffect(() => { if (open) setD(initial ?? defaultDecoder("modbusRtu", channels.length)); }, [open, initial, channels.length]);

  const chOpts = channels.map((c, i) => ({ value: String(i), label: `D${i} · ${c.name}` }));
  const optCh = [{ value: "", label: "Not used" }, ...chOpts];
  const set = (patch: Partial<DecoderConfig>) => setD((x) => ({ ...x, ...patch }) as DecoderConfig);
  const chSel = (label: string, value: number | null, onChange: (v: number | null) => void, optional = false) => (
    <Field label={label}>
      <SheetSelect aria-label={label} sheetTitle={label} value={value == null ? "" : String(value)} options={optional ? optCh : chOpts}
        onChange={(v) => onChange(v === "" ? null : Number(v))} />
    </Field>
  );
  const num = (label: string, value: number, onChange: (v: number) => void, step = 1) => (
    <Field label={label}>
      <input className="vt-input vt-input--mono" type="number" step={step} value={value} onChange={(e) => onChange(Number(e.target.value))} />
    </Field>
  );
  const baudField = (value: number, ch: number | null) => (
    <Field label="Baud rate">
      <div style={{ display: "flex", gap: 6 }}>
        <input className="vt-input vt-input--mono" list="la-bauds" type="number" value={value} onChange={(e) => set({ baud: Number(e.target.value) } as Partial<DecoderConfig>)} />
        <datalist id="la-bauds">{BAUDS.map((b) => <option key={b} value={b} />)}</datalist>
        <Button size="sm" disabled={ch == null || estimating} title="Guess the baud rate from the shortest pulses on the channel" onClick={async () => {
          if (ch == null) return;
          setEstimating(true);
          try {
            const b = await invoke<number | null>("la_estimate_baud", { channel: ch });
            if (b) set({ baud: b } as Partial<DecoderConfig>);
          } finally { setEstimating(false); }
        }}>Detect</Button>
      </div>
    </Field>
  );
  const lineFields = (parity: Parity, stopBits: number, inverted: boolean) => (
    <>
      <Field label="Parity">
        <SheetSelect aria-label="Parity" sheetTitle="Parity" value={parity} options={PARITIES} onChange={(v) => set({ parity: v as Parity } as Partial<DecoderConfig>)} />
      </Field>
      <Field label="Stop bits">
        <Segmented label="Stop bits" value={String(stopBits)} onChange={(v) => set({ stopBits: Number(v) } as Partial<DecoderConfig>)}
          options={[{ value: "1", label: "1" }, { value: "1.5", label: "1.5" }, { value: "2", label: "2" }]} />
      </Field>
      <label style={{ display: "flex", gap: 6, alignItems: "center", fontSize: "var(--fs-sm)" }}>
        <input type="checkbox" checked={inverted} onChange={(e) => set({ inverted: e.target.checked } as Partial<DecoderConfig>)} /> Inverted (idle low)
      </label>
    </>
  );

  const locked = !pro && PRO_DECODERS.includes(d.kind);

  return (
    <Dialog open={open} onClose={onClose} title={initial ? `Edit ${DECODER_NAMES[d.kind]} decoder` : "Add a decoder"} width={460} footer={(
      <>
        <Button variant="ghost" onClick={onClose}>Cancel</Button>
        <Button variant="primary" onClick={() => onSave(d)}>{initial ? "Apply" : "Add"}{locked && <> <ProBadge /></>}</Button>
      </>
    )}>
      <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
        {!initial && (
          <Field label="Protocol">
            <SheetSelect aria-label="Protocol" sheetTitle="Protocol" value={d.kind}
              options={(Object.keys(DECODER_NAMES) as DecoderKind[]).map((k) => ({ value: k, label: `${DECODER_NAMES[k]}${!pro && PRO_DECODERS.includes(k) ? " (Pro)" : ""}` }))}
              onChange={(v) => setD(defaultDecoder(v as DecoderKind, channels.length))} />
          </Field>
        )}
        {d.kind === "modbusRtu" && (
          <>
            <span style={hint}>
              Request channel: the master's TX, or the RS-485 bus seen through one receiver (both directions). Response channel: the slave's TX, if you probe it separately.
            </span>
            {chSel("Request channel (or bus)", d.requestChannel, (v) => set({ requestChannel: v }), false)}
            {chSel("Response channel", d.responseChannel, (v) => set({ responseChannel: v }), true)}
            {baudField(d.baud, d.requestChannel)}
            {lineFields(d.parity, d.stopBits, d.inverted)}
            {num("Response timeout (ms)", d.timeoutMs, (v) => set({ timeoutMs: v }))}
          </>
        )}
        {d.kind === "uart" && (
          <>
            {chSel("Channel", d.channel, (v) => set({ channel: v ?? 0 }))}
            {baudField(d.baud, d.channel)}
            <Field label="Data bits">
              <Segmented label="Data bits" value={String(d.dataBits)} onChange={(v) => set({ dataBits: Number(v) })} options={[5, 6, 7, 8, 9].map((n) => ({ value: String(n), label: String(n) }))} />
            </Field>
            {lineFields(d.parity, d.stopBits, d.inverted)}
            <label style={{ display: "flex", gap: 6, alignItems: "center", fontSize: "var(--fs-sm)" }}>
              <input type="checkbox" checked={d.msbFirst} onChange={(e) => set({ msbFirst: e.target.checked })} /> MSB first
            </label>
          </>
        )}
        {d.kind === "spi" && (
          <>
            {chSel("Clock (SCK)", d.clk, (v) => set({ clk: v ?? 0 }))}
            {chSel("MOSI", d.mosi, (v) => set({ mosi: v }), true)}
            {chSel("MISO", d.miso, (v) => set({ miso: v }), true)}
            {chSel("Chip select", d.cs, (v) => set({ cs: v }), true)}
            <Field label="Mode (CPOL, CPHA)">
              <Segmented label="SPI mode" value={String(d.mode)} onChange={(v) => set({ mode: Number(v) })} options={[0, 1, 2, 3].map((m) => ({ value: String(m), label: String(m) }))} />
            </Field>
            {num("Bits per word", d.wordBits, (v) => set({ wordBits: Math.min(Math.max(v, 4), 32) }))}
            <label style={{ display: "flex", gap: 6, alignItems: "center", fontSize: "var(--fs-sm)" }}>
              <input type="checkbox" checked={d.lsbFirst} onChange={(e) => set({ lsbFirst: e.target.checked })} /> LSB first
            </label>
            <label style={{ display: "flex", gap: 6, alignItems: "center", fontSize: "var(--fs-sm)" }}>
              <input type="checkbox" checked={d.csActiveHigh} onChange={(e) => set({ csActiveHigh: e.target.checked })} /> Chip select active high
            </label>
            <Field label="Device">
              <SheetSelect aria-label="Device" sheetTitle="Device" value={d.device} onChange={(v) => set({ device: v as "none" | "mcp2515" })}
                options={[{ value: "none", label: "Words only" }, { value: "mcp2515", label: "Microchip MCP2515 CAN controller" }]} />
            </Field>
          </>
        )}
        {d.kind === "i2c" && (
          <>
            {chSel("SCL", d.scl, (v) => set({ scl: v ?? 0 }))}
            {chSel("SDA", d.sda, (v) => set({ sda: v ?? 1 }))}
            {num("Ignore pulses up to (samples)", d.minPulse, (v) => set({ minPulse: Math.max(0, v) }))}
          </>
        )}
        {d.kind === "can" && (
          <>
            <span style={hint}>Probe CAN_RX (or CAN_TX) on the logic side of the transceiver: dominant is low. Classic CAN; CAN FD frames are marked, not decoded.</span>
            {chSel("Channel", d.channel, (v) => set({ channel: v ?? 0 }))}
            <Field label="Bit rate">
              <SheetSelect aria-label="Bit rate" sheetTitle="Bit rate" value={String(d.bitrate)} onChange={(v) => set({ bitrate: Number(v) })}
                options={CAN_RATES.map((r) => ({ value: String(r), label: r >= 1e6 ? `${r / 1e6} Mbit/s` : `${r / 1000} kbit/s` }))} />
            </Field>
            {num("Sample point (%)", Math.round(d.samplePoint * 1000) / 10, (v) => set({ samplePoint: Math.min(Math.max(v, 50), 95) / 100 }), 0.5)}
            <label style={{ display: "flex", gap: 6, alignItems: "center", fontSize: "var(--fs-sm)" }}>
              <input type="checkbox" checked={d.inverted} onChange={(e) => set({ inverted: e.target.checked })} /> Inverted (dominant high)
            </label>
          </>
        )}
      </div>
    </Dialog>
  );
}
