import { Badge, Button, Callout, EmptyState, ProBadge, Tabs, pickSavePath } from "@mactools/ui";
import { invoke } from "@tauri-apps/api/core";
import { ChevronDown, ChevronRight, Download, RefreshCw, RotateCcw, TrendingUp } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { STATES, ds402State, entryValue, extractBits, hex, parseAddr } from "./ecat";
import type { DeviceInfo, EntryView, IoImage, ObjectView, PdoEntryView, PdoMapView, RegistersView, Sii, StateView } from "./types";
import { type AddLog, type RequirePro, StateBadge, card, dim, hint, kv, row, scroller, th } from "./ui";

type Sub = "overview" | "process" | "objects" | "registers" | "eeprom";

interface Props {
  device: DeviceInfo;
  state: StateView | undefined;
  pdo: PdoMapView | undefined;
  image: IoImage | undefined;
  pro: boolean;
  requirePro: RequirePro;
  addLog: AddLog;
  onRequestState: (state: string, positions: number[]) => void;
  busy: boolean;
  trending: Set<string>;
  onToggleTrend: (key: string, label: string) => void;
}

export default function DeviceDetail(p: Props) {
  const [sub, setSub] = useState<Sub>("overview");
  const d = p.device;
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8, flex: 1, minHeight: 0, minWidth: 0 }}>
      <div style={row}>
        <strong style={{ fontSize: "var(--fs-md)" }}>{d.name || `Device ${d.position}`}</strong>
        <span style={dim}>{d.description}</span>
        <span style={{ flex: 1 }} />
        <StateBadge state={p.state?.state ?? d.state} alText={p.state?.alText ?? d.alText} />
      </div>
      <Tabs
        label="Device views"
        value={sub}
        onChange={setSub}
        items={[
          { value: "overview", label: "Overview" },
          { value: "process", label: "Process data" },
          { value: "objects", label: "CoE objects" },
          { value: "registers", label: "Registers" },
          { value: "eeprom", label: "EEPROM" },
        ]}
      />
      <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
        {sub === "overview" && <Overview {...p} />}
        {sub === "process" && <ProcessData {...p} />}
        {sub === "objects" && <Objects {...p} />}
        {sub === "registers" && <Registers {...p} />}
        {sub === "eeprom" && <Eeprom {...p} />}
      </div>
    </div>
  );
}

function Overview({ device: d, state, onRequestState, busy, pro, requirePro }: Props) {
  const cur = state?.state ?? d.state;
  return (
    <div style={{ overflow: "auto", display: "flex", flexDirection: "column", gap: 8 }}>
      <div style={card}>
        <strong>State of this device</strong>
        <div style={{ ...row, margin: "8px 0" }}>
          {STATES.map((s) => (
            <Button
              key={s}
              size="sm"
              variant={s === cur ? "primary" : undefined}
              disabled={busy}
              onClick={() => {
                if (s === "OP" && !pro && !requirePro("OP with outputs")) return;
                onRequestState(s, [d.position]);
              }}
            >
              {s}
              {s === "OP" && !pro && <ProBadge />}
            </Button>
          ))}
        </div>
        <span style={hint}>
          Moves this device only. Going up, it passes every state on the way. A device that refuses keeps its state and shows the AL status code, often the
          fastest way to find a wrong mapping or sync setting.
        </span>
      </div>
      <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(300px, 1fr))", gap: 8 }}>
        <div style={card}>
          <strong>Identity</strong>
          <div style={{ ...kv, marginTop: 8 }}>
            <span style={dim}>Vendor</span>
            <span className="mono">{hex(d.vendorId, 8)}{d.vendorName ? ` ${d.vendorName}` : ""}</span>
            <span style={dim}>Product code</span>
            <span className="mono">{hex(d.productCode, 8)}</span>
            <span style={dim}>Revision</span>
            <span className="mono">{hex(d.revision, 8)}</span>
            <span style={dim}>Serial</span>
            <span className="mono">{d.serial}</span>
            <span style={dim}>Position</span>
            <span className="mono">{d.position}</span>
            <span style={dim}>Station address</span>
            <span className="mono">{hex(d.address, 4)}</span>
            <span style={dim}>Alias</span>
            <span className="mono">{d.alias || "none"}</span>
          </div>
        </div>
        <div style={card}>
          <strong>EtherCAT slave controller</strong>
          <div style={{ ...kv, marginTop: 8 }}>
            <span style={dim}>Type</span>
            <span className="mono">{hex(d.escType, 2)} {d.escName}</span>
            <span style={dim}>Revision, build</span>
            <span className="mono">{d.escRevision}, {d.escBuild}</span>
            <span style={dim}>FMMUs, SMs</span>
            <span className="mono">{d.fmmus}, {d.syncManagers}</span>
            <span style={dim}>Process RAM</span>
            <span className="mono">{d.ramKb} KB</span>
            <span style={dim}>Distributed clocks</span>
            <span>{d.dc}{d.dc !== "none" ? `, propagation delay ${d.propagationDelayNs} ns` : ""}</span>
            <span style={dim}>Mailbox</span>
            <span>{d.mailbox.length ? d.mailbox.join(", ") : "none"}{d.coe ? (d.sdoInfo ? " (SDO Information)" : " (no SDO Information)") : ""}</span>
          </div>
        </div>
        <div style={card}>
          <strong>Ports</strong>
          <div style={{ display: "grid", gridTemplateColumns: "repeat(4, 1fr)", gap: 6, marginTop: 8 }}>
            {d.ports.map((pt, i) => (
              <div key={i} style={{ border: "1px solid var(--border)", borderRadius: "var(--radius)", padding: 6, opacity: pt.kind === "not implemented" ? 0.45 : 1 }}>
                <div style={{ fontWeight: 600, fontSize: "var(--fs-sm)" }}>Port {i}</div>
                <div style={hint}>{pt.kind}</div>
                <Badge tone={pt.communication ? "success" : pt.link ? "warn" : "neutral"}>
                  {pt.communication ? "comm" : pt.link ? "link" : pt.loopClosed ? "closed" : "open"}
                </Badge>
              </div>
            ))}
          </div>
          <span style={{ ...hint, display: "block", marginTop: 6 }}>Port 0 points to the master. A closed last port is normal at the end of a line.</span>
        </div>
      </div>
    </div>
  );
}

