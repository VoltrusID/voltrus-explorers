import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Badge, Button, Callout, IconButton, StatusDot } from "@mactools/ui";
import { Database, Eye, EyeOff, ListPlus, Tags, X } from "lucide-react";
import { s7, type WatchResult } from "../../lib/s7";
import { createWatchTable, listDeviceDocs, readDoc, updateDoc, openDoc } from "../../lib/deviceProject";
import {
  appendToWatchDoc,
  docTitle,
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
    </div>
  );
}
