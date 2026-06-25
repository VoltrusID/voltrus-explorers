import { useState } from "react";
import { useStore } from "../../store";
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
      <div style={{ padding: "10px 12px", color: "var(--text-muted)", fontSize: 11 }}>
        Build a program to see available variables for mapping.
      </div>
    );
  }

  return (
    <div style={{ padding: "6px 0", fontSize: 12 }}>
      {/* Toolbar */}
      <div style={{ display: "flex", gap: 4, padding: "0 10px", marginBottom: 6 }}>
        <button
          className="io-btn"
          style={{ flex: 1, fontSize: 10, padding: "3px 0" }}
          onClick={autoMapGpio}
          title="Auto-map relay0-5, buzzer, red/green/blue to GPIO"
        >
          Auto-Map
        </button>
        <button
          className="io-btn"
          style={{ fontSize: 10, padding: "3px 8px" }}
          onClick={() => startAdd()}
          title="Add mapping manually"
        >
          + Map
        </button>
        {ioMappings.length > 0 && (
          <button
            className="io-btn"
            style={{ fontSize: 10, padding: "3px 6px", color: "#e55" }}
            onClick={clearIoMappings}
          >
            Clear
          </button>
        )}
      </div>

      {/* Inline editor */}
      {editing && (
        <div
          style={{
            margin: "0 8px 6px",
            background: "var(--surface)",
            borderRadius: 4,
            padding: 8,
            border: "1px solid var(--accent)",
          }}
        >
          {/* Row 1: Variable + Direction */}
          <div style={{ display: "flex", gap: 4, marginBottom: 4 }}>
            <select
              value={form.variable}
              onChange={(e) => setForm({ ...form, variable: e.target.value })}
              style={{ flex: 1, fontSize: 11, padding: "2px 4px" }}
              disabled={editIndex !== null}
            >
              <option value="">Variable...</option>
              {editIndex === null
                ? unmapped.map((v) => (
                    <option key={v.name} value={v.name}>{v.name}</option>
                  ))
                : <option value={form.variable}>{form.variable}</option>}
            </select>
            <select
              value={form.direction}
              onChange={(e) => setForm({ ...form, direction: e.target.value as "input" | "output" })}
              style={{ flexShrink: 0, width: 52, fontSize: 11, padding: "2px 4px" }}
            >
              <option value="output">OUT</option>
              <option value="input">IN</option>
            </select>
          </div>
          {/* Row 2: Protocol + Address */}
          <div style={{ display: "flex", gap: 4, marginBottom: 6 }}>
            <select
              value={form.driver_id}
              onChange={(e) => setForm({ ...form, driver_id: e.target.value, address: "" })}
              style={{ flexShrink: 0, width: 68, fontSize: 11, padding: "2px 4px" }}
            >
              {PROTOCOLS.map((p) => (
                <option key={p.id} value={p.id}>{p.label}</option>
              ))}
            </select>
            {form.driver_id === "gpio" ? (
              <select
                value={form.address}
                onChange={(e) =>
                  setForm({ ...form, address: e.target.value, protocol_address: `GPIO${e.target.value}` })
                }
                style={{ flex: 1, fontSize: 11, padding: "2px 4px" }}
              >
                <option value="">Pin...</option>
                {GPIO_OPTIONS.map((o) => (
                  <option key={o.value} value={o.value}>{o.label}</option>
                ))}
              </select>
            ) : (
              <input
                type="text"
                value={form.address}
                onChange={(e) => setForm({ ...form, address: e.target.value, protocol_address: e.target.value })}
                placeholder={ADDR_PLACEHOLDERS[form.driver_id] || "Address"}
                style={{ flex: 1, fontSize: 11, padding: "2px 4px" }}
              />
            )}
          </div>
          {/* Row 3: Actions */}
          <div style={{ display: "flex", gap: 4 }}>
            <button
              className="io-btn"
              style={{ flex: 1, fontSize: 10, background: "var(--accent)", color: "#fff", border: "none" }}
              onClick={save}
            >
              Save
            </button>
            <button className="io-btn" style={{ fontSize: 10 }} onClick={cancel}>
              Cancel
            </button>
          </div>
        </div>
      )}

      {/* Active mappings — compact rows */}
      {ioMappings.length > 0 && (
        <div style={{ marginBottom: 6 }}>
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
                  gap: 4,
                  padding: "3px 10px",
                  opacity: stillExists ? 1 : 0.5,
                  borderBottom: "1px solid var(--border)",
                }}
              >
                {/* Direction badge */}
                <span
                  style={{
                    fontSize: 9,
                    fontWeight: 700,
                    color: m.direction === "output" ? "#3b82f6" : "#10b981",
                    width: 18,
                    flexShrink: 0,
                  }}
                >
                  {m.direction === "output" ? "OUT" : "IN"}
                </span>
                {/* Variable name */}
                <span
                  style={{
                    fontSize: 11,
                    color: stillExists ? "var(--text)" : "var(--danger)",
                    flex: 1,
                    overflow: "hidden",
                    textOverflow: "ellipsis",
                    whiteSpace: "nowrap",
                    fontFamily: '"SF Mono", monospace',
                  }}
                  title={!stillExists ? `${m.variable} — variable no longer exists` : m.variable}
                >
                  {m.variable}{!stillExists && " !"}
                </span>
                {/* Protocol + Address */}
                <span
                  style={{
                    fontSize: 10,
                    color: "var(--text-muted)",
                    flexShrink: 0,
                  }}
                >
                  {gpioLabel || `${protoLabel}:${m.address}`}
                </span>
                {/* Edit/Delete */}
                <button
                  onClick={() => startEdit(i)}
                  style={{
                    background: "none",
                    border: "none",
                    color: "var(--text-muted)",
                    cursor: "pointer",
                    fontSize: 10,
                    padding: "0 2px",
                    lineHeight: 1,
                  }}
                  title="Edit mapping"
                >
                  &#x270E;
                </button>
                <button
                  onClick={() => removeIoMapping(i)}
                  style={{
                    background: "none",
                    border: "none",
                    color: "#e55",
                    cursor: "pointer",
                    fontSize: 10,
                    padding: "0 2px",
                    lineHeight: 1,
                  }}
                  title="Remove mapping"
                >
                  &#x2715;
                </button>
              </div>
            );
          })}
        </div>
      )}

      {/* Unmapped variables — compact chip list */}
      {unmapped.length > 0 && !editing && (
        <div style={{ padding: "0 10px" }}>
          <div style={{ fontSize: 10, color: "var(--text-muted)", marginBottom: 4 }}>
            Unmapped ({unmapped.length})
          </div>
          <div style={{ display: "flex", flexWrap: "wrap", gap: 3 }}>
            {unmapped.map((v) => (
              <button
                key={v.name}
                className="io-btn"
                style={{ fontSize: 10, padding: "1px 6px", fontFamily: '"SF Mono", monospace' }}
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
        <div style={{ padding: "4px 10px 0", fontSize: 10, color: "var(--text-muted)" }}>
          {ioMappings.length}/{compiledVariables.length} mapped
        </div>
      )}
    </div>
  );
}
