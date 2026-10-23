import { Button, IconButton, ProBadge, SheetSelect, useIsPhone } from "@mactools/ui";
import { Plus, X } from "lucide-react";
import { useConnRuntime } from "../connections";
import { fieldsOf, sessionLabel } from "../sessions";
import { useStore } from "../store";

interface Props {
  sessionCount: number;
  onAdd: () => void;
  onClose: (id: string) => void;
}

const SEP = "\u0001";

/** One tab per master connection. Every connection keeps polling while another tab is in front. On a phone: one picker that opens a sheet. */
export function ConnectionTabs({ sessionCount, onAdd, onClose }: Props) {
  const activeId = useStore((s) => s.activeSessionId);
  const switchSession = useStore((s) => s.switchSession);
  // A joined string keeps the selector stable: the tabs only re-render when an id or a label changes.
  const entries = useStore((s) => s.sessions.map((x) => `${x.id}${SEP}${sessionLabel(fieldsOf(s, x.id)!)}`).join("\u0002"));
  const runtime = useConnRuntime((s) => s.byId);
  const phone = useIsPhone();

  const sessions = entries.split("\u0002").map((entry) => {
    const [id, label] = entry.split(SEP);
    return { id, label, connected: runtime[id]?.connected ?? false };
  });

  if (phone) {
    const dot = (connected: boolean) => <span className={connected ? "vt-dot vt-dot--success" : "vt-dot"} aria-hidden="true" />;
    const active = sessions.find((s) => s.id === activeId);
    return (
      <SheetSelect
        sheetTitle="Connections"
        aria-label="Connection"
        value={activeId}
        onChange={switchSession}
        display={
          <span style={{ display: "inline-flex", alignItems: "center", gap: 8 }}>
            {dot(active?.connected ?? false)}
            {active?.label}
          </span>
        }
        options={sessions.map((s) => ({
          value: s.id,
          label: s.label,
          hint: s.connected ? "Connected" : undefined,
          icon: dot(s.connected),
          trailing:
            sessionCount > 1 ? (
              <IconButton
                label={`Close ${s.label}`}
                onClick={(e) => {
                  e.stopPropagation();
                  onClose(s.id);
                }}
              >
                <X size={18} strokeWidth={1.75} />
              </IconButton>
            ) : undefined,
        }))}
        footer={(close) => (
          <Button
            block
            icon={<Plus size={16} strokeWidth={1.75} />}
            onClick={() => {
              close();
              onAdd();
            }}
          >
            New connection <ProBadge />
          </Button>
        )}
      />
    );
  }

  return (
    <div role="tablist" aria-label="Connections" className="vt-tabs vt-tabs--conn" style={{ flexShrink: 0, background: "var(--card)", alignItems: "center" }}>
      {sessions.map(({ id, label, connected }) => {
        const selected = id === activeId;
        return (
          <div key={id} style={{ display: "flex", alignItems: "center" }}>
            <button
              type="button"
              role="tab"
              aria-selected={selected}
              tabIndex={selected ? 0 : -1}
              className="vt-tab"
              onClick={() => switchSession(id)}
              title={connected ? `${label} (connected)` : label}
            >
              <span className={connected ? "vt-dot vt-dot--success" : "vt-dot"} aria-hidden="true" />
              {label}
            </button>
            {sessionCount > 1 && (
              <IconButton label={`Close ${label}`} size="sm" onClick={() => onClose(id)}>
                <X size={12} strokeWidth={1.75} />
              </IconButton>
            )}
          </div>
        );
      })}
      <button type="button" className="vt-btn vt-btn--sm vt-btn--ghost" onClick={onAdd} aria-label="Add connection" style={{ marginLeft: 4, flexShrink: 0 }}>
        <Plus size={14} strokeWidth={1.75} /> Connection <ProBadge />
      </button>
    </div>
  );
}
