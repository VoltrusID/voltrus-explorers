import { useEffect } from "react";
import { Button } from "@mactools/ui";
import { LogIn } from "lucide-react";
import { useCloudStore } from "../../store/cloud";
import "./cloud.css";

// Signed-in identity row for the Cloud sidebar section: email + plan chip +
// logout, or a sign-in prompt when there is no session.
export default function AccountBadge() {
  const user = useCloudStore((s) => s.user);
  const token = useCloudStore((s) => s.token);
  const plan = useCloudStore((s) => s.plan);
  const whoami = useCloudStore((s) => s.whoami);
  const logout = useCloudStore((s) => s.logout);
  const openAuth = useCloudStore((s) => s.openAuth);
  const openUpgrade = useCloudStore((s) => s.openUpgrade);

  // Restored session (token in localStorage) has no user until /me answers.
  useEffect(() => {
    if (token && !user) void whoami();
  }, [token, user, whoami]);

  if (!token) {
    return (
      <div className="cld-row">
        <Button
          size="sm"
          variant="primary"
          block
          icon={<LogIn size={14} strokeWidth={1.75} />}
          onClick={openAuth}
        >
          Sign in to Voltrus Cloud
        </Button>
      </div>
    );
  }

  const planName = plan && (plan.plan === "solo" || plan.plan === "team") ? plan.plan : plan ? "free" : "…";

  return (
    <div className="cld-row cld-row--spread">
      <span
        className="cld-email"
        title={user ? `Signed in as ${user.email}` : "Restoring session…"}
      >
        {user ? user.email : "Restoring session…"}
      </span>
      <button
        className={`plan-chip plan-chip-${planName}`}
        onClick={openUpgrade}
        title="Manage your plan"
      >
        {planName}
      </button>
      <Button size="sm" variant="ghost" onClick={logout}>
        Log out
      </Button>
    </div>
  );
}
