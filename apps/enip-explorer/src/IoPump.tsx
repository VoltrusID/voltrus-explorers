import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { ChevronDown, ChevronRight, Play, Square } from "lucide-react";
import { Badge, Button, Callout, Dialog, EmptyState, Field, HexViewer } from "@mactools/ui";
import { decodeStruct, loadDecoders, type Decoder } from "./decoders";

interface IoSample {
  sequence: number;
  connection_id: number;
  bytes: number[];
  timestamp_ms: number;
}

interface ForwardOpenParams {
  o_to_t_connection_id: number;
  t_to_o_connection_id: number;
  connection_serial: number;
  vendor_id: number;
  originator_serial: number;
  o_to_t_rpi_us: number;
  o_to_t_params: number;
  t_to_o_rpi_us: number;
  t_to_o_params: number;
  timeout_ticks: number;
  timeout_multiplier: number;
  transport_class_trigger: number;
  connection_path: number[];
}

interface Props {
  onLog?: (direction: "tx" | "rx" | "error" | "info", message: string) => void;
  onClose: () => void;
}

function fmtTime(ms: number) {
  return new Date(ms).toLocaleTimeString("en-US", { hour12: false, hour: "2-digit", minute: "2-digit", second: "2-digit", fractionalSecondDigits: 3 });
}

function parseField(s: string, fallback: number): number {
  const t = s.trim();
  if (!t) return fallback;
  const n = t.toLowerCase().startsWith("0x") ? parseInt(t, 16) : Number(t);
  return Number.isFinite(n) ? n : fallback;
}

