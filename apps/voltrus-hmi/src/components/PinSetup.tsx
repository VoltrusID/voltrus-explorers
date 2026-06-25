import { useState } from "react";
import { useStore } from "../store";
import type { AccessConfig } from "../types";

async function hashPin(pin: string, salt: string): Promise<string> {
  const data = new TextEncoder().encode(salt + pin);
  const buf = await crypto.subtle.digest("SHA-256", data);
  return Array.from(new Uint8Array(buf))
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

function generateSalt(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  return Array.from(bytes)
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

const styles = {
  container: {
    display: "flex",
    flexDirection: "column" as const,
    height: "100%",
    fontSize: 12,
  },
  section: {
    padding: "10px 12px",
    borderBottom: "1px solid var(--border)",
  },
  sectionTitle: {
    fontSize: 11,
    fontWeight: 600,
    textTransform: "uppercase" as const,
    letterSpacing: 0.5,
    color: "var(--text-muted)",
    marginBottom: 8,
  },
  row: {
    display: "flex",
    alignItems: "center",
    gap: 6,
    marginBottom: 6,
  },
  input: {
    flex: 1,
    padding: "4px 8px",
    border: "1px solid var(--border)",
    borderRadius: 4,
    background: "#1e1e2e",
    color: "var(--text)",
    fontSize: 12,
    outline: "none",
  },
  btn: {
    padding: "4px 10px",
    border: "1px solid var(--border)",
    borderRadius: 4,
    background: "var(--surface)",
    color: "var(--text)",
    fontSize: 11,
    cursor: "pointer",
    whiteSpace: "nowrap" as const,
  },
  btnActive: {
    padding: "4px 10px",
    border: "1px solid var(--accent)",
    borderRadius: 4,
    background: "var(--accent)",
    color: "#000",
    fontSize: 11,
    cursor: "pointer",
    whiteSpace: "nowrap" as const,
  },
  statusSet: {
    fontSize: 10,
    color: "#22c55e",
    flexShrink: 0,
  },
  statusUnset: {
    fontSize: 10,
    color: "#ef4444",
    flexShrink: 0,
  },
  note: {
    fontSize: 11,
    color: "var(--text-muted)",
    padding: "8px 12px",
    lineHeight: 1.5,
  },
  numberInput: {
    width: 60,
    padding: "4px 8px",
    border: "1px solid var(--border)",
    borderRadius: 4,
    background: "#1e1e2e",
    color: "var(--text)",
    fontSize: 12,
    outline: "none",
    textAlign: "right" as const,
  },
  label: {
    fontSize: 11,
    color: "var(--text)",
    minWidth: 72,
  },
};

export function PinSetup() {
  const accessConfig = useStore((s) => s.accessConfig);
  const setAccessConfig = useStore((s) => s.setAccessConfig);

  const [operatorPin, setOperatorPin] = useState("");
  const [supervisorPin, setSupervisorPin] = useState("");
  const [autoLogout, setAutoLogout] = useState(
    accessConfig?.autoLogoutMinutes ?? 5
  );
  const [setting, setSetting] = useState<"operator" | "supervisor" | null>(null);

  const handleSetPin = async (role: "operator" | "supervisor") => {
    const pin = role === "operator" ? operatorPin : supervisorPin;
    if (!pin || pin.length !== 4 || !/^\d{4}$/.test(pin)) return;

    setSetting(role);
    try {
      const salt = accessConfig?.salt || generateSalt();
      const hash = await hashPin(pin, salt);

      const config: AccessConfig = {
        operatorPinHash:
          role === "operator" ? hash : (accessConfig?.operatorPinHash ?? ""),
        supervisorPinHash:
          role === "supervisor" ? hash : (accessConfig?.supervisorPinHash ?? ""),
        salt,
        autoLogoutMinutes: autoLogout,
      };

      setAccessConfig(config);

      if (role === "operator") {
        setOperatorPin("");
      } else {
        setSupervisorPin("");
      }
    } finally {
      setSetting(null);
    }
  };

  const handleAutoLogoutChange = (minutes: number) => {
    setAutoLogout(minutes);
    if (accessConfig) {
      setAccessConfig({ ...accessConfig, autoLogoutMinutes: minutes });
    }
  };

  const operatorSet = !!accessConfig?.operatorPinHash;
  const supervisorSet = !!accessConfig?.supervisorPinHash;

  return (
    <div style={styles.container}>
      {/* Operator PIN */}
      <div style={styles.section}>
        <div style={styles.sectionTitle}>Operator PIN</div>
        <div style={styles.row}>
          <input
            type="password"
            maxLength={4}
            placeholder="4-digit PIN"
            value={operatorPin}
            onChange={(e) => {
              const v = e.target.value.replace(/\D/g, "").slice(0, 4);
              setOperatorPin(v);
            }}
            style={styles.input}
          />
          <button
            onClick={() => handleSetPin("operator")}
            disabled={operatorPin.length !== 4 || setting === "operator"}
            style={
              operatorPin.length === 4 && setting !== "operator"
                ? styles.btnActive
                : { ...styles.btn, opacity: operatorPin.length === 4 ? 1 : 0.5 }
            }
          >
            {setting === "operator" ? "..." : "Set"}
          </button>
          <span style={operatorSet ? styles.statusSet : styles.statusUnset}>
            {operatorSet ? "PIN set" : "No PIN set"}
          </span>
        </div>
      </div>

      {/* Supervisor PIN */}
      <div style={styles.section}>
        <div style={styles.sectionTitle}>Supervisor PIN</div>
        <div style={styles.row}>
          <input
            type="password"
            maxLength={4}
            placeholder="4-digit PIN"
            value={supervisorPin}
            onChange={(e) => {
              const v = e.target.value.replace(/\D/g, "").slice(0, 4);
              setSupervisorPin(v);
            }}
            style={styles.input}
          />
          <button
            onClick={() => handleSetPin("supervisor")}
            disabled={supervisorPin.length !== 4 || setting === "supervisor"}
            style={
              supervisorPin.length === 4 && setting !== "supervisor"
                ? styles.btnActive
                : { ...styles.btn, opacity: supervisorPin.length === 4 ? 1 : 0.5 }
            }
          >
            {setting === "supervisor" ? "..." : "Set"}
          </button>
          <span style={supervisorSet ? styles.statusSet : styles.statusUnset}>
            {supervisorSet ? "PIN set" : "No PIN set"}
          </span>
        </div>
      </div>

      {/* Auto-logout timer */}
      <div style={styles.section}>
        <div style={styles.sectionTitle}>Auto-Logout</div>
        <div style={styles.row}>
          <span style={styles.label}>Timeout</span>
          <input
            type="number"
            min={1}
            max={120}
            value={autoLogout}
            onChange={(e) => {
              const v = parseInt(e.target.value, 10);
              if (!isNaN(v) && v >= 1) handleAutoLogoutChange(Math.min(v, 120));
            }}
            style={styles.numberInput}
          />
          <span style={{ fontSize: 11, color: "var(--text-muted)" }}>min</span>
        </div>
      </div>

      {/* Per-widget note */}
      <div style={styles.note}>
        Set per-widget role in the Properties panel when a widget is selected.
      </div>
    </div>
  );
}