function ProcessData({ device, pdo, image, pro, requirePro, addLog, state, trending, onToggleTrend }: Props) {
  if (!pdo) return <EmptyState title="Reading the PDO mapping" description="It comes from the CoE assignment objects, or from the EEPROM for devices without CoE." />;
  const inSafe = state?.state === "SAFE-OP" || state?.state === "OP";
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8, flex: 1, minHeight: 0 }}>
      <div style={row}>
        <Badge>Mapping from {pdo.source}</Badge>
        {!image?.inputs.length && !image?.outputs.length && (
          <span style={hint}>{inSafe ? "Waiting for process data" : "Values appear once the bus is in SAFE-OP (inputs) or OP (outputs)."}</span>
        )}
      </div>
      <div style={scroller}>
        <EntryTable
          position={device.position}
          title="Inputs (TxPDO)"
          pdos={pdo.inputs}
          img={image?.inputs ?? []}
          trending={trending}
          onToggleTrend={onToggleTrend}
          deviceName={device.name}
        />
        <EntryTable
          position={device.position}
          title="Outputs (RxPDO)"
          pdos={pdo.outputs}
          img={image?.outputs ?? []}
          writable={state?.state === "OP"}
          pro={pro}
          requirePro={requirePro}
          addLog={addLog}
          trending={trending}
          onToggleTrend={onToggleTrend}
          deviceName={device.name}
        />
      </div>
    </div>
  );
}

export function EntryTable(props: {
  position: number;
  deviceName: string;
  title: string;
  pdos: PdoMapView["inputs"];
  img: number[];
  writable?: boolean;
  pro?: boolean;
  requirePro?: RequirePro;
  addLog?: AddLog;
  trending: Set<string>;
  onToggleTrend: (key: string, label: string) => void;
}) {
  const { pdos, img } = props;
  if (pdos.length === 0) return <div style={{ ...hint, padding: 10 }}>{props.title}: none</div>;
  return (
    <table className="vt-table" style={{ width: "100%" }}>
      <thead>
        <tr>
          <th style={{ ...th, width: "34%" }}>{props.title}</th>
          <th style={{ ...th, width: 90 }}>Object</th>
          <th style={{ ...th, width: 92 }}>Type</th>
          <th style={{ ...th, width: 70, textAlign: "right" }}>Bit</th>
          <th style={th}>Value</th>
          <th style={{ ...th, width: 32 }} />
        </tr>
      </thead>
      <tbody>
        {pdos.map((pd) => [
          <tr key={`p${pd.index}`}>
            <td colSpan={6} style={{ fontWeight: 600, background: "var(--bg-subtle, transparent)" }}>
              <span className="mono">{hex(pd.index, 4)}</span> {pd.name}
            </td>
          </tr>,
          ...pd.entries.map((e, i) =>
            e.index === 0 ? (
              <tr key={`${pd.index}-${i}`} style={{ opacity: 0.5 }}>
                <td style={dim}>gap</td>
                <td />
                <td>{e.bits} bits</td>
                <td className="mono" style={{ textAlign: "right" }}>{e.bitOffset}</td>
                <td />
                <td />
              </tr>
            ) : (
              <EntryRow key={`${pd.index}-${i}`} entry={e} {...props} />
            ),
          ),
        ])}
      </tbody>
    </table>
  );
}

