import { useState } from "react";
import { useStore } from "../../store";
import { Badge, Button, EmptyState, IconButton, Segmented } from "@mactools/ui";
import { Pencil, Plus, Trash2, WandSparkles } from "lucide-react";
import type { IoMapping } from "../../types";
import { PROTOCOLS } from "../../constants/protocols";

const GPIO_OPTIONS = [
  { value: "0", label: "Relay 0" },
  { value: "1", label: "Relay 1" },
  { value: "2", label: "Relay 2" },
  { value: "3", label: "Relay 3" },
  { value: "4", label: "Relay 4" },
  { value: "5", label: "Relay 5" },
  { value: "6", label: "Buzzer" },
  { value: "7", label: "Red" },
  { value: "8", label: "Green" },
  { value: "9", label: "Blue" },
];

const ADDR_PLACEHOLDERS: Record<string, string> = {
  modbus: "40001",
  s7: "DB1.DBW0",
  enip: "Tag1",
  mqtt: "output/x",
  opcua: "ns=2;s=X",
};

const emptyMapping: IoMapping = {
  variable: "",
  direction: "output",
  address: "",
  driver_id: "gpio",
  protocol_address: "",
};

export default function IoMappingPanel() {
  const {
    compiledVariables,
    ioMappings,
    addIoMapping,
    updateIoMapping,
    removeIoMapping,
    clearIoMappings,
    autoMapGpio,
  } = useStore();

  const [editing, setEditing] = useState(false);
  const [editIndex, setEditIndex] = useState<number | null>(null);
  const [form, setForm] = useState<IoMapping>({ ...emptyMapping });

  const mappedVars = new Set(ioMappings.map((m) => m.variable));
  const unmapped = compiledVariables.filter((v) => !mappedVars.has(v.name));

  const startAdd = (variableName?: string) => {
    setForm({ ...emptyMapping, variable: variableName || "" });
    setEditing(true);
    setEditIndex(null);
  };

  const startEdit = (index: number) => {
    setForm({ ...ioMappings[index] });
    setEditIndex(index);
    setEditing(true);
  };

  const save = () => {
    if (!form.variable || !form.address) return;
    if (editIndex !== null) {
      updateIoMapping(editIndex, { ...form });
    } else {
      addIoMapping({ ...form });
    }
    setEditing(false);
    setEditIndex(null);
  };

  const cancel = () => {
    setEditing(false);
    setEditIndex(null);
  };

  if (compiledVariables.length === 0) {
    return (
      <EmptyState
        compact
        icon={<Pencil size={18} strokeWidth={1.75} />}
        title="Nothing to map yet"
        description="Build a program to see its variables here for mapping to I/O."
      />
    );
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8, fontSize: "var(--fs-sm)" }}>
      {/* Toolbar */}
      <div style={{ display: "flex", gap: 4 }}>
        <Button
          size="sm"
          variant="default"
          icon={<WandSparkles size={14} strokeWidth={1.75} />}
          style={{ flex: 1 }}
          onClick={autoMapGpio}
          title="Auto-map relay0-5, buzzer, red/green/blue to GPIO"
        >
          Auto-map
        </Button>
        <Button
          size="sm"
          variant="default"
          icon={<Plus size={14} strokeWidth={1.75} />}
          onClick={() => startAdd()}
          title="Add mapping manually"
        >
          Map
        </Button>
        {ioMappings.length > 0 && (
          <Button
            size="sm"
            variant="danger"
            icon={<Trash2 size={14} strokeWidth={1.75} />}
            onClick={clearIoMappings}
            title="Remove all mappings"
          >
            Clear
          </Button>
        )}
      </div>

      {/* Inline editor */}
      {editing && (
        <div
          style={{
            background: "var(--card)",
            borderRadius: "var(--radius)",
            padding: 8,
            border: "1px solid var(--border-light)",
            display: "flex",
            flexDirection: "column",
            gap: 6,
          }}
        >
          {/* Row 1: Variable + Direction */}
          <div style={{ display: "flex", gap: 6 }}>
            <select
              className="vt-select vt-select--sm"
              value={form.variable}
              onChange={(e) => setForm({ ...form, variable: e.target.value })}
              style={{ flex: 1 }}
              disabled={editIndex !== null}
              aria-label="Variable"
            >
              <option value="">Variable</option>
              {editIndex === null
                ? unmapped.map((v) => (
                    <option key={v.name} value={v.name}>{v.name}</option>
                  ))
                : <option value={form.variable}>{form.variable}</option>}
            </select>
            <Segmented
              size="sm"
              label="Direction"
              value={form.direction}
              onChange={(d) => setForm({ ...form, direction: d })}
              options={[
                { value: "output", label: "Out" },
                { value: "input", label: "In" },
              ]}
            />
          </div>
          {/* Row 2: Protocol + Address */}
          <div style={{ display: "flex", gap: 6 }}>
            <select
              className="vt-select vt-select--sm"
              value={form.driver_id}
              onChange={(e) => setForm({ ...form, driver_id: e.target.value, address: "" })}
              style={{ width: 90, flexShrink: 0 }}
              aria-label="Protocol"
            >
              {PROTOCOLS.map((p) => (
                <option key={p.id} value={p.id}>{p.label}</option>
              ))}
            </select>
            {form.driver_id === "gpio" ? (
              <select
                className="vt-select vt-select--sm"
                value={form.address}
                onChange={(e) =>
                  setForm({ ...form, address: e.target.value, protocol_address: `GPIO${e.target.value}` })
                }
                style={{ flex: 1 }}
                aria-label="GPIO pin"
              >
                <option value="">Pin</option>
                {GPIO_OPTIONS.map((o) => (
                  <option key={o.value} value={o.value}>{o.label}</option>
                ))}
              </select>
            ) : (
              <input
                className="vt-input vt-input--sm vt-input--mono"
                type="text"
                value={form.address}
                onChange={(e) => setForm({ ...form, address: e.target.value, protocol_address: e.target.value })}
                placeholder={ADDR_PLACEHOLDERS[form.driver_id] || "Address"}
                style={{ flex: 1 }}
                aria-label="Protocol address"
              />
            )}
          </div>
          {/* Row 3: Actions */}
          <div style={{ display: "flex", gap: 4 }}>
            <Button size="sm" variant="primary" style={{ flex: 1 }} onClick={save}>
              Save
            </Button>
            <Button size="sm" variant="ghost" onClick={cancel}>
              Cancel
            </Button>
          </div>
        </div>
      )}

      {/* Active mappings — compact rows */}
      {ioMappings.length > 0 && (
        <div style={{ display: "flex", flexDirection: "column" }}>
          {ioMappings.map((m, i) => {
            const stillExists = compiledVariables.some((v) => v.name === m.variable);
            const gpioLabel = m.driver_id === "gpio"
              ? GPIO_OPTIONS.find((o) => o.value === m.address)?.label || m.address
              : null;
            const protoLabel = PROTOCOLS.find((p) => p.id === m.driver_id)?.label || m.driver_id;

            return (
              <div
                key={i}
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: 6,
                  padding: "3px 0",
                  opacity: stillExists ? 1 : 0.5,
                  borderBottom: "1px solid var(--border)",
                }}
              >
                <Badge tone={m.direction === "output" ? "info" : "accent"}>{m.direction === "output" ? "Out" : "In"}</Badge>
                <span
                  style={{
                    fontSize: "var(--fs-xs)",
                    fontFamily: "var(--font-mono)",
                    color: stillExists ? "var(--text)" : "var(--danger)",
                    flex: 1,
                    minWidth: 0,
                    overflow: "hidden",
                    textOverflow: "ellipsis",
                    whiteSpace: "nowrap",
                  }}
                  title={!stillExists ? `${m.variable}: variable no longer exists` : m.variable}
                >
                  {m.variable}{!stillExists && " !"}
                </span>
                <span
                  style={{
                    fontSize: "var(--fs-xs)",
                    fontFamily: "var(--font-mono)",
                    color: "var(--text-muted)",
                    flexShrink: 0,
                  }}
                >
                  {gpioLabel || `${protoLabel}:${m.address}`}
                </span>
                <IconButton label="Edit mapping" title="Edit mapping" size="sm" onClick={() => startEdit(i)}>
                  <Pencil size={12} strokeWidth={1.75} />
                </IconButton>
                <IconButton label="Remove mapping" title="Remove mapping" size="sm" danger onClick={() => removeIoMapping(i)}>
                  <Trash2 size={12} strokeWidth={1.75} />
                </IconButton>
              </div>
            );
          })}
        </div>
      )}

      {/* Unmapped variables — compact chip list */}
      {unmapped.length > 0 && !editing && (
        <div>
          <div className="vt-label" style={{ marginBottom: 4 }}>
            Unmapped ({unmapped.length})
          </div>
          <div style={{ display: "flex", flexWrap: "wrap", gap: 3 }}>
            {unmapped.map((v) => (
              <button
                key={v.name}
                className="vt-btn vt-btn--sm vt-btn--ghost"
                style={{ fontFamily: "var(--font-mono)", height: 22, padding: "0 6px" }}
                onClick={() => startAdd(v.name)}
                title={`Map ${v.name} to I/O`}
              >
                {v.name}
              </button>
            ))}
          </div>
        </div>
      )}

      {/* Mapped count summary */}
      {compiledVariables.length > 0 && (
        <div style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)", fontVariantNumeric: "tabular-nums" }}>
          {ioMappings.length}/{compiledVariables.length} mapped
        </div>
      )}
    </div>
  );
}
