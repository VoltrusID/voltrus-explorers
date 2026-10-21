import { useState } from "react";
import { Download, Pencil, Trash2 } from "lucide-react";
import { Button, Field, IconButton, Section, useProStatus } from "@mactools/ui";
import { useStore } from "./store";
import type { Device } from "./types";
import { DATA_TYPES } from "./types";
import { uid, FREE_TAG_LIMIT } from "./helpers";

interface Props {
  device: Device;
  /** When set, a "Load from Simulator" button is shown (sim running with a template). */
  onLoadSim?: () => void;
}

export default function TagsPanel({ device, onLoadSim }: Props) {
  const setDeviceTags = useStore((st) => st.setDeviceTags);
  const [editingTagId, setEditingTagId] = useState<string | null>(null);
  const [newName, setNewName] = useState("");
  const [newAddr, setNewAddr] = useState("");
  const [newType, setNewType] = useState("DWord");
  const { pro, requirePro } = useProStatus();
  const tags = device.tags ?? [];

  const reset = () => { setEditingTagId(null); setNewName(""); setNewAddr(""); };

  const addTag = () => {
    if (!newName.trim() || !newAddr.trim()) return;
    if (tags.length >= FREE_TAG_LIMIT && !requirePro("More than 10 live tags")) return;
    setDeviceTags(device.id, (prev) => [
      ...prev,
      { id: uid(), name: newName.trim(), address: newAddr.trim(), data_type: newType },
    ]);
    setNewName(""); setNewAddr("");
  };

  const saveTag = () => {
    if (!editingTagId || !newName.trim() || !newAddr.trim()) return;
    setDeviceTags(device.id, (prev) =>
      prev.map((t) =>
        t.id === editingTagId ? { ...t, name: newName.trim(), address: newAddr.trim(), data_type: newType } : t,
      ),
    );
    reset();
  };

  const editTag = (t: { id: string; name: string; address: string; data_type: string }) => {
    setEditingTagId(t.id);
    setNewName(t.name); setNewAddr(t.address); setNewType(t.data_type);
  };

  const removeTag = (id: string) => {
    setDeviceTags(device.id, (prev) => prev.filter((x) => x.id !== id));
  };

  return (
    <Section title="Tags" meta={pro ? String(tags.length) : `${tags.length} of ${FREE_TAG_LIMIT}`}>
      <div style={{ display: "flex", flexDirection: "column", gap: 8, padding: 14 }}>
        {onLoadSim && (
          <Button size="sm" icon={<Download size={14} strokeWidth={1.75} />} onClick={onLoadSim} title="Replace this device's tags with the simulator's points">
            Load from simulator
          </Button>
        )}

        <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
          <Field label="Name">
            {(id) => (
              <input id={id} className="vt-input" placeholder="Name" value={newName} onChange={(e) => setNewName(e.target.value)} />
            )}
          </Field>
          <Field label="Address">
            {(id) => (
              <input id={id} className="vt-input" placeholder="DB1.DBD0, IW100, MW50" value={newAddr} onChange={(e) => setNewAddr(e.target.value)} />
            )}
          </Field>
          <Field label="Type">
            {(id) => (
              <select id={id} className="vt-select" value={newType} onChange={(e) => setNewType(e.target.value)}>
                {DATA_TYPES.map((dt) => <option key={dt} value={dt}>{dt}</option>)}
              </select>
            )}
          </Field>
          {editingTagId ? (
            <div style={{ display: "flex", gap: 6 }}>
              <Button size="sm" variant="primary" style={{ flex: 1 }} onClick={saveTag}>Save</Button>
              <Button size="sm" style={{ flex: 1 }} onClick={reset}>Cancel</Button>
            </div>
          ) : (
            <Button size="sm" variant="primary" block disabled={!newName.trim() || !newAddr.trim()} onClick={addTag}>
              Add tag
            </Button>
          )}
        </div>

        <div style={{ display: "flex", flexDirection: "column" }}>
          {tags.map((t) => (
            <div key={t.id} style={{ display: "flex", justifyContent: "space-between", alignItems: "center", padding: "6px 0", borderBottom: "1px solid var(--border)", fontSize: "var(--fs-sm)" }}>
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ color: "var(--text)", whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{t.name}</div>
                <div className="mono" style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)" }}>{t.address} &middot; {t.data_type}</div>
              </div>
              <div style={{ display: "flex", gap: 2 }}>
                <IconButton size="sm" label={`Edit ${t.name}`} onClick={() => editTag(t)}>
                  <Pencil size={14} strokeWidth={1.75} />
                </IconButton>
                <IconButton size="sm" danger label={`Delete ${t.name}`} onClick={() => removeTag(t.id)}>
                  <Trash2 size={14} strokeWidth={1.75} />
                </IconButton>
              </div>
            </div>
          ))}
        </div>
      </div>
    </Section>
  );
}
