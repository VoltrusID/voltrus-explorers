import { useCallback, useEffect, useRef, useState } from "react";
import { Badge, Button, Callout, Dialog, EmptyState, Field, IconButton, StatusDot } from "@mactools/ui";
import { Activity, Blocks, Eye, EyeOff, Plug, Play, Radar, ScrollText, Square, Unplug, X } from "lucide-react";
import { bridgeMode } from "../../api/bridge";
import {
  defaultTypeFor,
  loadTarget,
  loadWatchRows,
  modeTone,
  s7,
  saveTarget,
  saveWatchRows,
  WATCH_ROWS_EVENT,
  WATCH_TYPES,
  type BlockEntry,
  type DiagBuffer,
  type FoundDevice,
  type OnlineInfo,
  type S7Target,
  type WatchResult,
  type WatchRow,
} from "../../lib/s7";

const STATUS_POLL_MS = 2000;
const WATCH_POLL_MS = 500;

function errorText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/**
 * Online with a Siemens S7 CPU over S7comm: accessible devices, CPU identity and mode, RUN/STOP,
 * a watch table with monitor and modify, the diagnostic buffer and the block list.
 */
export default function OnlinePanel() {
  if (bridgeMode !== "tauri") {
    return (
      <EmptyState
        compact
        icon={<Plug size={18} strokeWidth={1.75} />}
        title="Desktop app only"
        description="Going online with an S7 CPU needs a direct network connection. Use the Voltrus PLC desktop app."
      />
    );
  }
  return <OnlinePanelDesktop />;
}

