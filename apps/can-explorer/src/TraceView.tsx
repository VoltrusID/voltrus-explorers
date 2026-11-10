import { Badge, Button, EmptyState, ProBadge } from "@mactools/ui";
import { Circle, FileDown, FileUp, List, Pause, Play, Square } from "lucide-react";
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { clockUs, fmtNum, hex, idText, parseFilter } from "./can";
import type { FrameView } from "./types";
import { hint, row } from "./ui";

const ROW_H = 24;

interface Props {
  frames: FrameView[];
  filter: string;
  paused: boolean;
  onPause: (p: boolean) => void;
  onClear: () => void;
  onExport: (kind: "candump" | "csv", frames: FrameView[]) => void;
  onImport: () => void;
  recording: string | null;
  onRecord: () => void;
  onStopRecord: () => void;
  /** Shown above the table for an imported trace. */
  source: string | null;
}

export default function TraceView({ frames, filter, paused, onPause, onClear, onExport, onImport, recording, onRecord, onStopRecord, source }: Props) {
  const [relative, setRelative] = useState(true);
  const [selected, setSelected] = useState<number | null>(null);
  const shown = useMemo(() => {
    const keep = parseFilter(filter);
    return frames.filter(keep);
  }, [frames, filter]);

  // Virtual scrolling: only the rows in view are rendered.
  const scroller = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [height, setHeight] = useState(400);
  const follow = useRef(true);
  useLayoutEffect(() => {
    const el = scroller.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setHeight(el.clientHeight));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  useEffect(() => {
    const el = scroller.current;
    if (el && follow.current && !paused) el.scrollTop = el.scrollHeight;
  }, [shown.length, paused]);

  const first = Math.max(0, Math.floor(scrollTop / ROW_H) - 10);
  const last = Math.min(shown.length, Math.ceil((scrollTop + height) / ROW_H) + 10);
  const t0 = shown[0]?.ts ?? 0;
  const current = selected != null ? shown[selected] : null;

  return (
    <div style={{ display: "flex", flexDirection: "column", flex: 1, minHeight: 0, gap: 8 }}>
      <div style={row}>
        <Button size="sm" icon={paused ? <Play size={14} strokeWidth={1.75} /> : <Pause size={14} strokeWidth={1.75} />} onClick={() => onPause(!paused)}>
          {paused ? "Resume" : "Pause"}
        </Button>
        <label style={{ display: "flex", gap: 6, alignItems: "center", fontSize: "var(--fs-sm)" }}>
          <input type="checkbox" checked={relative} onChange={(e) => setRelative(e.target.checked)} /> Relative time
        </label>
        <Badge>{shown.length === frames.length ? `${frames.length} frames` : `${shown.length} of ${frames.length} frames`}</Badge>
        {source && <Badge tone="neutral" title={source}>Imported: {source.split(/[\\/]/).pop()}</Badge>}
        <span style={{ flex: 1 }} />
        {recording ? (
          <Button size="sm" variant="danger" icon={<Square size={14} strokeWidth={1.75} />} onClick={onStopRecord} title={recording}>Stop recording</Button>
        ) : (
          <Button size="sm" icon={<Circle size={14} strokeWidth={1.75} />} onClick={onRecord}>Record to file <ProBadge /></Button>
        )}
        <Button size="sm" icon={<FileUp size={14} strokeWidth={1.75} />} onClick={onImport}>Open log</Button>
        <Button size="sm" icon={<FileDown size={14} strokeWidth={1.75} />} disabled={shown.length === 0} onClick={() => onExport("candump", shown)}>candump <ProBadge /></Button>
        <Button size="sm" disabled={shown.length === 0} onClick={() => onExport("csv", shown)}>CSV <ProBadge /></Button>
        <Button size="sm" onClick={() => { onClear(); setSelected(null); }} disabled={frames.length === 0}>Clear</Button>
      </div>
      {frames.length === 0 ? (
        <EmptyState
          icon={<List size={20} strokeWidth={1.75} />}
          title="The trace is empty"
          description="Every frame, in order, with its time. Open a candump log to look at a recording offline."
        />
      ) : (
        <div style={{ display: "flex", flex: 1, minHeight: 0, gap: 8 }}>
          <div
            ref={scroller}
            onScroll={(e) => {
              const el = e.currentTarget;
              setScrollTop(el.scrollTop);
              follow.current = el.scrollTop + el.clientHeight >= el.scrollHeight - ROW_H * 2;
            }}
            style={{ flex: 1, minWidth: 0, overflow: "auto", background: "var(--card)", border: "1px solid var(--border)", borderRadius: "var(--radius-lg)" }}
          >
            <div className="mono" style={{ display: "grid", gridTemplateColumns: COLS, position: "sticky", top: 0, zIndex: 1, background: "var(--card)", borderBottom: "1px solid var(--border)", fontSize: "var(--fs-xs)", color: "var(--text-dim)", height: ROW_H, alignItems: "center", padding: "0 8px" }}>
              <span>Time</span><span /><span>ID</span><span>DLC</span><span>Data</span><span>Decoded</span>
            </div>
            <div style={{ height: shown.length * ROW_H, position: "relative" }}>
              {shown.slice(first, last).map((f, i) => {
                const idx = first + i;
                return (
                  <div
                    key={idx}
                    className="mono"
                    onClick={() => setSelected(idx)}
                    style={{
                      position: "absolute", top: idx * ROW_H, left: 0, right: 0, height: ROW_H, display: "grid", gridTemplateColumns: COLS,
                      alignItems: "center", padding: "0 8px", fontSize: "var(--fs-xs)", cursor: "pointer", whiteSpace: "nowrap",
                      background: idx === selected ? "var(--accent-soft, color-mix(in srgb, var(--accent) 18%, transparent))" : undefined,
                      borderBottom: "1px solid var(--border)",
                    }}
                  >
                    <span>{relative ? ((f.ts - t0) / 1e6).toFixed(6) : clockUs(f.ts)}</span>
                    <span style={{ color: f.dir === "tx" ? "var(--accent-text)" : "var(--text-muted)" }}>{f.dir.toUpperCase()}{f.fd ? " FD" : ""}</span>
                    <span>{idText(f.id, f.ext)}</span>
                    <span>{f.rtr ? `R${f.dlc}` : f.dlc}</span>
                    <span style={{ overflow: "hidden", textOverflow: "ellipsis" }}>{hex(f.data)}</span>
                    <span style={{ overflow: "hidden", textOverflow: "ellipsis", color: "var(--text-dim)", fontFamily: "var(--font-sans, inherit)" }}>
                      {f.msg ? `${f.msg} ` : ""}{f.label ?? ""}
                    </span>
                  </div>
                );
              })}
            </div>
          </div>
          <div style={{ width: 280, flexShrink: 0, background: "var(--card)", border: "1px solid var(--border)", borderRadius: "var(--radius-lg)", padding: 12, overflow: "auto", fontSize: "var(--fs-xs)" }}>
            {current ? (
              <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
                <strong style={{ fontSize: "var(--fs-sm)" }}>{current.msg ?? idText(current.id, current.ext)}</strong>
                <span style={{ color: "var(--text-dim)" }}>{clockUs(current.ts)} · {current.dir.toUpperCase()} · {current.ext ? "29-bit" : "11-bit"}{current.fd ? ` · CAN FD${current.brs ? " BRS" : ""}` : ""}</span>
                {current.label && <span>{current.label}</span>}
                <span className="mono" style={{ wordBreak: "break-all" }}>{hex(current.data) || "(no data)"}</span>
                {(current.sigs ?? []).map(([n, v, u, l]) => (
                  <div key={n} style={{ display: "flex", gap: 8, borderBottom: "1px solid var(--border)", padding: "2px 0" }}>
                    <span style={{ color: "var(--text-dim)" }}>{n}</span>
                    <span style={{ flex: 1 }} />
                    <span className="mono">{l ?? fmtNum(v)}{u ? ` ${u}` : ""}</span>
                  </div>
                ))}
              </div>
            ) : (
              <span style={hint}>Select a frame to see its signals.</span>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

const COLS = "112px 44px 84px 40px minmax(160px, 1.2fr) minmax(120px, 1fr)";
