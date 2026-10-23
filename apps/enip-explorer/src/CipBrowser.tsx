import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Send, Settings2 } from "lucide-react";
import { Button, Dialog, Field, Tabs, Callout, HexViewer, ProBadge, SheetSelect, useProStatus } from "@mactools/ui";
import { DecoderEditor } from "./DecoderEditor";
import { decodeStruct, loadDecoders, type Decoder } from "./decoders";
import { CipTree } from "./CipTree";
import { type EdsMap } from "./eds";

// Common CIP object classes for quick fill.
const CLASSES: Array<[string, number]> = [
  ["Identity 0x01", 0x01],
  ["MsgRouter 0x02", 0x02],
  ["Assembly 0x04", 0x04],
  ["TCP/IP 0xF5", 0xf5],
  ["EtherNetLink 0xF6", 0xf6],
  ["Symbol 0x6B", 0x6b],
  ["ConnMgr 0x06", 0x06],
  ["ConnTrkr 0x03", 0x03],
];

function parseHex(text: string): number[] {
  const clean = text.replace(/0x/gi, "").replace(/[^0-9a-fA-F]/g, "");
  const out: number[] = [];
  for (let i = 0; i + 1 < clean.length; i += 2) {
    out.push(parseInt(clean.slice(i, i + 2), 16));
  }
  return out;
}

function decodeAttempt(bytes: number[]): string {
  if (bytes.length === 1) return `BOOL/SINT (u8): ${bytes[0]}  (0x${bytes[0].toString(16)})`;
  if (bytes.length === 2) {
    const dv = new DataView(new Uint8Array(bytes).buffer);
    return `INT (i16): ${dv.getInt16(0, true)}  UINT (u16): ${dv.getUint16(0, true)}`;
  }
  if (bytes.length === 4) {
    const dv = new DataView(new Uint8Array(bytes).buffer);
    return `DINT (i32): ${dv.getInt32(0, true)}  REAL (f32): ${dv.getFloat32(0, true).toFixed(4)}`;
  }
  if (bytes.length >= 4) {
    // Try CIP STRING (len prefix)
    const len = bytes[0] | (bytes[1] << 8);
    if (len > 0 && len <= bytes.length - 4) {
      const str = bytes.slice(4, 4 + len).map((b) => String.fromCharCode(b)).join("");
      if (/^[\x20-\x7e]*$/.test(str)) return `STRING: "${str}"`;
    }
  }
  return `${bytes.length} byte(s)`;
}

interface Props {
  onClose: () => void;
  onLog?: (direction: "tx" | "rx" | "error" | "info", message: string) => void;
  eds: EdsMap;
}

const inputStyle: React.CSSProperties = {
  background: "var(--input-bg)",
  border: "1px solid var(--border)",
  borderRadius: 6,
  padding: "5px 8px",
  color: "var(--text)",
  fontSize: 12,
  outline: "none",
  width: "100%",
};

const cellHead: React.CSSProperties = {
  padding: "3px 6px",
  borderBottom: "1px solid var(--border)",
  color: "var(--text-muted)",
  fontWeight: 500,
  textAlign: "left",
  fontSize: 11,
};

const cell: React.CSSProperties = { padding: "3px 6px", fontSize: 11 };

