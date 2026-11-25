import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Badge, Button, Callout, IconButton, StatusDot } from "@mactools/ui";
import { Database, Eye, EyeOff, FileCode, ListPlus, Tags, X } from "lucide-react";
import { s7, type WatchResult } from "../../lib/s7";
import { createWatchTable, listDeviceDocs, readDoc, updateDoc, openDoc, writeProjectFile } from "../../lib/deviceProject";
import {
  appendToWatchDoc,
  dbToSt,
  docTitle,
  relayoutDb,
  type DeclRow,
  offsetText,
  parseTagDoc,
  parseWatchDoc,
  serializeTagDoc,
  serializeWatchDoc,
  tagWatchType,
  watchRowsForTags,
  withDbNumber,
  type TagDoc,
  type TagDocRow,
} from "../../lib/s7Docs";
import { errorText, useS7 } from "../../lib/s7Store";
import { useStore } from "../../store";
import { ModeBadge } from "./common";
import { valueClass } from "./WatchTableEditor";

const POLL_MS = 1000;
const NEW_TABLE = "__new__";

/**
 * A tag table (`.s7tags`): PLC tags with absolute addresses, or the members of a standard-access
 * DB with their offsets. Tags can be monitored here and sent to a watch table.
 */
export default function TagTableEditor({ path, value, onChange }: { path: string; value: string; onChange: (v: string) => void }) {
  const parsed = useMemo(() => {
    try {
      return { doc: parseTagDoc(value), error: null };
    } catch (e) {
      return { doc: null, error: errorText(e) };
    }
  }, [value]);
  if (!parsed.doc) {
    return <div className="s7-doc"><Callout tone="danger">This tag table cannot be read: {parsed.error}</Callout></div>;
  }
  return <TagTable path={path} doc={parsed.doc} onChange={(d) => onChange(serializeTagDoc(d))} />;
}

