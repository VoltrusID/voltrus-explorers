import { useState } from "react";
import { Download, FileDown, FileUp, Pencil, Trash2 } from "lucide-react";
import { Button, Dialog, Field, IconButton, ProBadge, Section, openTextFile, saveTextFile, useProStatus } from "@mactools/ui";
import { useStore } from "./store";
import type { Device } from "./types";
import { DATA_TYPES } from "./types";
import { uid, FREE_TAG_LIMIT } from "./helpers";
import { effectiveDataType, parseTagCsv, tagsToCsv, type TagCsvImport } from "./tagCsv";

interface Props {
  device: Device;
  /** When set, a "Load from Simulator" button is shown (sim running with a template). */
  onLoadSim?: () => void;
}

export default function TagsPanel({ device, onLoadSim }: Props) {
  const setDeviceTags = useStore((st) => st.setDeviceTags);
  const addLog = useStore((st) => st.addLog);
  const [pendingImport, setPendingImport] = useState<(TagCsvImport & { path: string }) | null>(null);
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
      { id: uid(), name: newName.trim(), address: newAddr.trim(), data_type: effectiveDataType(newAddr, newType) },
    ]);
    setNewName(""); setNewAddr("");
  };

  const saveTag = () => {
    if (!editingTagId || !newName.trim() || !newAddr.trim()) return;
    setDeviceTags(device.id, (prev) =>
      prev.map((t) =>
        t.id === editingTagId ? { ...t, name: newName.trim(), address: newAddr.trim(), data_type: effectiveDataType(newAddr, newType) } : t,
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

  const exportCsv = async () => {
    if (!requirePro("Tag list CSV export")) return;
    try {
      const path = await saveTextFile(`${device.label}-tags.csv`, tagsToCsv(tags));
      if (path) addLog("info", `[${device.label}] Exported ${tags.length} tags to ${path}`);
    } catch (e) {
      addLog("error", `[${device.label}] Tag export failed: ${String(e)}`);
    }
  };

  const chooseImportFile = async () => {
    if (!requirePro("Tag list CSV import")) return;
    try {
      const file = await openTextFile();
      if (!file) return;
      const parsed = parseTagCsv(file.text);
      parsed.errors.forEach((message) => addLog("error", `${file.path}: ${message}`));
      if (parsed.tags.length === 0) {
        addLog("error", `[${device.label}] No usable tags in ${file.path}`);
        return;
      }
      setPendingImport({ ...parsed, path: file.path });
    } catch (e) {
      addLog("error", `[${device.label}] Tag import failed: ${String(e)}`);
    }
  };

  const applyImport = (mode: "replace" | "add") => {
    if (!pendingImport) return;
    const incoming = pendingImport.tags.map((t) => ({ id: uid(), ...t }));
    let added = incoming.length;
    if (mode === "replace") {
      setDeviceTags(device.id, incoming);
    } else {
      const have = new Set(tags.map((t) => `${t.name}\u0000${t.address}`));
      const fresh = incoming.filter((t) => !have.has(`${t.name}\u0000${t.address}`));
      added = fresh.length;
      setDeviceTags(device.id, (prev) => [...prev, ...fresh]);
    }
    const skipped = pendingImport.errors.length;
    addLog(
      "info",
      `[${device.label}] Imported ${added} tags from ${pendingImport.path}${skipped > 0 ? ` (${skipped} row(s) skipped)` : ""}`,
    );
    pendingImport.notes.forEach((note) => addLog("info", note));
    setPendingImport(null);
  };

  return (
    <Section title="Tags" meta={
        pro || tags.length <= FREE_TAG_LIMIT
          ? pro ? String(tags.length) : `${tags.length} of ${FREE_TAG_LIMIT}`
          : `${FREE_TAG_LIMIT} of ${tags.length} polled`
      }>
      <div style={{ display: "flex", flexDirection: "column", gap: 8, padding: 14 }}>
        {onLoadSim && (
          <Button size="sm" icon={<Download size={14} strokeWidth={1.75} />} onClick={onLoadSim} title="Replace this device's tags with the simulator's points">
            Load from simulator
          </Button>
        )}

        <div style={{ display: "flex", gap: 6 }}>
          <Button size="sm" style={{ flex: 1 }} icon={<FileUp size={14} strokeWidth={1.75} />} onClick={chooseImportFile} title="Import tags from a CSV file (TIA Portal tag table export or Name,Address,Type)">
            Import CSV <ProBadge />
          </Button>
          <Button size="sm" style={{ flex: 1 }} icon={<FileDown size={14} strokeWidth={1.75} />} disabled={tags.length === 0} onClick={exportCsv} title="Export this PLC's tags to a CSV file">
            Export CSV <ProBadge />
          </Button>
        </div>

        <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
          <Field label="Name">
            {(id) => (
              <input id={id} className="vt-input" placeholder="Name" value={newName} onChange={(e) => setNewName(e.target.value)} />
            )}
          </Field>
          <Field label="Address">
            {(id) => (
              <input id={id} className="vt-input" placeholder="DB1.DBD0, IW100, MW50, T5, C10" value={newAddr} onChange={(e) => setNewAddr(e.target.value)} />
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
      <Dialog
        open={pendingImport !== null}
        onClose={() => setPendingImport(null)}
        title="Import tags"
        width={420}
        footer={
          <>
            <Button size="sm" onClick={() => setPendingImport(null)}>Cancel</Button>
            <Button size="sm" onClick={() => applyImport("add")}>Add to existing</Button>
            <Button size="sm" variant="primary" onClick={() => applyImport("replace")}>Replace existing</Button>
          </>
        }
      >
        {pendingImport && (
          <div style={{ display: "flex", flexDirection: "column", gap: 8, fontSize: "var(--fs-sm)", color: "var(--text-dim)", lineHeight: 1.5 }}>
            <div>
              <strong style={{ color: "var(--text)" }}>{pendingImport.tags.length} tags</strong> found in{" "}
              <span className="mono">{pendingImport.path.split(/[\\/]/).pop()}</span>.
              {pendingImport.errors.length > 0 && ` ${pendingImport.errors.length} row(s) will be skipped; the reasons are in the comm log.`}
            </div>
            {pendingImport.notes.map((note) => <div key={note}>{note}</div>)}
            <div>
              "Replace existing" swaps the {tags.length} tags on {device.label} for these. "Add to existing" keeps them and skips tags with the same name and address.
            </div>
          </div>
        )}
      </Dialog>
    </Section>
  );
}
