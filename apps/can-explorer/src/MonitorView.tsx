import { Badge, Button, EmptyState, Segmented } from "@mactools/ui";
import { Activity, ChevronDown, ChevronRight, TrendingUp } from "lucide-react";
import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import {
  ascii, fmtNum, frameFilter, idText, MON_SIG_H, MONITOR_MAX_IDS, monitorLayout, periodText, sigColumns, sortMonitor, visibleRange,
  type MonitorRow, type MonitorSort,
} from "./can";
import { FrameSearchInputs, useFrameSearch } from "./FrameSearch";
import { Bytes, hint, row } from "./ui";

interface Props {
  rows: MonitorRow[];
  filter: string;
  onClear: () => void;
  /** Signal key `MSG.Signal` currently trended. */
  trending: Set<string>;
  onToggleTrend: (key: string, label: string) => void;
  /** Identifiers dropped to keep the table within `MONITOR_MAX_IDS`. */
  evicted?: number;
}

const COLS = "24px 92px 96px 40px minmax(180px, 1fr) 84px 84px minmax(140px, 30%)";
const NO_TRENDS = new Set<string>();

export default function MonitorView({ rows, filter, onClear, trending, onToggleTrend, evicted = 0 }: Props) {
  const [sort, setSort] = useState<MonitorSort>("id");
  const [open, setOpen] = useState<Set<string>>(new Set());
  const [showAscii, setShowAscii] = useState(false);
  const [now, setNow] = useState(() => Date.now() * 1000);
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now() * 1000), 250);
    return () => clearInterval(t);
  }, []);
  const search = useFrameSearch();

  const query = useMemo(() => frameFilter({ ids: filter, text: search.text, data: search.data }), [filter, search]);
  const shown = useMemo(() => sortMonitor(rows.filter(query.test), sort), [rows, query, sort]);

  // Virtual scrolling: rows have known heights (one line per 16 data bytes, plus the signals of an opened row), so only
  // the ones in view are rendered.
  const scroller = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [view, setView] = useState({ width: 900, height: 400 });
  useLayoutEffect(() => {
    const el = scroller.current;
    if (!el) return;
    const measure = () => setView({ width: el.clientWidth, height: el.clientHeight });
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [rows.length === 0]);
  const layout = useMemo(() => monitorLayout(shown, (k) => open.has(k), view.width), [shown, open, view.width]);
  const [first, last] = visibleRange(layout.tops, layout.heights, scrollTop, view.height);
  const cols = sigColumns(view.width);

  const toggle = useCallback((k: string) => setOpen((prev) => { const n = new Set(prev); if (n.has(k)) n.delete(k); else n.add(k); return n; }), []);
  const trendRef = useRef(onToggleTrend);
  trendRef.current = onToggleTrend;
  const toggleTrend = useCallback((k: string, label: string) => trendRef.current(k, label), []);

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
        <FrameSearchInputs value={search} />
        <Badge>{shown.length} of {rows.length} IDs</Badge>
        {evicted > 0 && (
          <Badge
            tone="warn"
            title={`The monitor keeps the ${MONITOR_MAX_IDS.toLocaleString("en-US")} most recently seen identifiers. Older ones were dropped ${evicted.toLocaleString("en-US")} times to make room; they come back when they are seen again. Clear to start over.`}
          >
            {evicted.toLocaleString("en-US")} IDs not shown
          </Badge>
        )}
        <span style={{ flex: 1 }} />
        <Button size="sm" onClick={onClear}>Clear</Button>
      </div>
      {query.error && <span role="alert" style={{ ...hint, color: "var(--danger-text, var(--danger))" }}>{query.error}</span>}
      <div
        ref={scroller}
        role="table"
        aria-label="Identifiers"
        aria-rowcount={shown.length + 1}
        onScroll={(e) => setScrollTop(e.currentTarget.scrollTop)}
        style={{ flex: 1, minHeight: 0, overflow: "auto", background: "var(--card)", border: "1px solid var(--border)", borderRadius: "var(--radius-lg)" }}
      >
        <div
          role="row"
          style={{
            display: "grid", gridTemplateColumns: COLS, position: "sticky", top: 0, zIndex: 1, height: 30, alignItems: "center", padding: "0 4px",
            background: "var(--card)", borderBottom: "1px solid var(--border)", fontSize: "var(--fs-xs)", color: "var(--text-dim)", fontWeight: 500,
          }}
        >
          <span role="columnheader" />
          <span role="columnheader">ID</span>
          <span role="columnheader" />
          <span role="columnheader">DLC</span>
          <span role="columnheader">Data</span>
          <span role="columnheader" style={{ textAlign: "right" }}>Count</span>
          <span role="columnheader" style={{ textAlign: "right" }}>Period</span>
          <span role="columnheader" style={{ paddingLeft: 12 }}>Message</span>
        </div>
        {shown.length === 0 ? (
          <div style={{ ...hint, padding: 12 }}>No identifier matches the filter.</div>
        ) : (
          <div style={{ height: layout.total, position: "relative" }}>
            {shown.slice(first, last).map((r, i) => {
              const idx = first + i;
              const expanded = open.has(r.key);
              return (
                <MonitorLine
                  key={r.key}
                  r={r}
                  index={idx}
                  top={layout.tops[idx]}
                  height={layout.heights[idx]}
                  expanded={expanded}
                  stale={now - r.lastTs > Math.max(3e6, (r.periodMs ?? 0) * 3000)}
                  hotNow={now - r.lastChange < 1e6 ? now : 0}
                  showAscii={showAscii}
                  trending={expanded ? trending : NO_TRENDS}
                  sigCols={cols}
                  onToggle={toggle}
                  onToggleTrend={toggleTrend}
                />
              );
            })}
          </div>
        )}
      </div>
      <span style={hint}>Bytes that changed in the last second are highlighted. Rows fade when an ID goes quiet. Click a decoded row for its signals.</span>
    </div>
  );
}

