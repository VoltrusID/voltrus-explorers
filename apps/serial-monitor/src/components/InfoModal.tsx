import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { Button, Dialog, ProAboutRow, useProStatus } from "@mactools/ui";

interface InfoModalProps {
  show: boolean;
  onClose: () => void;
}

export function InfoModal({ show, onClose }: InfoModalProps) {
  const { status } = useProStatus();
  const [appVersion, setAppVersion] = useState("");

  useEffect(() => {
    getVersion().then(setAppVersion).catch(() => {});
  }, []);

  return (
    <Dialog
      open={show}
      onClose={onClose}
      title={appVersion ? `Serial Monitor v${appVersion}` : "Serial Monitor"}
      width={380}
      footer={
        <>
          {/* No outbound source link in App Store builds: review treats links to builds that unlock paid
              features elsewhere as steering away from in-app purchase. */}
          {status && !status.store && (
            <Button variant="ghost" onClick={() => invoke("open_url", { url: "https://github.com/VoltrusID/voltrus-explorers" })}>
              Source on GitHub
            </Button>
          )}
          <Button onClick={onClose}>Close</Button>
        </>
      }
    >
      <div style={{ fontSize: "var(--fs-md)", color: "var(--text-dim)", lineHeight: 1.6, marginBottom: 16 }}>
        Monitor, send, and analyze serial port data over RS232 and RS485.
        <br />
        <br />
        <strong style={{ color: "var(--text)" }}>HEX Mode</strong>
        <br />
        Send raw hexadecimal bytes to connected devices.
        <br />
        <br />
        <strong style={{ color: "var(--text)" }}>ASCII Mode</strong>
        <br />
        Send plain text strings over the serial connection.
        <br />
        <br />
        {status && !status.store && (
          <>
        <strong style={{ color: "var(--text)" }}>Virtual Port</strong>
        <br />
        Create a real PTY-based virtual serial port for testing. Other apps can connect to it.
        <br />
        <br />
          </>
        )}
        <span style={{ color: "var(--text-muted)" }}>Part of the </span>
        <strong style={{ color: "var(--accent-text)" }}>Voltrus</strong>
        <span style={{ color: "var(--text-muted)" }}> suite by </span>
        <strong style={{ color: "var(--text)" }}>JoyoDigitama</strong>
      </div>
      <ProAboutRow />
    </Dialog>
  );
}
