import { Button, Callout, Dialog, Field, SheetSelect, saveTextFile } from "@mactools/ui";
import { exportTags, formatsFor, type ExportResult, type FormatId } from "@mactools/export";
import { useMemo, useState } from "react";
import { buildTagList, type ExportConnection } from "../scadaExport";
import type { ReadBlock } from "../types";

interface Props {
  open: boolean;
  onClose: () => void;
  blocks: ReadBlock[];
  connection: ExportConnection;
  /** Starting device name: the connection profile, or the host. */
  defaultName: string;
  onLog: (direction: "info" | "error", message: string) => void;
}

const FORMATS = formatsFor("modbus");
const OPTIONS = FORMATS.map((f) => ({ value: f.id, label: f.label, hint: f.hint }));
const PREVIEW_LINES = 14;

/** Pick a SCADA format, see what it will say about this tag list, save it. Pro is checked before this opens. */
export function ExportDialog({ open, onClose, blocks, connection, defaultName, onLog }: Props) {
  const [format, setFormat] = useState<FormatId>("ignition");
  const [name, setName] = useState<string | null>(null);
  const deviceName = (name ?? defaultName).trim() || "Modbus device";

  const { out, error } = useMemo((): { out: ExportResult | null; error: string | null } => {
    if (!open) return { out: null, error: null };
    try {
      return { out: exportTags(format, buildTagList(blocks, deviceName, connection)), error: null };
    } catch (e) {
      return { out: null, error: String(e) };
    }
  }, [open, format, blocks, deviceName, connection]);

  const save = async () => {
    if (!out) return;
    try {
      const filter = { name: FORMATS.find((f) => f.id === format)!.label, extensions: [out.fileName.split(".").pop() ?? "txt"] };
      const path = await saveTextFile(out.fileName, out.content, filter);
      if (path) {
        onLog("info", `Exported ${blocks.reduce((n, b) => n + b.quantity, 0)} tags as ${filter.name} to ${path}`);
        onClose();
      }
    } catch (e) {
      onLog("error", `Export failed: ${String(e)}`);
    }
  };

  const preview = out ? out.content.split("\n").slice(0, PREVIEW_LINES).join("\n") : "";
  const total = out ? out.content.split("\n").length - 1 : 0;

  return (
    <Dialog
      open={open}
      onClose={onClose}
      title="Export tags to SCADA"
      width={560}
      footer={(
        <>
          <Button variant="ghost" onClick={onClose}>Cancel</Button>
          <Button variant="primary" onClick={save} disabled={!out || blocks.length === 0}>Save file</Button>
        </>
      )}
    >
      <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
        <Field label="Format">
          <SheetSelect value={format} options={OPTIONS} onChange={setFormat} sheetTitle="Export format" />
        </Field>
        <Field label="Device name">
          <input className="vt-input" value={name ?? defaultName} onChange={(e) => setName(e.target.value)} />
        </Field>
        {blocks.length === 0 && <Callout tone="warn">There are no read blocks to export. Add a block first.</Callout>}
        {error && <Callout tone="danger">{error}</Callout>}
        {out && out.warnings.length > 0 && (
          <Callout tone="warn">
            <ul style={{ margin: 0, paddingLeft: 18 }}>
              {out.warnings.map((w, i) => <li key={i}>{w}</li>)}
            </ul>
          </Callout>
        )}
        {out && (
          <div>
            <div style={{ fontSize: "var(--fs-xs)", color: "var(--text-dim)", marginBottom: 4 }}>
              {out.fileName}{total > PREVIEW_LINES ? ` (first ${PREVIEW_LINES} of ${total} lines)` : ""}
            </div>
            <pre style={{ margin: 0, fontFamily: "var(--font-mono)", padding: 8, maxHeight: 220, overflow: "auto", fontSize: "var(--fs-xs)", background: "var(--bg)", border: "1px solid var(--border)", borderRadius: "var(--radius)" }}>{preview}</pre>
          </div>
        )}
      </div>
    </Dialog>
  );
}
