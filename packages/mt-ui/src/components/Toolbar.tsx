import { useEffect, useState, type ReactNode } from "react";
import { Download, Info, LogOut, Settings } from "lucide-react";
import { Button, IconButton } from "../primitives";

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
}: ToolbarProps) {
  // Native fullscreen hides the traffic lights, so the inset goes with them.
  const [fullscreen, setFullscreen] = useState(false);
  useEffect(() => {
    if (!MAC_OVERLAY) return;
    const check = () => setFullscreen(window.innerWidth >= screen.width && window.innerHeight >= screen.height);
    check();
    window.addEventListener("resize", check);
    return () => window.removeEventListener("resize", check);
  }, []);

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
