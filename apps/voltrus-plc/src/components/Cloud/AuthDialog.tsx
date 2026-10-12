import { useState, type FormEvent } from "react";
import { Button, Callout, Dialog, Field } from "@mactools/ui";
import { useCloudStore } from "../../store/cloud";
import "./cloud.css";

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
    <Dialog
      open
      onClose={dismissAuth}
      title={mode === "login" ? "Sign in to Voltrus Cloud" : "Create Voltrus Cloud account"}
      width={360}
      footer={
        <>
          <Button variant="ghost" onClick={() => setMode(mode === "login" ? "register" : "login")}>
            {mode === "login" ? "Need an account? Register" : "Have an account? Sign in"}
          </Button>
          <Button onClick={dismissAuth}>
            Cancel
          </Button>
          <Button variant="primary" type="submit" form="cld-auth-form" disabled={busy}>
            {busy ? "Please wait…" : mode === "login" ? "Sign in" : "Register"}
          </Button>
        </>
      }
    >
      <form id="cld-auth-form" onSubmit={submit} className="cld-form">
        <Field label="Email">
          {(id) => (
            <input
              id={id}
              className="vt-input"
              type="email"
              required
              autoComplete="email"
              placeholder="Email"
              aria-label="Email"
              value={email}
              onChange={(e) => setEmail(e.target.value)}
            />
          )}
        </Field>
        <Field label="Password">
          {(id) => (
            <input
              id={id}
              className="vt-input"
              type="password"
              required
              autoComplete={mode === "login" ? "current-password" : "new-password"}
              placeholder="Password"
              aria-label="Password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
            />
          )}
        </Field>
        {error && <Callout tone="danger">{error}</Callout>}
      </form>
    </Dialog>
  );
}
