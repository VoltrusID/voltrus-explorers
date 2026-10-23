import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { ChevronDown, ChevronRight, Play, Square } from "lucide-react";
import { Badge, Button, Callout, Dialog, EmptyState, Field, HexViewer, SheetSelect } from "@mactools/ui";
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
  large: boolean;
  o_to_t_run_idle: boolean;
  connection_path: number[];
}

interface Props {
  onLog?: (direction: "tx" | "rx" | "error" | "info", message: string) => void;
  onClose: () => void;
}

const IO_MODES = [
  { value: "implicit", label: "Implicit I/O (class 1, UDP)" },
  { value: "explicit", label: "Explicit (class 3)" },
] as const;

function fmtTime(ms: number) {
  return new Date(ms).toLocaleTimeString("en-US", { hour12: false, hour: "2-digit", minute: "2-digit", second: "2-digit", fractionalSecondDigits: 3 });
}

function parseField(s: string, fallback: number): number {
  const t = s.trim();
  if (!t) return fallback;
  const n = t.toLowerCase().startsWith("0x") ? parseInt(t, 16) : Number(t);
  return Number.isFinite(n) ? n : fallback;
}

/** Logical EPath segment: 8-bit, 16-bit (+pad) or 32-bit (+pad) form of `base` (0x20 class, 0x24 instance, 0x2C connection point). */
function logicalSegment(base: number, value: number): number[] {
  if (value <= 0xff) return [base, value];
  if (value <= 0xffff) return [base | 1, 0, value & 0xff, value >> 8];
  return [base | 2, 0, value & 0xff, (value >> 8) & 0xff, (value >> 16) & 0xff, (value >>> 24) & 0xff];
}

/** "1,0" / "1/0" → port segments [0x01, 0x00]. Numeric pairs only. */
function routeSegments(text: string): number[] {
  const nums = text.split(/[,/]/).map((t) => t.trim()).filter(Boolean).map(Number);
  if (nums.length % 2 !== 0 || nums.some((n) => !Number.isInteger(n) || n < 0 || n > 14)) {
    throw new Error("Route prefix must be port,slot pairs (e.g. 1,0)");
  }
  return nums;
}

/**
 * Network connection parameters (ODVA Vol 1 §3-5.5.1.1): point-to-point, variable size,
 * low priority. 16 bits for Forward_Open; 32 bits (flags in the upper half) for Large_Forward_Open.
 */
function networkParams(size: number, large: boolean): number {
  const flags = 0x4200; // connection type 2 (P2P) in bits 13-14, variable-size bit 9
  return large ? flags * 0x10000 + (size & 0xffff) : flags | (size & 0x1ff);
}