function OnlinePanelDesktop() {
  const [target, setTarget] = useState<S7Target>(loadTarget);
  const [info, setInfo] = useState<OnlineInfo | null>(null);
  const [mode, setMode] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [scanOpen, setScanOpen] = useState(false);
  const [confirm, setConfirm] = useState<null | "run" | "stop">(null);

  const online = info !== null;

  // Pick up a connection that is still open (the panel was closed and reopened).
  useEffect(() => {
    s7.status().then((st) => {
      if (st.online && st.target) {
        setInfo((cur) => cur ?? { target: st.target!, pdu_size: 0, cpu: null, cpu_error: null, mode: st.mode });
        setMode(st.mode);
      }
    }).catch(() => {});
  }, []);

  // Poll the operating mode while online; a lost connection takes the panel offline.
  useEffect(() => {
    if (!online) return;
    const id = window.setInterval(async () => {
      try {
        const st = await s7.status();
        if (!st.online) {
          setInfo(null);
          setMode(null);
          setError(st.error ?? "Connection to the CPU was lost");
        } else if (st.mode) {
          setMode(st.mode);
        }
      } catch { /* next poll */ }
    }, STATUS_POLL_MS);
    return () => window.clearInterval(id);
  }, [online]);

  const goOnline = async (t: S7Target = target) => {
    setBusy(true);
    setError(null);
    try {
      const result = await s7.goOnline(t);
      setInfo(result);
      setMode(result.mode);
      setTarget(t);
      saveTarget(t);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const goOffline = async () => {
    await s7.goOffline().catch(() => {});
    setInfo(null);
    setMode(null);
  };

  const changeMode = async (run: boolean, cold: boolean) => {
    setConfirm(null);
    setBusy(true);
    setError(null);
    try {
      setMode(await s7.setMode(run, cold));
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="s7-panel">
      <section className="vt-section">
        <div className="vt-section__header">
          <span className="vt-section__title">Connection</span>
          {online && <StatusDot tone="success" label="Online" />}
        </div>
        <div className="s7-target">
          <Field label="IP address" stacked>
            {(id) => (
              <input
                id={id}
                className="vt-input vt-input--mono vt-input--sm"
                value={target.host}
                placeholder="192.168.0.1 or host:port"
                disabled={online || busy}
                onChange={(e) => setTarget({ ...target, host: e.target.value })}
                onKeyDown={(e) => { if (e.key === "Enter" && !online) void goOnline(); }}
              />
            )}
          </Field>
          <Field label="Rack" stacked>
            {(id) => (
              <input id={id} className="vt-input vt-input--mono vt-input--sm" type="number" min={0} max={7}
                value={target.rack} disabled={online || busy}
                onChange={(e) => setTarget({ ...target, rack: Number(e.target.value) || 0 })} />
            )}
          </Field>
          <Field label="Slot" stacked>
            {(id) => (
              <input id={id} className="vt-input vt-input--mono vt-input--sm" type="number" min={0} max={31}
                value={target.slot} disabled={online || busy}
                onChange={(e) => setTarget({ ...target, slot: Number(e.target.value) || 0 })} />
            )}
          </Field>
        </div>
        <div className="s7-row">
          {online ? (
            <Button size="sm" icon={<Unplug size={14} />} onClick={goOffline}>Go offline</Button>
          ) : (
            <Button size="sm" variant="primary" icon={<Plug size={14} />} loading={busy}
              disabled={!target.host.trim()} onClick={() => goOnline()}>
              Go online
            </Button>
          )}
          <Button size="sm" variant="ghost" icon={<Radar size={14} />} disabled={online} onClick={() => setScanOpen(true)}>
            Accessible devices
          </Button>
        </div>
        {error && <Callout tone="danger">{error}</Callout>}
      </section>

      {online && info && (
        <>
          <section className="vt-section">
            <div className="vt-section__header">
              <span className="vt-section__title">CPU</span>
              <Badge tone={modeTone(mode)} mono>{mode ?? "?"}</Badge>
            </div>
            <CpuDetails info={info} />
            <div className="s7-row">
              <Button size="sm" icon={<Play size={14} />} disabled={busy || mode === "RUN"} onClick={() => setConfirm("run")}>
                RUN
              </Button>
              <Button size="sm" variant="danger" icon={<Square size={14} />} disabled={busy || mode === "STOP"}
                onClick={() => setConfirm("stop")}>
                STOP
              </Button>
            </div>
          </section>
          <WatchSection />
          <DiagSection />
          <BlocksSection />
        </>
      )}

      {!online && (
        <Callout tone="info">
          Works with S7-300/400, and with S7-1200/1500 when PUT/GET access is allowed in the CPU's
          protection settings. Watch addresses use absolute notation (MW10, DB1.DBD4, I0.0);
          optimized DBs cannot be reached this way.
        </Callout>
      )}

      <ModeDialog
        action={confirm}
        target={info?.target.host ?? ""}
        onCancel={() => setConfirm(null)}
        onConfirm={changeMode}
      />
      <ScanDialog
        open={scanOpen}
        onClose={() => setScanOpen(false)}
        onPick={(d) => {
          setScanOpen(false);
          const t = { host: d.ip, port: d.port, rack: d.rack ?? target.rack, slot: d.slot ?? target.slot };
          setTarget(t);
          void goOnline(t);
        }}
      />
    </div>
  );
}

function CpuDetails({ info }: { info: OnlineInfo }) {
  const cpu = info.cpu;
  if (!cpu) {
    return <p className="s7-muted">{info.cpu_error ? `Identity not available: ${info.cpu_error}` : "Identity not read yet"}</p>;
  }
  const rows: [string, string][] = [
    ["Module", cpu.module_type],
    ["Order no.", cpu.order_code],
    ["Firmware", cpu.firmware_version],
    ["Station", cpu.as_name],
    ["Name", cpu.module_name],
    ["Serial no.", cpu.serial_number],
  ];
  return (
    <dl className="s7-kv">
      {rows.filter(([, v]) => v).map(([k, v]) => (
        <div key={k}><dt>{k}</dt><dd>{v}</dd></div>
      ))}
      {info.pdu_size > 0 && <div><dt>PDU</dt><dd>{info.pdu_size} bytes</dd></div>}
    </dl>
  );
}

function ModeDialog({ action, target, onCancel, onConfirm }: {
  action: null | "run" | "stop";
  target: string;
  onCancel: () => void;
  onConfirm: (run: boolean, cold: boolean) => void;
}) {
  const [cold, setCold] = useState(false);
  useEffect(() => { if (action) setCold(false); }, [action]);
  const stop = action === "stop";
  return (
    <Dialog
      open={action !== null}
      onClose={onCancel}
      title={stop ? "Stop the CPU?" : "Start the CPU?"}
      width={420}
      footer={
        <>
          <Button size="sm" variant="ghost" onClick={onCancel}>Cancel</Button>
          <Button size="sm" variant={stop ? "danger" : "primary"} onClick={() => onConfirm(!stop, cold)}>
            {stop ? "Stop CPU" : cold ? "Cold restart" : "Warm restart"}
          </Button>
        </>
      }
    >
      {stop ? (
        <p>
          The CPU at <code>{target}</code> stops executing its program and its outputs go to their
          configured substitute values. Make sure the machine is in a safe state.
        </p>
      ) : (
        <>
          <p>The CPU at <code>{target}</code> starts executing its program and drives its outputs.</p>
          <label className="s7-check">
            <input type="checkbox" checked={cold} onChange={(e) => setCold(e.target.checked)} />
            Cold restart (reset retentive data to start values)
          </label>
        </>
      )}
    </Dialog>
  );
}

function ScanDialog({ open, onClose, onPick }: { open: boolean; onClose: () => void; onPick: (d: FoundDevice) => void }) {
  const [subnet, setSubnet] = useState("");
  const [scanning, setScanning] = useState(false);
  const [devices, setDevices] = useState<FoundDevice[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (open && !subnet) s7.suggestSubnet().then((s) => { if (s) setSubnet(s); }).catch(() => {});
  }, [open, subnet]);

  const scan = async () => {
    setScanning(true);
    setError(null);
    setDevices(null);
    try {
      setDevices(await s7.scan(subnet));
    } catch (e) {
      setError(errorText(e));
    } finally {
      setScanning(false);
    }
  };

  return (
    <Dialog open={open} onClose={onClose} title="Accessible devices" width={560}
      footer={<Button size="sm" variant="ghost" onClick={onClose}>Close</Button>}>
      <div className="s7-row">
        <input
          className="vt-input vt-input--mono vt-input--sm"
          style={{ flex: 1 }}
          aria-label="Subnet"
          value={subnet}
          placeholder="192.168.0.0/24"
          onChange={(e) => setSubnet(e.target.value)}
          onKeyDown={(e) => { if (e.key === "Enter") void scan(); }}
        />
        <Button size="sm" variant="primary" icon={<Radar size={14} />} loading={scanning} disabled={!subnet.trim()} onClick={scan}>
          Scan
        </Button>
      </div>
      <p className="s7-muted">
        Looks for hosts answering on TCP port 102 and asks each for its identity. CPUs without an IP
        address yet do not show up.
      </p>
      {error && <Callout tone="danger">{error}</Callout>}
      {devices && devices.length === 0 && <p className="s7-muted">No S7 devices found on {subnet}.</p>}
      {devices && devices.length > 0 && (
        <table className="vt-table s7-table">
          <thead>
            <tr><th>Address</th><th>Device</th><th>Rack/slot</th><th /></tr>
          </thead>
          <tbody>
            {devices.map((d) => (
              <tr key={d.ip}>
                <td className="mono">{d.ip}</td>
                <td>
                  <div>{d.module_type ?? d.module_name ?? "Unknown device"}</div>
                  <div className="s7-muted">{[d.station_name, d.order_code, d.firmware].filter(Boolean).join(" · ") || d.note}</div>
                </td>
                <td className="mono">{d.rack !== null ? `${d.rack}/${d.slot}` : "–"}</td>
                <td>
                  <Button size="sm" disabled={d.rack === null} onClick={() => onPick(d)}>Go online</Button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </Dialog>
  );
}

function WatchSection() {
  const [rows, setRows] = useState<WatchRow[]>(loadWatchRows);
  const [results, setResults] = useState<WatchResult[]>([]);
  const [monitoring, setMonitoring] = useState(false);
  const [newAddress, setNewAddress] = useState("");
  const [rowError, setRowError] = useState<Record<string, string>>({});
  const reading = useRef(false);

  const update = (next: WatchRow[]) => {
    setRows(next);
    saveWatchRows(next);
  };

  // Rows added by an import while the panel is open.
  useEffect(() => {
    const reload = () => setRows(loadWatchRows());
    window.addEventListener(WATCH_ROWS_EVENT, reload);
    return () => window.removeEventListener(WATCH_ROWS_EVENT, reload);
  }, []);

  const readOnce = useCallback(async () => {
    if (reading.current || rows.length === 0) return;
    reading.current = true;
    try {
      setResults(await s7.readWatch(rows));
    } catch (e) {
      setResults(rows.map(() => ({ value: null, error: errorText(e) })));
      setMonitoring(false);
    } finally {
      reading.current = false;
    }
  }, [rows]);

  useEffect(() => {
    if (!monitoring) return;
    void readOnce();
    const id = window.setInterval(readOnce, WATCH_POLL_MS);
    return () => window.clearInterval(id);
  }, [monitoring, readOnce]);

  const add = () => {
    const address = newAddress.trim().toUpperCase();
    if (!address) return;
    update([...rows, { id: crypto.randomUUID(), address, dataType: defaultTypeFor(address), modify: "" }]);
    setNewAddress("");
  };

  const modify = async (row: WatchRow) => {
    try {
      await s7.writeWatch(row, row.modify);
      setRowError(({ [row.id]: _, ...rest }) => rest);
      void readOnce();
    } catch (e) {
      setRowError((cur) => ({ ...cur, [row.id]: errorText(e) }));
    }
  };

  return (
    <section className="vt-section">
      <div className="vt-section__header">
        <span className="vt-section__title">Watch table</span>
        <span className="s7-row">
          {rows.length > 0 && !monitoring && (
            <Button size="sm" variant="ghost" onClick={() => { update([]); setResults([]); }}>Clear</Button>
          )}
          <Button size="sm" variant={monitoring ? "primary" : "default"} disabled={rows.length === 0}
            icon={monitoring ? <EyeOff size={14} /> : <Eye size={14} />}
            onClick={() => setMonitoring(!monitoring)}>
            {monitoring ? "Stop monitoring" : "Monitor all"}
          </Button>
        </span>
      </div>
      {rows.length > 0 && (
        <table className="vt-table s7-table s7-watch">
          <thead>
            <tr><th>Name / address</th><th>Type</th><th>Monitor value</th><th>Modify value</th><th /></tr>
          </thead>
          <tbody>
            {rows.map((row, i) => {
              const r = results[i];
              const err = rowError[row.id] ?? r?.error ?? null;
              return (
                <tr key={row.id}>
                  <td>
                    {row.name && <div className="s7-name" title={row.name}>{row.name}</div>}
                    <div className={row.name ? "mono s7-muted" : "mono"}>{row.address}</div>
                  </td>
                  <td>
                    <select className="vt-select vt-select--sm" aria-label={`Type of ${row.address}`} value={row.dataType}
                      onChange={(e) => update(rows.map((x) => x.id === row.id ? { ...x, dataType: e.target.value } : x))}>
                      {!row.dataType && <option value="">auto</option>}
                      {WATCH_TYPES.map((t) => <option key={t} value={t}>{t}</option>)}
                    </select>
                  </td>
                  <td className="mono" title={err ?? undefined}>
                    {err ? <span className="s7-error">{err}</span> : (r?.value ?? "")}
                  </td>
                  <td>
                    <input className="vt-input vt-input--mono vt-input--sm" aria-label={`Modify value for ${row.address}`}
                      value={row.modify}
                      onChange={(e) => update(rows.map((x) => x.id === row.id ? { ...x, modify: e.target.value } : x))}
                      onKeyDown={(e) => { if (e.key === "Enter" && row.modify.trim()) void modify(row); }} />
                  </td>
                  <td className="s7-actions">
                    <Button size="sm" disabled={!row.modify.trim()} title="Write the modify value now" onClick={() => modify(row)}>
                      Modify
                    </Button>
                    <IconButton size="sm" label={`Remove ${row.address}`} onClick={() => update(rows.filter((x) => x.id !== row.id))}>
                      <X size={14} />
                    </IconButton>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
      <div className="s7-row">
        <input
          className="vt-input vt-input--mono vt-input--sm"
          style={{ flex: 1 }}
          aria-label="Address to watch"
          placeholder="Add address: MW10, DB1.DBD4, Q0.1"
          value={newAddress}
          onChange={(e) => setNewAddress(e.target.value)}
          onKeyDown={(e) => { if (e.key === "Enter") add(); }}
        />
        <Button size="sm" disabled={!newAddress.trim()} onClick={add}>Add</Button>
      </div>
    </section>
  );
}

function DiagSection() {
  const [diag, setDiag] = useState<DiagBuffer | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const read = async () => {
    setLoading(true);
    setError(null);
    try { setDiag(await s7.diagBuffer()); } catch (e) { setError(errorText(e)); } finally { setLoading(false); }
  };
  return (
    <section className="vt-section">
      <div className="vt-section__header">
        <span className="vt-section__title">Diagnostic buffer</span>
        <Button size="sm" variant="ghost" icon={<ScrollText size={14} />} loading={loading} onClick={read}>
          {diag ? "Refresh" : "Read"}
        </Button>
      </div>
      {error && <Callout tone="danger">{error}</Callout>}
      {diag && diag.entries.length === 0 && <p className="s7-muted">The diagnostic buffer is empty.</p>}
      {diag && diag.entries.length > 0 && (
        <ol className="s7-diag">
          {diag.entries.map((e, i) => (
            <li key={i}>
              <span className="s7-diag__no">{i + 1}</span>
              <span className="s7-diag__time mono">{e.timestamp}</span>
              <span className="s7-diag__text">{e.event_text}</span>
              <span className="s7-diag__id mono">16#{e.event_number.toString(16).toUpperCase().padStart(4, "0")}</span>
            </li>
          ))}
        </ol>
      )}
    </section>
  );
}

function BlocksSection() {
  const [blocks, setBlocks] = useState<BlockEntry[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const read = async () => {
    setLoading(true);
    setError(null);
    try { setBlocks(await s7.listBlocks()); } catch (e) { setError(errorText(e)); } finally { setLoading(false); }
  };
  const groups = new Map<string, BlockEntry[]>();
  for (const b of blocks ?? []) groups.set(b.block_type, [...(groups.get(b.block_type) ?? []), b]);
  return (
    <section className="vt-section">
      <div className="vt-section__header">
        <span className="vt-section__title">Program blocks</span>
        <Button size="sm" variant="ghost" icon={<Blocks size={14} />} loading={loading} onClick={read}>
          {blocks ? "Refresh" : "Read"}
        </Button>
      </div>
      {error && <Callout tone="danger">{error}</Callout>}
      {blocks && blocks.length === 0 && <p className="s7-muted">No blocks listed.</p>}
      {[...groups.entries()].map(([type, list]) => (
        <div key={type} className="s7-blocks">
          <span className="s7-blocks__type">{type} <span className="s7-muted">({list.length})</span></span>
          <span className="s7-blocks__list mono">
            {list.map((b) => `${type}${b.block_number}${b.language ? ` ${b.language}` : ""}`).join(", ")}
          </span>
        </div>
      ))}
      {!blocks && !error && (
        <p className="s7-muted"><Activity size={12} /> Lists the blocks in the CPU (S7-300/400; S7-1200/1500 may refuse).</p>
      )}
    </section>
  );
}
