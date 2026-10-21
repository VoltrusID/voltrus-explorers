import { useState, useCallback, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Check, ClipboardList, RefreshCw, X } from "lucide-react";
import { Badge, Button, EmptyState, IconButton } from "@mactools/ui";
import type { AlarmSummaryEntry, EventInfoEntry, EventInformationResponse } from "../types";
import { formatObjectType } from "../types";

/** BACnetEventState numbers (ASHRAE 135 clause 21), keyed by the names alarm.rs emits. */
const EVENT_STATE_NUMBERS: Record<string, number> = {
  normal: 0,
  fault: 1,
  offnormal: 2,
  highLimit: 3,
  lowLimit: 4,
  lifeSafetyAlarm: 5,
};

function eventStateNumber(name: string): number | null {
  if (name in EVENT_STATE_NUMBERS) return EVENT_STATE_NUMBERS[name];
  const unknown = /^unknown\((\d+)\)$/.exec(name);
  return unknown ? Number(unknown[1]) : null;
}

/**
 * Index into an event summary's timestamps ([to-offnormal, to-fault, to-normal])
 * for the transition that put the object into `state`.
 */
function transitionIndex(state: number): number {
  if (state === 0) return 2; // to-normal
  if (state === 1) return 1; // to-fault
  return 0; // offnormal, high-limit, low-limit, life-safety-alarm, ... -> to-offnormal
}

/** Event summaries for one object. The device returns its whole event list. */
async function fetchObjectEvents(objectType: string, objectInstance: number): Promise<EventInfoEntry[]> {
  const response = await invoke<EventInformationResponse>("get_event_information");
  if (response.more_events) {
    throw new Error("The device reported more event summaries than it returned");
  }
  return response.entries.filter(
    (e) => e.object_type === objectType && e.object_instance === objectInstance,
  );
}

interface AlarmPanelProps {
  connected: boolean;
  /** Phone layout: one card per alarm instead of a five-column table. */
  compact?: boolean;
}

