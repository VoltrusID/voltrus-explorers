import { useEffect } from "react";
import { useCloudStore } from "../../store/cloud";

// Signed-in identity row for the Cloud sidebar section: email + logout, or a
// sign-in prompt when there is no session.
export default function AccountBadge() {
  const user = useCloudStore((s) => s.user);
  const token = useCloudStore((s) => s.token);
  const whoami = useCloudStore((s) => s.whoami);
  const logout = useCloudStore((s) => s.logout);
  const openAuth = useCloudStore((s) => s.openAuth);

  // Restored session (token in localStorage) has no user until /me answers.
  useEffect(() => {
    if (token && !user) void whoami();
  }, [token, user, whoami]);

  if (!token) {
    return (
      <div style={{ padding: "8px 12px" }}>
        <button className="toolbar-btn" style={{ width: "100%" }} onClick={openAuth}>
          Sign in to Voltrus Cloud
        </button>
      </div>
    );
  }

  return (
    <div
      style={{
        padding: "8px 12px",
        display: "flex",
        alignItems: "center",
        gap: 6,
        fontSize: 11,
      }}
    >
      <span
        style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}
        title={user ? `Signed in as ${user.email}` : "Restoring session…"}
      >
        {user ? user.email : "Restoring session…"}
      </span>
      <button className="toolbar-btn" style={{ fontSize: 10, padding: "2px 8px" }} onClick={logout}>
        Log out
      </button>
    </div>
  );
}
