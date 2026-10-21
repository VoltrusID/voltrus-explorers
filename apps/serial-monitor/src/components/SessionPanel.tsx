import { useEffect, useRef } from "react";
import { X } from "lucide-react";
import { IconButton, useProStatus } from "@mactools/ui";
import type { Session } from "../types";
import type { SendMacro } from "../profiles";
import { configSummary, shortPortName } from "../sessions";
import { SendComposer } from "./SendComposer";
import { TrafficTable } from "./TrafficTable";

interface SessionPanelProps {
  session: Session;
  /** The session the sidebar is editing. */
  active: boolean;
  /** Several ports are open: show the header that names this one. */
  showHeader: boolean;
  /** Hidden panels stay mounted so repeat-send keeps running. */
  visible: boolean;
  /** Table grows to the available height (mobile) instead of a fixed height. */
  fill: boolean;
  tableHeight: number;
  onSelect: () => void;
  onClose: () => void;
  onUpdate: (patch: Partial<Session>) => void;
  /** Send the composer text; `keepInput` leaves it in the field (repeat-send). */
  onSend: (keepInput: boolean) => void;
  macros: SendMacro[];
  onMacrosChange: (list: SendMacro[]) => void;
}

/** Traffic table and send bar of one port. */
export function SessionPanel({
  session: s,
  active,
  showHeader,
  visible,
  fill,
  tableHeight,
  onSelect,
  onClose,
  onUpdate,
  onSend,
  macros,
  onMacrosChange,
}: SessionPanelProps) {
  const { pro } = useProStatus();
  const sendRef = useRef(onSend);
  sendRef.current = onSend;
  const updateRef = useRef(onUpdate);
  updateRef.current = onUpdate;

  useEffect(() => {
    if (!s.repeating) return;
    if (!s.connected || !pro) {
      updateRef.current({ repeating: false });
      return;
    }
    const timer = setInterval(() => sendRef.current(true), s.repeatMs);
    return () => clearInterval(timer);
  }, [s.repeating, s.connected, s.repeatMs, pro]);

  return (
    <div
      style={{
        display: visible ? "flex" : "none",
        flexDirection: "column",
        flex: "1 1 0",
        minWidth: showHeader && !fill ? 320 : 0,
        minHeight: fill ? 0 : undefined,
        gap: 8,
      }}
    >
      {showHeader && (
        <div
          onClick={onSelect}
          style={{
            display: "flex",
            alignItems: "center",
            gap: 8,
            padding: "4px 8px",
            cursor: "pointer",
            borderRadius: "var(--radius-sm)",
            border: `1px solid ${active ? "var(--accent)" : "var(--border)"}`,
            background: "var(--surface)",
            fontSize: "var(--fs-sm)",
          }}
        >
          <span
            style={{
              width: 8, height: 8, borderRadius: "50%", flexShrink: 0,
              background: s.connected ? "var(--success)" : "var(--text-muted)",
            }}
          />
          <strong style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
            {shortPortName(s.config.port_name)}
          </strong>
          <span style={{ color: "var(--text-muted)", fontSize: "var(--fs-xs)", flex: 1 }}>
            {s.connected ? configSummary(s.config) : "Not connected"}
          </span>
          <IconButton
            label={s.connected ? "Disconnect and remove this port" : "Remove this port"}
            size="sm"
            onClick={(e) => { e.stopPropagation(); onClose(); }}
          >
            <X size={14} strokeWidth={1.75} />
          </IconButton>
        </div>
      )}
      <div
        style={fill
          ? { flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }
          : { height: tableHeight, minHeight: 80, display: "flex", flexDirection: "column", flexShrink: 0 }}
      >
        <TrafficTable
          data={s.traffic}
          emptyMessage="No traffic yet. Connect to a port or start the simulation."
        />
      </div>
      <SendComposer
        format={s.sendFormat} onFormatChange={(sendFormat) => onUpdate({ sendFormat })}
        value={s.sendData} onValueChange={(sendData) => onUpdate({ sendData })}
        onSend={() => onSend(false)} disabled={!s.connected}
        macros={macros} onMacrosChange={onMacrosChange}
        repeating={s.repeating} onRepeatingChange={(repeating) => onUpdate({ repeating })}
        repeatMs={s.repeatMs} onRepeatMsChange={(repeatMs) => onUpdate({ repeatMs })}
      />
    </div>
  );
}
