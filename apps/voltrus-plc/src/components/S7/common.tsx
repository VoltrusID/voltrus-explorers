// Pieces shared by the S7 views: the connection form, CPU identity, mode badge and the dialogs
// for RUN/STOP and accessible devices (rendered once, at the app root).
import { useEffect, useState } from "react";
import { Badge, Button, Callout, Dialog, Field } from "@mactools/ui";
import { Play, Plug, Radar, Square, Unplug } from "lucide-react";
import { modeTone, s7, type FoundDevice, type OnlineInfo } from "../../lib/s7";
import { errorText, useS7 } from "../../lib/s7Store";

export function ModeBadge({ mode }: { mode: string | null }) {
  return <Badge tone={modeTone(mode)} mono>{mode ?? "?"}</Badge>;
}

export function ConnectionForm({ compact = false }: { compact?: boolean }) {
  const { target, info, busy, error, setTarget, goOnline, goOffline, setScanOpen } = useS7();
  const online = info !== null;
  return (
    <div className="s7-connection">
      <div className={compact ? "s7-target" : "s7-target s7-target--wide"}>
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
          <Button size="sm" icon={<Unplug size={14} />} onClick={() => void goOffline()}>Go offline</Button>
        ) : (
          <Button size="sm" variant="primary" icon={<Plug size={14} />} loading={busy}
            disabled={!target.host.trim()} onClick={() => void goOnline()}>
            Go online
          </Button>
        )}
        <Button size="sm" variant="ghost" icon={<Radar size={14} />} disabled={online} onClick={() => setScanOpen(true)}>
          Accessible devices
        </Button>
      </div>
      {error && <Callout tone="danger">{error}</Callout>}
    </div>
  );
}

export function CpuDetails({ info, wide = false }: { info: OnlineInfo; wide?: boolean }) {
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
  if (wide) {
    rows.push(
      ["Address", `${info.target.host}${info.target.port !== 102 ? `:${info.target.port}` : ""}`],
      ["Rack / slot", `${info.target.rack} / ${info.target.slot}`],
    );
    if (cpu.copyright) rows.push(["Copyright", cpu.copyright]);
  }
  return (
    <dl className={wide ? "s7-kv s7-kv--wide" : "s7-kv"}>
      {rows.filter(([, v]) => v).map(([k, v]) => (
        <div key={k}><dt>{k}</dt><dd>{v}</dd></div>
      ))}
      {info.pdu_size > 0 && <div><dt>PDU</dt><dd>{info.pdu_size} bytes</dd></div>}
    </dl>
  );
}

export function ModeButtons() {
  const { mode, busy, requestMode } = useS7();
  return (
    <div className="s7-row">
      <Button size="sm" icon={<Play size={14} />} disabled={busy || mode === "RUN"} onClick={() => requestMode("run")}>
        RUN
      </Button>
      <Button size="sm" variant="danger" icon={<Square size={14} />} disabled={busy || mode === "STOP"}
        onClick={() => requestMode("stop")}>
        STOP
      </Button>
    </div>
  );
}

/** RUN/STOP confirmation and the accessible-devices scan. Mounted once by the app. */
export function S7Dialogs() {
  const { confirm, info, scanOpen, target, cancelMode, setMode, setScanOpen, goOnline } = useS7();
  return (
    <>
      <ModeDialog action={confirm} target={info?.target.host ?? ""} onCancel={cancelMode} onConfirm={(run, cold) => void setMode(run, cold)} />
      <ScanDialog
        open={scanOpen}
        onClose={() => setScanOpen(false)}
        onPick={(d) => {
          setScanOpen(false);
          void goOnline({ host: d.ip, port: d.port, rack: d.rack ?? target.rack, slot: d.slot ?? target.slot });
        }}
      />
    </>
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

/** S7 online is new and has only been tested against S7 simulators so far. */
export const S7_BETA_NOTE = "Beta: tested against S7 simulators. Tell us which CPU you use: support@voltrus.id";

export function S7BetaBadge() {
  return <Badge tone="warn" title={S7_BETA_NOTE}>Beta</Badge>;
}

/** Offline note on what the S7 functions need from the CPU. */
export function S7AccessNote() {
  return (
    <Callout tone="info">
      Works with S7-300/400, and with S7-1200/1500 when PUT/GET access is allowed in the CPU's
      protection settings. Addresses use absolute notation (MW10, DB1.DBD4, I0.0); optimized DBs
      cannot be reached this way.
      <br />
      {S7_BETA_NOTE}
    </Callout>
  );
}
