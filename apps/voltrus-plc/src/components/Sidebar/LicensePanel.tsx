import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";

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
    <div style={{ padding: "8px 12px", fontSize: 12 }}>
      {license?.activated ? (
        <div>
          <div style={{
            display: "flex", alignItems: "center", gap: 8,
            color: "var(--accent)", marginBottom: 8,
          }}>
            <span style={{
              width: 8, height: 8, borderRadius: "50%",
              background: "var(--accent)", display: "inline-block",
            }} />
            <span style={{ fontWeight: 600 }}>Licensed</span>
          </div>
          <div style={{ color: "var(--text-secondary)", marginBottom: 4 }}>
            Key: {license.license_key.slice(0, 8)}...{license.license_key.slice(-4)}
          </div>
          <div style={{ display: "flex", gap: 6, marginTop: 8 }}>
            <button
              className="toolbar-btn"
              style={{ fontSize: 10, padding: "2px 8px" }}
              onClick={validate}
              disabled={loading}
            >
              Validate
            </button>
            <button
              className="toolbar-btn"
              style={{ fontSize: 10, padding: "2px 8px", color: "var(--danger)" }}
              onClick={deactivate}
              disabled={loading}
            >
              Deactivate
            </button>
          </div>
        </div>
      ) : (
        <div>
          <div style={{ color: "var(--text-secondary)", marginBottom: 8 }}>
            Enter your license key to activate Voltrus PLC.
          </div>
          <input
            type="text"
            value={key}
            onChange={(e) => setKey(e.target.value)}
            placeholder="XXXX-XXXX-XXXX-XXXX"
            style={{
              width: "100%",
              background: "var(--bg-highlight)",
              border: "1px solid var(--border)",
              borderRadius: 4,
              color: "var(--text-primary)",
              padding: "6px 8px",
              fontSize: 12,
              fontFamily: "var(--font-mono)",
              marginBottom: 8,
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") activate();
            }}
          />
          <button
            className="toolbar-btn primary"
            style={{ width: "100%", fontSize: 11 }}
            onClick={activate}
            disabled={loading || !key.trim()}
          >
            {loading ? "Activating..." : "Activate License"}
          </button>
          <div style={{ marginTop: 8, fontSize: 11, color: "var(--text-muted)" }}>
            Don't have a key?{" "}
            <a
              href="https://voltrus.id/voltrus-plc/#buy"
              target="_blank"
              rel="noopener"
              style={{ color: "var(--accent)", textDecoration: "none" }}
            >
              Get one here
            </a>
          </div>
        </div>
      )}

      {error && (
        <div style={{ marginTop: 8, color: "var(--danger)", fontSize: 11 }}>
          {error}
        </div>
      )}
      {success && (
        <div style={{ marginTop: 8, color: "var(--accent)", fontSize: 11 }}>
          {success}
        </div>
      )}
    </div>
  );
}
