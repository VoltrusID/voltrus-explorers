import { Fragment, useState, useEffect, useRef, useCallback } from "react";
import { ArrowDownToLine, ArrowLeft, ArrowRight, Download, Trash2 } from "lucide-react";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger, EmptyState, IconButton, Segmented, saveTextFile, useIsPhone, useProStatus } from "@mactools/ui";
import { NO_TRAFFIC, useTrafficStore, type TcpEvent, type TrafficEntry } from "./trafficStore";
import { useStore } from "./store";

const FC_NAMES: Record<number, string> = {
  1: "Read Coils",
  2: "Read Discrete",
  3: "Read Holding",
  4: "Read Input",
  5: "Write Coil",
  6: "Write Single",
  15: "Write Coils",
  16: "Write Multiple",
};

/** Exception responses carry the request's function code with the high bit set (Modbus spec §7). */
function fcName(code: number): string {
  if (code & 0x80) return `Exception: ${FC_NAMES[code & 0x7f] ?? `FC${code & 0x7f}`}`;
  return FC_NAMES[code] || `FC${code}`;
}

const thStyle: React.CSSProperties = {
  textAlign: "left",
  padding: "4px 8px",
  borderBottom: "1px solid var(--border)",
  color: "var(--text-muted)",
  fontSize: "var(--fs-xs)",
  fontWeight: 600,
  position: "sticky",
  top: 0,
  background: "var(--card)",
  whiteSpace: "nowrap",
};



