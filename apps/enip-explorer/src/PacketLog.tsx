import { useEffect, useRef, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { Download } from "lucide-react";
import { Button, Callout, Dialog, EmptyState, Segmented, HexViewer, ProBadge, saveTextFile, useProStatus } from "@mactools/ui";

interface PacketRecord {
  direction: "Tx" | "Rx";
  command: number;
  session: number;
  timestamp_ms: number;
  bytes: number[];
}

const CMD_NAMES: Record<number, string> = {
  0x0063: "ListIdentity",
  0x0065: "RegisterSession",
  0x0066: "UnregisterSession",
  0x006f: "SendRRData",
  0x0070: "SendUnitData",
};

function fmtTime(ms: number) {
  const d = new Date(ms);
  return d.toLocaleTimeString("en-US", { hour12: false, hour: "2-digit", minute: "2-digit", second: "2-digit", fractionalSecondDigits: 3 });
}

function bytesToHex(bytes: number[]): string {
  return bytes.map((b) => b.toString(16).padStart(2, "0")).join(" ");
}

async function exportPacketsCsv(packets: PacketRecord[]): Promise<string | null> {
  const esc = (v: unknown) => {
    const s = String(v ?? "");
    return /[",\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
  };
  const header = ["timestamp_ms", "time", "direction", "command", "command_hex", "session", "size_bytes", "hex"].join(",");
  const rows = packets.map((p) =>
    [
      p.timestamp_ms,
      fmtTime(p.timestamp_ms),
      p.direction,
      CMD_NAMES[p.command] ?? `0x${p.command.toString(16)}`,
      `0x${p.command.toString(16).padStart(4, "0")}`,
      p.session,
      p.bytes.length,
      bytesToHex(p.bytes),
    ]
      .map(esc)
      .join(","),
  );
  const csv = [header, ...rows].join("\n");
  return saveTextFile(`enip-packets-${new Date().toISOString().slice(0, 19).replace(/:/g, "-")}.csv`, csv);
}

const CAP = 500;

export function PacketLog({ onClose }: { onClose: () => void }) {
  const [packets, setPackets] = useState<PacketRecord[]>([]);
  const [filter, setFilter] = useState<"All" | "Tx" | "Rx">("All");
  const [openIdx, setOpenIdx] = useState<number | null>(null);
  const [exportError, setExportError] = useState<string | null>(null);
  const { requirePro } = useProStatus();
  const unlistenRef = useRef<UnlistenFn | null>(null);

  useEffect(() => {
    let active = true;
    listen<PacketRecord>("enip://packet", (e) => {
      if (!active) return;
      setPackets((prev) => {
        const next = [...prev, e.payload];
        return next.length > CAP ? next.slice(next.length - CAP) : next;
      });
    }).then((fn) => {
      unlistenRef.current = fn;
    });
    return () => {
      active = false;
      unlistenRef.current?.();
    };
  }, []);

  const visible = filter === "All" ? packets : packets.filter((p) => p.direction === filter);

  return (
    <Dialog open onClose={onClose} title="Packet logger" width={720}>
      <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
        <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
          <Segmented
            size="sm"
            label="Filter packets"
            value={filter}
            onChange={setFilter}
            options={[
              { value: "All", label: "All" },
              { value: "Tx", label: "Tx" },
              { value: "Rx", label: "Rx" },
            ]}
          />
          <span style={{ flex: 1 }} />
          <Button size="sm" icon={<Download size={14} />} onClick={() => {
            if (!requirePro("Packet log export")) return;
            setExportError(null);
            exportPacketsCsv(packets).catch((e) => setExportError(`Export failed: ${String(e)}`));
          }} disabled={packets.length === 0}>
            Export CSV <ProBadge />
          </Button>
          <Button size="sm" variant="danger" onClick={() => setPackets([])} disabled={packets.length === 0}>
            Clear
          </Button>
        </div>

        {exportError && <Callout tone="danger">{exportError}</Callout>}

        <div style={{ height: 380, display: "flex", flexDirection: "column" }}>
          {visible.length === 0 ? (
            <EmptyState
              title="No packets captured"
              description="Raw wire frames appear here once you connect and send requests."
            />
          ) : (
            <div style={{ flex: 1, minHeight: 0, overflow: "auto" }}>
              {visible.map((p, i) => {
                const realIdx = packets.indexOf(p);
                const isTx = p.direction === "Tx";
                return (
                  <div key={i} style={{ borderBottom: "1px solid var(--border)" }}>
                    <div
                      onClick={() => setOpenIdx(openIdx === realIdx ? null : realIdx)}
                      className="mono"
                      style={{ display: "flex", gap: 8, padding: "3px 6px", cursor: "pointer", fontSize: 11 }}
                      onMouseEnter={(e) => ((e.currentTarget as HTMLElement).style.background = "var(--hover-overlay)")}
                      onMouseLeave={(e) => ((e.currentTarget as HTMLElement).style.background = "transparent")}
                    >
                      <span style={{ color: "var(--text-muted)", flexShrink: 0 }}>{fmtTime(p.timestamp_ms)}</span>
                      <span style={{ color: isTx ? "var(--accent)" : "var(--success)", fontWeight: 600, width: 24 }}>{isTx ? "TX" : "RX"}</span>
                      <span style={{ color: "var(--text)", minWidth: 110 }}>{CMD_NAMES[p.command] ?? `0x${p.command.toString(16)}`}</span>
                      <span style={{ color: "var(--text-dim)" }}>{p.bytes.length} B</span>
                    </div>
                    {openIdx === realIdx && (
                      <div style={{ padding: "4px 6px 8px" }}>
                        <HexViewer data={new Uint8Array(p.bytes)} />
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
          )}
        </div>
        <div style={{ fontSize: 11, color: "var(--text-muted)" }}>
          Click a row to expand the hex dump. Most recent {CAP} frames retained.
        </div>
      </div>
    </Dialog>
  );
}
