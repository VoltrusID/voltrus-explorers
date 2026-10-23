import { exportTags, formatsFor, type ExportOptions, type ExportResult, type FormatId, type Protocol, type TagList } from "@mactools/export";
import { useMemo, useState } from "react";
import { saveTextFile } from "./files";
import { Button, Callout, Dialog, Field } from "./primitives";
import { SheetSelect } from "./sheetSelect";

export interface ExportTagsDialogProps {
  open: boolean;
  onClose: () => void;
  protocol: Protocol;
  /** The tag list for the device name the user typed. Called while the dialog is open, so keep it cheap. */
  buildList: (deviceName: string) => TagList;
  /** Starting device name: the connection profile, or the host. */
  defaultName: string;
  /** One-line result for the communication log. */
  onLog: (level: "info" | "error", message: string) => void;
}

const PREVIEW_LINES = 14;

/**
 * Pick a SCADA format, see what it will say about this tag list, save it. The app checks Pro before it opens this, and
 * decides what the tags are; the formats live in `@mactools/export`.
 */
export function ExportTagsDialog({ open, onClose, protocol, buildList, defaultName, onLog }: ExportTagsDialogProps) {
  const formats = useMemo(() => formatsFor(protocol), [protocol]);
  const options = useMemo(() => formats.map((f) => ({ value: f.id, label: f.label, hint: f.hint })), [formats]);
  const [format, setFormat] = useState<FormatId>(formats[0].id);
  const [name, setName] = useState<string | null>(null);
  const deviceName = (name ?? defaultName).trim() || "Device";
  const [chosen, setChosen] = useState<ExportOptions>({});
  const formatOptions = useMemo(() => formats.find((f) => f.id === format)?.options?.filter((o) => o.protocols.includes(protocol)) ?? [], [formats, format, protocol]);
  const active = useMemo(() => Object.fromEntries(formatOptions.map((o) => [o.id, chosen[o.id] === true])) as ExportOptions, [formatOptions, chosen]);

  const { out, error, count } = useMemo((): { out: ExportResult | null; error: string | null; count: number } => {
    if (!open) return { out: null, error: null, count: 0 };
    try {
      const list = buildList(deviceName);
      return { out: exportTags(format, list, active), error: null, count: list.tags.length };
    } catch (e) {
      return { out: null, error: String(e), count: 0 };
    }
  }, [open, format, deviceName, buildList, active]);

  const save = async () => {
    if (!out) return;
    try {
      const label = formats.find((f) => f.id === format)?.label ?? "Export";
      const path = await saveTextFile(out.fileName, out.content, { name: label, extensions: [out.fileName.split(".").pop() ?? "txt"] });
      if (path) {
        onLog("info", `Exported ${count} tags as ${label} to ${path}`);
        onClose();
      }
    } catch (e) {
      onLog("error", `Export failed: ${String(e)}`);
    }
  };

  const lines = out ? out.content.split("\n") : [];
  const total = Math.max(0, lines.length - 1);

  return (
    <Dialog
      open={open}
      onClose={onClose}
      title="Export tags to SCADA"
      width={560}
      footer={(
        <>
          <Button variant="ghost" onClick={onClose}>Cancel</Button>
          <Button variant="primary" onClick={save} disabled={!out || count === 0}>Save file</Button>
        </>
      )}
    >
      <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
        <Field label="Format">
          <SheetSelect value={format} options={options} onChange={setFormat} sheetTitle="Export format" />
        </Field>
        {formatOptions.map((o) => (
          <label key={o.id} style={{ display: "flex", alignItems: "flex-start", gap: 8, fontSize: "var(--fs-sm)" }}>
            <input type="checkbox" checked={active[o.id] === true} onChange={(e) => setChosen({ ...chosen, [o.id]: e.target.checked })} style={{ marginTop: 3 }} />
            <span>
              {o.label}
              <span style={{ display: "block", fontSize: "var(--fs-xs)", color: "var(--text-dim)" }}>{o.hint}</span>
            </span>
          </label>
        ))}
        <Field label="Device name">
          <input className="vt-input" value={name ?? defaultName} onChange={(e) => setName(e.target.value)} />
        </Field>
        {open && count === 0 && !error && <Callout tone="warn">There are no tags to export yet.</Callout>}
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
            <pre style={{ margin: 0, fontFamily: "var(--font-mono)", padding: 8, maxHeight: 220, overflow: "auto", fontSize: "var(--fs-xs)", background: "var(--bg)", border: "1px solid var(--border)", borderRadius: "var(--radius)" }}>
              {lines.slice(0, PREVIEW_LINES).join("\n")}
            </pre>
          </div>
        )}
      </div>
    </Dialog>
  );
}
