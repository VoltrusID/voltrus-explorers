import { useState, type FormEvent } from "react";
import { Button, Segmented } from "@mactools/ui";
import { Zap } from "lucide-react";
import { bridgeMode } from "../../api/bridge";
import { useCloudStore } from "../../store/cloud";
import "./cloud.css";

// Full-page sign-in/register for the web build. The cloud IDE requires an
// account for sync, but local (in-browser simulation) editing stays available
// via "Continue without an account" — the paywall gates cloud storage, not
// the editor. Desktop (tauri mode) keeps the lazy AuthDialog instead.
const SKIP_KEY = "vplc_auth_skipped";

export default function AuthScreen() {
  const token = useCloudStore((s) => s.token);
  const busy = useCloudStore((s) => s.busy);
  const error = useCloudStore((s) => s.error);
  const login = useCloudStore((s) => s.login);
  const register = useCloudStore((s) => s.register);

  const [mode, setMode] = useState<"login" | "register">("login");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [skipped, setSkipped] = useState(() => sessionStorage.getItem(SKIP_KEY) === "1");

  if (bridgeMode === "tauri" || token || skipped) return null;

  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (busy) return;
    if (mode === "login") void login(email.trim(), password);
    else void register(email.trim(), password);
  };

  const skip = () => {
    sessionStorage.setItem(SKIP_KEY, "1");
    setSkipped(true);
  };

  return (
    <div className="auth-screen">
      <div className="auth-card" role="dialog" aria-modal="true" aria-labelledby="auth-screen-title">
        <div className="auth-brand">
          <span className="auth-logo"><Zap size={22} strokeWidth={1.75} /></span>
          <h1 id="auth-screen-title">Voltrus PLC Cloud</h1>
          <p className="auth-tagline">IEC 61131-3 programming in your browser.</p>
        </div>

        <div className="cld-auth-tabs">
          <Segmented<"login" | "register">
            value={mode}
            onChange={setMode}
            label="Sign in or register"
            block
            options={[
              { value: "login", label: "Sign in" },
              { value: "register", label: "Register" },
            ]}
          />
        </div>

        <form onSubmit={submit} className="auth-form">
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
            <div role="alert" className="auth-error">
              {error}
            </div>
          )}
          <Button className="auth-submit" variant="primary" block type="submit" disabled={busy}>
            {busy ? "Please wait…" : mode === "login" ? "Sign in" : "Create account"}
          </Button>
        </form>

        <button className="auth-skip" onClick={skip}>
          Continue without an account — edit locally
        </button>
        <p className="auth-footnote">
          Free plan: 1 cloud project, 25 files. Your programs stay on your device until you
          save them to the cloud.
        </p>
      </div>
    </div>
  );
}
