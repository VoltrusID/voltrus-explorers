import { useState, type FormEvent } from "react";
import { Badge, Button, Dialog, Field, Segmented } from "../primitives";

const DATA_TYPES = [
  { value: "int16", label: "Int16" },
  { value: "uint16", label: "UInt16" },
  { value: "int32", label: "Int32" },
  { value: "uint32", label: "UInt32" },
  { value: "float32", label: "Float32" },
  { value: "float64", label: "Float64" },
  { value: "bool", label: "Bool" },
  { value: "string", label: "String" },
] as const;

interface ValueEditorProps {
  address?: string;
  currentValue?: string;
  currentDataType?: string;
  /** Override the type dropdown vocabulary (e.g. CIP vs Modbus names). */
  types?: { value: string; label: string }[];
  onSubmit: (value: string, dataType: string) => void;
  onCancel: () => void;
}

export function ValueEditor({ address, currentValue, currentDataType, types, onSubmit, onCancel }: ValueEditorProps) {
  const typeOptions = types ?? DATA_TYPES;
  const [value, setValue] = useState(currentValue ?? "");
  const [dataType, setDataType] = useState<string>(currentDataType ?? typeOptions[0]?.value ?? "float64");

  const handleSubmit = (e: FormEvent) => {
    e.preventDefault();
    onSubmit(value, dataType);
  };

  return (
    <Dialog
      open
      onClose={onCancel}
      title="Write value"
      width={320}
      footer={
        <>
          <Button onClick={onCancel}>Cancel</Button>
          <Button type="submit" form="vt-value-editor-form" variant="primary" disabled={dataType !== "bool" && !value}>
            Write
          </Button>
        </>
      }
    >
      <form id="vt-value-editor-form" onSubmit={handleSubmit} className="vt-stack" style={{ gap: 12 }}>
        {address && (
          <div>
            <Badge tone="accent" mono>{address}</Badge>
          </div>
        )}
        <Field label="Type">
          <select
            className="vt-select"
            value={dataType}
            onChange={(e) => setDataType(e.target.value)}
          >
            {typeOptions.map((dt) => (
              <option key={dt.value} value={dt.value}>
                {dt.label}
              </option>
            ))}
          </select>
        </Field>
        <Field label="Value">
          {dataType === "bool" ? (
            <Segmented
              block
              label="Boolean value"
              value={value as "true" | "false"}
              onChange={setValue}
              options={[
                { value: "false", label: "False" },
                { value: "true", label: "True" },
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
      </form>
    </Dialog>
  );
}
