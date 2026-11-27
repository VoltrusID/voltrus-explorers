import { Button, EmptyState, StatusDot } from "@mactools/ui";
import { Activity, Plug, Table2 } from "lucide-react";
import { bridgeMode } from "../../api/bridge";
import { createWatchTable, openOnlineTab } from "../../lib/deviceProject";
import { useS7 } from "../../lib/s7Store";
import { ConnectionForm, CpuDetails, ModeBadge, ModeButtons, S7AccessNote, S7BetaBadge } from "../S7/common";

/**
 * The Online inspector: connection, CPU identity and mode at a glance. Diagnostics, watch tables
 * and tag tables open as editor tabs.
 */
export default function OnlinePanel() {
  const { info, mode } = useS7();
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
  const online = info !== null;
  return (
    <div className="s7-panel">
      <section className="vt-section">
        <div className="vt-section__header">
          <span className="vt-section__title">Connection</span>
          <S7BetaBadge />
          {online && <StatusDot tone="success" label="Online" />}
        </div>
        <ConnectionForm compact />
      </section>
      {online && info && (
        <section className="vt-section">
          <div className="vt-section__header">
            <span className="vt-section__title">CPU</span>
            <ModeBadge mode={mode} />
          </div>
          <CpuDetails info={info} />
          <ModeButtons />
        </section>
      )}
      <section className="vt-section">
        <div className="s7-row">
          <Button size="sm" icon={<Activity size={14} />} onClick={openOnlineTab}>Online &amp; diagnostics</Button>
          <Button size="sm" variant="ghost" icon={<Table2 size={14} />} onClick={() => void createWatchTable()}>New watch table</Button>
        </div>
      </section>
      {!online && <S7AccessNote />}
    </div>
  );
}
