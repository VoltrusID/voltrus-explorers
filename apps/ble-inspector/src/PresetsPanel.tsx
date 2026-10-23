import { useEffect, useRef, useState } from "react";
import { ArrowDown, ArrowUp, Pencil, Play, Plus, Square, Trash2, X } from "lucide-react";
import { Button, Callout, Dialog, EmptyState, Field, IconButton, Section, Segmented, SheetSelect } from "@mactools/ui";
import { NumInput } from "./ChartToolbar";
import { hexToBytes, presetToHex } from "./decode";
import { gattName, shortUuid } from "./gatt";
import { MAX_DELAY_MS, MAX_REPEAT, newId, type Macro, type WriteMode, type WritePreset } from "./store";

const charLabel = (uuid: string) => {
  const name = gattName(uuid);
  const short = shortUuid(uuid);
  if (!name) return short;
  return short.startsWith("0x") ? `${name} (${short})` : name;
};

/* ── Preset dialog ──────────────────────────────────────────────────────── */

interface PresetDialogProps {
  /** Preset being edited, or a prefilled draft for a new one. `null` closes the dialog. */
  draft: WritePreset | null;
  /** Characteristics of the connected device that accept writes. */
  chars: string[];
  onSave: (preset: WritePreset) => void;
  onClose: () => void;
}

