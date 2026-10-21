import { useState, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Check, ClipboardList, RefreshCw, X } from "lucide-react";
import { Badge, Button, EmptyState, IconButton } from "@mactools/ui";
import type { AlarmSummaryEntry, EventInfoEntry } from "../types";
import { formatObjectType } from "../types";

interface AlarmPanelProps {
  connected: boolean;
}

export function AlarmPanel({ connected }: AlarmPanelProps) {
  const [alarms, setAlarms] = useState<AlarmSummaryEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Ack inline state
  const [ackRow, setAckRow] = useState<string | null>(null);
  const [ackText, setAckText] = useState<string>("");

  // Event info panel
  const [eventInfo, setEventInfo] = useState<EventInfoEntry[] | null>(null);
  const [eventInfoKey, setEventInfoKey] = useState<string | null>(null);
  const [eventInfoLoading, setEventInfoLoading] = useState(false);

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

  const handleAck = async (objType: string, objInstance: number) => {
    if (!ackText.trim()) return;

    setError(null);
    try {
      await invoke("acknowledge_alarm_cmd", {
        objectType: objType,
        objectInstance: objInstance,
        eventState: 0,
        ackText: ackText.trim(),
      });
      setAckRow(null);
      setAckText("");
      refresh();
    } catch (e) {
      setError(String(e));
    }
  };

  const handleGetEventInfo = async (objType: string, objInstance: number) => {
    const key = `${objType}-${objInstance}`;
    if (eventInfoKey === key) {
      setEventInfo(null);
      setEventInfoKey(null);
      return;
    }

    setEventInfoLoading(true);
    setEventInfoKey(key);
    try {
      const result = await invoke<EventInfoEntry[]>("get_event_information", {
        objectType: objType,
        objectInstance: objInstance,
      });
      setEventInfo(result);
    } catch (e) {
      setError(String(e));
      setEventInfo(null);
    } finally {
      setEventInfoLoading(false);
    }
  };

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

        {alarms.length > 0 && (
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
                            placeholder="Ack text…"
                            value={ackText}
                            onChange={(e) => setAckText(e.target.value)}
                            autoFocus
                            onKeyDown={(e) => {
                              if (e.key === "Enter") handleAck(alarm.object_type, alarm.object_instance);
                              if (e.key === "Escape") setAckRow(null);
                            }}
                          />
                          <Button
                            size="sm"
                            variant="primary"
                            disabled={!ackText.trim()}
                            onClick={() => handleAck(alarm.object_type, alarm.object_instance)}
                          >
                            Send
                          </Button>
                        </div>
                      )}
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
              onClick={() => {
                setEventInfo(null);
                setEventInfoKey(null);
              }}
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
              description="This object has no event information."
            />
          )}
          {!eventInfoLoading && eventInfo && eventInfo.length > 0 && (
            <table className="vt-table">
              <thead>
                <tr>
                  <th style={{ textAlign: "left" }}>Event state</th>
                  <th style={{ textAlign: "left" }}>Type</th>
                  <th style={{ textAlign: "left" }}>Notify type</th>
                  <th style={{ textAlign: "left" }}>Message</th>
                  <th style={{ textAlign: "left" }}>Timestamp</th>
                </tr>
              </thead>
              <tbody>
                {eventInfo.map((ev, i) => (
                  <tr key={i}>
                    <td>
                      <Badge tone={ev.event_state.toLowerCase() === "normal" ? "success" : "danger"}>
                        {ev.event_state}
                      </Badge>
                    </td>
                    <td className="is-mono">{ev.event_type}</td>
                    <td className="is-mono">{ev.notify_type}</td>
                    <td style={{ whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis", maxWidth: 220 }}>
                      {ev.message_text || "—"}
                    </td>
                    <td className="is-mono" style={{ color: "var(--text-muted)", whiteSpace: "nowrap" }}>
                      {ev.time_stamp || "—"}
                    </td>
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
