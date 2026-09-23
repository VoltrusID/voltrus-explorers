import { useState, type FormEvent } from "react";
import { useCloudStore } from "../../store/cloud";

// Cloud sign-in / registration modal. Auto-shows when the cloud store sets
// authRequired (401 from any cloud action); server error messages from 4xx
// responses are rendered verbatim.
export default function AuthDialog() {
  const authRequired = useCloudStore((s) => s.authRequired);
  const busy = useCloudStore((s) => s.busy);
  const error = useCloudStore((s) => s.error);
  const login = useCloudStore((s) => s.login);
  const register = useCloudStore((s) => s.register);
  const dismissAuth = useCloudStore((s) => s.dismissAuth);

  const [mode, setMode] = useState<"login" | "register">("login");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");

  if (!authRequired) return null;

  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (busy) return;
    if (mode === "login") void login(email.trim(), password);
    else void register(email.trim(), password);
  };

  return (
    <div className="modal-overlay" onClick={dismissAuth}>
      <div
        className="modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="cloud-auth-title"
        onClick={(e) => e.stopPropagation()}
      >
        <h3 id="cloud-auth-title">{mode === "login" ? "Sign in to Voltrus Cloud" : "Create Voltrus Cloud account"}</h3>
        <form onSubmit={submit} style={{ display: "flex", flexDirection: "column", gap: 8 }}>
          <input
            className="auth-input"
            type="email"
            required
            autoComplete="email"
            placeholder="Email"
            aria-label="Email"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
          />
          <input
            className="auth-input"
            type="password"
            required
            autoComplete={mode === "login" ? "current-password" : "new-password"}
            placeholder="Password"
            aria-label="Password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
          />
          {error && (
            <div role="alert" style={{ color: "var(--danger)", fontSize: 11 }}>
              {error}
            </div>
          )}
          <button className="toolbar-btn primary" type="submit" disabled={busy}>
            {busy ? "Please wait…" : mode === "login" ? "Sign in" : "Register"}
          </button>
        </form>
        <button
          className="toolbar-btn"
          style={{ marginTop: 8 }}
          onClick={() => setMode(mode === "login" ? "register" : "login")}
        >
          {mode === "login" ? "Need an account? Register" : "Have an account? Sign in"}
        </button>
        <button className="toolbar-btn" style={{ marginTop: 8 }} onClick={dismissAuth}>
          Cancel
        </button>
      </div>
    </div>
  );
}
