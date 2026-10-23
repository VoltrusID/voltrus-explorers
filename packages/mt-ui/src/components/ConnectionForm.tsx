import { useState, useEffect, type FormEvent } from "react";
import { X } from "lucide-react";
import { Button, Field } from "../primitives";
import { SheetSelect } from "../sheetSelect";

export interface FormField {
  name: string;
  label: string;
  type: "text" | "number" | "select" | "password";
  default?: string | number;
  options?: { value: string; label: string }[];
  placeholder?: string;
}

interface ConnectionFormProps {
  fields: FormField[];
  onSubmit: (values: Record<string, string | number>) => void;
  onDisconnect?: () => void;
  onCancel?: () => void;
  connected?: boolean;
  loading?: boolean;
  initialValues?: Record<string, string | number>;
  onChange?: (values: Record<string, string | number>) => void;
}

export function ConnectionForm({
  fields,
  onSubmit,
  onDisconnect,
  onCancel,
  connected = false,
  loading = false,
  initialValues: externalValues,
  onChange,
}: ConnectionFormProps) {
  const defaults: Record<string, string | number> = {};
  for (const f of fields) {
    defaults[f.name] = f.default ?? (f.type === "number" ? 0 : "");
  }
  const [values, setValues] = useState(externalValues ?? defaults);

  useEffect(() => {
    if (externalValues) setValues(externalValues);
  }, [externalValues]);

  const update = (next: Record<string, string | number>) => {
    setValues(next);
    onChange?.(next);
  };

  const handleSubmit = (e: FormEvent) => {
    e.preventDefault();
    onSubmit(values);
  };

  return (
    <form onSubmit={handleSubmit} className="vt-stack vt-rows" style={{ gap: 12 }}>
      {fields.map((field) => (
        <Field key={field.name} label={field.label}>
          {field.type === "select" ? (
            <SheetSelect
              sheetTitle={field.label}
              value={values[field.name]}
              onChange={(v) => update({ ...values, [field.name]: v })}
              disabled={connected}
              options={field.options ?? []}
            />
          ) : (
            <input
              type={field.type}
              className={field.type === "number" ? "vt-input vt-input--mono" : "vt-input"}
              value={values[field.name]}
              onChange={(e) =>
                update({
                  ...values,
                  [field.name]:
                    field.type === "number"
                      ? Number(e.target.value)
                      : e.target.value,
                })
              }
              placeholder={field.placeholder}
              disabled={connected}
            />
          )}
        </Field>
      ))}

      <div className="vt-divider" style={{ margin: "4px 0" }} />

      {!connected ? (
        <div style={{ display: "grid", gridTemplateColumns: loading ? "1fr auto" : "1fr", gap: 6 }}>
          <Button type="submit" variant="primary" block loading={loading}>
            {loading ? "Connecting…" : "Connect"}
          </Button>
          {loading && (
            <Button
              type="button"
              onClick={onCancel}
              aria-label="Cancel connection"
              icon={<X size={14} strokeWidth={1.75} />}
            />
          )}
        </div>
      ) : (
        <Button type="button" variant="danger" block onClick={onDisconnect}>
          Disconnect
        </Button>
      )}
    </form>
  );
}