function EntryRow({ entry: e, img, writable, pro, requirePro, addLog, position, trending, onToggleTrend, deviceName }: {
  entry: PdoEntryView;
  img: number[];
  writable?: boolean;
  pro?: boolean;
  requirePro?: RequirePro;
  addLog?: AddLog;
  position: number;
  deviceName: string;
  trending: Set<string>;
  onToggleTrend: (key: string, label: string) => void;
}) {
  const [draft, setDraft] = useState("");
  const have = img.length * 8 >= e.bitOffset + e.bits;
  const raw = have ? extractBits(img, e.bitOffset, e.bits) : null;
  const value = raw === null ? "" : entryValue(e, raw);
  const isBool = e.bits === 1;
  const key = `${position}:${e.index}:${e.sub}:${e.bitOffset}`;
  const write = async (text: string) => {
    if (!requirePro?.("Writing outputs")) return;
    try {
      await invoke("ecat_write_output", { position, bitOffset: e.bitOffset, bits: e.bits, dataType: e.dataType, text });
      addLog?.("tx", `${deviceName} ${e.name} = ${text}`);
      setDraft("");
    } catch (err) {
      addLog?.("error", `${e.name}: ${String(err)}`);
    }
  };
  let extra = "";
  if (raw !== null && e.index === 0x6041 && e.bits === 16) extra = ds402State(Number(raw));
  return (
    <tr>
      <td>{e.name}</td>
      <td className="mono">{hex(e.index, 4).slice(2)}:{e.sub.toString(16).toUpperCase().padStart(2, "0")}</td>
      <td>{e.typeName}</td>
      <td className="mono" style={{ textAlign: "right" }}>{e.bitOffset}</td>
      <td>
        <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
          <span className="mono" style={{ minWidth: 70 }}>{value}</span>
          {extra && <Badge tone="info">{extra}</Badge>}
          {writable !== undefined && (
            isBool ? (
              <Button size="sm" disabled={!writable} onClick={() => void write(value === "TRUE" ? "0" : "1")}>
                {value === "TRUE" ? "Off" : "On"}{!pro && <ProBadge />}
              </Button>
            ) : (
              <>
                <input
                  className="vt-input vt-input--mono"
                  style={{ width: 110 }}
                  aria-label={`New value for ${e.name}`}
                  placeholder={writable ? "new value" : "OP only"}
                  disabled={!writable}
                  value={draft}
                  onChange={(ev) => setDraft(ev.target.value)}
                  onKeyDown={(ev) => { if (ev.key === "Enter" && draft.trim()) void write(draft); }}
                />
                <Button size="sm" disabled={!writable || !draft.trim()} onClick={() => void write(draft)}>Set{!pro && <ProBadge />}</Button>
              </>
            )
          )}
        </div>
      </td>
      <td>
        <button
          type="button"
          className="vt-btn vt-btn--sm vt-btn--ghost"
          aria-label={`Trend ${e.name}`}
          title="Trend"
          onClick={() => onToggleTrend(key, `${deviceName} ${e.name}`)}
          style={{ color: trending.has(key) ? "var(--accent-text)" : undefined }}
        >
          <TrendingUp size={14} />
        </button>
      </td>
    </tr>
  );
}

