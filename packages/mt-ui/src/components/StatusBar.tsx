import type { ReactNode } from "react";

interface StatusBarProps {
  connected: boolean;
  latency?: number;
  pollCount?: number;
  error?: string | null;
  device?: { ip: string; port: number } | null;
  /** Overrides the default "Connected"/"Disconnected" wording (e.g. "Server running"). */
  label?: string;
  /** App-specific trailing items. */
  children?: ReactNode;
}

export function StatusBar({ connected, latency, pollCount, error, device, label, children }: StatusBarProps) {
  return (
    <footer className="vt-statusbar" role="status">
      <span className="vt-statusbar__item">
        <span className={connected ? "vt-dot vt-dot--success" : "vt-dot"} aria-hidden="true" />
        <span style={{ color: connected ? "var(--text-dim)" : undefined }}>
          {label ?? (connected ? "Connected" : "Disconnected")}
        </span>
      </span>
      {connected && device && (
        <span className="vt-statusbar__item mono" style={{ fontSize: "var(--fs-xs)" }}>
          {device.ip}:{device.port}
        </span>
      )}
      {latency !== undefined && connected && <span className="vt-statusbar__item">{latency} ms</span>}
      {pollCount !== undefined && connected && (
        <span className="vt-statusbar__item">{pollCount.toLocaleString()} polls</span>
      )}
      {children}
      {error && (
        <span className="vt-statusbar__item" style={{ color: "var(--danger)", marginLeft: "auto" }} role="alert">
          {error}
        </span>
      )}
    </footer>
  );
}
