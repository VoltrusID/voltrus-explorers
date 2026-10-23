import { useEffect, useState, type ReactNode } from "react";
import { Download, Info, LogOut, Settings } from "lucide-react";
import { Button, IconButton } from "../primitives";
import { StatusChip, useIsPhone } from "../phone";

// Every app's macOS window uses titleBarStyle "Overlay" (tauri.conf.json), so
// this header IS the title bar there: it reserves room for the traffic lights
// and is the window drag region. Touch-capable "Macintosh" UAs are iPadOS,
// where there is no title bar.
const MAC_OVERLAY =
  "__TAURI_INTERNALS__" in window && /Macintosh/.test(navigator.userAgent) && navigator.maxTouchPoints === 0;

interface ToolbarProps {
  title: string;
  connected: boolean;
  /** App icon URL (the bundled 32x32 PNG). */
  icon?: string;
  /** Secondary text after the name, e.g. the connected endpoint. */
  context?: ReactNode;
  /** App-specific controls rendered before the standard actions. */
  actions?: ReactNode;
  onDisconnect?: () => void;
  onExport?: () => void;
  onSettings?: () => void;
  onInfo?: () => void;
  /** Phone header only (where there is no status bar): overrides "Connected"/"Disconnected", e.g. "Server running". */
  statusLabel?: string;
  /** Phone header only: round-trip time and poll count of the connection, shown under the title while connected. */
  latency?: number;
  pollCount?: number;
  /** Phone header only: the last error, shown as a banner under the header. */
  error?: string | null;
}

export function Toolbar({
  title,
  connected,
  icon,
  context,
  actions,
  onDisconnect,
  onExport,
  onSettings,
  onInfo,
  statusLabel,
  latency,
  pollCount,
  error,
}: ToolbarProps) {
  const phone = useIsPhone();
  // Native fullscreen hides the traffic lights, so the inset goes with them.
  const [fullscreen, setFullscreen] = useState(false);
  useEffect(() => {
    if (!MAC_OVERLAY) return;
    const check = () => setFullscreen(window.innerWidth >= screen.width && window.innerHeight >= screen.height);
    check();
    window.addEventListener("resize", check);
    return () => window.removeEventListener("resize", check);
  }, []);

  if (phone) {
    const meta = [context, connected && latency !== undefined ? `${latency} ms` : null].filter(Boolean);
    return (
      <header className="vm-header">
        <div className="vm-header__row">
          {icon && <img src={icon} alt="" className="vm-header__icon" draggable={false} />}
          <div className="vm-header__brand">
            <span className="vm-header__title">{title}</span>
            {meta.length > 0 && <span className="vm-header__context">{meta.join(" · ")}</span>}
          </div>
          <StatusChip connected={connected} label={statusLabel} polls={pollCount} />
          <div className="vm-header__actions">
            {actions}
            {onSettings && (
              <IconButton label="Settings" onClick={onSettings}>
                <Settings size={20} strokeWidth={1.75} />
              </IconButton>
            )}
            {onInfo && (
              <IconButton label="About" onClick={onInfo}>
                <Info size={20} strokeWidth={1.75} />
              </IconButton>
            )}
          </div>
        </div>
        {error && (
          <div className="vm-header__error" role="alert">
            {error}
          </div>
        )}
      </header>
    );
  }

  return (
    <header
      className={MAC_OVERLAY && !fullscreen ? "vt-titlebar vt-titlebar--overlay" : "vt-titlebar"}
      data-tauri-drag-region={MAC_OVERLAY ? "deep" : undefined}
    >
      <div className="vt-titlebar__brand">
        {icon && <img src={icon} alt="" className="vt-titlebar__icon" draggable={false} />}
        <span className="vt-titlebar__name">{title}</span>
        {context && <span className="vt-titlebar__context">{context}</span>}
      </div>
      <div className="vt-titlebar__actions">
        {actions}
        {onExport && (
          <Button size="sm" variant="ghost" icon={<Download size={14} />} onClick={onExport}>
            Export
          </Button>
        )}
        {connected && onDisconnect && (
          <Button size="sm" variant="danger" icon={<LogOut size={14} />} onClick={onDisconnect}>
            Disconnect
          </Button>
        )}
        {onSettings && (
          <IconButton label="Settings" onClick={onSettings}>
            <Settings size={16} strokeWidth={1.75} />
          </IconButton>
        )}
        {onInfo && (
          <IconButton label="About" onClick={onInfo}>
            <Info size={16} strokeWidth={1.75} />
          </IconButton>
        )}
      </div>
    </header>
  );
}