export function IoPump({ onLog, onClose }: Props) {
  const [rpiMs, setRpiMs] = useState(200);
  const [otInst, setOtInst] = useState(100);
  const [vendor, setVendor] = useState(0x0001);
  const [originator, setOriginator] = useState(0x12345678);
  const [advOpen, setAdvOpen] = useState(false);
  const [transportClass, setTransportClass] = useState("0xA3");
  const [timeoutMult, setTimeoutMult] = useState("1");
  const [timeoutTicks, setTimeoutTicks] = useState("0x0A");
  const [otParams, setOtParams] = useState("0x0200");
  const [toParams, setToParams] = useState("0x0200");
  const [running, setRunning] = useState(false);
  const [busy, setBusy] = useState(false);
  const [samples, setSamples] = useState<IoSample[]>([]);
  const [decoders, setDecoders] = useState<Decoder[]>([]);
  const [decoderId, setDecoderId] = useState("");
  const [error, setError] = useState("");
  const unlistenRef = useRef<UnlistenFn | null>(null);

  useEffect(() => {
    setDecoders(loadDecoders());
  }, []);

  useEffect(() => {
    if (!running) return;
    let active = true;
    listen<IoSample>("enip://io", (e) => {
      if (!active) return;
      setSamples((prev) => {
        const next = [...prev, e.payload];
        return next.length > 200 ? next.slice(next.length - 200) : next;
      });
    }).then((fn) => {
      unlistenRef.current = fn;
    });
    return () => {
      active = false;
      unlistenRef.current?.();
    };
  }, [running]);

  const start = async () => {
    setError("");
    setBusy(true);
    setSamples([]);
    try {
      const rpiUs = Math.max(20, rpiMs) * 1000;
      const params: ForwardOpenParams = {
        o_to_t_connection_id: 0x00000001,
        t_to_o_connection_id: 0x00000002,
        connection_serial: 0x0001,
        vendor_id: vendor,
        originator_serial: originator,
        o_to_t_rpi_us: rpiUs,
        o_to_t_params: parseField(otParams, 0x0200),
        t_to_o_rpi_us: rpiUs,
        t_to_o_params: parseField(toParams, 0x0200),
        timeout_ticks: parseField(timeoutTicks, 0x0A),
        timeout_multiplier: parseField(timeoutMult, 1),
        transport_class_trigger: parseField(transportClass, 0xA3),
        connection_path: [0x01, 0x00, 0x20, 0x04, 0x24, otInst],
      };
      const res = await invoke<{ t_to_o_connection_id: number }>("io_open", { params });
      setRunning(true);
      onLog?.("info", `ForwardOpen OK (T→O conn 0x${res.t_to_o_connection_id.toString(16)})`);
    } catch (e) {
      setError(String(e));
      onLog?.("error", `IO open failed: ${String(e)}`);
    } finally {
      setBusy(false);
    }
  };

  const stop = async () => {
    setBusy(true);
    try {
      await invoke("io_stop");
      onLog?.("info", "IO pump stopped (ForwardClose)");
    } catch (e) {
      onLog?.("error", `IO stop failed: ${String(e)}`);
    } finally {
      setRunning(false);
      setBusy(false);
    }
  };

  const last = samples[samples.length - 1];
  const activeDecoder = decoders.find((d) => d.id === decoderId) ?? null;
  const decoded = last && activeDecoder ? decodeStruct(last.bytes, activeDecoder.fields) : null;

  return (
    <Dialog open onClose={onClose} title="Implicit IO" width={640}>
      <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          {running && <Badge tone="success">Live</Badge>}
          <span style={{ fontSize: 11, color: "var(--text-muted)", lineHeight: 1.5 }}>
            ForwardOpen an IO connection, then consume cyclic T→O data. Works against the built-in adapter
            (start it in Adapter mode) or a real CIP device.
          </span>
        </div>

        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr 1fr", gap: 10 }}>
          <Field label="RPI (ms)">
            <input className="vt-input" type="number" min={20} value={rpiMs} disabled={running} onChange={(e) => setRpiMs(Math.max(20, Number(e.target.value)))} />
          </Field>
          <Field label="O→T assembly instance">
            <input className="vt-input" type="number" value={otInst} disabled={running} onChange={(e) => setOtInst(Number(e.target.value))} />
          </Field>
          <Field label="Decoder">
            <select className="vt-select" value={decoderId} onChange={(e) => setDecoderId(e.target.value)}>
              <option value="">None (raw)</option>
              {decoders.map((d) => <option key={d.id} value={d.id}>{d.name || "(unnamed)"}</option>)}
            </select>
          </Field>
        </div>

        <div style={{ border: "1px solid var(--border)", borderRadius: 6, background: "var(--bg)" }}>
          <button
            onClick={() => setAdvOpen((v) => !v)}
            style={{ width: "100%", textAlign: "left", background: "none", border: "none", cursor: "pointer", padding: "6px 10px", fontSize: 12, color: "var(--text-muted)", fontWeight: 500, display: "flex", alignItems: "center", gap: 4 }}
            aria-expanded={advOpen}
          >
            {advOpen ? <ChevronDown size={12} /> : <ChevronRight size={12} />} Connection Manager (advanced)
          </button>
          {advOpen && (
            <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr 1fr", gap: 10, padding: "0 10px 10px" }}>
              <Field label="Transport class/trigger">
                <input className="vt-input vt-input--mono" value={transportClass} disabled={running} onChange={(e) => setTransportClass(e.target.value)} />
              </Field>
              <Field label="Timeout multiplier">
                <input className="vt-input vt-input--mono" value={timeoutMult} disabled={running} onChange={(e) => setTimeoutMult(e.target.value)} />
              </Field>
              <Field label="Timeout ticks">
                <input className="vt-input vt-input--mono" value={timeoutTicks} disabled={running} onChange={(e) => setTimeoutTicks(e.target.value)} />
              </Field>
              <Field label="O→T params (hex)">
                <input className="vt-input vt-input--mono" value={otParams} disabled={running} onChange={(e) => setOtParams(e.target.value)} />
              </Field>
              <Field label="T→O params (hex)">
                <input className="vt-input vt-input--mono" value={toParams} disabled={running} onChange={(e) => setToParams(e.target.value)} />
              </Field>
            </div>
          )}
        </div>

        <div style={{ display: "flex", gap: 8 }}>
          {!running ? (
            <Button variant="primary" icon={<Play size={14} />} loading={busy} onClick={start}>
              {busy ? "Opening" : "Start (ForwardOpen)"}
            </Button>
          ) : (
            <Button variant="danger" icon={<Square size={14} />} loading={busy} onClick={stop}>
              Stop (ForwardClose)
            </Button>
          )}
        </div>

        {error && (
          <Callout tone="danger">
            <span style={{ fontSize: 12 }}>{error}</span>
          </Callout>
        )}

        <div style={{ flex: 1, minHeight: 140, maxHeight: "40vh", overflow: "auto" }}>
          {last ? (
            <>
              <div className="mono" style={{ fontSize: 11, color: "var(--text-muted)", marginBottom: 6 }}>
                seq {last.sequence} · conn 0x{last.connection_id.toString(16)} · {last.bytes.length}B · {fmtTime(last.timestamp_ms)} · {samples.length} sample(s)
              </div>
              {decoded && decoded.length > 0 && (
                <table className="vt-table" style={{ marginBottom: 8 }}>
                  <tbody>
                    {decoded.map((f, i) => (
                      <tr key={i}>
                        <td style={{ padding: "2px 6px", fontSize: 11, color: "var(--text)" }}>{f.name}</td>
                        <td style={{ padding: "2px 6px", fontSize: 11, color: "var(--text-muted)" }}>{f.type}</td>
                        <td className="mono" style={{ padding: "2px 6px", fontSize: 11, color: "var(--accent-text)" }}>{f.value}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              )}
              <HexViewer data={new Uint8Array(last.bytes)} />
            </>
          ) : (
            <EmptyState
              compact
              title={running ? "Waiting for cyclic T→O data" : "IO connection not open"}
              description={running ? "Samples appear here as the device cycles data." : "Configure the connection and click Start to send a ForwardOpen."}
            />
          )}
        </div>
      </div>
    </Dialog>
  );
}
