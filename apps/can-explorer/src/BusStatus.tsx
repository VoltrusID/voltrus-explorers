import { Badge } from "@mactools/ui";

/**
 * Controller health as the drivers report it on `can-stats` (optional: adapters that cannot tell leave it out).
 * `busState` is the CAN fault confinement state; `txErr` / `rxErr` the transmit and receive error counters (TEC, REC).
 */
export interface BusHealth {
  busState?: "active" | "warning" | "passive" | "bus-off";
  txErr?: number;
  rxErr?: number;
  /** Frames left out of the UI batches; another name the core may use for `Stats.dropped`. */
  notShown?: number;
}

const STATES = {
  active: { text: "Error active", tone: "neutral", help: "The controller takes part normally in the bus." },
  warning: { text: "Error warning", tone: "warn", help: "An error counter passed 96: the bus has errors (wiring, termination or bit rate)." },
  passive: { text: "Error passive", tone: "warn", help: "An error counter passed 127: the controller no longer sends active error flags. Check wiring, termination and bit rate." },
  "bus-off": { text: "Bus off", tone: "danger", help: "The transmit error counter passed 255 and the controller left the bus. It sends and receives nothing until it recovers or is restarted." },
} as const;

const count = (v: unknown) => (typeof v === "number" && Number.isFinite(v) && v >= 0 ? v : null);

/** Bus state and error counters for the status bar; renders nothing when the driver does not report them. */
export default function BusStatus({ stats }: { stats: (BusHealth & { dropped?: number }) | null }) {
  if (!stats) return null;
  const state = typeof stats.busState === "string" && stats.busState in STATES ? STATES[stats.busState] : null;
  const tec = count(stats.txErr);
  const rec = count(stats.rxErr);
  const notShown = stats.dropped == null ? count(stats.notShown) : null;
  if (!state && tec == null && rec == null && !notShown) return null;
  return (
    <span className="vt-statusbar__item" style={{ display: "inline-flex", gap: 6, alignItems: "center" }}>
      {state && <Badge tone={state.tone} title={state.help}>{state.text}</Badge>}
      {(tec != null || rec != null) && (
        <span className="mono" style={{ fontSize: "var(--fs-xs)", color: "var(--text-dim)" }} title="Transmit and receive error counters of the CAN controller">
          TEC {tec ?? "–"} · REC {rec ?? "–"}
        </span>
      )}
      {notShown ? (
        <span style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)" }} title="Frames the screen could not keep up with; they still count in RX">
          {notShown.toLocaleString("en-US")} not shown
        </span>
      ) : null}
    </span>
  );
}
