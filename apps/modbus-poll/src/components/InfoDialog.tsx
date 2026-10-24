import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { Button, Dialog, ListGroup, ListRow, ProAboutRow, useIsPhone, useProStatus } from "@mactools/ui";
import { useStore } from "../store";

interface Props {
  toggleTheme: () => void;
}

export function InfoDialog({ toggleTheme }: Props) {
  const { showInfo, setShowInfo, theme } = useStore();
  const { status } = useProStatus();
  const phone = useIsPhone();
  const [appVersion, setAppVersion] = useState("");

  useEffect(() => { getVersion().then(setAppVersion); }, []);

  useEffect(() => {
    if (!showInfo) return;
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") setShowInfo(false); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [showInfo, setShowInfo]);

  if (phone) {
    return (
      <Dialog open={showInfo} onClose={() => setShowInfo(false)} title="Modbus Poll">
        <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
          <span className="mono" style={{ color: "var(--text-muted)" }}>Version {appVersion}</span>
          <div style={{ color: "var(--text-dim)", lineHeight: 1.5 }}>
            Read, write, and simulate Modbus devices over TCP.
          </div>
          <ListGroup>
            <ListRow title="Master" subtitle="Connect to a device, poll registers, tap a row to write" />
            <ListRow title="Slave" subtitle="Run a Modbus TCP server and simulate a device from a template" />
          </ListGroup>
          {status?.store && <ProAboutRow />}
          {status && !status.store && (
            <Button block onClick={() => invoke("open_url", { url: "https://github.com/VoltrusID/voltrus-explorers" })}>
              Source on GitHub
            </Button>
          )}
          <div style={{ color: "var(--text-muted)", fontSize: "var(--fs-sm)" }}>
            Part of the <strong style={{ color: "var(--accent-text)" }}>Voltrus</strong> suite by JoyoDigitama
          </div>
        </div>
      </Dialog>
    );
  }

  if (!showInfo) return null;

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label="Application info"
      style={{ position: "fixed", inset: 0, background: "var(--scrim)", display: "flex", alignItems: "center", justifyContent: "center", zIndex: 9999 }}
      onClick={() => setShowInfo(false)}
    >
      <div
        style={{ background: "var(--card)", border: "1px solid var(--border)", borderRadius: "var(--radius-lg)", padding: 24, maxWidth: 360, width: "90%", boxShadow: "var(--shadow-pop)" }}
        onClick={(e) => e.stopPropagation()}
      >
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 12 }}>
          <div style={{ fontSize: "var(--fs-lg)", fontWeight: 600, color: "var(--text)" }}>Modbus Poll</div>
          <span style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)", fontFamily: "var(--font-mono)" }}>v{appVersion}</span>
        </div>
        <div style={{ fontSize: "var(--fs-sm)", color: "var(--text-dim)", lineHeight: 1.6, marginBottom: 16 }}>
          Read, write, and simulate Modbus devices over TCP.
          <br /><br />
          <strong style={{ color: "var(--text)" }}>Master Mode</strong><br />
          Connect to a Modbus TCP device to poll registers.<br />
          Add read blocks, click rows to write values.
          <br /><br />
          <strong style={{ color: "var(--text)" }}>Slave Mode</strong><br />
          Run a Modbus TCP server to simulate a device.<br />
          Load device templates, auto-simulate live data.
          <br /><br />
          <span style={{ color: "var(--text-muted)" }}>Part of the </span>
          <strong style={{ color: "var(--accent)" }}>Voltrus</strong>
          <span style={{ color: "var(--text-muted)" }}> suite by </span>
          <strong style={{ color: "var(--text)" }}>JoyoDigitama</strong>
        </div>
        <div style={{ borderTop: "1px solid var(--border)", paddingTop: 12, marginBottom: 12, display: "flex", justifyContent: "space-between", alignItems: "center" }}>
          <span className="vt-label" id="theme-label">Theme</span>
          <Button size="sm" variant="ghost" onClick={toggleTheme}>
            {theme === "dark" ? "Dark" : "Light"}
          </Button>
        </div>
        {status?.store && (
          // Store and licence-key builds: Pro state and its entry point. The App Store build carries no outbound
          // source link, since store review treats links to builds that unlock paid features elsewhere as steering
          // away from in-app purchase; the Windows direct-sale build is not in a store, so it keeps the link.
          <div style={{ borderTop: "1px solid var(--border)", paddingTop: 12, marginBottom: 12 }}>
            <ProAboutRow />
          </div>
        )}
        {(!status?.store || status.activation) && (
          <div style={{ borderTop: "1px solid var(--border)", paddingTop: 12, marginBottom: 12, display: "flex", justifyContent: "space-between", alignItems: "center" }}>
            <span className="vt-label">Source</span>
            <Button size="sm" variant="ghost" onClick={() => invoke("open_url", { url: "https://github.com/VoltrusID/voltrus-explorers" })}>
              GitHub
            </Button>
          </div>
        )}
        <Button block onClick={() => setShowInfo(false)}>Close</Button>
      </div>
    </div>
  );
}
