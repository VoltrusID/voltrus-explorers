import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { ChevronDown } from "lucide-react";
import { Dialog, EmptyState } from "@mactools/ui";

interface ScheduleDialogProps {
  objectInstance: number;
  onClose: () => void;
}

interface ScheduleEntry {
  time: string;
  value: string;
}

interface DaySchedule {
  day_name: string;
  entries: ScheduleEntry[];
}

interface ScheduleData {
  object_instance: number;
  weekly_schedule: DaySchedule[];
  schedule_default: string | null;
  effective_period_start: string | null;
  effective_period_end: string | null;
  object_property_references: string[];
}

const DAY_ORDER = [
  "Monday",
  "Tuesday",
  "Wednesday",
  "Thursday",
  "Friday",
  "Saturday",
  "Sunday",
];

export function ScheduleDialog({
  objectInstance,
  onClose,
}: ScheduleDialogProps) {
  const [data, setData] = useState<ScheduleData | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [showRefs, setShowRefs] = useState(false);

  useEffect(() => {
    invoke<ScheduleData>("read_schedule", { objectInstance })
      .then(setData)
      .catch((e) =>
        setError(e instanceof Error ? e.message : String(e))
      )
      .finally(() => setLoading(false));
  }, [objectInstance]);

  const sortedSchedule = data
    ? [...data.weekly_schedule].sort(
        (a, b) => DAY_ORDER.indexOf(a.day_name) - DAY_ORDER.indexOf(b.day_name)
      )
    : [];

  return (
    <Dialog open onClose={onClose} title={`Schedule #${objectInstance}`} width={640}>
        {/* Content */}
        {loading && (
          <div style={{ display: "flex", alignItems: "center", justifyContent: "center", gap: 8, padding: "16px 0", fontSize: "var(--fs-sm)", color: "var(--text-muted)" }}>
            <span className="vt-spinner" aria-hidden="true" />
            Loading schedule…
          </div>
        )}

        {error && (
          <div style={{ padding: "6px 10px", fontSize: "var(--fs-sm)", color: "var(--danger)", background: "var(--danger-soft)", borderRadius: "var(--radius)" }} role="alert">
            {error}
          </div>
        )}

        {!loading && !error && !data && (
          <EmptyState
            compact
            title="Schedule unavailable"
            description="The schedule properties could not be read from this object."
          />
        )}

        {!loading && !error && data && (
          <div style={{ overflowY: "auto", margin: "0 -18px", padding: "0 18px", maxHeight: "60vh" }}>
            {/* Effective period */}
            <div
              style={{
                fontSize: "var(--fs-sm)",
                color: "var(--text-dim)",
                marginBottom: 10,
                padding: "8px 10px",
                background: "var(--sunken)",
                borderRadius: "var(--radius)",
                border: "1px solid var(--border)",
              }}
            >
              {data.effective_period_start && data.effective_period_end ? (
                <>
                  <span style={{ color: "var(--text-muted)" }}>Start </span>
                  <span className="mono" style={{ color: "var(--text)" }}>
                    {data.effective_period_start}
                  </span>
                  <span style={{ color: "var(--text-muted)" }}> to end </span>
                  <span className="mono" style={{ color: "var(--text)" }}>
                    {data.effective_period_end}
                  </span>
                </>
              ) : (
                <span style={{ color: "var(--text-muted)" }}>Always active</span>
              )}
            </div>

            {/* Default value */}
            {data.schedule_default && (
              <div
                style={{
                  fontSize: "var(--fs-sm)",
                  color: "var(--text-dim)",
                  marginBottom: 12,
                }}
              >
                <span style={{ color: "var(--text-muted)" }}>Default </span>
                <span className="mono" style={{ color: "var(--text)" }}>
                  {data.schedule_default}
                </span>
              </div>
            )}

            {/* 7-day grid */}
            <div
              style={{
                display: "flex",
                flexDirection: "column",
                gap: 8,
                marginBottom: 12,
              }}
            >
              {sortedSchedule.map((day) => (
                <div
                  key={day.day_name}
                  style={{
                    border: "1px solid var(--border)",
                    borderRadius: "var(--radius-lg)",
                    overflow: "hidden",
                  }}
                >
                  <div
                    style={{
                      fontSize: "var(--fs-sm)",
                      fontWeight: 600,
                      color: "var(--text)",
                      padding: "6px 10px",
                      background: "var(--sunken)",
                      borderBottom: "1px solid var(--border)",
                    }}
                  >
                    {day.day_name}
                    {day.entries.length === 0 && (
                      <span
                        style={{
                          fontWeight: 400,
                          color: "var(--text-muted)",
                          marginLeft: 8,
                        }}
                      >
                        (no entries)
                      </span>
                    )}
                  </div>
                  {day.entries.length > 0 && (
                    <table className="vt-table">
                      <tbody>
                        {day.entries.map((entry, i) => (
                          <tr
                            key={i}
                            style={{
                              borderBottom:
                                i < day.entries.length - 1
                                  ? "1px solid var(--border)"
                                  : undefined,
                            }}
                          >
                            <td
                              className="is-mono"
                              style={{ whiteSpace: "nowrap", width: "35%" }}
                            >
                              {entry.time}
                            </td>
                            <td className="is-mono" style={{ color: "var(--text)" }}>
                              {entry.value}
                            </td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  )}
                </div>
              ))}
            </div>

            {/* Object property references */}
            {data.object_property_references.length > 0 && (
              <div
                style={{
                  border: "1px solid var(--border)",
                  borderRadius: "var(--radius-lg)",
                  overflow: "hidden",
                }}
              >
                <button
                  onClick={() => setShowRefs(!showRefs)}
                  aria-expanded={showRefs}
                  style={{
                    width: "100%",
                    background: "transparent",
                    border: "none",
                    padding: "8px 10px",
                    cursor: "pointer",
                    display: "flex",
                    alignItems: "center",
                    justifyContent: "space-between",
                    fontSize: "var(--fs-sm)",
                    fontWeight: 600,
                    color: "var(--text-muted)",
                  }}
                >
                  <span>
                    Property references (
                    {data.object_property_references.length})
                  </span>
                  <ChevronDown
                    size={14}
                    strokeWidth={1.75}
                    style={{
                      transition: "transform 0.15s",
                      transform: showRefs ? "rotate(180deg)" : "rotate(0deg)",
                      flexShrink: 0,
                    }}
                  />
                </button>
                {showRefs && (
                  <div
                    style={{
                      borderTop: "1px solid var(--border)",
                      padding: "8px 10px",
                    }}
                  >
                    {data.object_property_references.map((ref, i) => (
                      <div
                        key={i}
                        className="mono"
                        style={{
                          fontSize: "var(--fs-xs)",
                          color: "var(--text-dim)",
                          padding: "2px 0",
                        }}
                      >
                        {ref}
                      </div>
                    ))}
                  </div>
                )}
              </div>
            )}
          </div>
        )}
    </Dialog>
  );
}
