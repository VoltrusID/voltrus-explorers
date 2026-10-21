import { useEffect, useState } from "react";
import { Plus, Trash2, X } from "lucide-react";
import { Button, Dialog, EmptyState, IconButton } from "@mactools/ui";
import {
  FIELD_TYPES,
  loadDecoders,
  newDecoder,
  saveDecoders,
  type Decoder,
  type FieldType,
} from "./decoders";

const inputStyle: React.CSSProperties = {
  background: "var(--input-bg)",
  border: "1px solid var(--border)",
  borderRadius: 6,
  padding: "4px 8px",
  color: "var(--text)",
  fontSize: 12,
  outline: "none",
  width: "100%",
};

const selectStyle: React.CSSProperties = { ...inputStyle, cursor: "default" };

interface Props {
  onClose: () => void;
}

export function DecoderEditor({ onClose }: Props) {
  const [decoders, setDecoders] = useState<Decoder[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);

  useEffect(() => {
    const loaded = loadDecoders();
    setDecoders(loaded);
    if (loaded.length > 0) setSelectedId(loaded[0].id);
  }, []);

  const persist = (next: Decoder[]) => {
    setDecoders(next);
    saveDecoders(next);
  };

  const selected = decoders.find((d) => d.id === selectedId) ?? null;

  const addDecoder = () => {
    const d = newDecoder(`Decoder ${decoders.length + 1}`);
    persist([...decoders, d]);
    setSelectedId(d.id);
  };

  const removeDecoder = (id: string) => {
    const next = decoders.filter((d) => d.id !== id);
    persist(next);
    if (selectedId === id) setSelectedId(next[0]?.id ?? null);
  };

  const updateDecoder = (id: string, patch: Partial<Decoder>) => {
    persist(decoders.map((d) => (d.id === id ? { ...d, ...patch } : d)));
  };

  const updateField = (idx: number, patch: Partial<Decoder["fields"][number]>) => {
    if (!selected) return;
    const fields = selected.fields.map((f, i) => (i === idx ? { ...f, ...patch } : f));
    updateDecoder(selected.id, { fields });
  };

  const addField = () => {
    if (!selected) return;
    updateDecoder(selected.id, {
      fields: [...selected.fields, { name: `field${selected.fields.length + 1}`, type: "dint", size: 0 }],
    });
  };

  const removeField = (idx: number) => {
    if (!selected) return;
    updateDecoder(selected.id, { fields: selected.fields.filter((_, i) => i !== idx) });
  };

  return (
    <Dialog open onClose={onClose} title="Custom decoders" width={720}>
      <div style={{ display: "grid", gridTemplateColumns: "180px 1fr", gap: 12, minHeight: 360 }}>
        {/* Decoder list */}
        <div style={{ display: "flex", flexDirection: "column", gap: 4, overflowY: "auto", maxHeight: "60vh" }}>
          {decoders.map((d) => (
            <Button
              key={d.id}
              variant={d.id === selectedId ? "default" : "ghost"}
              aria-pressed={d.id === selectedId}
              block
              size="sm"
              style={{ justifyContent: "flex-start" }}
              onClick={() => setSelectedId(d.id)}
            >
              {d.name || "(unnamed)"}
            </Button>
          ))}
          <Button variant="ghost" size="sm" icon={<Plus size={14} />} onClick={addDecoder}>
            New decoder
          </Button>
        </div>

        {/* Editor */}
        <div style={{ display: "flex", flexDirection: "column", gap: 8, minWidth: 0 }}>
          {selected ? (
            <>
              <div style={{ display: "flex", gap: 6 }}>
                <input
                  style={inputStyle}
                  value={selected.name}
                  onChange={(e) => updateDecoder(selected.id, { name: e.target.value })}
                  placeholder="Decoder name"
                  aria-label="Decoder name"
                />
                <Button variant="danger" size="sm" icon={<Trash2 size={14} />} onClick={() => removeDecoder(selected.id)}>
                  Delete
                </Button>
              </div>

              <div style={{ display: "grid", gridTemplateColumns: "1fr 120px 70px 28px", gap: 6, fontSize: 11, fontWeight: 500, color: "var(--text-dim)" }}>
                <span>Field name</span><span>Type</span><span>Size (bytes)</span><span />
              </div>

              <div style={{ overflowY: "auto", flex: 1, maxHeight: "52vh" }}>
                {selected.fields.map((f, i) => (
                  <div key={i} style={{ display: "grid", gridTemplateColumns: "1fr 120px 70px 28px", gap: 6, marginBottom: 6 }}>
                    <input style={inputStyle} value={f.name} onChange={(e) => updateField(i, { name: e.target.value })} aria-label="Field name" />
                    <select
                      style={selectStyle}
                      value={f.type}
                      onChange={(e) => updateField(i, { type: e.target.value as FieldType })}
                      aria-label="Field type"
                    >
                      {FIELD_TYPES.map((t) => (
                        <option key={t.value} value={t.value}>{t.label}</option>
                      ))}
                    </select>
                    <input
                      style={inputStyle}
                      type="number"
                      min={0}
                      value={f.size}
                      disabled={f.type !== "bytes"}
                      onChange={(e) => updateField(i, { size: Number(e.target.value) })}
                      aria-label="Field size in bytes"
                    />
                    <IconButton size="sm" danger label={`Remove field ${f.name || i + 1}`} onClick={() => removeField(i)}>
                      <X size={12} />
                    </IconButton>
                  </div>
                ))}
                <Button variant="ghost" size="sm" icon={<Plus size={14} />} onClick={addField}>
                  Add field
                </Button>
              </div>
            </>
          ) : (
            <EmptyState
              title="No decoders"
              description="Create a decoder to define a CIP struct layout. Fields are decoded little-endian from the raw attribute bytes in the CIP Object Browser."
              action={
                <Button variant="primary" size="sm" icon={<Plus size={14} />} onClick={addDecoder}>
                  New decoder
                </Button>
              }
            />
          )}
        </div>
      </div>
    </Dialog>
  );
}
