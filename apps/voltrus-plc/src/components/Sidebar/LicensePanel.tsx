import { useState, useEffect } from "react";
import { invoke } from "../../api/bridge";
import { Button, StatusDot } from "@mactools/ui";

interface LicenseInfo {
  activated: boolean;
  license_key: string;
  activation_id: string;
  device_id: string;
}

export default function LicensePanel() {
  const [license, setLicense] = useState<LicenseInfo | null>(null);
  const [key, setKey] = useState("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [success, setSuccess] = useState("");

  useEffect(() => {
    invoke<LicenseInfo>("get_license_status").then(setLicense);
  }, []);

  const activate = async () => {
    if (!key.trim()) return;
    setLoading(true);
    setError("");
    setSuccess("");
    try {
      const result = await invoke<LicenseInfo>("activate_license", {
        licenseKey: key.trim(),
      });
      setLicense(result);
      setSuccess("License activated successfully.");
      setKey("");
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  const validate = async () => {
    setLoading(true);
    setError("");
    setSuccess("");
    try {
      const result = await invoke<LicenseInfo>("validate_license");
      setLicense(result);
      setSuccess("License is valid.");
    } catch (e) {
      setError(String(e));
      setLicense((prev) => prev ? { ...prev, activated: false } : null);
    } finally {
      setLoading(false);
    }
  };

  const deactivate = async () => {
    setLoading(true);
    try {
      await invoke("deactivate_license");
      setLicense(null);
      setSuccess("License removed.");
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8, fontSize: "var(--fs-sm)" }}>
      {license?.activated ? (
        <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
          <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
            <StatusDot tone="success" label="Licensed" />
            <span style={{ fontWeight: 600 }}>Licensed</span>
          </div>
          <div style={{ color: "var(--text-dim)", fontFamily: "var(--font-mono)", fontSize: "var(--fs-xs)" }}>
            Key: {license.license_key.slice(0, 8)}...{license.license_key.slice(-4)}
          </div>
          <div style={{ display: "flex", gap: 6 }}>
            <Button size="sm" variant="default" onClick={validate} disabled={loading}>
              Validate
            </Button>
            <Button size="sm" variant="ghost" style={{ color: "var(--danger)" }} onClick={deactivate} disabled={loading}>
              Deactivate
            </Button>
          </div>
        </div>
      ) : (
        <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
          <div style={{ color: "var(--text-dim)" }}>
            Enter your license key to activate Voltrus PLC.
          </div>
          <input
            className="vt-input vt-input--mono"
            type="text"
            value={key}
            onChange={(e) => setKey(e.target.value)}
            placeholder="XXXX-XXXX-XXXX-XXXX"
            aria-label="License key"
            onKeyDown={(e) => {
              if (e.key === "Enter") activate();
            }}
          />
          <Button
            variant="primary"
            block
            loading={loading}
            onClick={activate}
            disabled={loading || !key.trim()}
          >
            {loading ? "Activating..." : "Activate license"}
          </Button>
          <div style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)" }}>
            Don't have a key?{" "}
            <a
              href="https://voltrus.id/voltrus-plc/#buy"
              target="_blank"
              rel="noopener"
              style={{ color: "var(--accent-text)", textDecoration: "none" }}
            >
              Get one here
            </a>
          </div>
        </div>
      )}

      {error && (
        <div role="alert" style={{ color: "var(--danger)", fontSize: "var(--fs-xs)" }}>
          {error}
        </div>
      )}
      {success && (
        <div style={{ color: "var(--accent-text)", fontSize: "var(--fs-xs)" }}>
          {success}
        </div>
      )}
    </div>
  );
}