interface LineProps {
  r: MonitorRow;
  index: number;
  top: number;
  height: number;
  expanded: boolean;
  stale: boolean;
  /** The current time while a byte is highlighted, else 0 (so a quiet row is not re-rendered by the clock). */
  hotNow: number;
  showAscii: boolean;
  trending: Set<string>;
  sigCols: number;
  onToggle: (key: string) => void;
  onToggleTrend: (key: string, label: string) => void;
}

/** One identifier. Memoized: rows whose frame, place and highlight did not change are skipped. */
const MonitorLine = memo(function MonitorLine({ r, index, top, height, expanded, stale, hotNow, showAscii, trending, sigCols, onToggle, onToggleTrend }: LineProps) {
  const hasSigs = (r.sigs?.length ?? 0) > 0;
  return (
    <div
      role="row"
      aria-rowindex={index + 2}
      aria-expanded={hasSigs ? expanded : undefined}
      className="can-mon-row"
      onClick={() => hasSigs && onToggle(r.key)}
      style={{
        position: "absolute", top, left: 0, right: 0, height, padding: "0 4px", borderBottom: "1px solid var(--border)", fontSize: "var(--fs-sm)",
        cursor: hasSigs ? "pointer" : undefined, opacity: stale ? 0.55 : 1, overflow: "hidden",
      }}
    >
      <div style={{ display: "grid", gridTemplateColumns: COLS, alignItems: "start", minHeight: 30 }}>
        <span role="cell" style={{ height: 30, display: "flex", alignItems: "center" }}>
          {hasSigs ? (
            <button
              type="button"
              aria-label={expanded ? "Hide signals" : "Show signals"}
              onClick={(e) => { e.stopPropagation(); onToggle(r.key); }}
              style={{ all: "unset", cursor: "pointer", display: "flex", padding: 2 }}
            >
              {expanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
            </button>
          ) : null}
        </span>
        <span role="cell" className="mono" style={{ lineHeight: "30px" }}>{idText(r.id, r.ext)}</span>
        <span role="cell" style={{ height: 30, display: "flex", alignItems: "center", gap: 2, overflow: "hidden", whiteSpace: "nowrap" }}>
          {r.dir === "tx" ? <Badge tone="accent">TX</Badge> : null}
          {r.fd ? <Badge tone="info" title={r.brs ? "CAN FD, bit rate switch: the data phase ran at the data rate" : "CAN FD, no bit rate switch"}>{r.brs ? "FD·BRS" : "FD"}</Badge> : null}
          {r.esi ? <Badge tone="warn" title="Error state indicator: the sender was error passive">ESI</Badge> : null}
          {r.rtr ? <Badge tone="warn">RTR</Badge> : null}
        </span>
        <span role="cell" className="mono" style={{ lineHeight: "30px" }}>{r.dlc}</span>
        <span role="cell" style={{ display: "flex", gap: 8, alignItems: "flex-start", paddingTop: 5, minWidth: 0, overflow: "hidden" }}>
          <Bytes data={r.data} changedAt={r.changedAt} now={hotNow} />
          {showAscii && <span className="mono" style={{ color: "var(--text-muted)", whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{ascii(r.data)}</span>}
        </span>
        <span role="cell" className="mono" style={{ textAlign: "right", lineHeight: "30px" }}>{r.count}</span>
        <span role="cell" className="mono" style={{ textAlign: "right", lineHeight: "30px" }}>{periodText(r.periodMs)}</span>
        <span role="cell" style={{ fontSize: "var(--fs-xs)", lineHeight: "30px", paddingLeft: 12, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>
          {r.msg && <strong style={{ marginRight: 6 }}>{r.msg}</strong>}
          <span style={{ color: "var(--text-dim)" }}>{r.label}</span>
        </span>
      </div>
      {expanded && hasSigs && (
        <div style={{ display: "grid", gridTemplateColumns: `repeat(${sigCols}, minmax(0, 1fr))`, gridAutoRows: MON_SIG_H, columnGap: 16, padding: "0 12px 6px 28px" }}>
          {r.sigs!.map(([name, value, unit, label]) => {
            const k = `${r.msg}.${name}`;
            const on = trending.has(k);
            return (
              <div key={name} style={{ display: "flex", gap: 6, alignItems: "center", fontSize: "var(--fs-xs)", minWidth: 0 }}>
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
                <span style={{ color: "var(--text-dim)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{name}</span>
                <span style={{ flex: 1 }} />
                <span className="mono" style={{ whiteSpace: "nowrap" }}>{label ?? fmtNum(value)}{unit ? ` ${unit}` : ""}</span>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
});
