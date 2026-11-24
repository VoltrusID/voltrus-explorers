import { useRef, useState } from "react";
import { Button, Callout, Dialog } from "@mactools/ui";
import { FileInput } from "lucide-react";
import { invoke } from "../api/bridge";
import { useStore } from "../store";
import { ACCEPTED_EXTENSIONS, dbRows, planImport, type ImportPlan } from "../lib/plcImport";
import { appendWatchRows } from "../lib/s7";

/** Write a generated file into the project the way the New File flow does. */
async function writeProjectFile(name: string, content: string) {
  const store = useStore.getState();
  const path = store.projectDir ? `${store.projectDir}/${name}` : name;
  if (store.projectDir) {
    await invoke("save_file", { path, content });
    await store.openFile(path);
    return;
  }
  const { openFiles } = useStore.getState();
  const existing = openFiles.findIndex((f) => f.path === path);
  if (existing >= 0) {
    const next = [...openFiles];
    next[existing] = { ...next[existing], content, modified: true };
    useStore.setState({ openFiles: next, activeFileIndex: existing });
  } else {
    useStore.setState({
      openFiles: [...openFiles, { name, path, content, modified: false }],
      activeFileIndex: openFiles.length,
    });
  }
}

/**
 * Import from an engineering-software export: XML or spreadsheet tag tables, DB/UDT sources and
 * SCL sources become ST files; tags with absolute addresses can go to the Online watch table.
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

  const accept = async () => {
    if (!plan) return;
    setBusy(true);
    try {
      for (const f of plan.files) await writeProjectFile(f.name, f.content);
      const added = toWatch ? appendWatchRows(watchRows) : 0;
      const store = useStore.getState();
      store.addLog(
        "system",
        `Imported ${fileName}: ${plan.files.map((f) => f.name).join(", ")}` +
          (added ? `; ${added} tag(s) added to the Online watch table` : ""),
      );
      for (const w of plan.warnings) store.addLog("system", `${fileName}: check: ${w}`);
      if (added) store.setRightPanelTab("online");
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
                Add {watchRows.length} tag(s) to the Online watch table
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