export function CipBrowser({ onClose, onLog, eds }: Props) {
  const [view, setView] = useState<"tree" | "console">("tree");
  const [classId, setClassId] = useState("0x01");
  const [instanceId, setInstanceId] = useState("1");
  const [attrId, setAttrId] = useState("1");
  const [service, setService] = useState("0x0E");
  const [dataHex, setDataHex] = useState("");
  const [result, setResult] = useState<number[] | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [instances, setInstances] = useState<{ instance: number; data: number[] }[] | null>(null);
  const [instBusy, setInstBusy] = useState(false);
  const [decoders, setDecoders] = useState<Decoder[]>([]);
  const [activeDecoderId, setActiveDecoderId] = useState<string>("");
  const [showEditor, setShowEditor] = useState(false);
  const { requirePro } = useProStatus();

  useEffect(() => {
    setDecoders(loadDecoders());
  }, []);

  const activeDecoder = decoders.find((d) => d.id === activeDecoderId) ?? null;
  const decoded = useMemo(
    () => (result && activeDecoder ? decodeStruct(result, activeDecoder.fields) : null),
    [result, activeDecoder]
  );

  const num = (s: string, base = 16) => parseInt(s.trim(), base);

  // Reads (Get_Attribute_Single 0x0E, Get_Attributes_All 0x01) are free; every other service can change the device.
  const exec = async (svc: number, withData: boolean) => {
    if (svc !== 0x0e && svc !== 0x01 && !requirePro("CIP services and attribute writes")) return;
    setBusy(true);
    setError("");
    setResult(null);
    try {
      const data = withData ? parseHex(dataHex) : [];
      const out = await invoke<number[]>("cip_generic", {
        service: svc,
        classId: num(classId),
        instanceId: num(instanceId, 10),
        attributeId: num(attrId),
        data,
      });
      setResult(out);
      onLog?.("rx", `CIP svc 0x${svc.toString(16)} cls 0x${num(classId).toString(16)} inst ${num(instanceId, 10)} attr ${num(attrId)} → ${out.length}B`);
    } catch (e) {
      setError(String(e));
      onLog?.("error", `CIP svc 0x${svc.toString(16)} failed: ${String(e)}`);
    } finally {
      setBusy(false);
    }
  };

  const listInstances = async () => {
    setInstBusy(true);
    setError("");
    try {
      const list = await invoke<Array<{ instance: number; data: number[] }>>("list_class_instances", { classId: num(classId), max: 256 });
      setInstances(list);
      if (list.length > 0) { setResult(list[0].data); }
      onLog?.("rx", `Listed ${list.length} instances of class 0x${num(classId).toString(16)}`);
    } catch (e) {
      setError(String(e));
    } finally {
      setInstBusy(false);
    }
  };

  return (
    <Dialog open onClose={onClose} title="CIP Object Browser" width={640}>
      <div style={{ display: "flex", flexDirection: "column", gap: 12, minHeight: 0 }}>
        <div style={{ margin: "0 -18px" }}>
          <Tabs
            label="Browser view"
            value={view}
            onChange={setView}
            items={[
              { value: "tree", label: "Tree" },
              { value: "console", label: "Console" },
            ]}
            trailing={
              eds.params.length > 0 ? (
                <span style={{ fontSize: 11, color: "var(--text-muted)" }}>EDS: {eds.params.length} params</span>
              ) : undefined
            }
          />
        </div>

        {view === "tree" && (
          <div style={{ overflow: "auto", maxHeight: "58vh" }}>
            <CipTree eds={eds} onLog={onLog} />
          </div>
        )}

        {view === "console" && (
          <>
            <div>
              <span className="vt-label" style={{ display: "block", marginBottom: 6 }}>Class presets</span>
              <div style={{ display: "flex", flexWrap: "wrap", gap: 4 }}>
                {CLASSES.map(([label, id]) => (
                  <Button key={id} variant="ghost" size="sm" className="mono" onClick={() => setClassId(`0x${id.toString(16)}`)}>{label}</Button>
                ))}
              </div>
            </div>

            <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr 1fr", gap: 10 }}>
              <Field label="Class (hex)">
                <input className="vt-input vt-input--mono" style={inputStyle} value={classId} onChange={(e) => setClassId(e.target.value)} />
              </Field>
              <Field label="Instance (dec)">
                <input className="vt-input" style={inputStyle} value={instanceId} onChange={(e) => setInstanceId(e.target.value)} />
              </Field>
              <Field label="Attribute (dec)">
                <input className="vt-input" style={inputStyle} value={attrId} onChange={(e) => setAttrId(e.target.value)} />
              </Field>
            </div>

            <Field label="Service (hex): 0x0E Get, 0x10 Set, 0x01 GetAll, 0x4C ReadTag, 0x54 ForwardOpen, 0x4E ForwardClose">
              <input className="vt-input vt-input--mono" style={inputStyle} value={service} onChange={(e) => setService(e.target.value)} />
            </Field>

            <Field label="Write data (hex, for Set/Write)">
              <input className="vt-input vt-input--mono" style={inputStyle} value={dataHex} onChange={(e) => setDataHex(e.target.value)} placeholder="e.g. 01 00 or 0x0100" />
            </Field>

            <div style={{ display: "flex", gap: 6, alignItems: "center", flexWrap: "wrap" }}>
              <Button variant="primary" icon={<Send size={14} />} loading={busy} onClick={() => exec(num(service), false)}>Send</Button>
              <Button disabled={busy} onClick={() => exec(0x0E, false)}>Get Attr</Button>
              <Button disabled={busy} onClick={() => exec(0x01, false)}>Get All</Button>
              <Button disabled={busy} onClick={() => exec(0x10, true)}>Set Attr <ProBadge /></Button>
              <Button loading={instBusy} onClick={listInstances}>List Inst</Button>
              <span style={{ flex: 1 }} />
              <SheetSelect
                style={{ height: 28, fontSize: 12 }}
                value={activeDecoderId}
                onChange={(v) => setActiveDecoderId(v)}
                aria-label="Response decoder"
                sheetTitle="Response decoder"
                options={[{ value: "", label: "No decoder" }, ...decoders.map((d) => ({ value: d.id, label: d.name || "(unnamed)" }))]}
              />
              <Button icon={<Settings2 size={14} />} onClick={() => { if (requirePro("Custom response decoders")) setShowEditor(true); }}>Decoders <ProBadge /></Button>
            </div>

            {instances && instances.length > 0 && (
              <Field label={`Instances (${instances.length})`}>
                <SheetSelect
                  sheetTitle="Load instance"
                  placeholder="Load instance"
                  value=""
                  options={instances.map((it, i) => ({ value: String(i), label: `Instance ${it.instance} (${it.data.length}B)` }))}
                  onChange={(v) => { const it = instances[Number(v)]; if (it) setResult(it.data); }}
                />
              </Field>
            )}

            {error && (
              <Callout tone="danger">
                <span style={{ fontSize: 12 }}>{error}</span>
              </Callout>
            )}

            <div style={{ flex: 1, minHeight: 120, overflow: "auto" }}>
              {result && (
                <>
                  <div style={{ fontSize: 11, color: "var(--text-muted)", lineHeight: 1.5, marginBottom: 4 }}>
                    Tip: ForwardOpen (0x54) / ForwardClose (0x4E) target Connection Manager (class 0x06, instance 1); supply the ForwardOpen body as hex above. Cyclic IO (SendUnitData 0x0070) needs a device-specific pump.
                  </div>
                  <div className="mono" style={{ fontSize: 11, color: "var(--text-dim)", marginBottom: 6 }}>
                    {result.length} byte(s) · {decodeAttempt(result)}
                  </div>
                  {decoded && decoded.length > 0 && (
                    <table className="vt-table" style={{ marginBottom: 8 }}>
                      <thead>
                        <tr>
                          <th style={cellHead}>Offset</th>
                          <th style={cellHead}>Field</th>
                          <th style={cellHead}>Type</th>
                          <th style={cellHead}>Value</th>
                        </tr>
                      </thead>
                      <tbody>
                        {decoded.map((f, i) => (
                          <tr key={i}>
                            <td className="mono" style={cell}>{f.offset}</td>
                            <td style={cell}>{f.name}</td>
                            <td style={{ ...cell, color: "var(--text-muted)" }}>{f.type}</td>
                            <td className="mono" style={{ ...cell, color: "var(--accent-text)" }}>{f.value}</td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  )}
                  <HexViewer data={new Uint8Array(result)} />
                </>
              )}
            </div>
          </>
        )}
        {showEditor && <DecoderEditor onClose={() => { setShowEditor(false); setDecoders(loadDecoders()); }} />}
      </div>
    </Dialog>
  );
}
