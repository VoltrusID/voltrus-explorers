import { useState, type FormEvent } from "react";
import { Badge, Button, Callout, Dialog, Field, Segmented, SheetSelect } from "@mactools/ui";

export type ControlMode = "select" | "direct";

export type ControlCommand =
  | { kind: "crob"; code: number; count: number; onTimeMs: number; offTimeMs: number; mode: ControlMode }
  | { kind: "analog"; value: number; mode: ControlMode };

const CROB_OPERATIONS = [
  { value: "3", label: "Latch on" },
  { value: "4", label: "Latch off" },
  { value: "1", label: "Pulse on" },
  { value: "2", label: "Pulse off" },
];

const TCC_OPTIONS = [
  { value: "0", label: "None" },
  { value: "64", label: "Close" },
  { value: "128", label: "Trip" },
];

const MODE_KEY = "dnp3-explorer-control-mode";

interface ControlDialogProps {
  /** Point name shown in the header. */
  address: string;
  /** Group 12 CROB for binary points, Group 41 analog output otherwise. */
  binary: boolean;
  currentValue?: string;
  /** Throw to keep the dialog open and show the message; resolve to close it. */
  onSubmit: (command: ControlCommand) => Promise<void>;
  onCancel: () => void;
}

const wholeNumber = (text: string, min: number, max: number) => {
  const n = Number(text);
  return text.trim() !== "" && Number.isInteger(n) && n >= min && n <= max ? n : null;
};

export function ControlDialog({ address, binary, currentValue, onSubmit, onCancel }: ControlDialogProps) {
  const [mode, setMode] = useState<ControlMode>(() => (localStorage.getItem(MODE_KEY) === "direct" ? "direct" : "select"));
  const [operation, setOperation] = useState("3");
  const [tcc, setTcc] = useState("0");
  const [count, setCount] = useState("1");
  const [onTime, setOnTime] = useState("1000");
  const [offTime, setOffTime] = useState("0");
  const [value, setValue] = useState(currentValue ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const pulse = operation === "1" || operation === "2";

  const changeMode = (next: ControlMode) => {
    setMode(next);
    localStorage.setItem(MODE_KEY, next);
  };

  const buildCommand = (): ControlCommand | string => {
    if (!binary) {
      const numeric = Number(value);
      if (value.trim() === "" || !Number.isFinite(numeric)) return "Enter a number for the analog output.";
      return { kind: "analog", value: numeric, mode };
    }
    const countN = wholeNumber(count, 1, 255);
    if (countN === null) return "Count must be a whole number from 1 to 255.";
    if (!pulse) return { kind: "crob", code: Number(operation) | Number(tcc), count: countN, onTimeMs: 0, offTimeMs: 0, mode };
    const onN = wholeNumber(onTime, 0, 4294967295);
    const offN = wholeNumber(offTime, 0, 4294967295);
    if (onN === null || offN === null) return "On and off times are whole milliseconds.";
    return { kind: "crob", code: Number(operation) | Number(tcc), count: countN, onTimeMs: onN, offTimeMs: offN, mode };
  };

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    const command = buildCommand();
    if (typeof command === "string") { setError(command); return; }
    setBusy(true);
    setError(null);
    try {
      await onSubmit(command);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      open
      onClose={onCancel}
      title="Control"
      width={340}
      footer={
        <>
          <Button onClick={onCancel}>Cancel</Button>
          <Button type="submit" form="dnp3-control-form" variant="primary" loading={busy} disabled={busy}>
            {mode === "select" ? "Select and operate" : "Operate"}
          </Button>
        </>
      }
    >
      <form id="dnp3-control-form" onSubmit={submit} className="vt-stack" style={{ gap: 12 }}>
        <div>
          <Badge tone="accent" mono>{address}</Badge>
        </div>
        <Field label="Method" hint={mode === "select"
          ? "Sends SELECT, then OPERATE with the same object once the outstation accepts it."
          : "Sends DIRECT OPERATE in a single request."}>
          <Segmented
            block
            label="Control method"
            value={mode}
            onChange={changeMode}
            options={[
              { value: "select", label: "Select before operate" },
              { value: "direct", label: "Direct operate" },
            ]}
          />
        </Field>
        {binary ? (
          <>
            <Field label="Operation">
              <SheetSelect sheetTitle="Operation" value={operation} options={CROB_OPERATIONS} onChange={(v) => setOperation(v)} />
            </Field>
            <Field label="Trip / close" hint="Pairs the operation with a trip or close code, as breaker controls expect.">
              <SheetSelect sheetTitle="Trip / close" value={tcc} options={TCC_OPTIONS} onChange={(v) => setTcc(v)} />
            </Field>
            <div className="vt-field-row">
              <Field label="Count">
                <input className="vt-input vt-input--mono" type="number" min={1} max={255} value={count} onChange={(e) => setCount(e.target.value)} />
              </Field>
              {pulse && (
                <>
                  <Field label="On (ms)">
                    <input className="vt-input vt-input--mono" type="number" min={0} value={onTime} onChange={(e) => setOnTime(e.target.value)} />
                  </Field>
                  <Field label="Off (ms)">
                    <input className="vt-input vt-input--mono" type="number" min={0} value={offTime} onChange={(e) => setOffTime(e.target.value)} />
                  </Field>
                </>
              )}
            </div>
          </>
        ) : (
          <Field label="Value (float32)">
            <input
              type="text"
              className="vt-input vt-input--mono"
              value={value}
              onChange={(e) => setValue(e.target.value)}
              autoFocus
              placeholder={currentValue ? `Current: ${currentValue}` : "Enter value"}
            />
          </Field>
        )}
        {error && <Callout tone="danger">{error}</Callout>}
      </form>
    </Dialog>
  );
}
