import { Download, FileDown, FileUp, Pencil, Trash2 } from "lucide-react";
import { Button, Dialog, Field, IconButton, ProBadge, Section, SheetSelect } from "@mactools/ui";
import type { Device } from "./types";
import { DATA_TYPES } from "./types";
import { FREE_TAG_LIMIT } from "./helpers";
import { useTagsEditor } from "./useTagsEditor";

const DATA_TYPE_OPTIONS = DATA_TYPES.map((dt) => ({ value: dt, label: dt }));

interface Props {
  device: Device;
  /** When set, a "Load from Simulator" button is shown (sim running with a template). */
  onLoadSim?: () => void;
}

export default function TagsPanel({ device, onLoadSim }: Props) {
  const {
    tags, pro,
    pendingImport, setPendingImport,
    editingTagId, newName, setNewName, newAddr, setNewAddr, newType, setNewType,
    reset, addTag, saveTag, editTag, removeTag, exportCsv, chooseImportFile, applyImport,
  } = useTagsEditor(device);

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
              <SheetSelect id={id} sheetTitle="Type" value={newType} options={DATA_TYPE_OPTIONS} onChange={(v) => setNewType(v)} />
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