export function IoPump({ onLog, onClose }: Props) {
  const [mode, setMode] = useState<"implicit" | "explicit">("implicit");
  const [rpiMs, setRpiMs] = useState(100);
  const [cfgInst, setCfgInst] = useState(102);
  const [outInst, setOutInst] = useState(150);
  const [inInst, setInInst] = useState(100);
  const [outSize, setOutSize] = useState(4);
  const [inSize, setInSize] = useState(8);
  const [routePrefix, setRoutePrefix] = useState("");
  const [vendor, setVendor] = useState(0x0001);
  const [originator, setOriginator] = useState(0x12345678);
  const [advOpen, setAdvOpen] = useState(false);
  const [timeoutMult, setTimeoutMult] = useState("1");
  const [timeoutTicks, setTimeoutTicks] = useState("0x0A");
  const [large, setLarge] = useState(false);
  const [runIdle, setRunIdle] = useState(true);
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

  const buildParams = (): ForwardOpenParams => {
    const rpiUs = Math.max(10, rpiMs) * 1000;
    const prefix = routeSegments(routePrefix);
    let path: number[];
    let oSize: number;
    let tSize: number;
    let transport: number;
    if (mode === "implicit") {
      // Class 1 cyclic I/O: configuration, output (O→T) and input (T→O) assembly instances.
      path = [
        ...prefix,
        ...logicalSegment(0x20, 0x04),
        ...logicalSegment(0x24, cfgInst),
        ...logicalSegment(0x2c, outInst),
        ...logicalSegment(0x2c, inInst),
      ];
      // Connection size = 16-bit sequence count + optional 32-bit run/idle header + data.
      oSize = outSize + 2 + (runIdle ? 4 : 0);
      tSize = inSize + 2;
      transport = 0x01;
    } else {
      // Class 3 connected explicit messaging to the Message Router.
      path = [...prefix, ...logicalSegment(0x20, 0x02), ...logicalSegment(0x24, 0x01)];
      oSize = tSize = large ? 4000 : 500;
      transport = 0xa3;
    }
    return {
      o_to_t_connection_id: 0, // chosen by the target
      t_to_o_connection_id: (Date.now() & 0x7fffffff) >>> 0, // chosen by us, unique per open
      connection_serial: Math.floor(Math.random() * 0xffff),
      vendor_id: vendor,
      originator_serial: originator,
      o_to_t_rpi_us: rpiUs,
      o_to_t_params: networkParams(oSize, large),
      t_to_o_rpi_us: rpiUs,
      t_to_o_params: networkParams(tSize, large),
      timeout_ticks: parseField(timeoutTicks, 0x0a),
      timeout_multiplier: parseField(timeoutMult, 1),
      transport_class_trigger: transport,
      large,
      o_to_t_run_idle: mode === "implicit" && runIdle,
      connection_path: path,
    };
  };

  const start = async () => {
    setError("");
    setBusy(true);
    setSamples([]);
    try {
      const params = buildParams();
      const res = await invoke<{ t_to_o_connection_id: number }>("io_open", { params });
      setRunning(true);
      onLog?.("info", `Forward${large ? "Open (large)" : "Open"} OK (T→O conn 0x${res.t_to_o_connection_id.toString(16)})`);
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
            {mode === "implicit"
              ? "Class 1 implicit I/O: ForwardOpen over TCP, then cyclic T→O data arrives over UDP (port 2222) while this app sends O→T heartbeats every RPI. Works against the built-in adapter (Adapter mode) or a real CIP adapter."
              : "Class 3 connected explicit messaging: ForwardOpen to the Message Router, then one connected Get_Attribute_Single (Identity vendor id) is sent per RPI as SendUnitData on the TCP session — it keeps the connection alive and each reply shows as a sample."}
          </span>
        </div>

        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr 1fr", gap: 10 }}>
          <Field label="Connection">
            <SheetSelect sheetTitle="Connection" value={mode} disabled={running} onChange={(v) => setMode(v)} options={IO_MODES} />
          </Field>
          <Field label="RPI (ms)">
            <input className="vt-input" type="number" min={10} value={rpiMs} disabled={running} onChange={(e) => setRpiMs(Math.max(10, Number(e.target.value)))} />
          </Field>
          <Field label="Decoder">
            <SheetSelect sheetTitle="Decoder" value={decoderId} onChange={(v) => setDecoderId(v)}
              options={[{ value: "", label: "None (raw)" }, ...decoders.map((d) => ({ value: d.id, label: d.name || "(unnamed)" }))]} />
          </Field>
        </div>

        {mode === "implicit" && (
          <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr 1fr", gap: 10 }}>
            <Field label="Config instance">
              <input className="vt-input" type="number" value={cfgInst} disabled={running} onChange={(e) => setCfgInst(Number(e.target.value))} />
            </Field>
            <Field label="Output (O→T) instance">
              <input className="vt-input" type="number" value={outInst} disabled={running} onChange={(e) => setOutInst(Number(e.target.value))} />
            </Field>
            <Field label="Input (T→O) instance">
              <input className="vt-input" type="number" value={inInst} disabled={running} onChange={(e) => setInInst(Number(e.target.value))} />
            </Field>
            <Field label="Output size (bytes)">
              <input className="vt-input" type="number" min={0} value={outSize} disabled={running} onChange={(e) => setOutSize(Math.max(0, Number(e.target.value)))} />
            </Field>
            <Field label="Input size (bytes)">
              <input className="vt-input" type="number" min={0} value={inSize} disabled={running} onChange={(e) => setInSize(Math.max(0, Number(e.target.value)))} />
            </Field>
          </div>
        )}

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
              <Field label="Route prefix (port,slot)">
                <input className="vt-input vt-input--mono" placeholder="e.g. 1,0" value={routePrefix} disabled={running} onChange={(e) => setRoutePrefix(e.target.value)} />
              </Field>
              <Field label="Timeout multiplier code">
                <input className="vt-input vt-input--mono" value={timeoutMult} disabled={running} onChange={(e) => setTimeoutMult(e.target.value)} />
              </Field>
              <Field label="Timeout ticks">
                <input className="vt-input vt-input--mono" value={timeoutTicks} disabled={running} onChange={(e) => setTimeoutTicks(e.target.value)} />
              </Field>
              <label style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 12 }}>
                <input type="checkbox" checked={large} disabled={running} onChange={(e) => setLarge(e.target.checked)} />
                Large_Forward_Open (32-bit sizes)
              </label>
              {mode === "implicit" && (
                <label style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 12 }}>
                  <input type="checkbox" checked={runIdle} disabled={running} onChange={(e) => setRunIdle(e.target.checked)} />
                  O→T run/idle header
                </label>
              )}
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
