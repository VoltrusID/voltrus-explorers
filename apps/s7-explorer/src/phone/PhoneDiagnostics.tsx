import { Activity, RefreshCw } from "lucide-react";
import { Button, Callout, EmptyState, ListGroup, ListRow, PageTitle, Section, StatusDot } from "@mactools/ui";
import { useDiagnostics, type LogDir } from "../useDiagnostics";

interface Props {
  deviceId: string;
  deviceLabel: string;
  connected: boolean;
  onLog: (dir: LogDir, msg: string) => void;
  onConnect: () => void;
}

const hex = (n: number) => `0x${n.toString(16).padStart(4, "0").toUpperCase()}`;

/** Diagnostics page: operating mode, CPU facts as rows, the diagnostic buffer as a list. */
export default function PhoneDiagnostics({ deviceId, deviceLabel, connected, onLog, onConnect }: Props) {
  const { cpuInfo, plcMode, diagBuffer, loading, error, refreshAll, modeTone } = useDiagnostics(deviceId, connected, deviceLabel, onLog);
  const cpuRows: Array<[string, string | undefined]> = cpuInfo
    ? [
        ["Module", cpuInfo.module_type],
        ["Firmware", cpuInfo.firmware_version],
        ["Serial", cpuInfo.serial_number],
        ["AS name", cpuInfo.as_name],
        ["Module name", cpuInfo.module_name],
      ]
    : [];

  return (
    <div className="vm-body">
      <PageTitle
        title="Diagnostics"
        caption={deviceLabel}
        actions={
          connected ? (
            <Button size="sm" icon={<RefreshCw size={16} strokeWidth={1.75} style={loading ? { animation: "vt-spin 700ms linear infinite" } : undefined} />} disabled={loading} onClick={refreshAll}>
              {loading ? "Loading…" : "Refresh"}
            </Button>
          ) : undefined
        }
      />
      {error && <Callout tone="danger">{error}</Callout>}
      {!connected ? (
        <EmptyState
          icon={<Activity size={22} strokeWidth={1.75} />}
          title="Not connected"
          description="Connect to a PLC to read its mode, CPU details and diagnostic buffer."
          action={
            <Button variant="primary" onClick={onConnect}>
              Connect
            </Button>
          }
        />
      ) : (
        <>
          <Section title="Operating mode">
            <div className="s7-mode">
              <span className={`s7-mode__value s7-mode__value--${modeTone}`}>{plcMode?.mode ?? "–"}</span>
              <StatusDot tone={modeTone} label={`PLC mode ${plcMode?.mode ?? "unknown"}`} />
            </div>
          </Section>
          <Section title="CPU" className="s7-bare">
            {cpuInfo ? (
              <ListGroup>
                {cpuRows.map(([k, v]) => (
                  <ListRow key={k} title={k} value={<span className="s7-plain mono">{v || "–"}</span>} />
                ))}
              </ListGroup>
            ) : (
              <p className="s7-muted">No CPU information available.</p>
            )}
          </Section>
          <Section title="Diagnostic buffer" meta={`${diagBuffer?.entry_count ?? 0} entries`} className="s7-bare">
            {diagBuffer && diagBuffer.entries.length > 0 ? (
              <ListGroup>
                {diagBuffer.entries.map((entry, i) => (
                  <ListRow
                    key={i}
                    title={<span className="s7-wrap">{entry.event_text}</span>}
                    subtitle={
                      <span className="mono">
                        {hex(entry.event_number)} · {entry.timestamp}
                      </span>
                    }
                  />
                ))}
              </ListGroup>
            ) : (
              <p className="s7-muted">No diagnostic entries.</p>
            )}
          </Section>
        </>
      )}
    </div>
  );
}