export function AlarmPanel({ connected, compact = false }: AlarmPanelProps) {
  const [alarms, setAlarms] = useState<AlarmSummaryEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Ack inline state
  const [ackRow, setAckRow] = useState<string | null>(null);
  const [ackText, setAckText] = useState<string>("");

  // Event info panel. The ref holds the key of the request whose result may still be shown.
  const [eventInfo, setEventInfo] = useState<EventInfoEntry[] | null>(null);
  const [eventInfoKey, setEventInfoKey] = useState<string | null>(null);
  const [eventInfoLoading, setEventInfoLoading] = useState(false);
  const eventInfoKeyRef = useRef<string | null>(null);
  const [ackBusy, setAckBusy] = useState(false);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const result = await invoke<AlarmSummaryEntry[]>("get_alarm_summary");
      setAlarms(result);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  // The COV listener socket receives event notifications and forwards them here.
  useEffect(() => {
    if (!connected) return;
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    listen("alarm-notification", () => {
      void refresh();
    }).then((off) => {
      if (cancelled) off();
      else unlisten = off;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [connected, refresh]);

  const handleAck = async (alarm: AlarmSummaryEntry) => {
    const source = ackText.trim();
    if (!source) return;

    setError(null);
    setAckBusy(true);
    try {
      const state = eventStateNumber(alarm.alarm_state);
      if (state === null) {
        throw new Error(`Cannot acknowledge: unrecognised alarm state "${alarm.alarm_state}"`);
      }
      // AcknowledgeAlarm must quote the time stamp of the transition being acknowledged.
      const entries = await fetchObjectEvents(alarm.object_type, alarm.object_instance);
      const entry = entries[0];
      if (!entry) {
        throw new Error(`The device returned no event information for ${alarm.object_identifier}`);
      }
      const timeStamp = entry.transition_time_stamps[transitionIndex(state)];
      if (!timeStamp) {
        throw new Error(`The device returned no time stamp for the ${alarm.alarm_state} transition of ${alarm.object_identifier}`);
      }
      await invoke("acknowledge_alarm_cmd", {
        objectType: alarm.object_type,
        objectInstance: alarm.object_instance,
        eventState: state,
        timeStamp,
        ackSource: source,
      });
      setAckRow(null);
      setAckText("");
      refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setAckBusy(false);
    }
  };

  const closeEventInfo = () => {
    eventInfoKeyRef.current = null;
    setEventInfo(null);
    setEventInfoKey(null);
    setEventInfoLoading(false);
  };

  const handleGetEventInfo = async (objType: string, objInstance: number) => {
    const key = `${objType}-${objInstance}`;
    if (eventInfoKeyRef.current === key) {
      closeEventInfo();
      return;
    }

    eventInfoKeyRef.current = key;
    setEventInfoKey(key);
    setEventInfo(null);
    setEventInfoLoading(true);
    try {
      const entries = await fetchObjectEvents(objType, objInstance);
      if (eventInfoKeyRef.current !== key) return; // superseded or closed meanwhile
      setEventInfo(entries);
    } catch (e) {
      if (eventInfoKeyRef.current !== key) return;
      setError(e instanceof Error ? e.message : String(e));
      setEventInfo(null);
    } finally {
      if (eventInfoKeyRef.current === key) setEventInfoLoading(false);
    }
  };

  /** Events / Ack buttons and the inline acknowledge form for one alarm row. */
  const alarmActions = (alarm: AlarmSummaryEntry, key: string) => (
    <>
      <div style={{ display: "flex", gap: 4, flexWrap: "wrap" }}>
        <Button
          size="sm"
          variant="ghost"
          icon={<ClipboardList size={14} strokeWidth={1.75} />}
          loading={eventInfoKey === key && eventInfoLoading}
          disabled={eventInfoLoading}
          onClick={() => handleGetEventInfo(alarm.object_type, alarm.object_instance)}
        >
          Events
        </Button>
        <Button
          size="sm"
          icon={ackRow === key ? <X size={14} strokeWidth={1.75} /> : <Check size={14} strokeWidth={1.75} />}
          onClick={() => {
            setAckRow(ackRow === key ? null : key);
            setAckText("");
          }}
        >
          {ackRow === key ? "Cancel" : "Ack"}
        </Button>
      </div>
      {ackRow === key && (
        <div style={{ display: "flex", gap: 4, marginTop: 4, alignItems: "center" }}>
          <input
            className="vt-input vt-input--sm"
            type="text"
            placeholder="Acknowledged by…"
            value={ackText}
            onChange={(e) => setAckText(e.target.value)}
            autoFocus
            onKeyDown={(e) => {
              if (e.key === "Enter") handleAck(alarm);
              if (e.key === "Escape") setAckRow(null);
            }}
          />
          <Button
            size="sm"
            variant="primary"
            disabled={!ackText.trim()}
            loading={ackBusy}
            onClick={() => handleAck(alarm)}
          >
            Send
          </Button>
        </div>
      )}
    </>
  );

  return (
    <div style={{ display: "flex", flexDirection: "column", flex: 1, minHeight: 0, overflow: "hidden" }}>
      {/* Header */}
      <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", padding: "2px 2px 8px", flexShrink: 0 }}>
        <span style={{ fontSize: "var(--fs-md)", fontWeight: 600, color: "var(--text)" }}>Alarm summary</span>
        <Button
          size="sm"
          icon={<RefreshCw size={14} strokeWidth={1.75} />}
          loading={loading}
          disabled={!connected}
          onClick={refresh}
        >
          Refresh
        </Button>
      </div>

      {/* Error */}
      {error && (
        <div style={{ padding: "6px 10px", marginBottom: 8, fontSize: "var(--fs-xs)", color: "var(--danger)", background: "var(--danger-soft)", borderRadius: "var(--radius)", flexShrink: 0 }} role="alert">
          {error}
        </div>
      )}

      {/* Main table */}
      <div style={{ flex: 1, overflowY: "auto", minHeight: 0, background: "var(--card)", border: "1px solid var(--border)", borderRadius: "var(--radius-lg)" }}>
        {loading && alarms.length === 0 && (
          <div style={{ display: "flex", alignItems: "center", justifyContent: "center", gap: 8, padding: 24, fontSize: "var(--fs-sm)", color: "var(--text-muted)" }}>
            <span className="vt-spinner" aria-hidden="true" />
            Loading alarms…
          </div>
        )}

        {!loading && alarms.length === 0 && (
          <EmptyState
            icon={<ClipboardList size={20} strokeWidth={1.75} />}
            title="No alarm summary loaded"
            description="Refresh to read the device's alarm summary."
            action={
              <Button size="sm" icon={<RefreshCw size={14} strokeWidth={1.75} />} disabled={!connected} onClick={refresh}>
                Refresh
              </Button>
            }
          />
        )}

        {alarms.length > 0 && compact && (
          <div>
            {alarms.map((alarm) => {
              const key = `${alarm.object_type}-${alarm.object_instance}`;
              const isNormal = alarm.alarm_state.toLowerCase() === "normal";
              return (
                <div key={key} style={{ padding: 12, borderBottom: "1px solid var(--border)" }}>
                  <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 8 }}>
                    <span style={{ fontSize: "var(--fs-md)", fontWeight: 600, color: "var(--text)" }}>
                      {formatObjectType(alarm.object_type)} #{alarm.object_instance}
                    </span>
                    <Badge tone={isNormal ? "success" : "danger"}>{alarm.alarm_state}</Badge>
                  </div>
                  <div className="mono" style={{ margin: "4px 0 10px", fontSize: "var(--fs-xs)", color: "var(--text-muted)" }}>
                    Acked: {alarm.acked_transitions || "—"}
                  </div>
                  {alarmActions(alarm, key)}
                </div>
              );
            })}
          </div>
        )}

        {alarms.length > 0 && !compact && (
          <table className="vt-table">
            <thead>
              <tr>
                <th style={{ textAlign: "left", width: "26%" }}>Object</th>
                <th style={{ textAlign: "right", width: "10%" }}>Instance</th>
                <th style={{ textAlign: "left", width: "20%" }}>Alarm state</th>
                <th style={{ textAlign: "left", width: "14%" }}>Acked transitions</th>
                <th style={{ textAlign: "left", width: "30%" }}>Actions</th>
              </tr>
            </thead>
            <tbody>
              {alarms.map((alarm) => {
                const key = `${alarm.object_type}-${alarm.object_instance}`;
                const isNormal = alarm.alarm_state.toLowerCase() === "normal";
                return (
                  <tr key={key}>
                    <td>{formatObjectType(alarm.object_type)}</td>
                    <td className="is-num">{alarm.object_instance}</td>
                    <td>
                      <Badge tone={isNormal ? "success" : "danger"}>{alarm.alarm_state}</Badge>
                    </td>
                    <td className="is-mono" style={{ color: "var(--text-muted)", fontSize: "var(--fs-xs)" }}>
                      {alarm.acked_transitions}
                    </td>
                    <td>
                      {alarmActions(alarm, key)}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </div>

      {/* Event info sub-panel */}
      {eventInfoKey && (
        <div style={{ marginTop: 8, maxHeight: "40%", overflow: "auto", flexShrink: 0, background: "var(--card)", border: "1px solid var(--border)", borderRadius: "var(--radius-lg)" }}>
          <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", padding: "8px 12px", borderBottom: "1px solid var(--border)", position: "sticky", top: 0, background: "var(--card)", zIndex: 1 }}>
            <span style={{ display: "flex", alignItems: "baseline", gap: 8 }}>
              <span style={{ fontSize: "var(--fs-sm)", fontWeight: 600, color: "var(--text)" }}>Event information</span>
              <span className="mono" style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)" }}>
                {eventInfoKey.replace("-", " #")}
              </span>
            </span>
            <IconButton
              size="sm"
              label="Close event information"
              onClick={closeEventInfo}
            >
              <X size={14} strokeWidth={1.75} />
            </IconButton>
          </div>
          {eventInfoLoading && (
            <div style={{ display: "flex", alignItems: "center", justifyContent: "center", gap: 8, padding: 24, fontSize: "var(--fs-sm)", color: "var(--text-muted)" }}>
              <span className="vt-spinner" aria-hidden="true" />
              Loading events…
            </div>
          )}
          {!eventInfoLoading && eventInfo && eventInfo.length === 0 && (
            <EmptyState
              compact
              title="No events"
              description="The device reports no event information for this object."
            />
          )}
          {!eventInfoLoading && eventInfo && eventInfo.length > 0 && compact && (
            <div>
              {eventInfo.map((ev) => (
                <div key={ev.object_identifier} style={{ padding: "10px 12px", display: "grid", gridTemplateColumns: "auto 1fr", columnGap: 12, rowGap: 6, fontSize: "var(--fs-sm)" }}>
                  <span style={{ color: "var(--text-muted)" }}>Event state</span>
                  <span><Badge tone={ev.event_state === "normal" ? "success" : "danger"}>{ev.event_state}</Badge></span>
                  <span style={{ color: "var(--text-muted)" }}>Notify type</span>
                  <span className="mono">{ev.notify_type}</span>
                  <span style={{ color: "var(--text-muted)" }}>Acked</span>
                  <span className="mono" style={{ overflowWrap: "anywhere" }}>{ev.acknowledged_transitions || "—"}</span>
                  <span style={{ color: "var(--text-muted)" }}>Enabled</span>
                  <span className="mono" style={{ overflowWrap: "anywhere" }}>{ev.event_enable || "—"}</span>
                  <span style={{ color: "var(--text-muted)" }}>To offnormal</span>
                  <span className="mono">{ev.event_time_stamps[0] ?? "—"}</span>
                  <span style={{ color: "var(--text-muted)" }}>To fault</span>
                  <span className="mono">{ev.event_time_stamps[1] ?? "—"}</span>
                  <span style={{ color: "var(--text-muted)" }}>To normal</span>
                  <span className="mono">{ev.event_time_stamps[2] ?? "—"}</span>
                </div>
              ))}
            </div>
          )}
          {!eventInfoLoading && eventInfo && eventInfo.length > 0 && !compact && (
            <table className="vt-table">
              <thead>
                <tr>
                  <th style={{ textAlign: "left" }}>Event state</th>
                  <th style={{ textAlign: "left" }}>Notify type</th>
                  <th style={{ textAlign: "left" }}>Acked</th>
                  <th style={{ textAlign: "left" }}>Enabled</th>
                  <th style={{ textAlign: "left" }}>To offnormal</th>
                  <th style={{ textAlign: "left" }}>To fault</th>
                  <th style={{ textAlign: "left" }}>To normal</th>
                </tr>
              </thead>
              <tbody>
                {eventInfo.map((ev) => (
                  <tr key={ev.object_identifier}>
                    <td>
                      <Badge tone={ev.event_state === "normal" ? "success" : "danger"}>
                        {ev.event_state}
                      </Badge>
                    </td>
                    <td className="is-mono">{ev.notify_type}</td>
                    <td className="is-mono" style={{ color: "var(--text-muted)", fontSize: "var(--fs-xs)" }}>
                      {ev.acknowledged_transitions || "—"}
                    </td>
                    <td className="is-mono" style={{ color: "var(--text-muted)", fontSize: "var(--fs-xs)" }}>
                      {ev.event_enable || "—"}
                    </td>
                    {[0, 1, 2].map((i) => (
                      <td key={i} className="is-mono" style={{ color: "var(--text-muted)", whiteSpace: "nowrap" }}>
                        {ev.event_time_stamps[i] ?? "—"}
                      </td>
                    ))}
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
      )}
    </div>
  );
}
