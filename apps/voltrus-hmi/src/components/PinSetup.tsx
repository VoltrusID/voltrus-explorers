import { useEffect, useState } from "react";
import { useStore } from "../store";
import { stretchPin } from "../crypto";
import type { AccessConfig } from "../types";
import { Badge, Button } from "@mactools/ui";

function generateSalt(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  return Array.from(bytes)
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

export function PinSetup() {
  const accessConfig = useStore((s) => s.accessConfig);
  const setAccessConfig = useStore((s) => s.setAccessConfig);

  const [operatorPin, setOperatorPin] = useState("");
  const [supervisorPin, setSupervisorPin] = useState("");
  const [autoLogout, setAutoLogout] = useState(
    accessConfig?.autoLogoutMinutes ?? 5
  );
  const [setting, setSetting] = useState<"operator" | "supervisor" | null>(null);

  // Keep the local timeout field in sync when a project is (re)loaded while
  // this panel stays mounted — otherwise "Set" would persist a stale value.
  useEffect(() => {
    setAutoLogout(accessConfig?.autoLogoutMinutes ?? 5);
  }, [accessConfig?.salt, accessConfig?.autoLogoutMinutes]);

  const handleSetPin = async (role: "operator" | "supervisor") => {
    const pin = role === "operator" ? operatorPin : supervisorPin;
    if (!pin || pin.length !== 4 || !/^\d{4}$/.test(pin)) return;

    setSetting(role);
    try {
      const salt = accessConfig?.salt || generateSalt();
      // Must match the runtime's stretchPin (export/auth.rs) exactly.
      const hash = await Promise.resolve(stretchPin(salt, pin));

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
    <div className="acc-body">
      {/* Operator PIN */}
      <div className="sub-block">
        <div className="sub-title">
          <span>Operator PIN</span>
          {operatorSet ? <Badge tone="success">PIN set</Badge> : <Badge>No PIN set</Badge>}
        </div>
        <div className="sub-actions">
          <input
            type="password"
            maxLength={4}
            placeholder="4-digit PIN"
            aria-label="Operator PIN"
            value={operatorPin}
            onChange={(e) => {
              const v = e.target.value.replace(/\D/g, "").slice(0, 4);
              setOperatorPin(v);
            }}
            className="vt-input vt-input--mono"
            style={{ flex: 1, minWidth: 0 }}
          />
          <Button
            size="sm"
            variant="primary"
            onClick={() => handleSetPin("operator")}
            disabled={operatorPin.length !== 4 || setting === "operator"}
            loading={setting === "operator"}
            style={{ flexShrink: 0 }}
          >
            Set
          </Button>
        </div>
      </div>

      {/* Supervisor PIN */}
      <div className="sub-block">
        <div className="sub-title">
          <span>Supervisor PIN</span>
          {supervisorSet ? <Badge tone="success">PIN set</Badge> : <Badge>No PIN set</Badge>}
        </div>
        <div className="sub-actions">
          <input
            type="password"
            maxLength={4}
            placeholder="4-digit PIN"
            aria-label="Supervisor PIN"
            value={supervisorPin}
            onChange={(e) => {
              const v = e.target.value.replace(/\D/g, "").slice(0, 4);
              setSupervisorPin(v);
            }}
            className="vt-input vt-input--mono"
            style={{ flex: 1, minWidth: 0 }}
          />
          <Button
            size="sm"
            variant="primary"
            onClick={() => handleSetPin("supervisor")}
            disabled={supervisorPin.length !== 4 || setting === "supervisor"}
            loading={setting === "supervisor"}
            style={{ flexShrink: 0 }}
          >
            Set
          </Button>
        </div>
      </div>

      {/* Auto-logout timer */}
      <div className="sub-block">
        <div className="sub-title">Auto-logout</div>
        <div className="sub-actions">
          <span className="prop-label" style={{ width: "auto" }}>
            Timeout
          </span>
          <input
            type="number"
            min={1}
            max={120}
            aria-label="Auto-logout timeout in minutes"
            value={autoLogout}
            onChange={(e) => {
              const v = parseInt(e.target.value, 10);
              if (!isNaN(v) && v >= 1) handleAutoLogoutChange(Math.min(v, 120));
            }}
            className="vt-input vt-input--sm"
            style={{ width: 60, textAlign: "right", flexShrink: 0 }}
          />
          <span className="row-meta">min</span>
        </div>
      </div>

      {/* Per-widget note */}
      <div className="sub-block">
        <div className="sub-empty" style={{ textAlign: "left", padding: 0 }}>
          Set the per-widget role in the properties panel when a widget is selected.
        </div>
      </div>
    </div>
  );
}
