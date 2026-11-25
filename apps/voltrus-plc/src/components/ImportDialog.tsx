import { useRef, useState } from "react";
import { Button, Callout, Dialog } from "@mactools/ui";
import { FileInput } from "lucide-react";
import { useStore } from "../store";
import { ACCEPTED_EXTENSIONS, dbRows, planImport, type ImportPlan } from "../lib/plcImport";
import { createWatchTable, openDoc, tagDocName, writeProjectFile } from "../lib/deviceProject";
import { serializeTagDoc, tagDocFromDb, tagDocFromTable, TAGS_EXT } from "../lib/s7Docs";
import { toIdentifier } from "../lib/plcImport/common";

/**
 * Import from an engineering-software export: XML or spreadsheet tag tables, DB/UDT sources and
 * SCL sources become ST files. Tag tables and DBs also become tag-table documents of the device,
 * and tags with absolute addresses can go to a new watch table.
 */
export default function ImportDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const inputRef = useRef<HTMLInputElement>(null);
  const [fileName, setFileName] = useState("");
  const [plan, setPlan] = useState<ImportPlan | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [dbNumbers, setDbNumbers] = useState<Record<string, string>>({});
  const [toWatch, setToWatch] = useState(true);
  const [busy, setBusy] = useState(false);

  const reset = () => {
    setPlan(null);
    setError(null);
    setFileName("");
    setDbNumbers({});
    if (inputRef.current) inputRef.current.value = "";
  };

  const close = () => {
    reset();
    onClose();
  };

  const pick = async (file: File | undefined) => {
    if (!file) return;
    reset();
    setFileName(file.name);
    try {
      const p = await planImport(file.name, new Uint8Array(await file.arrayBuffer()));
      setPlan(p);
      setDbNumbers(Object.fromEntries(p.dbs.map((d) => [d.name, d.number !== undefined ? String(d.number) : ""])));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const numbers = Object.fromEntries(
    Object.entries(dbNumbers).filter(([, v]) => v.trim()).map(([k, v]) => [k, Number(v)]),
  );
  const watchRows = plan ? [...plan.watchRows, ...dbRows(plan.dbs, numbers)] : [];
  const tagDocs = plan
    ? [
        ...plan.tagTables.map((t) => ({ base: toIdentifier(t.name) || "PLC_tags", doc: tagDocFromTable(t) })),
        ...plan.dbs.map((d) => ({ base: toIdentifier(d.name) || "DB", doc: tagDocFromDb(d, numbers[d.name]) })),
      ]
    : [];

  const accept = async () => {
    if (!plan) return;
    setBusy(true);
    try {
      const created: string[] = [];
      let first: string | null = null;
      for (const f of plan.files) {
        await writeProjectFile(f.name, f.content, false);
        created.push(f.name);
      }
      for (const t of tagDocs) {
        const name = tagDocName(t.base);
        const path = await writeProjectFile(name, serializeTagDoc(t.doc), false);
        first ??= path;
        created.push(name);
      }
      const base = toIdentifier(fileName.replace(/\.[^.]+$/, "")) || "Imported";
      if (toWatch && watchRows.length > 0) {
        const path = await createWatchTable(watchRows.map((r) => ({ ...r, modify: "" })), `${base}_watch`);
        created.push(path.split("/").pop() ?? path);
      } else {
        const store = useStore.getState();
        const last = plan.files[plan.files.length - 1];
        const target = first ?? (last ? (store.projectDir ? `${store.projectDir}/${last.name}` : last.name) : null);
        if (target) await openDoc(target).catch(() => {});
      }
      const store = useStore.getState();
      store.addLog("system", `Imported ${fileName}: ${created.join(", ")}`);
      for (const w of plan.warnings) store.addLog("system", `${fileName}: check: ${w}`);
      close();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      open={open}
      onClose={close}
      title="Import from engineering export"
      width={560}
      footer={
        <>
          <Button size="sm" variant="ghost" onClick={close}>Cancel</Button>
          <Button size="sm" variant="primary" disabled={!plan || busy} loading={busy} onClick={accept}>
            Import
          </Button>
        </>
      }
    >
      <div className="import-dialog">
        <p className="s7-muted">
          PLC tag tables (XML or .xlsx export), data block and data type sources (.db, .udt) and SCL
          sources (.scl). Generate sources from blocks in your engineering software, then pick the file here.
        </p>
        <div className="s7-row">
          <input
            ref={inputRef}
            type="file"
            accept={ACCEPTED_EXTENSIONS}
            hidden
            onChange={(e) => void pick(e.target.files?.[0])}
          />
          <Button size="sm" icon={<FileInput size={14} />} onClick={() => inputRef.current?.click()}>
            Choose file…
          </Button>
          {fileName && <span className="mono s7-muted">{fileName}</span>}
        </div>

        {error && <Callout tone="danger">{error}</Callout>}

        {plan && (
          <>
            <Callout tone="info">{plan.summary}</Callout>
            <div>
              <div className="import-dialog__label">Files to create</div>
              <ul className="import-dialog__list mono">
                {plan.files.map((f) => <li key={f.name}>{f.name}</li>)}
                {tagDocs.map((t) => <li key={`tags-${t.base}`}>{t.base}{TAGS_EXT} <span className="s7-muted">(device tag table)</span></li>)}
              </ul>
            </div>

            {plan.dbs.some((d) => !d.optimized) && (
              <div>
                <div className="import-dialog__label">DB numbers (for watch-table addresses)</div>
                {plan.dbs.filter((d) => !d.optimized).map((d) => (
                  <label key={d.name} className="import-dialog__db">
                    <span>{d.name} <span className="s7-muted">({d.layout.size} bytes)</span></span>
                    <span className="s7-row">
                      DB
                      <input
                        className="vt-input vt-input--mono vt-input--sm"
                        style={{ width: 72 }}
                        inputMode="numeric"
                        value={dbNumbers[d.name] ?? ""}
                        placeholder="no."
                        onChange={(e) => setDbNumbers({ ...dbNumbers, [d.name]: e.target.value.replace(/\D/g, "") })}
                      />
                    </span>
                  </label>
                ))}
              </div>
            )}

            {watchRows.length > 0 && (
              <label className="s7-check">
                <input type="checkbox" checked={toWatch} onChange={(e) => setToWatch(e.target.checked)} />
                Create a watch table with {watchRows.length} tag(s)
              </label>
            )}

            {plan.warnings.length > 0 && (
              <div>
                <div className="import-dialog__label">Check these ({plan.warnings.length})</div>
                <ul className="import-dialog__list import-dialog__warnings">
                  {plan.warnings.map((w, i) => <li key={i}>{w}</li>)}
                </ul>
              </div>
            )}
          </>
        )}
      </div>
    </Dialog>
  );
}
