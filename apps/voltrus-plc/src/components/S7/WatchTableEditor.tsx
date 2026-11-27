import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Button, Callout, IconButton, StatusDot } from "@mactools/ui";
import { Eye, EyeOff, Plug, Repeat, Table2, X, Zap } from "lucide-react";
import { bridgeMode } from "../../api/bridge";
import { defaultTypeFor, s7, WATCH_TYPES, type WatchResult } from "../../lib/s7";
import { loadTagDocs } from "../../lib/deviceProject";
import {
  docTitle,
  parseWatchDoc,
  resolveSymbol,
  serializeWatchDoc,
  symbolTable,
  type Symbol,
  type WatchDoc,
  type WatchDocRow,
} from "../../lib/s7Docs";
import { errorText, useS7 } from "../../lib/s7Store";
import { ModeBadge } from "./common";

const WATCH_POLL_MS = 500;

export function useSymbols(): Map<string, Symbol> {
  const [symbols, setSymbols] = useState<Map<string, Symbol>>(new Map());
  useEffect(() => {
    let live = true;
    loadTagDocs().then((docs) => { if (live) setSymbols(symbolTable(docs)); }).catch(() => {});
    return () => { live = false; };
  }, []);
  return symbols;
}

/** True and FALSE get colour; everything else is plain. */
export function valueClass(v: string | null | undefined) {
  if (v === "TRUE") return "s7-val s7-val--true";
  if (v === "FALSE") return "s7-val s7-val--false";
  return "s7-val";
}

/**
 * A watch table (`.s7watch`): monitor values, and modify them once or permanently. Rows take an
 * absolute address or a symbol from the project's tag tables.
 */
export default function WatchTableEditor({ path, value, onChange }: { path: string; value: string; onChange: (v: string) => void }) {
  const parsed = useMemo(() => {
    try {
      return { doc: parseWatchDoc(value), error: null };
    } catch (e) {
      return { doc: null, error: errorText(e) };
    }
  }, [value]);

  if (!parsed.doc) {
    return <div className="s7-doc"><Callout tone="danger">This watch table cannot be read: {parsed.error}</Callout></div>;
  }
  return <WatchTable path={path} doc={parsed.doc} onChange={(d) => onChange(serializeWatchDoc(d))} />;
}

