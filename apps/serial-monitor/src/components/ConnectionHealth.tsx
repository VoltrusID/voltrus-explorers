import { useState, useEffect } from "react";

export function formatTimestamp(ms: number): string {
  const d = new Date(ms);
  return (
    d.toLocaleTimeString("en-US", { hour12: false }) +
    "." +
    String(d.getMilliseconds()).padStart(3, "0")
  );
}

export function ConnectionHealth({ lastDataTime }: { lastDataTime: number | null }) {
  const [now, setNow] = useState(Date.now());

  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, []);

  const idleMs = lastDataTime ? now - lastDataTime : null;
  const isReceiving = idleMs !== null && idleMs < 3000;
  const isIdle = idleMs !== null && idleMs >= 3000 && idleMs < 10000;
  const isStale = idleMs === null || idleMs >= 10000;

  let color: string;
  let label: string;
  if (isReceiving) {
    color = "var(--success)";
    label = "Receiving data";
  } else if (isIdle) {
    color = "var(--warn)";
    label = `No data for ${Math.round((idleMs! - idleMs! % 1000) / 1000)}s`;
  } else {
    color = "var(--danger)";
    label = lastDataTime === null ? "Waiting for data..." : `No data for ${Math.round((idleMs! - idleMs! % 1000) / 1000)}s`;
  }

  return (
    <div style={{
      display: "flex",
      alignItems: "center",
      gap: 6,
      padding: "4px 8px",
      borderRadius: "var(--radius-sm)",
      background: "var(--input-bg)",
      fontSize: "var(--fs-xs)",
      color: "var(--text-muted)",
    }}>
      <span style={{
        width: 7,
        height: 7,
        borderRadius: "50%",
        background: color,
        flexShrink: 0,
        animation: isReceiving ? "pulse 1.5s ease-in-out infinite" : undefined,
      }} />
      <span>{label}</span>
      <style>{`
        @keyframes pulse {
          0%, 100% { opacity: 1; }
          50% { opacity: 0.4; }
        }
      `}</style>
    </div>
  );
}