function TagTable({ path, doc, onChange }: { path: string; doc: TagDoc; onChange: (d: TagDoc) => void }) {
  const { info, mode, monitoring: monitoringMap, setMonitoring } = useS7();
  const addLog = useStore((s) => s.addLog);
  const online = info !== null;
  const monitoring = online && !!monitoringMap[path];
  const isDb = doc.kind === "db" && !!doc.db;
  const editable = !isDb;
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [results, setResults] = useState<WatchResult[]>([]);
  const [watchTarget, setWatchTarget] = useState<string>(NEW_TABLE);
  const [view, setView] = useState<"members" | "declaration">("members");
  const watchTables = listDeviceDocs("watch");
  const rowsRef = useRef(doc.rows);
  rowsRef.current = doc.rows;
  const reading = useRef(false);

  const rows = doc.rows;
  const patch = (i: number, p: Partial<TagDocRow>) => onChange({ ...doc, rows: rows.map((r, j) => (j === i ? { ...r, ...p } : r)) });

  const tick = useCallback(async () => {
    if (reading.current) return;
    const list = rowsRef.current.map((r) => ({ r, t: tagWatchType(r) })).filter((x) => x.r.address && x.t);
    if (list.length === 0) return;
    reading.current = true;
    try {
      const res = await s7.readWatch(list.map((x) => ({ address: x.r.address, dataType: x.t })));
      let k = 0;
      setResults(rowsRef.current.map((r) => (r.address && tagWatchType(r) ? res[k++] : { value: null, error: null })));
    } catch (e) {
      setResults(rowsRef.current.map(() => ({ value: null, error: errorText(e) })));
    } finally {
      reading.current = false;
    }
  }, []);

  useEffect(() => {
    if (!monitoring) return;
    void tick();
    const id = window.setInterval(tick, POLL_MS);
    return () => window.clearInterval(id);
  }, [monitoring, tick]);

  const chosen = selected.size > 0 ? rows.filter((_, i) => selected.has(i)) : rows;
  const toWatch = watchRowsForTags(doc, chosen);

  const sendToWatch = async () => {
    if (toWatch.length === 0) return;
    try {
      if (watchTarget === NEW_TABLE) {
        await createWatchTable(toWatch);
        addLog("system", `Created a watch table with ${toWatch.length} tag(s) from ${docTitle(path)}`);
        return;
      }
      const current = parseWatchDoc(await readDoc(watchTarget));
      const { doc: next, added } = appendToWatchDoc(current, toWatch);
      await updateDoc(watchTarget, serializeWatchDoc(next));
      addLog("system", `Added ${added} tag(s) to ${docTitle(watchTarget)}`);
      await openDoc(watchTarget);
    } catch (e) {
      addLog("system", `Could not add tags to the watch table: ${errorText(e)}`);
    }
  };

  const toggle = (i: number) => {
    const next = new Set(selected);
    if (next.has(i)) next.delete(i); else next.add(i);
    setSelected(next);
  };
  const allSelected = rows.length > 0 && selected.size === rows.length;

  return (
    <div className="s7-doc">
      <header className="s7-doc__header">
        {isDb ? <Database size={16} strokeWidth={1.75} /> : <Tags size={16} strokeWidth={1.75} />}
        <span className="s7-doc__title">{docTitle(path)}</span>
        <span className="s7-muted">
          {isDb ? `Data block · ${doc.db!.size} bytes` : "PLC tags"} · {rows.length} {rows.length === 1 ? "entry" : "entries"}
        </span>
        {isDb && (doc.db!.optimized ? <Badge tone="warn">Optimized access</Badge> : <Badge tone="neutral">Standard access</Badge>)}
        <span className="s7-doc__spacer" />
        {online ? (
          <>
            <StatusDot tone="success" label={`Online: ${info.target.host}`} />
            <ModeBadge mode={mode} />
          </>
        ) : (
          <StatusDot tone="neutral" label="Offline" />
        )}
      </header>
      {isDb && doc.decl && (
        <div className="vt-tabs s7-doc__views">
          <button className="vt-tab" aria-selected={view === "members"} onClick={() => setView("members")}>Members and offsets</button>
          <button className="vt-tab" aria-selected={view === "declaration"} onClick={() => setView("declaration")}>Declaration</button>
        </div>
      )}
      {isDb && doc.decl && view === "declaration" ? (
        <DbDeclaration path={path} doc={doc} onChange={onChange} />
      ) : (<>
      <div className="s7-doc__toolbar">
        {isDb && !doc.db!.optimized && (
          <label className="s7-row s7-dbno">
            DB number
            <input
              className="vt-input vt-input--mono vt-input--sm"
              style={{ width: 72 }}
              inputMode="numeric"
              value={doc.db!.number ?? ""}
              placeholder="no."
              onChange={(e) => {
                const digits = e.target.value.replace(/\D/g, "");
                onChange(withDbNumber(doc, digits ? Number(digits) : undefined));
              }}
            />
          </label>
        )}
        <Button size="sm" variant={monitoring ? "primary" : "default"} disabled={!online}
          icon={monitoring ? <EyeOff size={14} /> : <Eye size={14} />}
          title={online ? undefined : "Go online to monitor"}
          onClick={() => setMonitoring(path, !monitoring)}>
          {monitoring ? "Stop monitoring" : "Monitor"}
        </Button>
        <span className="vt-divider--v" />
        <select className="vt-select vt-select--sm" aria-label="Watch table to add to" value={watchTarget}
          onChange={(e) => setWatchTarget(e.target.value)}>
          <option value={NEW_TABLE}>New watch table</option>
          {watchTables.map((w) => <option key={w.path} value={w.path}>{docTitle(w.name)}</option>)}
        </select>
        <Button size="sm" icon={<ListPlus size={14} />} disabled={toWatch.length === 0} onClick={() => void sendToWatch()}
          title={toWatch.length === 0 ? "No tags with an address to add" : undefined}>
          Add {selected.size > 0 ? "selected" : "all"} to watch table ({toWatch.length})
        </Button>
      </div>
      {isDb && doc.db!.optimized && (
        <Callout tone="warn">
          Members of an optimized DB have no fixed offsets, so they cannot be read over PUT/GET. Switch
          the DB to standard access in your engineering software and import it again.
        </Callout>
      )}
      {isDb && !doc.db!.optimized && doc.db!.number === undefined && (
        <Callout tone="info">Enter the DB number to get absolute addresses for the members.</Callout>
      )}
      <div className="s7-grid-wrap">
        <table className="vt-table s7-grid s7-grid--edit">
          <thead>
            <tr>
              <th className="s7-grid__mark">
                <input type="checkbox" aria-label="Select all" checked={allSelected}
                  onChange={() => setSelected(allSelected ? new Set() : new Set(rows.map((_, i) => i)))} />
              </th>
              <th>Name</th>
              <th>Data type</th>
              {isDb && <th>Offset</th>}
              <th>Address</th>
              {monitoring && <th>Monitor value</th>}
              <th>Comment</th>
              {editable && <th className="s7-grid__mark" />}
            </tr>
          </thead>
          <tbody>
            {rows.map((row, i) => {
              const r = monitoring ? results[i] : undefined;
              return (
                <tr key={i} aria-selected={selected.has(i)}>
                  <td className="s7-grid__mark">
                    <input type="checkbox" aria-label={`Select ${row.name}`} checked={selected.has(i)} onChange={() => toggle(i)} />
                  </td>
                  <td>
                    {editable
                      ? <input className="s7-cell" aria-label={`Name of row ${i + 1}`} value={row.name} onChange={(e) => patch(i, { name: e.target.value })} />
                      : <span className="s7-cell-text" style={{ paddingLeft: `${(row.name.split(".").length - 1) * 12 + 6}px` }}>{row.name}</span>}
                  </td>
                  <td>
                    {editable
                      ? <input className="s7-cell mono" aria-label={`Data type of row ${i + 1}`} value={row.dataType} onChange={(e) => patch(i, { dataType: e.target.value })} />
                      : <span className="s7-cell-text mono">{row.dataType}</span>}
                  </td>
                  {isDb && <td><span className="s7-cell-text mono">{offsetText(row)}</span></td>}
                  <td>
                    {editable
                      ? <input className="s7-cell mono" aria-label={`Address of row ${i + 1}`} value={row.address}
                          onChange={(e) => patch(i, { address: e.target.value.toUpperCase().replace(/^%/, "") })} />
                      : <span className="s7-cell-text mono">{row.address || "–"}</span>}
                  </td>
                  {monitoring && (
                    <td className="mono s7-grid__value" title={r?.error ?? undefined}>
                      {r?.error ? <span className="s7-error">{r.error}</span> : <span className={valueClass(r?.value)}>{r?.value ?? ""}</span>}
                    </td>
                  )}
                  <td>
                    <input className="s7-cell" aria-label={`Comment of row ${i + 1}`} value={row.comment}
                      onChange={(e) => patch(i, { comment: e.target.value })} />
                  </td>
                  {editable && (
                    <td className="s7-grid__mark">
                      <IconButton size="sm" label={`Remove ${row.name || `row ${i + 1}`}`}
                        onClick={() => { setSelected(new Set()); onChange({ ...doc, rows: rows.filter((_, j) => j !== i) }); }}>
                        <X size={12} />
                      </IconButton>
                    </td>
                  )}
                </tr>
              );
            })}
            {editable && (
              <tr className="s7-grid__new">
                <td className="s7-grid__mark" />
                <td colSpan={monitoring ? 6 : 5}>
                  <button className="s7-add-row" onClick={() => onChange({ ...doc, rows: [...rows, { name: `Tag_${rows.length + 1}`, dataType: "Bool", address: "", comment: "" }] })}>
                    &lt;Add new&gt;
                  </button>
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
      </>)}
    </div>
  );
}

/** Edit a DB's declaration; offsets and addresses follow on every change. */
function DbDeclaration({ path, doc, onChange }: { path: string; doc: TagDoc; onChange: (d: TagDoc) => void }) {
  const addLog = useStore((s) => s.addLog);
  const decl = doc.decl ?? [];
  const error = useMemo(() => relayoutDb(doc).error, [doc]);
  const update = (next: DeclRow[], db = doc.db!) => onChange(relayoutDb({ ...doc, db, decl: next }).doc);
  const patch = (i: number, p: Partial<DeclRow>) => update(decl.map((r, j) => (j === i ? { ...r, ...p } : r)));
  const offsetOf = (name: string) => {
    const r = doc.rows.find((x) => x.name === name || x.name.startsWith(`${name}.`) || x.name.startsWith(`${name}[`));
    return r ? offsetText(r) : "";
  };
  const generateSt = async () => {
    try {
      const { fileName, st } = dbToSt(doc);
      await writeProjectFile(fileName, st);
      addLog("system", `Generated ${fileName} from ${docTitle(path)}`);
    } catch (e) {
      addLog("system", `Could not generate ST: ${errorText(e)}`);
    }
  };
  return (
    <>
      <div className="s7-doc__toolbar">
        <label className="s7-check s7-check--inline">
          <input type="checkbox" checked={doc.db!.optimized}
            onChange={(e) => update(decl, { ...doc.db!, optimized: e.target.checked })} />
          Optimized block access
        </label>
        <span className="vt-divider--v" />
        <Button size="sm" icon={<FileCode size={14} />} disabled={!!error && !doc.rows.length} onClick={() => void generateSt()}
          title="Write the DB as ST types and a program declaring it">
          Generate ST
        </Button>
        <span className="s7-muted">{doc.db!.size} bytes{doc.db!.optimized ? " (optimized: no fixed offsets)" : ""}</span>
      </div>
      {error && <Callout tone="danger">{error}</Callout>}
      <div className="s7-grid-wrap">
        <table className="vt-table s7-grid s7-grid--edit">
          <thead>
            <tr>
              <th className="s7-grid__no">#</th><th>Name</th><th>Data type</th><th>Start value</th>
              {!doc.db!.optimized && <th>Offset</th>}<th>Comment</th><th className="s7-grid__mark" />
            </tr>
          </thead>
          <tbody>
            {decl.map((r, i) => (
              <tr key={i}>
                <td className="s7-grid__no">{i + 1}</td>
                <td><input className="s7-cell" aria-label={`Name of member ${i + 1}`} value={r.name} onChange={(e) => patch(i, { name: e.target.value })} /></td>
                <td>
                  <input className="s7-cell mono" aria-label={`Data type of member ${i + 1}`} value={r.type} list="s7-db-types"
                    onChange={(e) => patch(i, { type: e.target.value })} />
                </td>
                <td><input className="s7-cell mono" aria-label={`Start value of member ${i + 1}`} value={r.initial ?? ""}
                  onChange={(e) => patch(i, { initial: e.target.value || undefined })} /></td>
                {!doc.db!.optimized && <td><span className="s7-cell-text mono">{offsetOf(r.name)}</span></td>}
                <td><input className="s7-cell" aria-label={`Comment of member ${i + 1}`} value={r.comment} onChange={(e) => patch(i, { comment: e.target.value })} /></td>
                <td className="s7-grid__mark">
                  <IconButton size="sm" label={`Remove ${r.name}`} onClick={() => update(decl.filter((_, j) => j !== i))}><X size={12} /></IconButton>
                </td>
              </tr>
            ))}
            <tr className="s7-grid__new">
              <td className="s7-grid__no" />
              <td colSpan={doc.db!.optimized ? 5 : 6}>
                <button className="s7-add-row" onClick={() => update([...decl, { name: `Member_${decl.length + 1}`, type: "Int", comment: "" }])}>&lt;Add new&gt;</button>
              </td>
            </tr>
          </tbody>
        </table>
        <datalist id="s7-db-types">
          {["Bool", "Byte", "Word", "DWord", "Int", "DInt", "UInt", "UDInt", "Real", "LReal", "Time", "Char", "String[20]", "Array[0..9] of Int", "Struct a : Int; b : Bool; End_Struct"]
            .map((t) => <option key={t} value={t} />)}
        </datalist>
      </div>
      <p className="s7-muted s7-dv__pad">
        Types as in a DB source: <code>Int</code>, <code>String[20]</code>, <code>Array[1..8] of Real</code>,
        a data type in quotes, or <code>Struct … End_Struct</code>.
      </p>
    </>
  );
}
