import { Plus } from "lucide-react";
import { Button, ProBadge, Section, useProStatus } from "@mactools/ui";
import type { Session } from "../types";
import { FREE_SESSION_LIMIT, shortPortName } from "../sessions";

interface Props {
  sessions: Session[];
  activeId: string;
  onSelect: (id: string) => void;
  onAdd: () => void;
}

/** Open ports as chips; the sidebar below edits the selected one. Free keeps one port, Pro several. */
export function SessionsSection({ sessions, activeId, onSelect, onAdd }: Props) {
  const { pro } = useProStatus();
  return (
    <Section title="Ports">
      <div style={{ display: "flex", flexWrap: "wrap", gap: 6 }}>
        {sessions.map((s) => (
          <button
            key={s.id}
            type="button"
            aria-pressed={s.id === activeId}
            onClick={() => onSelect(s.id)}
            style={{
              display: "flex", alignItems: "center", gap: 6, maxWidth: "100%",
              padding: "3px 8px", cursor: "pointer", font: "inherit", fontSize: "var(--fs-xs)",
              color: "var(--text)", background: "var(--surface)",
              borderRadius: "var(--radius-sm)",
              border: `1px solid ${s.id === activeId ? "var(--accent)" : "var(--border)"}`,
            }}
          >
            <span
              style={{
                width: 7, height: 7, borderRadius: "50%", flexShrink: 0,
                background: s.connected ? "var(--success)" : "var(--text-muted)",
              }}
            />
            <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
              {shortPortName(s.config.port_name)}
            </span>
          </button>
        ))}
      </div>
      <Button
        block
        size="sm"
        icon={<Plus size={14} strokeWidth={1.75} />}
        onClick={onAdd}
      >
        Add port {!pro && sessions.length >= FREE_SESSION_LIMIT && <ProBadge />}
      </Button>
    </Section>
  );
}