export function PresetDialog({ draft, chars, onSave, onClose }: PresetDialogProps) {
  const [name, setName] = useState("");
  const [charUuid, setCharUuid] = useState("");
  const [mode, setMode] = useState<WriteMode>("hex");
  const [value, setValue] = useState("");

  useEffect(() => {
    if (!draft) return;
    setName(draft.name);
    setCharUuid(draft.charUuid || chars[0] || "");
    setMode(draft.mode);
    setValue(draft.value);
    // Only reset when a different draft opens, not when the characteristic list refreshes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [draft]);

  const bytes = mode === "hex" ? hexToBytes(value) : new TextEncoder().encode(value);
  const valueError = mode === "hex" && bytes === null ? "Use pairs of hex digits, for example 01 ff 0a." : undefined;
  const ok = name.trim() !== "" && charUuid.trim() !== "" && value !== "" && !valueError;
  const options = chars.includes(charUuid) || charUuid === "" ? chars : [charUuid, ...chars];

  return (
    <Dialog
      open={draft !== null}
      onClose={onClose}
      title={draft && draft.name ? "Edit write preset" : "New write preset"}
      width={440}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            disabled={!ok}
            onClick={() => draft && onSave({ id: draft.id, name: name.trim(), charUuid: charUuid.trim(), mode, value })}
          >
            Save preset
          </Button>
        </>
      }
    >
      <div className="vt-stack" style={{ paddingTop: 12 }}>
        <Field label="Name">
          {(id) => <input id={id} className="vt-input" value={name} onChange={(e) => setName(e.target.value)} placeholder="LED on" autoFocus />}
        </Field>
        <Field label="Characteristic" hint={options.length === 0 ? "Connect to a device to pick from its writable characteristics, or type a UUID." : undefined}>
          {(id) =>
            options.length > 0 ? (
              <SheetSelect
                id={id}
                sheetTitle="Characteristic"
                value={charUuid}
                options={options.map((u) => ({ value: u, label: charLabel(u) }))}
                onChange={(v) => setCharUuid(v)}
              />
            ) : (
              <input id={id} className="vt-input vt-input--mono" value={charUuid} onChange={(e) => setCharUuid(e.target.value)} placeholder="6e400002-b5a3-f393-e0a9-e50e24dcca9e" />
            )
          }
        </Field>
        <Field label="Value is">
          <Segmented
            label="Value type"
            value={mode}
            options={[
              { value: "hex", label: "Hex bytes" },
              { value: "text", label: "Text (UTF-8)" },
            ]}
            onChange={setMode}
          />
        </Field>
        <Field label="Value" error={valueError} hint={bytes && value !== "" ? `${bytes.length} byte${bytes.length === 1 ? "" : "s"}` : undefined}>
          {(id) => (
            <input
              id={id}
              className="vt-input vt-input--mono"
              value={value}
              aria-invalid={valueError ? true : undefined}
              onChange={(e) => setValue(e.target.value)}
              placeholder={mode === "hex" ? "01 ff 0a" : "Text to send"}
            />
          )}
        </Field>
      </div>
    </Dialog>
  );
}

/* ── Macro dialog ───────────────────────────────────────────────────────── */

interface MacroDialogProps {
  draft: Macro | null;
  presets: WritePreset[];
  onSave: (macro: Macro) => void;
  onClose: () => void;
}

function MacroDialog({ draft, presets, onSave, onClose }: MacroDialogProps) {
  const [name, setName] = useState("");
  const [repeat, setRepeat] = useState(1);
  const [steps, setSteps] = useState<Macro["steps"]>([]);

  useEffect(() => {
    if (!draft) return;
    setName(draft.name);
    setRepeat(draft.repeat);
    setSteps(draft.steps);
  }, [draft]);

  const move = (i: number, d: -1 | 1) =>
    setSteps((prev) => {
      const next = [...prev];
      const j = i + d;
      if (j < 0 || j >= next.length) return prev;
      [next[i], next[j]] = [next[j], next[i]];
      return next;
    });

  const ok = name.trim() !== "" && steps.length > 0;

  return (
    <Dialog
      open={draft !== null}
      onClose={onClose}
      title={draft && draft.name ? "Edit macro" : "New macro"}
      width={520}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" disabled={!ok} onClick={() => draft && onSave({ id: draft.id, name: name.trim(), repeat, steps })}>
            Save macro
          </Button>
        </>
      }
    >
      <div className="vt-stack" style={{ paddingTop: 12 }}>
        <div style={{ display: "flex", gap: 12 }}>
          <Field label="Name" className="ble-grow">
            {(id) => <input id={id} className="vt-input" value={name} onChange={(e) => setName(e.target.value)} placeholder="Blink LED" autoFocus />}
          </Field>
          <Field label="Repeat">
            <NumInput label="Repeat count" value={repeat} min={1} integer width={72} onCommit={(n) => setRepeat(Math.min(MAX_REPEAT, n))} />
          </Field>
        </div>
        <div className="vt-label">Steps run in order, then the whole list repeats.</div>
        {steps.map((s, i) => (
          <div key={i} className="ble-step">
            <span className="mono" style={{ width: 20, color: "var(--text-muted)" }}>
              {i + 1}
            </span>
            <SheetSelect
              size="sm"
              style={{ flex: 1, minWidth: 0 }}
              aria-label={`Preset for step ${i + 1}`}
              sheetTitle="Preset"
              value={s.presetId}
              options={presets.map((p) => ({ value: p.id, label: p.name }))}
              onChange={(v) => setSteps((prev) => prev.map((x, j) => (j === i ? { ...x, presetId: v } : x)))}
            />
            <span style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)" }}>then wait</span>
            <NumInput
              label={`Delay after step ${i + 1} in milliseconds`}
              value={s.delayMs}
              min={0}
              integer
              width={72}
              onCommit={(n) => setSteps((prev) => prev.map((x, j) => (j === i ? { ...x, delayMs: Math.min(MAX_DELAY_MS, n) } : x)))}
            />
            <span style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)" }}>ms</span>
            <IconButton size="sm" label="Move step up" disabled={i === 0} onClick={() => move(i, -1)}>
              <ArrowUp size={13} strokeWidth={1.75} />
            </IconButton>
            <IconButton size="sm" label="Move step down" disabled={i === steps.length - 1} onClick={() => move(i, 1)}>
              <ArrowDown size={13} strokeWidth={1.75} />
            </IconButton>
            <IconButton size="sm" danger label="Remove step" onClick={() => setSteps((prev) => prev.filter((_, j) => j !== i))}>
              <X size={13} strokeWidth={1.75} />
            </IconButton>
          </div>
        ))}
        <div>
          <Button
            size="sm"
            icon={<Plus size={13} strokeWidth={1.75} />}
            disabled={presets.length === 0}
            onClick={() => setSteps((prev) => [...prev, { presetId: presets[0].id, delayMs: 100 }])}
          >
            Add step
          </Button>
        </div>
      </div>
    </Dialog>
  );
}