export default function TrafficPanel() {
  // The frames of the connection whose tab is in front.
  const activeId = useStore((s) => s.activeSessionId);
  const { entries, tcpEvents } = useTrafficStore((s) => s.byConn[activeId] ?? NO_TRAFFIC);
  const clearTraffic = useTrafficStore((s) => s.clear);
  const { requirePro } = useProStatus();
  const phone = useIsPhone();
  const [filterDir, setFilterDir] = useState<string>("all");
  const [filterText, setFilterText] = useState("");
  const [autoScroll, setAutoScroll] = useState(true);
  const [expandedRows, setExpandedRows] = useState<Set<number>>(new Set());
  const [tab, setTab] = useState<"modbus" | "tcp">("modbus");
  const [exportMsg, setExportMsg] = useState<{ text: string; failed?: boolean } | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (autoScroll && scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [entries, tcpEvents, autoScroll, filterDir, filterText, tab]);

  const filtered = entries.filter((e) => {
    if (filterDir !== "all" && e.direction !== filterDir) return false;
    if (filterText) {
      const t = filterText.toLowerCase();
      const fc = fcName(e.function_code);
      return (
        fc.toLowerCase().includes(t) ||
        String(e.address).includes(t) ||
        e.data.toLowerCase().includes(t) ||
        String(e.length).includes(t)
      );
    }
    return true;
  });

  const toggleRow = (index: number) => {
    setExpandedRows((prev) => {
      const next = new Set(prev);
      if (next.has(index)) next.delete(index);
      else next.add(index);
      return next;
    });
  };

  const collapseAll = () => setExpandedRows(new Set());

  const formatTime = useCallback((ts: number) => {
    const d = new Date(ts);
    return `${d.getHours().toString().padStart(2, "0")}:${d.getMinutes().toString().padStart(2, "0")}:${d.getSeconds().toString().padStart(2, "0")}.${d.getMilliseconds().toString().padStart(3, "0")}`;
  }, []);

  const formatTimeFull = useCallback((ts: number) => {
    return new Date(ts).toLocaleTimeString("en-US", {
      hour12: false,
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
      fractionalSecondDigits: 3,
    });
  }, []);

  const formatDelta = useCallback((ts: number, prevTs: number | null) => {
    if (!prevTs) return "";
    const delta = ts - prevTs;
    if (delta < 1) return "<1ms";
    return `${delta}ms`;
  }, []);

  const parseRawData = (data: string): string[] => {
    try {
      const stripped = data.replace(/^\[|]$/g, "").replace(/"/g, "");
      return stripped.split(", ").map((s) => s.replace(/^0x/, ""));
    } catch {
      return [];
    }
  };

  const formatAddr = (addr: number, len: number) => {
    if (len <= 1) return String(addr);
    return `${addr}-${addr + len - 1}`;
  };

  const copyAll = useCallback(() => {
    if (!requirePro("Copying all traffic")) return;
    const data = tab === "modbus" ? filtered : tcpEvents;
    let text: string;
    if (tab === "modbus") {
      text = data.map((e) => {
        const fe = e as TrafficEntry;
        const fc = fcName(fe.function_code);
        return `${formatTime(fe.timestamp)} ${fe.direction} ${fc} addr=${formatAddr(fe.address, fe.length)} len=${fe.length} ${fe.data}`;
      }).join("\n");
    } else {
      text = data.map((e) => {
        const te = e as TcpEvent;
        return `${formatTime(te.timestamp)} ${te.event_type} ${te.detail}`;
      }).join("\n");
    }
    navigator.clipboard.writeText(text);
    setExportMsg({ text: "Copied to clipboard" });
    setTimeout(() => setExportMsg(null), 1500);
  }, [filtered, tcpEvents, tab, formatTime, requirePro]);

  const exportCsv = useCallback(async () => {
    if (!requirePro("Traffic CSV export")) return;
    let csv: string;
    const ts = new Date().toISOString().slice(0, 19).replace(/:/g, "-");
    let filename: string;
    if (tab === "modbus") {
      const header = "Timestamp,Direction,FunctionCode,Function,Address,AddressRange,Length,Data";
      const rows = filtered.map((e) => {
        const fc = fcName(e.function_code);
        return `${formatTimeFull(e.timestamp)},${e.direction},${e.function_code},"${fc}",${e.address},"${formatAddr(e.address, e.length)}",${e.length},"${e.data.replace(/"/g, '""')}"`;
      });
      csv = [header, ...rows].join("\n");
      filename = `modbus-traffic-${ts}.csv`;
    } else {
      const header = "Timestamp,Event,Detail";
      const rows = tcpEvents.map((e) => {
        return `${formatTimeFull(e.timestamp)},"${e.event_type}","${e.detail.replace(/"/g, '""')}"`;
      });
      csv = [header, ...rows].join("\n");
      filename = `modbus-tcp-events-${ts}.csv`;
    }
    try {
      const path = await saveTextFile(filename, csv);
      if (!path) return; // panel cancelled
      setExportMsg({ text: `Saved: ${path}` });
      setTimeout(() => setExportMsg(null), 3000);
    } catch (e) {
      setExportMsg({ text: `Export failed: ${String(e)}`, failed: true });
      setTimeout(() => setExportMsg(null), 8000);
    }
  }, [filtered, tcpEvents, tab, formatTimeFull, requirePro]);

  const detailBody = (e: TrafficEntry, size: number) => (
    <>
      <div style={{ display: "grid", gridTemplateColumns: "80px 1fr", gap: "2px 8px", fontSize: size, fontFamily: '"SF Mono", "JetBrains Mono", monospace' }}>
        <span style={{ color: "var(--text-muted)" }}>Direction:</span>
        <span style={{ color: e.direction === "TX" ? "var(--warn)" : "var(--accent)", fontWeight: 600 }}>{e.direction}</span>
        <span style={{ color: "var(--text-muted)" }}>Function:</span>
        <span style={{ color: "var(--text)" }}>{`${fcName(e.function_code)} (FC${e.function_code.toString(16).toUpperCase().padStart(2, "0")})`}</span>
        <span style={{ color: "var(--text-muted)" }}>Address:</span>
        <span style={{ color: "var(--text)" }}>{formatAddr(e.address, e.length)} (0x{e.address.toString(16).toUpperCase().padStart(4, "0")})</span>
        <span style={{ color: "var(--text-muted)" }}>Registers:</span>
        <span style={{ color: "var(--text)" }}>{e.length} word{e.length > 1 ? "s" : ""}</span>
        <span style={{ color: "var(--text-muted)" }}>Time:</span>
        <span style={{ color: "var(--text)" }}>{new Date(e.timestamp).toISOString()}</span>
      </div>
      {e.data.startsWith("[") ? (
        parseRawData(e.data).length > 0 && (
          <div style={{ marginTop: 4, display: "flex", flexWrap: "wrap", gap: 3, fontSize: size, fontFamily: '"SF Mono", "JetBrains Mono", monospace' }}>
            {parseRawData(e.data).map((hex, i) => (
              <span key={i} style={{ padding: "1px 4px", borderRadius: 2, background: "var(--bg)", border: "1px solid var(--border)", color: "var(--text)" }}>0x{hex.toUpperCase()}</span>
            ))}
          </div>
        )
      ) : (
        e.data && <div style={{ marginTop: 4, fontSize: size, color: e.function_code & 0x80 ? "var(--danger)" : "var(--text)", fontFamily: '"SF Mono", "JetBrains Mono", monospace' }}>{e.data}</div>
      )}
    </>
  );

  const renderDetail = (e: TrafficEntry) => (
    <tr>
      <td colSpan={7} style={{ padding: "6px 12px 8px", background: "var(--card)", borderBottom: "1px solid var(--border)" }}>
        {detailBody(e, 10)}
      </td>
    </tr>
  );

  /** Phone: one row per frame (arrow, function and address, the data under it, time and gap at the right); a tap opens the detail. */
  const renderModbusPhone = () => (
    <div ref={scrollRef} style={{ flex: 1, minHeight: 0, overflow: "auto", borderRadius: "var(--radius-lg)", background: "var(--card)" }}>
      {filtered.length === 0 ? (
        <EmptyState compact title="No frames" description={entries.length === 0 ? "Requests and replies appear here while the app talks to a device." : "No frames match the filter."} />
      ) : (
        filtered.map((e, i) => {
          const isExpanded = expandedRows.has(i);
          const tx = e.direction === "TX";
          const exception = (e.function_code & 0x80) !== 0;
          return (
            <div key={i} style={{ borderTop: i > 0 ? "1px solid var(--hairline)" : undefined }}>
              <div
                role="button"
                tabIndex={0}
                title="Click to expand frame details"
                onClick={() => toggleRow(i)}
                onKeyDown={(ev) => { if (ev.key === "Enter" || ev.key === " ") { ev.preventDefault(); toggleRow(i); } }}
                style={{ display: "flex", alignItems: "center", gap: 12, minHeight: 56, padding: "8px 16px", background: isExpanded ? "var(--fill)" : undefined }}
              >
                <span aria-label={tx ? "Request" : "Reply"} style={{ width: 32, height: 32, borderRadius: "50%", flexShrink: 0, display: "flex", alignItems: "center", justifyContent: "center", background: tx ? "var(--warn-soft)" : "var(--success-soft)", color: tx ? "var(--warn)" : "var(--accent-text)" }}>
                  {tx ? <ArrowRight size={16} strokeWidth={2.25} /> : <ArrowLeft size={16} strokeWidth={2.25} />}
                </span>
                <span style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column", gap: 1 }}>
                  <span style={{ fontSize: "var(--fs-md)", fontWeight: 600, color: exception ? "var(--danger)" : "var(--text)", whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>
                    {fcName(e.function_code)} · {formatAddr(e.address, e.length)}
                  </span>
                  <span className="mono" style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)", whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>
                    {e.data}
                  </span>
                </span>
                <span className="mono" style={{ flexShrink: 0, textAlign: "right", fontSize: "var(--fs-xs)", color: "var(--text-muted)", lineHeight: 1.45 }}>
                  {formatTime(e.timestamp).slice(0, 8)}
                  <br />
                  {formatDelta(e.timestamp, i > 0 ? filtered[i - 1].timestamp : null) || "\u00a0"}
                </span>
              </div>
              {isExpanded && <div style={{ padding: "4px 16px 14px 60px", background: "var(--fill)" }}>{detailBody(e, 12)}</div>}
            </div>
          );
        })
      )}
    </div>
  );

  const renderTcpPhone = () => (
    <div ref={scrollRef} style={{ flex: 1, minHeight: 0, overflow: "auto", borderRadius: "var(--radius-lg)", background: "var(--card)" }}>
      {tcpEvents.length === 0 ? (
        <EmptyState compact title="No TCP events" description="Events appear here once a client connects to the slave server." />
      ) : (
        tcpEvents.map((e, i) => {
          const color = e.event_type.includes("error") || e.event_type.includes("fail") || e.event_type === "disconnect"
            ? "var(--danger)"
            : e.event_type === "connect" ? "var(--accent-text)"
            : e.event_type.includes("timeout") ? "var(--warn)"
            : "var(--text-muted)";
          return (
            <div key={i} style={{ display: "flex", alignItems: "center", gap: 12, minHeight: 56, padding: "8px 16px", borderTop: i > 0 ? "1px solid var(--hairline)" : undefined }}>
              <span style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column", gap: 1 }}>
                <span style={{ fontSize: "var(--fs-md)", fontWeight: 600, color }}>{e.event_type}</span>
                <span style={{ fontSize: "var(--fs-sm)", color: "var(--text-dim)", overflowWrap: "anywhere" }}>{e.detail}</span>
              </span>
              <span className="mono" style={{ flexShrink: 0, fontSize: "var(--fs-xs)", color: "var(--text-muted)" }}>{formatTime(e.timestamp).slice(0, 8)}</span>
            </div>
          );
        })
      )}
    </div>
  );

  const renderModbusTab = () => (
    <div ref={scrollRef} style={{ flex: 1, overflow: "auto", fontFamily: '"SF Mono", "JetBrains Mono", monospace', fontSize: 11 }}>
      <table style={{ width: "100%", borderCollapse: "collapse" }}>
        <thead>
          <tr>
            <th style={thStyle}>Time</th>
            <th style={thStyle}>&Delta;</th>
            <th style={thStyle}>Dir</th>
            <th style={thStyle}>FC</th>
            <th style={thStyle}>Addr</th>
            <th style={thStyle}>Len</th>
            <th style={thStyle}>Data</th>
          </tr>
        </thead>
        <tbody>
          {filtered.map((e, i) => {
            const prevTs = i > 0 ? filtered[i - 1].timestamp : null;
            const isExpanded = expandedRows.has(i);
            return (
              <Fragment key={i}>
                <tr
                  onClick={() => toggleRow(i)}
                  style={{
                    borderBottom: isExpanded ? "none" : "1px solid var(--border)",
                    background: isExpanded ? "var(--hover-overlay)" : e.direction === "TX" ? "transparent" : "var(--success-soft)",
                    cursor: "pointer",
                  }}
                  title="Click to expand frame details"
                >
                  <td style={{ padding: "2px 8px", color: "var(--text-dim)", fontSize: 10, whiteSpace: "nowrap" }}>{formatTime(e.timestamp)}</td>
                  <td style={{ padding: "2px 4px", color: "var(--text-muted)", fontSize: "var(--fs-xs)", whiteSpace: "nowrap" }}>{formatDelta(e.timestamp, prevTs)}</td>
                  <td style={{ padding: "2px 6px", color: e.direction === "TX" ? "var(--warn)" : "var(--accent)", fontWeight: 600, fontSize: 10, width: 16 }}>
                    {e.direction === "TX" ? "\u2192" : "\u2190"}
                  </td>
                  <td style={{ padding: "2px 8px", color: e.function_code & 0x80 ? "var(--danger)" : "var(--text-muted)", fontSize: 10 }}>{fcName(e.function_code)}</td>
                  <td style={{ padding: "2px 8px", color: "var(--text)", fontSize: 10 }}>{formatAddr(e.address, e.length)}</td>
                  <td style={{ padding: "2px 8px", color: "var(--text-dim)", fontSize: 10 }}>{e.length}</td>
                  <td style={{ padding: "2px 8px", color: "var(--text-dim)", fontSize: 10, maxWidth: 300, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{isExpanded ? "" : e.data}</td>
                </tr>
                {isExpanded && renderDetail(e)}
              </Fragment>
            );
          })}
        </tbody>
      </table>
    </div>
  );

  const renderTcpTab = () => (
    <div ref={scrollRef} style={{ flex: 1, overflow: "auto", fontFamily: '"SF Mono", "JetBrains Mono", monospace', fontSize: 11 }}>
      {tcpEvents.length === 0 ? (
        <EmptyState compact title="No TCP events" description="Events appear here once a client connects to the slave server." />
      ) : (
        <table style={{ width: "100%", borderCollapse: "collapse" }}>
          <thead>
            <tr>
              <th style={thStyle}>Time</th>
              <th style={thStyle}>Event</th>
              <th style={thStyle}>Detail</th>
            </tr>
          </thead>
          <tbody>
            {tcpEvents.map((e, i) => {
              const evtColor = e.event_type.includes("error") || e.event_type.includes("fail") || e.event_type === "disconnect"
                ? "var(--danger)"
                : e.event_type === "connect" ? "var(--success)"
                : e.event_type.includes("timeout") ? "var(--warn)"
                : "var(--text-muted)";
              return (
                <tr key={i} style={{ borderBottom: "1px solid var(--border)" }}>
                  <td style={{ padding: "2px 8px", color: "var(--text-dim)", fontSize: 10, whiteSpace: "nowrap" }}>{formatTime(e.timestamp)}</td>
                  <td style={{ padding: "2px 8px", color: evtColor, fontSize: 10, fontWeight: 600 }}>{e.event_type}</td>
                  <td style={{ padding: "2px 8px", color: "var(--text)", fontSize: 10 }}>{e.detail}</td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
    </div>
  );

  if (phone) {
    const exportMenu = (entries.length > 0 || tcpEvents.length > 0) && (
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <IconButton outline label="Export traffic">
            <Download size={18} strokeWidth={1.75} />
          </IconButton>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuItem onSelect={copyAll}>Copy all to clipboard</DropdownMenuItem>
          <DropdownMenuItem onSelect={exportCsv}>Export to CSV</DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    );
    return (
      <div style={{ display: "flex", flexDirection: "column", flex: 1, minHeight: 0, gap: 12 }}>
        <Segmented
          block
          label="Traffic view"
          value={tab}
          onChange={setTab}
          options={[
            { value: "modbus", label: `Modbus ${filtered.length}` },
            { value: "tcp", label: `TCP ${tcpEvents.length}` },
          ]}
        />
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          {tab === "modbus" && (
            <div style={{ flex: 1, minWidth: 0 }}>
              <Segmented
                block
                size="sm"
                label="Filter direction"
                value={filterDir}
                onChange={setFilterDir}
                options={[{ value: "all", label: "All" }, { value: "TX", label: "TX" }, { value: "RX", label: "RX" }]}
              />
            </div>
          )}
          {tab === "tcp" && <div style={{ flex: 1 }} />}
          <IconButton outline label="Auto-scroll" pressed={autoScroll} onClick={() => setAutoScroll((v) => !v)}>
            <ArrowDownToLine size={18} strokeWidth={1.75} />
          </IconButton>
          {exportMenu}
          <IconButton outline label="Clear all traffic" onClick={() => { clearTraffic(activeId); setExpandedRows(new Set()); }}>
            <Trash2 size={18} strokeWidth={1.75} />
          </IconButton>
        </div>
        {tab === "modbus" && (
          <input
            className="vt-input"
            placeholder="Filter by function, address or data"
            aria-label="Filter traffic entries"
            value={filterText}
            onChange={(e) => setFilterText(e.target.value)}
          />
        )}
        {exportMsg && (
          <span style={{ fontSize: "var(--fs-sm)", color: exportMsg.failed ? "var(--danger)" : "var(--accent-text)" }} role={exportMsg.failed ? "alert" : "status"} aria-live="polite">{exportMsg.text}</span>
        )}
        {tab === "modbus" ? renderModbusPhone() : renderTcpPhone()}
      </div>
    );
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", flex: 1, overflow: "hidden" }}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", padding: "4px 8px", borderBottom: "1px solid var(--border)", flexShrink: 0, gap: 8 }}>
        <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
          <div className="vt-tabs" role="tablist" aria-label="Traffic view">
            {(["modbus", "tcp"] as const).map((t) => (
              <button
                key={t}
                role="tab"
                aria-selected={tab === t}
                className="vt-tab"
                onClick={() => setTab(t)}
              >
                {t === "modbus" ? "Modbus" : "TCP"}
                <span className="vt-tab__count">{t === "modbus" ? filtered.length : tcpEvents.length}</span>
              </button>
            ))}
          </div>
        </div>
        <div style={{ display: "flex", gap: 4, alignItems: "center" }}>
          {tab === "modbus" && (
            <>
              {(["All", "TX", "RX"] as const).map((f) => {
                const key = f === "All" ? "all" : f;
                return (
                  <button key={f} onClick={() => setFilterDir(key)} aria-pressed={filterDir === key} className="vt-btn vt-btn--sm">
                    {f}
                  </button>
                );
              })}
              {expandedRows.size > 0 && (
                <button onClick={collapseAll} aria-label="Collapse all expanded rows" className="vt-btn vt-btn--sm vt-btn--ghost">
                  Collapse
                </button>
              )}
              <input
                className="vt-input vt-input--sm"
                placeholder="Filter"
                aria-label="Filter traffic entries"
                value={filterText}
                onChange={(e) => setFilterText(e.target.value)}
                style={{ width: 90 }}
              />
            </>
          )}
          <label className="vt-check">
            <input type="checkbox" checked={autoScroll} onChange={(e) => setAutoScroll(e.target.checked)} aria-label="Auto-scroll" />
            Auto
          </label>
          {(entries.length > 0 || tcpEvents.length > 0) && (
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <button className="vt-btn vt-btn--sm vt-btn--ghost">Export</button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuItem onSelect={copyAll}>Copy all to clipboard</DropdownMenuItem>
                <DropdownMenuItem onSelect={exportCsv}>Export to CSV</DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          )}
          {exportMsg && (
            <span style={{ fontSize: "var(--fs-xs)", color: exportMsg.failed ? "var(--danger)" : "var(--accent-text)" }} role={exportMsg.failed ? "alert" : "status"} aria-live="polite">{exportMsg.text}</span>
          )}
          <button onClick={() => { clearTraffic(activeId); setExpandedRows(new Set()); }} aria-label="Clear all traffic" className="vt-btn vt-btn--sm vt-btn--ghost">
            Clear
          </button>
        </div>
      </div>
      {tab === "modbus" ? renderModbusTab() : renderTcpTab()}
    </div>
  );
}
