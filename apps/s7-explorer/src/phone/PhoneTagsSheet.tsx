import { useState } from "react";
import { Download, FileDown, FileUp, Plus, Trash2 } from "lucide-react";
import { Button, Dialog, EmptyState, Field, ListGroup, ListRow, ProBadge, Section, SheetSelect } from "@mactools/ui";
import type { Device } from "../types";
import { DATA_TYPES } from "../types";
import { useTagsEditor } from "../useTagsEditor";

interface Props {
  device: Device;
  /** Set while the simulator runs a template: offers to copy its points into this PLC. */
  onLoadSim?: () => void;
  onClose: () => void;
}

/** Tags of a PLC as a sheet: the list (tap to edit), add, CSV import and export. The editor is a second sheet. */
export default function PhoneTagsSheet({ device, onLoadSim, onClose }: Props) {
  const ed = useTagsEditor(device);
  const [editorOpen, setEditorOpen] = useState(false);

  const openNew = () => {
    ed.reset();
    setEditorOpen(true);
  };
  const openEdit = (t: Device["tags"][number]) => {
    ed.editTag(t);
    setEditorOpen(true);
  };
  const closeEditor = () => {
    ed.reset();
    setEditorOpen(false);
  };
  const save = () => {
    if (ed.editingTagId ? ed.saveTag() : ed.addTag()) setEditorOpen(false);
  };
  const complete = ed.newName.trim() !== "" && ed.newAddr.trim() !== "";

  return (
    <>
      <Dialog
        open
        onClose={onClose}
        title="Tags"
        footer={
          <Button variant="primary" block icon={<Plus size={16} strokeWidth={1.75} />} onClick={openNew}>
            Add tag
          </Button>
        }
      >
        <div className="s7-sheet-stack">
          {ed.tags.length === 0 ? (
            <EmptyState compact title="No tags" description="Add one, import a CSV or copy the simulator's points." />
          ) : (
            <ListGroup>
              {ed.tags.map((t) => (
                <ListRow
                  key={t.id}
                  title={t.name}
                  subtitle={
                    <span className="mono">
                      {t.address} · {t.data_type}
                    </span>
                  }
                  chevron
                  onClick={() => openEdit(t)}
                />
              ))}
            </ListGroup>
          )}
          <Section title="Tag list">
            <div className="s7-actions">
              {onLoadSim && (
                <Button block icon={<Download size={16} strokeWidth={1.75} />} onClick={onLoadSim}>
                  Load from simulator
                </Button>
              )}
              <Button block icon={<FileUp size={16} strokeWidth={1.75} />} onClick={ed.chooseImportFile}>
                Import CSV <ProBadge />
              </Button>
              <Button block icon={<FileDown size={16} strokeWidth={1.75} />} disabled={ed.tags.length === 0} onClick={ed.exportCsv}>
                Export CSV <ProBadge />
              </Button>
            </div>
          </Section>
        </div>
      </Dialog>

      {editorOpen && (
        <Dialog
          open
          onClose={closeEditor}
          title={ed.editingTagId ? "Edit tag" : "New tag"}
          footer={
            <>
              <Button onClick={closeEditor}>Cancel</Button>
              <Button variant="primary" disabled={!complete} onClick={save}>
                {ed.editingTagId ? "Save" : "Add tag"}
              </Button>
            </>
          }
        >
          <div className="s7-sheet-stack">
            <Section title="Tag">
              <div className="vt-stack vt-rows">
                <Field label="Name">
                  {(id) => (
                    <input id={id} className="vt-input" placeholder="Pump speed" value={ed.newName} onChange={(e) => ed.setNewName(e.target.value)} autoCorrect="off" />
                  )}
                </Field>
                <Field label="Address" hint="DB1.DBD0, IW100, QX0.1, MW50, T5 or C10">
                  {(id) => (
                    <input
                      id={id}
                      className="vt-input vt-input--mono"
                      placeholder="DB1.DBD0"
                      value={ed.newAddr}
                      autoCapitalize="characters"
                      autoCorrect="off"
                      spellCheck={false}
                      onChange={(e) => ed.setNewAddr(e.target.value)}
                    />
                  )}
                </Field>
                <Field label="Type">
                  <SheetSelect sheetTitle="Data type" value={ed.newType} onChange={ed.setNewType} options={DATA_TYPES.map((d) => ({ value: d, label: d }))} />
                </Field>
              </div>
            </Section>
            {ed.editingTagId && (
              <Button
                variant="ghost"
                block
                icon={<Trash2 size={16} strokeWidth={1.75} />}
                className="s7-remove"
                onClick={() => {
                  ed.removeTag(ed.editingTagId!);
                  closeEditor();
                }}
              >
                Delete tag
              </Button>
            )}
          </div>
        </Dialog>
      )}

      {ed.pendingImport && (
        <Dialog
          open
          onClose={() => ed.setPendingImport(null)}
          title="Import tags"
          footer={
            <>
              <Button onClick={() => ed.setPendingImport(null)}>Cancel</Button>
              <Button onClick={() => ed.applyImport("add")}>Add to existing</Button>
              <Button variant="primary" onClick={() => ed.applyImport("replace")}>
                Replace existing
              </Button>
            </>
          }
        >
          <div className="s7-copy">
            <p>
              <strong>{ed.pendingImport.tags.length} tags</strong> found in <span className="mono">{ed.pendingImport.path.split(/[\\/]/).pop()}</span>.
              {ed.pendingImport.errors.length > 0 && ` ${ed.pendingImport.errors.length} rows will be skipped. The reasons are in the log.`}
            </p>
            {ed.pendingImport.notes.map((note) => (
              <p key={note}>{note}</p>
            ))}
            <p>
              Replace existing swaps the {ed.tags.length} tags on {device.label} for these. Add to existing keeps them and skips tags with the same name and address.
            </p>
          </div>
        </Dialog>
      )}
    </>
  );
}
