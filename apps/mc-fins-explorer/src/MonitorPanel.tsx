import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Grid3x3 } from "lucide-react";
import { EmptyState, Field, Segmented, ValueEditor, useProStatus } from "@mactools/ui";
import type { Device } from "./types";
import { formatWord, offsetAddress } from "./helpers";

type Fmt = "dec" | "sdec" | "hex" | "bin";

interface Props {
  device: Device;
  connected: boolean;
  pollMs: number;
  onLog: (dir: "info" | "rx" | "tx" | "error", msg: string) => void;
  onWrite: (address: string, dataType: string, value: string) => Promise<string | null>;
}

const COLS = 8;

/** A live grid of words from a head address, like the device batch monitor of GX Works or CX-Programmer. */
export default function MonitorPanel({ device, connected, pollMs, onLog, onWrite }: Props) {
  const { requirePro } = useProStatus();
  const [head, setHead] = useState(device.protocol === "mc" ? "D100" : "D100");
  const [count, setCount] = useState(32);
  const [fmt, setFmt] = useState<Fmt>("dec");
  const [words, setWords] = useState<number[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [editing, setEditing] = useState<{ address: string; value: string } | null>(null);
  const [writeError, setWriteError] = useState<string | null>(null);
  const busy = useRef(false);

  useEffect(() => {
    setWords(null);
    if (!connected || !head.trim()) return;
    let alive = true;
    const tick = async () => {
      if (busy.current) return;
      busy.current = true;
      try {
        const w = await invoke<number[]>("read_block", { deviceId: device.id, head, count });
        if (alive) { setWords(w); setError(null); }
      } catch (e) {
        if (alive) { setWords(null); setError(String(e)); }
      } finally {
        busy.current = false;
      }
    };
    void tick();
    const t = setInterval(tick, pollMs);
    return () => { alive = false; clearInterval(t); };
  }, [connected, device.id, head, count, pollMs]);

  const rows = words ? Math.ceil(words.length / COLS) : 0;

  return (
    <div style={{ display: "flex", flexDirection: "column", minHeight: 0, flex: 1 }}>
      <div style={{ display: "flex", alignItems: "flex-end", gap: 10, padding: "10px 14px", borderBottom: "1px solid var(--border)", flexWrap: "wrap" }}>
        <div style={{ width: 120 }}>
          <Field label="Head address">
            {(id) => <input id={id} className="vt-input mono" value={head} onChange={(e) => setHead(e.target.value.toUpperCase())} />}
          </Field>
        </div>
        <div style={{ width: 90 }}>
          <Field label="Words">
            {(id) => (
              <input id={id} type="number" min={1} max={1024} className="vt-input" value={count}
                onChange={(e) => setCount(Math.min(1024, Math.max(1, Number(e.target.value) || 1)))} />
            )}
          </Field>
        </div>
        <Segmented<Fmt>
          label="Number format"
          size="sm"
          value={fmt}
          onChange={setFmt}
          options={[
            { value: "dec", label: "Unsigned" },
            { value: "sdec", label: "Signed" },
            { value: "hex", label: "Hex" },
            { value: "bin", label: "Binary" },
          ]}
        />
      </div>
      <div style={{ flex: 1, minHeight: 0, overflow: "auto" }}>
        {!connected ? (
          <EmptyState compact icon={<Grid3x3 size={18} strokeWidth={1.75} />} title="Not connected" description="Connect to a PLC to watch a block of words." />
        ) : error ? (
          <div role="alert" style={{ padding: 14, color: "var(--danger)", fontSize: "var(--fs-sm)" }}>{error}</div>
        ) : !words ? (
          <div style={{ padding: 14, color: "var(--text-muted)", fontSize: "var(--fs-sm)" }}>Reading…</div>
        ) : (
          <table className="vt-table mf-grid" style={{ width: "100%", tableLayout: "fixed" }}>
            <thead>
              <tr>
                <th style={{ width: 90 }}>Address</th>
                {Array.from({ length: COLS }, (_, c) => <th key={c} className="mono">+{c}</th>)}
              </tr>
            </thead>
            <tbody>
              {Array.from({ length: rows }, (_, r) => (
                <tr key={r}>
                  <td className="mono" style={{ color: "var(--text-muted)" }}>{offsetAddress(device.protocol, head, r * COLS)}</td>
                  {Array.from({ length: COLS }, (_, c) => {
                    const i = r * COLS + c;
                    if (i >= words.length) return <td key={c} />;
                    const address = offsetAddress(device.protocol, head, i);
                    return (
                      <td
                        key={c}
                        className="mono mf-cell"
                        title={`${address}: ${words[i]} (0x${words[i].toString(16).toUpperCase().padStart(4, "0")})`}
                        style={{ fontSize: fmt === "bin" ? "var(--fs-xs)" : undefined, whiteSpace: "nowrap" }}
                        onClick={() => {
                          if (!requirePro("Writing values to the PLC")) return;
                          setWriteError(null);
                          setEditing({ address, value: formatWord(words[i], fmt === "sdec" ? "sdec" : "dec") });
                        }}
                      >
                        {formatWord(words[i], fmt)}
                      </td>
                    );
                  })}
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
      {editing && (
        <ValueEditor
          address={editing.address}
          currentValue={editing.value}
          currentDataType={fmt === "sdec" ? "Int16" : fmt === "hex" ? "Hex16" : "UInt16"}
          types={[
            { value: "UInt16", label: "UInt16" },
            { value: "Int16", label: "Int16" },
            { value: "Hex16", label: "Hex16" },
            { value: "BCD16", label: "BCD16" },
          ]}
          error={writeError}
          onSubmit={async (value, dataType) => {
            const err = await onWrite(editing.address, dataType, value);
            if (err) {
              setWriteError(err);
              onLog("error", `[${device.label}] Write ${value} to ${editing.address} failed: ${err}`);
            } else {
              onLog("tx", `[${device.label}] Wrote ${value} (${dataType}) to ${editing.address}`);
              setEditing(null);
            }
          }}
          onCancel={() => setEditing(null)}
        />
      )}
    </div>
  );
}