/* ── Panel ──────────────────────────────────────────────────────────────── */

interface Props {
  presets: WritePreset[];
  macros: Macro[];
  onMacrosChange: (macros: Macro[]) => void;
  /** Open the preset dialog for this preset, or a blank one for `null`. */
  onEditPreset: (preset: WritePreset | null) => void;
  onDeletePreset: (id: string) => void;
  /** Writable characteristic UUIDs of the connected device (empty when disconnected). */
  writableChars: string[];
  connected: boolean;
  /** Write hex to a characteristic of the connected device; rejects on failure. */
  onWrite: (charUuid: string, hex: string) => Promise<void>;
  onInfo: (message: string) => void;
  onError: (message: string) => void;
}

export default function PresetsPanel({
  presets,
  macros,
  onMacrosChange,
  onEditPreset,
  onDeletePreset,
  writableChars,
  connected,
  onWrite,
  onInfo,
  onError,
}: Props) {
  const [macroDraft, setMacroDraft] = useState<Macro | null>(null);
  const [running, setRunning] = useState<{ id: string; text: string } | null>(null);
  const abort = useRef(false);

  useEffect(
    () => () => {
      abort.current = true;
    },
    [],
  );

  const send = async (p: WritePreset) => {
    const hex = presetToHex(p.mode, p.value);
    if (hex === null) {
      onError(`Preset "${p.name}" has an invalid hex value`);
      return;
    }
    try {
      await onWrite(p.charUuid, hex);
    } catch {
      // onWrite already logged the failure.
    }
  };

  const runMacro = async (m: Macro) => {
    abort.current = false;
    try {
      for (let r = 0; r < m.repeat; r++) {
        for (let i = 0; i < m.steps.length; i++) {
          if (abort.current) {
            onInfo(`Macro "${m.name}" stopped`);
            return;
          }
          const step = m.steps[i];
          const p = presets.find((x) => x.id === step.presetId);
          if (!p) continue;
          const hex = presetToHex(p.mode, p.value);
          if (hex === null) throw new Error(`preset "${p.name}" has an invalid hex value`);
          setRunning({ id: m.id, text: `Step ${i + 1} of ${m.steps.length}, run ${r + 1} of ${m.repeat}` });
          await onWrite(p.charUuid, hex);
          for (let left = step.delayMs; left > 0 && !abort.current; left -= 100) {
            await new Promise((res) => setTimeout(res, Math.min(100, left)));
          }
        }
      }
      onInfo(`Macro "${m.name}" finished`);
    } catch (e) {
      onError(`Macro "${m.name}" stopped: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setRunning(null);
    }
  };

  const saveMacro = (m: Macro) => {
    onMacrosChange(macros.some((x) => x.id === m.id) ? macros.map((x) => (x.id === m.id ? m : x)) : [...macros, m]);
    setMacroDraft(null);
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
      {!connected && (
        <Callout tone="info">Connect to a device to send presets and run macros. You can still create and edit them now.</Callout>
      )}

      <Section
        title="Write presets"
        meta={`${presets.length}`}
        actions={
          <Button size="sm" icon={<Plus size={13} strokeWidth={1.75} />} onClick={() => onEditPreset(null)}>
            New preset
          </Button>
        }
      >
        {presets.length === 0 ? (
          <EmptyState
            compact
            title="No presets yet"
            description="Save a value you send often, such as a command or a mode byte. You can also use Save on a characteristic's write box."
          />
        ) : (
          <div className="vt-panel">
            {presets.map((p) => {
              const available = connected && writableChars.includes(p.charUuid);
              return (
                <div key={p.id} className="ble-row">
                  <div style={{ flex: 1, minWidth: 0 }}>
                    <div style={{ fontSize: "var(--fs-sm)", fontWeight: 500, color: "var(--text)" }}>{p.name}</div>
                    <div className="mono" style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }} title={p.charUuid}>
                      {charLabel(p.charUuid)} · {p.mode === "hex" ? p.value : `"${p.value}"`}
                    </div>
                  </div>
                  <Button
                    size="sm"
                    icon={<Play size={12} strokeWidth={1.75} />}
                    disabled={!available || running !== null}
                    title={available ? "Write this value now" : "Connect to a device that has this characteristic"}
                    onClick={() => send(p)}
                  >
                    Send
                  </Button>
                  <IconButton size="sm" label={`Edit ${p.name}`} onClick={() => onEditPreset(p)}>
                    <Pencil size={13} strokeWidth={1.75} />
                  </IconButton>
                  <IconButton size="sm" danger label={`Delete ${p.name}`} onClick={() => onDeletePreset(p.id)}>
                    <Trash2 size={13} strokeWidth={1.75} />
                  </IconButton>
                </div>
              );
            })}
          </div>
        )}
      </Section>

      <Section
        title="Macros"
        meta={`${macros.length}`}
        actions={
          <Button
            size="sm"
            icon={<Plus size={13} strokeWidth={1.75} />}
            disabled={presets.length === 0}
            title={presets.length === 0 ? "Create a preset first" : undefined}
            onClick={() => setMacroDraft({ id: newId(), name: "", repeat: 1, steps: [{ presetId: presets[0].id, delayMs: 100 }] })}
          >
            New macro
          </Button>
        }
      >
        {macros.length === 0 ? (
          <EmptyState
            compact
            title="No macros yet"
            description="A macro sends several presets in order with a pause after each one, and can repeat the whole list."
          />
        ) : (
          <div className="vt-panel">
            {macros.map((m) => {
              const isRunning = running?.id === m.id;
              const ready = connected && m.steps.length > 0 && m.steps.every((s) => {
                const p = presets.find((x) => x.id === s.presetId);
                return p && writableChars.includes(p.charUuid);
              });
              return (
                <div key={m.id} className="ble-row">
                  <div style={{ flex: 1, minWidth: 0 }}>
                    <div style={{ fontSize: "var(--fs-sm)", fontWeight: 500, color: "var(--text)" }}>{m.name}</div>
                    <div style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)" }}>
                      {isRunning
                        ? running.text
                        : `${m.steps.length} step${m.steps.length === 1 ? "" : "s"}${m.repeat > 1 ? `, repeated ${m.repeat} times` : ""}`}
                    </div>
                  </div>
                  {isRunning ? (
                    <Button size="sm" variant="danger" icon={<Square size={12} strokeWidth={1.75} />} onClick={() => (abort.current = true)}>
                      Stop
                    </Button>
                  ) : (
                    <Button
                      size="sm"
                      icon={<Play size={12} strokeWidth={1.75} />}
                      disabled={!ready || running !== null}
                      title={ready ? "Run this macro" : "Connect to a device that has every characteristic the steps write to"}
                      onClick={() => runMacro(m)}
                    >
                      Run
                    </Button>
                  )}
                  <IconButton size="sm" label={`Edit ${m.name}`} disabled={running !== null} onClick={() => setMacroDraft(m)}>
                    <Pencil size={13} strokeWidth={1.75} />
                  </IconButton>
                  <IconButton
                    size="sm"
                    danger
                    label={`Delete ${m.name}`}
                    disabled={running !== null}
                    onClick={() => onMacrosChange(macros.filter((x) => x.id !== m.id))}
                  >
                    <Trash2 size={13} strokeWidth={1.75} />
                  </IconButton>
                </div>
              );
            })}
          </div>
        )}
      </Section>

      <MacroDialog draft={macroDraft} presets={presets} onSave={saveMacro} onClose={() => setMacroDraft(null)} />
    </div>
  );
}