function WatchTable({ path, doc, onChange }: { path: string; doc: WatchDoc; onChange: (d: WatchDoc) => void }) {
  const { info, mode, monitoring: monitoringMap, setMonitoring, goOnline, busy, target } = useS7();
  const online = info !== null;
  const monitoring = online && !!monitoringMap[path];
  const [permanent, setPermanent] = useState(false);
  const [results, setResults] = useState<WatchResult[]>([]);
  const [rowError, setRowError] = useState<Record<number, string>>({});
  const [draft, setDraft] = useState("");
  const symbols = useSymbols();
  const reading = useRef(false);
  const rowsRef = useRef(doc.rows);
  rowsRef.current = doc.rows;
  const permanentRef = useRef(permanent);
  permanentRef.current = permanent;

  // Permanent modify never survives going offline or stopping the monitor.
  useEffect(() => { if (!monitoring) setPermanent(false); }, [monitoring]);

  const rows = doc.rows;
  const setRows = (next: WatchDocRow[]) => onChange({ ...doc, rows: next });
  const patch = (i: number, p: Partial<WatchDocRow>) => setRows(rows.map((r, j) => (j === i ? { ...r, ...p } : r)));

  const writeTicked = useCallback(async (list: WatchDocRow[]) => {
    const errors: Record<number, string> = {};
    for (const [i, r] of list.entries()) {
      if (!r.modifyOn || !r.modify.trim() || !r.address) continue;
      try {
        await s7.writeWatch(r, r.modify);
      } catch (e) {
        errors[i] = errorText(e);
      }
    }
    setRowError(errors);
  }, []);

  const tick = useCallback(async () => {
    const list = rowsRef.current.filter((r) => r.address);
    if (reading.current || list.length === 0) return;
    reading.current = true;
    try {
      if (permanentRef.current) await writeTicked(rowsRef.current);
      const res = await s7.readWatch(list);
      // Map results back onto all rows (rows without an address have none).
      let k = 0;
      setResults(rowsRef.current.map((r) => (r.address ? res[k++] : { value: null, error: null })));
    } catch (e) {
      setResults(rowsRef.current.map(() => ({ value: null, error: errorText(e) })));
    } finally {
      reading.current = false;
    }
  }, [writeTicked]);

  useEffect(() => {
    if (!monitoring) return;
    void tick();
    const id = window.setInterval(tick, WATCH_POLL_MS);
    return () => window.clearInterval(id);
  }, [monitoring, tick]);

  const rowFromText = (text: string): WatchDocRow | null => {
    const t = text.trim();
    if (!t) return null;
    const sym = resolveSymbol(t, symbols);
    if (sym) return { name: sym.name, address: sym.address, dataType: sym.dataType, modify: "" };
    const address = t.toUpperCase().replace(/^%/, "");
    return { address, dataType: defaultTypeFor(address), modify: "" };
  };

  const addDraft = () => {
    const r = rowFromText(draft);
    if (!r) return;
    setRows([...rows, r]);
    setDraft("");
  };

  const ticked = rows.filter((r) => r.modifyOn && r.modify.trim()).length;
  const desktop = bridgeMode === "tauri";

  return (
    <div className="s7-doc">
      <header className="s7-doc__header">
        <Table2 size={16} strokeWidth={1.75} />
        <span className="s7-doc__title">{docTitle(path)}</span>
        <span className="s7-muted">Watch table · {rows.length} {rows.length === 1 ? "row" : "rows"}</span>
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
        {online ? (
          <Button size="sm" variant={monitoring ? "primary" : "default"} disabled={rows.length === 0}
            icon={monitoring ? <EyeOff size={14} /> : <Eye size={14} />}
            onClick={() => setMonitoring(path, !monitoring)}>
            {monitoring ? "Stop monitoring" : "Monitor all"}
          </Button>
        ) : (
          <Button size="sm" variant="primary" icon={<Plug size={14} />} loading={busy}
            disabled={!desktop || !target.host.trim()} onClick={() => void goOnline()}
            title={target.host ? `Go online with ${target.host}` : "Set the CPU address in Online & diagnostics first"}>
            Go online
          </Button>
        )}
        <span className="vt-divider--v" />
        <Button size="sm" icon={<Zap size={14} />} disabled={!online || ticked === 0}
          title="Write the ticked modify values once"
          onClick={async () => { await writeTicked(rows); void tick(); }}>
          Modify now{ticked ? ` (${ticked})` : ""}
        </Button>
        <Button size="sm" variant={permanent ? "danger" : "ghost"} icon={<Repeat size={14} />}
          disabled={!monitoring || ticked === 0}
          title="Write the ticked values on every monitor cycle"
          onClick={() => setPermanent(!permanent)}>
          {permanent ? "Stop permanent modify" : "Modify permanently"}
        </Button>
      </div>
      {permanent && (
        <Callout tone="warn">
          Writing {ticked} value(s) every {WATCH_POLL_MS} ms. The program can still overwrite them
          between writes: this is not a force.
        </Callout>
      )}
      <div className="s7-grid-wrap">
        <table className="vt-table s7-grid s7-grid--edit">
          <thead>
            <tr>
              <th className="s7-grid__no">i</th>
              <th>Name</th>
              <th>Address</th>
              <th>Display format</th>
              <th>Monitor value</th>
              <th>Modify value</th>
              <th className="s7-grid__mark" title="Modify this row">
                <Zap size={12} aria-label="Modify" />
              </th>
              <th>Comment</th>
              <th className="s7-grid__mark" />
            </tr>
          </thead>
          <tbody>
            {rows.map((row, i) => {
              const r = monitoring ? results[i] : undefined;
              const err = rowError[i] ?? r?.error ?? null;
              return (
                <tr key={i}>
                  <td className="s7-grid__no">{i + 1}</td>
                  <td>
                    <input className="s7-cell" aria-label={`Name of row ${i + 1}`} value={row.name ?? ""}
                      placeholder="—"
                      onChange={(e) => patch(i, { name: e.target.value || undefined })}
                      onBlur={(e) => {
                        const sym = resolveSymbol(e.target.value, symbols);
                        if (sym) patch(i, { name: sym.name, address: sym.address, dataType: sym.dataType });
                      }} />
                  </td>
                  <td>
                    <input className="s7-cell mono" aria-label={`Address of row ${i + 1}`} value={row.address}
                      onChange={(e) => {
                        const address = e.target.value.toUpperCase();
                        patch(i, { address, dataType: row.dataType || defaultTypeFor(address) });
                      }} />
                  </td>
                  <td>
                    <select className="s7-cell" aria-label={`Display format of row ${i + 1}`} value={row.dataType}
                      onChange={(e) => patch(i, { dataType: e.target.value })}>
                      {!row.dataType && <option value="">auto</option>}
                      {WATCH_TYPES.map((t) => <option key={t} value={t}>{t}</option>)}
                    </select>
                  </td>
                  <td className="mono s7-grid__value" title={err ?? undefined}>
                    {err ? <span className="s7-error">{err}</span> : <span className={valueClass(r?.value)}>{r?.value ?? ""}</span>}
                  </td>
                  <td>
                    <input className="s7-cell mono" aria-label={`Modify value of row ${i + 1}`} value={row.modify}
                      onChange={(e) => patch(i, { modify: e.target.value, modifyOn: e.target.value.trim() ? true : row.modifyOn })} />
                  </td>
                  <td className="s7-grid__mark">
                    <input type="checkbox" aria-label={`Modify row ${i + 1}`} checked={!!row.modifyOn}
                      onChange={(e) => patch(i, { modifyOn: e.target.checked || undefined })} />
                  </td>
                  <td>
                    <input className="s7-cell" aria-label={`Comment of row ${i + 1}`} value={row.comment ?? ""}
                      onChange={(e) => patch(i, { comment: e.target.value || undefined })} />
                  </td>
                  <td className="s7-grid__mark">
                    <IconButton size="sm" label={`Remove row ${i + 1}`} onClick={() => setRows(rows.filter((_, j) => j !== i))}>
                      <X size={12} />
                    </IconButton>
                  </td>
                </tr>
              );
            })}
            <tr className="s7-grid__new">
              <td className="s7-grid__no">{rows.length + 1}</td>
              <td colSpan={8}>
                <input
                  className="s7-cell mono"
                  aria-label="Add a row"
                  placeholder="<Add new>  tag name or address: Motor_On, MW10, DB1.DBD4"
                  value={draft}
                  list={`${path}-symbols`}
                  onChange={(e) => setDraft(e.target.value)}
                  onKeyDown={(e) => { if (e.key === "Enter") addDraft(); }}
                  onBlur={addDraft}
                />
                <datalist id={`${path}-symbols`}>
                  {[...symbols.values()].slice(0, 500).map((s) => <option key={s.name} value={s.name}>{s.address}</option>)}
                </datalist>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>
  );
}