function Objects({ device, pro, requirePro, addLog }: Props) {
  const [objects, setObjects] = useState<ObjectView[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [open, setOpen] = useState<Map<number, EntryView[] | "loading">>(new Map());
  const [filter, setFilter] = useState("");
  const [manual, setManual] = useState({ index: "", sub: "0" });
  const [manualResult, setManualResult] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setObjects(await invoke<ObjectView[]>("ecat_objects", { position: device.position }));
      setOpen(new Map());
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [device.position]);

  useEffect(() => {
    setObjects(null);
    setOpen(new Map());
    setError(null);
    if (device.sdoInfo) void load();
  }, [device.position, device.sdoInfo, load]);

  const expand = async (index: number) => {
    const next = new Map(open);
    if (next.has(index)) {
      next.delete(index);
      setOpen(next);
      return;
    }
    next.set(index, "loading");
    setOpen(next);
    try {
      const entries = await invoke<EntryView[]>("ecat_object_entries", { position: device.position, index });
      setOpen((m) => new Map(m).set(index, entries));
    } catch (e) {
      addLog("error", String(e));
      setOpen((m) => { const n = new Map(m); n.delete(index); return n; });
    }
  };

  const writeEntry = async (e: EntryView, text: string) => {
    if (!requirePro("SDO writes")) return;
    try {
      await invoke("ecat_sdo_write", { position: device.position, index: e.index, sub: e.sub, dataType: e.dataType, bits: e.bits, text });
      const entries = await invoke<EntryView[]>("ecat_object_entries", { position: device.position, index: e.index });
      setOpen((m) => new Map(m).set(e.index, entries));
    } catch (err) {
      addLog("error", `SDO ${hex(e.index, 4)}:${e.sub}: ${String(err)}`);
    }
  };

  const readManual = async () => {
    const index = parseAddr(manual.index);
    const sub = Number(manual.sub);
    if (index === null || !Number.isInteger(sub) || sub < 0 || sub > 255) {
      setManualResult("Enter an index in hex (e.g. 1018) and a sub-index 0-255");
      return;
    }
    try {
      const v = await invoke<{ raw: string; value: string }>("ecat_sdo_read", { position: device.position, index, sub, dataType: 0 });
      setManualResult(`${hex(index, 4)}:${sub} = ${v.raw}`);
    } catch (e) {
      setManualResult(String(e));
    }
  };

  const shown = useMemo(() => {
    const f = filter.trim().toLowerCase();
    return (objects ?? []).filter((o) => !f || o.name.toLowerCase().includes(f) || hex(o.index, 4).toLowerCase().includes(f));
  }, [objects, filter]);

  if (!device.coe) return <EmptyState title="No CoE mailbox" description="This device has no CANopen over EtherCAT mailbox, so it has no object dictionary. Its process data layout comes from its EEPROM (Process data tab)." />;

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8, flex: 1, minHeight: 0 }}>
      <div style={row}>
        {device.sdoInfo ? (
          <>
            <Button size="sm" icon={<RefreshCw size={14} strokeWidth={1.75} />} disabled={loading} onClick={() => void load()}>{loading ? "Reading" : "Reload"}</Button>
            <input className="vt-input" style={{ width: 200 }} aria-label="Filter objects" placeholder="Filter: name or index" value={filter} onChange={(e) => setFilter(e.target.value)} />
            {objects && <Badge>{shown.length} of {objects.length} objects</Badge>}
          </>
        ) : (
          <span style={hint}>This device has no SDO Information service, so its object list cannot be read. Read entries by index below.</span>
        )}
        <span style={{ flex: 1 }} />
        <input className="vt-input vt-input--mono" style={{ width: 80 }} aria-label="Index" placeholder="index" value={manual.index} onChange={(e) => setManual({ ...manual, index: e.target.value })} />
        <input className="vt-input vt-input--mono" style={{ width: 54 }} aria-label="Sub-index" placeholder="sub" value={manual.sub} onChange={(e) => setManual({ ...manual, sub: e.target.value })} />
        <Button size="sm" onClick={() => void readManual()}>Read SDO</Button>
      </div>
      {manualResult && <Callout>{manualResult}</Callout>}
      {error && <Callout tone="danger">{error}</Callout>}
      {objects && (
        <div style={scroller}>
          <table className="vt-table" style={{ width: "100%" }}>
            <thead>
              <tr>
                <th style={{ ...th, width: 24 }} />
                <th style={{ ...th, width: 90 }}>Index</th>
                <th style={th}>Name</th>
                <th style={{ ...th, width: 120 }}>Type</th>
                <th style={{ ...th, width: 120 }}>Access</th>
                <th style={{ ...th, width: "32%" }}>Value</th>
              </tr>
            </thead>
            <tbody>
              {shown.map((o) => {
                const ent = open.get(o.index);
                return [
                  <tr key={o.index} onClick={() => void expand(o.index)} style={{ cursor: "pointer" }}>
                    <td>{ent ? <ChevronDown size={14} /> : <ChevronRight size={14} />}</td>
                    <td className="mono">{hex(o.index, 4)}</td>
                    <td>{o.name}</td>
                    <td>{o.objectCode === "VAR" ? o.typeName : `${o.objectCode} (${o.maxSub})`}</td>
                    <td />
                    <td />
                  </tr>,
                  ...(ent === "loading"
                    ? [<tr key={`${o.index}-l`}><td /><td colSpan={5} style={hint}>Reading</td></tr>]
                    : (ent ?? []).map((e) => <ObjectEntryRow key={`${o.index}:${e.sub}`} entry={e} pro={pro} onWrite={writeEntry} />)),
                ];
              })}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}

function ObjectEntryRow({ entry: e, pro, onWrite }: { entry: EntryView; pro: boolean; onWrite: (e: EntryView, text: string) => void }) {
  const [draft, setDraft] = useState<string | null>(null);
  return (
    <tr>
      <td />
      <td className="mono" style={dim}>:{e.sub.toString(16).toUpperCase().padStart(2, "0")}</td>
      <td>{e.name}</td>
      <td>{e.typeName}{e.bits && !e.typeName.startsWith("BIT") ? <span style={dim}> {e.bits}b</span> : null}</td>
      <td style={hint}>{e.access}</td>
      <td>
        {draft === null ? (
          <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
            <span className="mono" title={e.raw ?? undefined}>{e.error ? <span style={{ color: "var(--danger)" }}>{e.error}</span> : e.value}</span>
            <span style={{ flex: 1 }} />
            {e.writable && (
              <Button size="sm" variant="ghost" onClick={() => setDraft(e.value ?? "")}>Edit{!pro && <ProBadge />}</Button>
            )}
          </div>
        ) : (
          <div style={{ display: "flex", gap: 6 }}>
            <input
              className="vt-input vt-input--mono"
              autoFocus
              aria-label={`New value for ${e.name}`}
              value={draft}
              onChange={(ev) => setDraft(ev.target.value)}
              onKeyDown={(ev) => {
                if (ev.key === "Enter") { onWrite(e, draft); setDraft(null); }
                if (ev.key === "Escape") setDraft(null);
              }}
            />
            <Button size="sm" variant="primary" onClick={() => { onWrite(e, draft); setDraft(null); }}>Write</Button>
            <Button size="sm" variant="ghost" onClick={() => setDraft(null)}>Cancel</Button>
          </div>
        )}
      </td>
    </tr>
  );
}

function Registers({ device, pro, requirePro, addLog }: Props) {
  const [view, setView] = useState<RegistersView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [auto, setAuto] = useState(false);
  const [w, setW] = useState({ addr: "", hex: "" });

  const load = useCallback(async () => {
    try {
      setView(await invoke<RegistersView>("ecat_registers", { position: device.position }));
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, [device.position]);

  useEffect(() => { void load(); }, [load]);
  useEffect(() => {
    if (!auto) return;
    const t = setInterval(() => void load(), 1000);
    return () => clearInterval(t);
  }, [auto, load]);

  const resetCounters = async () => {
    if (!requirePro("Resetting error counters")) return;
    try { await invoke("ecat_reset_counters", { position: device.position }); await load(); } catch (e) { addLog("error", String(e)); }
  };

  const writeReg = async () => {
    if (!requirePro("Register writes")) return;
    const addr = parseAddr(w.addr);
    if (addr === null) { addLog("error", "Register address in hex, e.g. 0120"); return; }
    try {
      await invoke("ecat_write_register", { position: device.position, addr, hex: w.hex });
      await load();
    } catch (e) {
      addLog("error", String(e));
    }
  };

  if (error) return <Callout tone="danger">{error}</Callout>;
  if (!view) return <EmptyState title="Reading registers" />;
  const errorsTotal = view.errorCounters.reduce((n, [, v]) => n + v, 0);
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8, flex: 1, minHeight: 0 }}>
      <div style={row}>
        <Button size="sm" icon={<RefreshCw size={14} strokeWidth={1.75} />} onClick={() => void load()}>Read</Button>
        <label style={{ display: "flex", gap: 6, alignItems: "center", fontSize: "var(--fs-sm)" }}>
          <input type="checkbox" checked={auto} onChange={(e) => setAuto(e.target.checked)} /> Every second
        </label>
        <span style={{ flex: 1 }} />
        <input className="vt-input vt-input--mono" style={{ width: 80 }} aria-label="Register address" placeholder="addr" value={w.addr} onChange={(e) => setW({ ...w, addr: e.target.value })} />
        <input className="vt-input vt-input--mono" style={{ width: 140 }} aria-label="Bytes to write" placeholder="bytes, e.g. 02 00" value={w.hex} onChange={(e) => setW({ ...w, hex: e.target.value })} />
        <Button size="sm" disabled={!w.addr || !w.hex} onClick={() => void writeReg()}>Write{!pro && <ProBadge />}</Button>
      </div>
      <div style={{ ...scroller, display: "flex", flexDirection: "column", gap: 12, padding: 8 }}>
        <div>
          <div style={{ ...row, marginBottom: 6 }}>
            <strong>Error counters</strong>
            <Badge tone={errorsTotal ? "warn" : "success"}>{errorsTotal ? `${errorsTotal} counted` : "all zero"}</Badge>
            <span style={{ flex: 1 }} />
            <Button size="sm" icon={<RotateCcw size={14} strokeWidth={1.75} />} onClick={() => void resetCounters()}>Reset{!pro && <ProBadge />}</Button>
          </div>
          <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fill, minmax(190px, 1fr))", gap: 4 }}>
            {view.errorCounters.map(([n, v]) => (
              <div key={n} style={{ display: "flex", justifyContent: "space-between", fontSize: "var(--fs-sm)", padding: "2px 6px", borderRadius: 4, background: v ? "color-mix(in srgb, var(--warn) 18%, transparent)" : undefined }}>
                <span style={dim}>{n}</span>
                <span className="mono">{v}</span>
              </div>
            ))}
          </div>
          <span style={hint}>RX errors on a port point at the cable or connector before that port; lost links count unplugs and power loss.</span>
        </div>
        <table className="vt-table" style={{ width: "100%" }}>
          <thead>
            <tr>
              <th style={{ ...th, width: 70 }}>Address</th>
              <th style={th}>Register</th>
              <th style={{ ...th, width: 170 }}>Hex</th>
              <th style={{ ...th, width: "36%" }}>Meaning</th>
            </tr>
          </thead>
          <tbody>
            {view.registers.map((r) => (
              <tr key={r.addr}>
                <td className="mono">{hex(r.addr, 4).slice(2)}</td>
                <td>{r.name}</td>
                <td className="mono" style={{ wordBreak: "break-all" }}>{r.hex}</td>
                <td style={hint}>{r.decoded}</td>
              </tr>
            ))}
          </tbody>
        </table>
        <div>
          <strong>Sync managers</strong>
          <table className="vt-table" style={{ width: "100%", marginTop: 4 }}>
            <thead>
              <tr><th>SM</th><th>Start</th><th>Length</th><th>Mode</th><th>Direction</th><th>Control</th><th>Status</th><th>Enabled</th></tr>
            </thead>
            <tbody>
              {view.syncManagers.map((s) => (
                <tr key={s.index}>
                  <td>{s.index}</td>
                  <td className="mono">{hex(s.start, 4)}</td>
                  <td className="mono">{s.length}</td>
                  <td>{s.mode}</td>
                  <td>{s.direction}</td>
                  <td className="mono">{hex(s.control, 2)}</td>
                  <td className="mono">{hex(s.status, 2)}{s.mailboxFull ? " (full)" : ""}</td>
                  <td>{s.enabled ? "yes" : "no"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        <div>
          <strong>FMMUs</strong>
          <table className="vt-table" style={{ width: "100%", marginTop: 4 }}>
            <thead>
              <tr><th>FMMU</th><th>Logical start</th><th>Length</th><th>Bits</th><th>Physical</th><th>Type</th><th>Active</th></tr>
            </thead>
            <tbody>
              {view.fmmus.map((f) => (
                <tr key={f.index} style={{ opacity: f.active ? 1 : 0.5 }}>
                  <td>{f.index}</td>
                  <td className="mono">{hex(f.logicalStart, 8)}</td>
                  <td className="mono">{f.length}</td>
                  <td className="mono">{f.logicalBits[0]}..{f.logicalBits[1]}</td>
                  <td className="mono">{hex(f.physicalStart, 4)}.{f.physicalBit}</td>
                  <td>{f.kind}</td>
                  <td>{f.active ? "yes" : "no"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>
    </div>
  );
}

function Eeprom({ device, requirePro, addLog, pro }: Props) {
  const [sii, setSii] = useState<Sii | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    setSii(null);
    invoke<Sii>("ecat_sii", { position: device.position }).then(setSii).catch((e) => setError(String(e)));
  }, [device.position]);

  const save = async () => {
    if (!requirePro("Saving the EEPROM")) return;
    try {
      const bytes = await invoke<number[]>("ecat_eeprom", { position: device.position });
      const path = await pickSavePath(`${device.name || "device"}-${device.serial || device.position}.bin`, { name: "EEPROM image", extensions: ["bin"] });
      if (!path) return;
      await invoke("save_bytes", { path, bytes });
      addLog("info", `EEPROM of ${device.name} saved to ${path} (${bytes.length} bytes)`);
    } catch (e) {
      addLog("error", String(e));
    }
  };

  if (error) return <Callout tone="danger">{error}</Callout>;
  if (!sii) return <EmptyState title="Reading the EEPROM" description="The SII is read 8 bytes at a time through the ESC; a 2 KB EEPROM takes a moment on a real network." />;
  const g = sii.general;
  return (
    <div style={{ overflow: "auto", display: "flex", flexDirection: "column", gap: 8 }}>
      <div style={row}>
        <Badge tone={sii.checksumOk ? "success" : "danger"}>{sii.checksumOk ? "Header checksum OK" : "Header checksum wrong"}</Badge>
        <Badge>{sii.size} bytes, SII version {sii.version}</Badge>
        <span style={{ flex: 1 }} />
        <Button size="sm" icon={<Download size={14} strokeWidth={1.75} />} onClick={() => void save()}>Save .bin{!pro && <ProBadge />}</Button>
      </div>
      <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(300px, 1fr))", gap: 8 }}>
        <div style={card}>
          <strong>General</strong>
          <div style={{ ...kv, marginTop: 8 }}>
            <span style={dim}>Name</span><span>{g?.name}</span>
            <span style={dim}>Order number</span><span>{g?.order}</span>
            <span style={dim}>Group</span><span>{g?.group}</span>
            <span style={dim}>E-bus current</span><span>{g ? `${g.ebusCurrent} mA` : ""}</span>
            <span style={dim}>CiA 402 axes</span><span>{g?.ds402Channels ?? 0}</span>
            <span style={dim}>Configured alias</span><span className="mono">{sii.alias}</span>
          </div>
        </div>
        <div style={card}>
          <strong>Mailbox</strong>
          <div style={{ ...kv, marginTop: 8 }}>
            <span style={dim}>Master to device</span><span className="mono">{hex(sii.stdRx[0], 4)}, {sii.stdRx[1]} bytes</span>
            <span style={dim}>Device to master</span><span className="mono">{hex(sii.stdTx[0], 4)}, {sii.stdTx[1]} bytes</span>
            <span style={dim}>Bootstrap</span><span className="mono">{hex(sii.bootRx[0], 4)}/{sii.bootRx[1]}, {hex(sii.bootTx[0], 4)}/{sii.bootTx[1]}</span>
            <span style={dim}>Categories</span><span>{sii.categories.map(([t, n]) => `${t} (${n * 2} B)`).join(", ")}</span>
          </div>
        </div>
      </div>
      <div style={card}>
        <strong>Sync managers (defaults)</strong>
        <table className="vt-table" style={{ width: "100%", marginTop: 4 }}>
          <thead><tr><th>SM</th><th>Start</th><th>Length</th><th>Control</th><th>Use</th></tr></thead>
          <tbody>
            {sii.syncManagers.map((s, i) => (
              <tr key={i}><td>{i}</td><td className="mono">{hex(s.start, 4)}</td><td className="mono">{s.len}</td><td className="mono">{hex(s.control, 2)}</td><td>{s.kind}</td></tr>
            ))}
          </tbody>
        </table>
      </div>
      <div style={card}>
        <strong>Strings</strong>
        <ol style={{ margin: "6px 0 0", paddingLeft: 22, fontSize: "var(--fs-sm)" }}>
          {sii.strings.map((s, i) => <li key={i} className="mono">{s}</li>)}
        </ol>
      </div>
    </div>
  );
}
