import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Button, Dialog } from "@mactools/ui";

export interface IdentityInfo {
  vendor_id: number;
  vendor_name: string;
  device_type: number;
  product_code: number;
  revision_major: number;
  revision_minor: number;
  status: number;
  serial_number: number;
  product_name: string;
}

const row: React.CSSProperties = {
  display: "flex",
  justifyContent: "space-between",
  gap: 12,
  padding: "6px 0",
  borderBottom: "1px solid var(--border)",
  fontSize: 12,
};

const labelStyle: React.CSSProperties = { color: "var(--text-muted)" };
const valueStyle: React.CSSProperties = {
  color: "var(--text)",
  fontVariantNumeric: "tabular-nums",
};

function serialHex(n: number): string {
  return `0x${(n >>> 0).toString(16).padStart(8, "0").toUpperCase()}`;
}

interface Props {
  onClose: () => void;
}

export function IdentityPanel({ onClose }: Props) {
  const [info, setInfo] = useState<IdentityInfo | null>(null);
  const [error, setError] = useState("");

  const load = async () => {
    setError("");
    try {
      setInfo(await invoke<IdentityInfo>("get_device_identity"));
    } catch (e) {
      setError(String(e));
    }
  };

  useEffect(() => {
    load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <Dialog open onClose={onClose} title="Device identity" width={420}>
      <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
        {error ? (
          <>
            <div style={{ fontSize: 12, color: "var(--danger)", lineHeight: 1.6 }}>{error}</div>
            <div>
              <Button size="sm" onClick={load}>Retry</Button>
            </div>
          </>
        ) : !info ? (
          <div style={{ fontSize: 12, color: "var(--text-muted)" }}>Reading Identity object (0x01)</div>
        ) : (
          <div>
            <div style={row}><span style={labelStyle}>Product</span><span style={valueStyle}>{info.product_name || "—"}</span></div>
            <div style={row}><span style={labelStyle}>Vendor</span><span style={valueStyle}>{info.vendor_name || `Vendor ${info.vendor_id}`}</span></div>
            <div style={row}><span style={labelStyle}>Vendor ID</span><span className="mono" style={valueStyle}>0x{info.vendor_id.toString(16).toUpperCase()}</span></div>
            <div style={row}><span style={labelStyle}>Device type</span><span className="mono" style={valueStyle}>0x{info.device_type.toString(16).toUpperCase()}</span></div>
            <div style={row}><span style={labelStyle}>Product code</span><span className="mono" style={valueStyle}>0x{info.product_code.toString(16).toUpperCase()}</span></div>
            <div style={row}><span style={labelStyle}>Revision</span><span className="mono" style={valueStyle}>{info.revision_major}.{info.revision_minor}</span></div>
            <div style={row}><span style={labelStyle}>Serial</span><span className="mono" style={valueStyle}>{serialHex(info.serial_number)}</span></div>
            <div style={{ ...row, borderBottom: "none" }}><span style={labelStyle}>Status</span><span className="mono" style={valueStyle}>0x{info.status.toString(16).padStart(4, "0").toUpperCase()}</span></div>
          </div>
        )}
        <div style={{ fontSize: 11, color: "var(--text-muted)", lineHeight: 1.5 }}>
          Read via GetAttributeSingle on Identity object (class 0x01, instance 1).
        </div>
      </div>
    </Dialog>
  );
}
