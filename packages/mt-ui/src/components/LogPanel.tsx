import { useRef, useEffect, useState, useCallback } from "react";
import { Copy, FileDown, Download, Trash2 } from "lucide-react";
import { Badge, EmptyState, IconButton, Segmented, type Tone } from "../primitives";
import { saveTextFile } from "../files";

export interface LogEntry {
  timestamp: number;
  direction: "tx" | "rx" | "info" | "error";
  message: string;
}

const dirConfig: Record<string, { tone: Tone; label: string }> = {
  tx: { tone: "accent", label: "TX" },
  rx: { tone: "success", label: "RX" },
  info: { tone: "neutral", label: "INFO" },
  error: { tone: "danger", label: "ERR" },
};

type FilterValue = "all" | "tx" | "rx" | "info";
type DirectionFilter = Exclude<FilterValue, "all">;

const filterOptions: { value: FilterValue; label: string }[] = [
  { value: "all", label: "All" },
  { value: "tx", label: "TX" },
  { value: "rx", label: "RX" },
  { value: "info", label: "Info" },
];

function fmtTs(ts: number) {
  return new Date(ts).toLocaleTimeString("en-US", {
    hour12: false,
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

function fmtTsFull(ts: number) {
  return new Date(ts).toLocaleTimeString("en-US", {
    hour12: false,
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    fractionalSecondDigits: 3,
  });
}

interface LogPanelProps {
  entries: LogEntry[];
  maxHeight?: string;
  /** Called with the saved path after a successful CSV export. */
  onExport?: (path: string) => void;
  /** Asked before the CSV save panel opens; return false to cancel (e.g. to show a Pro upsell). */
  beforeExport?: () => boolean;
  onClear?: () => void;
  filterDirection?: string | null;
  /** Default file name prefix of exported CSVs. */
  exportPrefix?: string;
}

const VISIBLE_LIMIT = 500;

export function LogPanel({ entries, maxHeight, onExport, beforeExport, onClear, filterDirection, exportPrefix = "comm-log" }: LogPanelProps) {
  const bottomRef = useRef<HTMLDivElement>(null);
  const [showExport, setShowExport] = useState(false);
  const [copiedIndex, setCopiedIndex] = useState<number | null>(null);
  const [exportMsg, setExportMsg] = useState<{ text: string; failed?: boolean } | null>(null);
  const exportRef = useRef<HTMLDivElement>(null);
  const [directionFilter, setDirectionFilter] = useState<DirectionFilter | null>(() =>
    filterDirection === "tx" || filterDirection === "rx" || filterDirection === "info" ? filterDirection : null,
  );

  const filtered = directionFilter
    ? entries.filter((e) => e.direction === directionFilter)
    : entries;

  // Only render the latest entries to keep DOM small
  const visible = filtered.length > VISIBLE_LIMIT ? filtered.slice(-VISIBLE_LIMIT) : filtered;
  const hiddenCount = filtered.length - visible.length;

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [filtered.length]);

  // Close export dropdown on outside click
  useEffect(() => {
    if (!showExport) return;
    const onClick = (e: MouseEvent) => {
      if (exportRef.current && !exportRef.current.contains(e.target as Node)) {
        setShowExport(false);
      }
    };
    document.addEventListener("mousedown", onClick);
    return () => document.removeEventListener("mousedown", onClick);
  }, [showExport]);

  const copyRow = useCallback((entry: LogEntry, index: number) => {
    const d = dirConfig[entry.direction];
    const text = `${fmtTs(entry.timestamp)} ${d.label} ${entry.message}`;
    navigator.clipboard.writeText(text).then(() => {
      setCopiedIndex(index);
      setTimeout(() => setCopiedIndex(null), 800);
    });
  }, []);

  const copyAll = useCallback(() => {
    const text = filtered.map((e) => {
      const d = dirConfig[e.direction];
      return `${fmtTs(e.timestamp)} ${d.label} ${e.message}`;
    }).join("\n");
    navigator.clipboard.writeText(text);
    setExportMsg({ text: "Copied to clipboard" });
    setTimeout(() => setExportMsg(null), 1500);
    setShowExport(false);
  }, [filtered]);

  const exportCsv = useCallback(async () => {
    setShowExport(false);
    if (beforeExport && !beforeExport()) return;
    const header = "Timestamp,Direction,Message";
    const rows = filtered.map((e) => {
      const d = dirConfig[e.direction];
      return `${fmtTsFull(e.timestamp)},${d.label},"${e.message.replace(/"/g, '""')}"`;
    });
    const csv = [header, ...rows].join("\n");
    const filename = `${exportPrefix}-${new Date().toISOString().slice(0, 19).replace(/:/g, "-")}.csv`;
    try {
      const path = await saveTextFile(filename, csv);
      if (!path) return; // panel cancelled
      setExportMsg({ text: `Saved: ${path}` });
      setTimeout(() => setExportMsg(null), 3000);
      onExport?.(path);
    } catch (e) {
      setExportMsg({ text: `Export failed: ${String(e)}`, failed: true });
      setTimeout(() => setExportMsg(null), 8000);
    }
  }, [filtered, onExport, beforeExport, exportPrefix]);

  return (
    <div style={{
      display: "flex",
      flexDirection: "column",
      maxHeight: maxHeight ?? undefined,
      flex: maxHeight ? undefined : 1,
      minHeight: 0,
      overflow: "hidden",
    }}>
      <div className="vt-toolbar-row" style={{ background: "var(--card)" }}>
        <span style={{ fontSize: "var(--fs-sm)", fontWeight: 600, color: "var(--text)", marginRight: "auto" }}>
          Communication log
        </span>
        <Segmented
          size="sm"
          label="Filter log direction"
          value={directionFilter ?? "all"}
          options={filterOptions}
          onChange={(v) => setDirectionFilter(v === "all" ? null : v)}
        />
        {exportMsg && (
          <span
            role={exportMsg.failed ? "alert" : undefined}
            style={{ fontSize: "var(--fs-xs)", color: exportMsg.failed ? "var(--danger)" : "var(--accent-text)" }}
          >
            {exportMsg.text}
          </span>
        )}
        {entries.length > 0 && onClear && (
          <IconButton size="sm" label="Clear log" onClick={onClear}>
            <Trash2 size={14} strokeWidth={1.75} />
          </IconButton>
        )}
        {entries.length > 0 && (
          <IconButton size="sm" label="Copy all to clipboard" onClick={copyAll}>
            <Copy size={14} strokeWidth={1.75} />
          </IconButton>
        )}
        {entries.length > 0 && (
          <div style={{ position: "relative" }} ref={exportRef}>
            <IconButton
              size="sm"
              label="Export log"
              aria-haspopup="menu"
              aria-expanded={showExport}
              pressed={showExport}
              onClick={() => setShowExport((v) => !v)}
            >
              <Download size={14} strokeWidth={1.75} />
            </IconButton>
            {showExport && (
              <div className="vt-popover" style={{ position: "absolute", right: 0, top: "calc(100% + 4px)" }}>
                <button type="button" className="vt-menu-item" onClick={copyAll}>
                  <Copy size={14} strokeWidth={1.75} />
                  Copy all to clipboard
                </button>
                <button type="button" className="vt-menu-item" onClick={exportCsv}>
                  <FileDown size={14} strokeWidth={1.75} />
                  Export to CSV
                </button>
              </div>
            )}
          </div>
        )}
      </div>

      {/* Log rows */}
      <div
        style={{
          flex: 1,
          overflow: "auto",
          background: "var(--bg)",
          fontFamily: "var(--font-mono)",
          fontSize: "var(--fs-sm)",
          fontVariantNumeric: "tabular-nums",
          display: "flex",
          flexDirection: "column",
        }}
      >
        {filtered.length === 0 ? (
          <EmptyState
            compact
            title={entries.length === 0 ? "No traffic yet" : "No matching entries"}
            description={
              entries.length === 0
                ? "TX, RX and info entries appear here as the app communicates with the device."
                : "No entries match the selected direction filter."
            }
          />
        ) : (
          <>
            {hiddenCount > 0 && (
              <div style={{ padding: "4px 12px", color: "var(--text-muted)", fontSize: "var(--fs-xs)", textAlign: "center", borderBottom: "1px solid var(--border)", flexShrink: 0 }}>
                {hiddenCount} older entries hidden
              </div>
            )}
            {visible.map((entry, i) => {
            const d = dirConfig[entry.direction];
            const realIndex = i;
            return (
              <div
                key={realIndex}
                onClick={() => copyRow(entry, realIndex)}
                title="Click to copy"
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: 8,
                  padding: "3px 12px",
                  borderBottom: "1px solid var(--border)",
                  cursor: "pointer",
                  flexShrink: 0,
                }}
                onMouseEnter={(e) => { (e.currentTarget as HTMLElement).style.background = "var(--hover-overlay)"; }}
                onMouseLeave={(e) => { (e.currentTarget as HTMLElement).style.background = "transparent"; }}
              >
                <span style={{ color: "var(--text-muted)", flexShrink: 0 }}>
                  {fmtTs(entry.timestamp)}
                </span>
                <Badge tone={d.tone} mono style={{ flexShrink: 0 }}>
                  {d.label}
                </Badge>
                <span style={{ color: "var(--text)", flex: 1, minWidth: 0 }}>{entry.message}</span>
                {copiedIndex === realIndex && (
                  <span style={{ fontSize: "var(--fs-xs)", color: "var(--accent-text)", flexShrink: 0 }}>Copied</span>
                )}
              </div>
            );
          })}
          </>
        )}
        <div ref={bottomRef} />
      </div>
    </div>
  );
}
