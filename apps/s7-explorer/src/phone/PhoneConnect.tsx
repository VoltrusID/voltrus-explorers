import { useEffect, useState, type ReactNode } from "react";
import { Trash2 } from "lucide-react";
import { Button, Field, PageTitle, Section, SheetSelect } from "@mactools/ui";
import type { Device } from "../types";
import { CPU_PRESETS, CUSTOM_CPU, cpuAccessHint, cpuPresetFor } from "./cpuPresets";

export interface ConnectDraft {
  host: string;
  port: number;
  rack: number;
  slot: number;
}

interface Props {
  device: Device;
  connected: boolean;
  loading: boolean;
  pollMs: number;
  onPollMs: (ms: number) => void;
  /** The saved PLCs as chips (owned by the app: every device is a panel). */
  chips: ReactNode;
  onConnect: (config: ConnectDraft) => void;
  onDisconnect: () => void;
  onRename: (label: string) => void;
  /** Present when there is more than one PLC. */
  onRemove?: () => void;
}

const POLL_CHOICES = [250, 500, 1000, 2000, 5000];
const pollLabel = (ms: number) => (ms >= 1000 && ms % 1000 === 0 ? `${ms / 1000} s` : `${ms} ms`);

const fromDevice = (d: Device): ConnectDraft => ({ host: d.host, port: d.port, rack: d.rack, slot: d.slot });

/** The Connect tab: one card with host, port, rack and slot as rows, a big Connect button, then how often to poll. */
export default function PhoneConnect({ device, connected, loading, pollMs, onPollMs, chips, onConnect, onDisconnect, onRename, onRemove }: Props) {
  const [draft, setDraft] = useState<ConnectDraft>(() => fromDevice(device));
  // The simulator page can point this PLC at itself.
  useEffect(() => setDraft(fromDevice(device)), [device.host, device.port, device.rack, device.slot]); // eslint-disable-line react-hooks/exhaustive-deps

  const preset = cpuPresetFor(draft.rack, draft.slot);
  const accessHint = cpuAccessHint(preset);
  const pollOptions = (POLL_CHOICES.includes(pollMs) ? POLL_CHOICES : [...POLL_CHOICES, pollMs].sort((a, b) => a - b)).map((ms) => ({ value: ms, label: pollLabel(ms) }));
  const num = (v: string) => Math.max(0, Math.round(Number(v) || 0));

  return (
    <div className="vm-body">
      <PageTitle title="Connect" caption={connected ? `Connected to ${device.host}` : "Reach a controller over ISO-on-TCP"} />
      {chips}
      <Section title="Controller">
        <form
          className="vt-stack vt-rows"
          onSubmit={(e) => {
            e.preventDefault();
            if (!connected) onConnect(draft);
          }}
        >
          <Field label="Name">
            {(id) => (
              <input id={id} className="vt-input" value={device.label} onChange={(e) => onRename(e.target.value)} autoCapitalize="words" autoCorrect="off" enterKeyHint="done" />
            )}
          </Field>
          <Field label="IP address">
            {(id) => (
              <input
                id={id}
                className="vt-input vt-input--mono"
                inputMode="url"
                autoCapitalize="off"
                autoCorrect="off"
                spellCheck={false}
                value={draft.host}
                disabled={connected}
                onChange={(e) => setDraft({ ...draft, host: e.target.value })}
              />
            )}
          </Field>
          <Field label="Port">
            {(id) => (
              <input id={id} className="vt-input vt-input--mono" type="number" inputMode="numeric" min={1} max={65535} value={draft.port} disabled={connected} onChange={(e) => setDraft({ ...draft, port: num(e.target.value) })} />
            )}
          </Field>
          <Field label="CPU type" hint={accessHint || undefined}>
            <SheetSelect
              sheetTitle="CPU type"
              value={preset}
              disabled={connected}
              onChange={(id) => {
                const p = CPU_PRESETS.find((x) => x.id === id);
                if (p) setDraft({ ...draft, rack: p.rack, slot: p.slot });
              }}
              options={[
                ...CPU_PRESETS.map((p) => ({ value: p.id, label: p.label, hint: p.hint })),
                { value: CUSTOM_CPU, label: "Custom", hint: "Set the rack and slot yourself", disabled: true },
              ]}
            />
          </Field>
          <Field label="Rack">
            {(id) => (
              <input id={id} className="vt-input vt-input--mono" type="number" inputMode="numeric" min={0} max={7} value={draft.rack} disabled={connected} onChange={(e) => setDraft({ ...draft, rack: num(e.target.value) })} />
            )}
          </Field>
          <Field label="Slot">
            {(id) => (
              <input id={id} className="vt-input vt-input--mono" type="number" inputMode="numeric" min={0} max={31} value={draft.slot} disabled={connected} onChange={(e) => setDraft({ ...draft, slot: num(e.target.value) })} />
            )}
          </Field>
          {connected ? (
            <Button type="button" variant="danger" block size="lg" onClick={onDisconnect}>
              Disconnect
            </Button>
          ) : (
            <Button type="submit" variant="primary" block size="lg" loading={loading} disabled={!draft.host.trim()}>
              {loading ? "Connecting…" : "Connect"}
            </Button>
          )}
        </form>
      </Section>
      <Section title="Polling">
        <Field label="Poll interval">
          <SheetSelect sheetTitle="Poll interval" value={pollMs} onChange={onPollMs} options={pollOptions} />
        </Field>
      </Section>
      {onRemove && (
        <Button variant="ghost" block icon={<Trash2 size={16} strokeWidth={1.75} />} onClick={onRemove} className="s7-remove">
          Remove {device.label}
        </Button>
      )}
    </div>
  );
}
