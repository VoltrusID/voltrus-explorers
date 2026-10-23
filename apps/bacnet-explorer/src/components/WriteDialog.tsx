import { useState, type FormEvent } from "react";
import { Badge, Button, Callout, Dialog, Field, Segmented, SheetSelect } from "@mactools/ui";
import { PRIORITY_OPTIONS } from "../writeValue";

interface WriteDialogProps {
  /** What is written, e.g. "Analog Output #1 · Present Value". */
  address: string;
  currentValue: string;
  /** Types the object accepts (one entry: analog objects take Real, binary Enumerated, multi-state Unsigned). */
  types: { value: string; label: string }[];
  /** Failure of the last write, shown in the dialog so the user can correct it and retry. */
  error?: string | null;
  /** Write `value` at `priority` (`""` is the device default). May be async; the dialog disables its buttons while it runs. */
  onWrite: (value: string, priority: string) => void | Promise<void>;
  /** Clear the command at `priority`, so the next priority in the array takes over. */
  onRelinquish: (priority: string) => void | Promise<void>;
  onCancel: () => void;
}

/** The write dialog of the property inspector: value, command priority, and a button that relinquishes that priority. */
export function WriteDialog({ address, currentValue, types, error, onWrite, onRelinquish, onCancel }: WriteDialogProps) {
  const kind = types[0]?.value ?? "real";
  const [value, setValue] = useState(currentValue);
  const [priority, setPriority] = useState("");
  const [busy, setBusy] = useState(false);

  const run = async (action: () => void | Promise<void>) => {
    setBusy(true);
    try {
      await action();
    } finally {
      setBusy(false);
    }
  };
  const submit = (e: FormEvent) => {
    e.preventDefault();
    void run(() => onWrite(value, priority));
  };

  return (
    <Dialog
      open
      onClose={onCancel}
      title="Write value"
      width={340}
      footer={
        <>
          <Button onClick={onCancel}>Cancel</Button>
          <Button variant="danger" disabled={busy} onClick={() => void run(() => onRelinquish(priority))}>
            Relinquish
          </Button>
          <Button type="submit" form="bacnet-write-form" variant="primary" loading={busy} disabled={busy || (kind !== "bool" && !value)}>
            Write
          </Button>
        </>
      }
    >
      <form id="bacnet-write-form" onSubmit={submit} className="vt-stack" style={{ gap: 12 }}>
        <div>
          <Badge tone="accent" mono>{address}</Badge>
        </div>
        <Field label="Type">
          <SheetSelect sheetTitle="Data type" value={kind} onChange={() => undefined} options={types} disabled={types.length < 2} />
        </Field>
        <Field label="Value">
          {kind === "bool" ? (
            <Segmented
              block
              label="Binary value"
              value={value as "true" | "false"}
              onChange={setValue}
              options={[
                { value: "false", label: "Inactive" },
                { value: "true", label: "Active" },
              ]}
            />
          ) : (
            <input
              type="text"
              className="vt-input vt-input--mono"
              value={value}
              onChange={(e) => setValue(e.target.value)}
              autoFocus
              placeholder={currentValue ? `Current: ${currentValue}` : "Enter value"}
            />
          )}
        </Field>
        <Field label="Priority" hint="Relinquish clears this priority, so the next one in the priority array takes over.">
          <SheetSelect sheetTitle="Command priority" value={priority} onChange={setPriority} options={PRIORITY_OPTIONS} />
        </Field>
        {error && <Callout tone="danger">{error}</Callout>}
      </form>
    </Dialog>
  );
}
