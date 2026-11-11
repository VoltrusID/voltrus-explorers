import { Badge, Button, EmptyState, Segmented } from "@mactools/ui";
import { Activity, ChevronDown, ChevronRight, TrendingUp } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { ascii, fmtNum, idText, parseFilter, periodText, sortMonitor, type MonitorRow, type MonitorSort } from "./can";
import { Bytes, hint, row, th } from "./ui";

interface Props {
  rows: MonitorRow[];
  filter: string;
  onClear: () => void;
  /** Signal key `MSG.Signal` currently trended. */
  trending: Set<string>;
  onToggleTrend: (key: string, label: string) => void;
}

export default function MonitorView({ rows, filter, onClear, trending, onToggleTrend }: Props) {
  const [sort, setSort] = useState<MonitorSort>("id");
  const [open, setOpen] = useState<Set<string>>(new Set());
  const [showAscii, setShowAscii] = useState(false);
  const [now, setNow] = useState(() => Date.now() * 1000);
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now() * 1000), 250);
    return () => clearInterval(t);
  }, []);

  const shown = useMemo(() => {
    const keep = parseFilter(filter);
    return sortMonitor(rows.filter(keep), sort);
  }, [rows, filter, sort]);

  const toggle = (k: string) => setOpen((prev) => { const n = new Set(prev); if (n.has(k)) n.delete(k); else n.add(k); return n; });

  if (rows.length === 0) {
    return (
      <EmptyState
        icon={<Activity size={20} strokeWidth={1.75} />}
        title="No frames yet"
        description="Every identifier on the bus gets one row here, with its count, period and the bytes that change. Frames you send show as TX rows."
      />
    );
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", flex: 1, minHeight: 0, gap: 8 }}>
      <div style={row}>
        <Segmented
          label="Sort"
          value={sort}
          onChange={setSort}
          options={[{ value: "id", label: "ID" }, { value: "count", label: "Count" }, { value: "rate", label: "Rate" }, { value: "recent", label: "Recent" }]}
        />
        <label style={{ display: "flex", gap: 6, alignItems: "center", fontSize: "var(--fs-sm)" }}>
          <input type="checkbox" checked={showAscii} onChange={(e) => setShowAscii(e.target.checked)} /> ASCII
        </label>
        <Badge>{shown.length} of {rows.length} IDs</Badge>
        <span style={{ flex: 1 }} />
        <Button size="sm" onClick={onClear}>Clear</Button>
      </div>
      <div style={{ flex: 1, minHeight: 0, overflow: "auto", background: "var(--card)", border: "1px solid var(--border)", borderRadius: "var(--radius-lg)" }}>
        <table className="vt-table" style={{ width: "100%" }}>
          <thead>
            <tr>
              <th style={{ ...th, width: 24 }} />
              <th style={{ ...th, width: 92 }}>ID</th>
              <th style={{ ...th, width: 64 }} />
              <th style={{ ...th, width: 36 }}>DLC</th>
              <th style={th}>Data</th>
              <th style={{ ...th, width: 80, textAlign: "right" }}>Count</th>
              <th style={{ ...th, width: 84, textAlign: "right" }}>Period</th>
              <th style={{ ...th, width: "32%" }}>Message</th>
            </tr>
          </thead>
          <tbody>
            {shown.map((r) => {
              const expanded = open.has(r.key);
              const stale = now - r.lastTs > Math.max(3e6, (r.periodMs ?? 0) * 3000);
              const hasSigs = (r.sigs?.length ?? 0) > 0;
              return [
                <tr key={r.key} onClick={() => hasSigs && toggle(r.key)} style={{ cursor: hasSigs ? "pointer" : undefined, opacity: stale ? 0.55 : 1 }}>
                  <td>{hasSigs ? (expanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />) : null}</td>
                  <td className="mono">{idText(r.id, r.ext)}</td>
                  <td>
                    {r.dir === "tx" ? <Badge tone="accent">TX</Badge> : null}
                    {r.fd ? <Badge tone="info" title={r.brs ? "CAN FD, bit rate switch: the data phase ran at the data rate" : "CAN FD, no bit rate switch"}>{r.brs ? "FD·BRS" : "FD"}</Badge> : null}
                    {r.esi ? <Badge tone="warn" title="Error state indicator: the sender was error passive">ESI</Badge> : null}
                    {r.rtr ? <Badge tone="warn">RTR</Badge> : null}
                  </td>
                  <td className="mono">{r.dlc}</td>
                  <td>
                    <Bytes data={r.data} changedAt={r.changedAt} now={now} />
                    {showAscii && <span className="mono" style={{ color: "var(--text-muted)", marginLeft: 8 }}>{ascii(r.data)}</span>}
                  </td>
                  <td className="mono" style={{ textAlign: "right" }}>{r.count}</td>
                  <td className="mono" style={{ textAlign: "right" }}>{periodText(r.periodMs)}</td>
                  <td style={{ fontSize: "var(--fs-xs)" }}>
                    {r.msg && <strong style={{ marginRight: 6 }}>{r.msg}</strong>}
                    <span style={{ color: "var(--text-dim)" }}>{r.label}</span>
                  </td>
                </tr>,
                expanded && hasSigs ? (
                  <tr key={r.key + ":sigs"}>
                    <td />
                    <td colSpan={7} style={{ paddingTop: 0 }}>
                      <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fill, minmax(230px, 1fr))", gap: "2px 16px" }}>
                        {r.sigs!.map(([name, value, unit, label]) => {
                          const k = `${r.msg}.${name}`;
                          const on = trending.has(k);
                          return (
                            <div key={name} style={{ display: "flex", gap: 6, alignItems: "center", fontSize: "var(--fs-xs)" }}>
                              <button
                                type="button"
                                className="vt-btn vt-btn--sm vt-btn--ghost"
                                aria-pressed={on}
                                title={on ? "Remove from the trend" : "Add to the trend"}
                                onClick={(e) => { e.stopPropagation(); onToggleTrend(k, `${r.msg} ${name}`); }}
                                style={{ padding: 2, color: on ? "var(--accent-text)" : "var(--text-muted)" }}
                              >
                                <TrendingUp size={13} />
                              </button>
                              <span style={{ color: "var(--text-dim)" }}>{name}</span>
                              <span style={{ flex: 1 }} />
                              <span className="mono">{label ?? fmtNum(value)}{unit ? ` ${unit}` : ""}</span>
                            </div>
                          );
                        })}
                      </div>
                    </td>
                  </tr>
                ) : null,
              ];
            })}
          </tbody>
        </table>
      </div>
      <span style={hint}>Bytes that changed in the last second are highlighted. Rows fade when an ID goes quiet. Click a decoded row for its signals.</span>
    </div>
  );
}
