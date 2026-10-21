import { useStore } from "../store";

export function MobileTabBar() {
  const { mobileTab, setMobileTab, mode, slaveRunning } = useStore();

  const tabs = ([
    { id: "connect" as const, label: "Connect" },
    { id: "data" as const, label: "Data" },
    ...(mode === "master" ? [{ id: "blocks" as const, label: "Blocks" }] : [{ id: "template" as const, label: "Template" }]),
    { id: "log" as const, label: "Log" },
  ] as const);

  return (
    <nav style={{
      display: "flex",
      background: "var(--card)",
      borderTop: "1px solid var(--border)",
      flexShrink: 0,
      paddingBottom: "env(safe-area-inset-bottom, 0px)",
    }}>
      {tabs.map((tab) => (
        <button
          key={tab.id}
          onClick={() => setMobileTab(tab.id)}
          style={{
            flex: 1,
            padding: "10px 0",
            fontSize: "var(--fs-xs)",
            fontWeight: 600,
            cursor: "pointer",
            border: "none",
            borderBottom: mobileTab === tab.id ? "2px solid var(--accent)" : "2px solid transparent",
            color: mobileTab === tab.id ? "var(--accent)" : "var(--text-muted)",
            background: "transparent",
            transition: "color 0.15s, border-color 0.15s",
            textAlign: "center",
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            gap: 4,
          }}
        >
          {tab.label}
          {mode === "slave" && slaveRunning && tab.id === "connect" && (
            <span style={{ width: 6, height: 6, borderRadius: "50%", background: "var(--accent)", display: "inline-block" }} />
          )}
        </button>
      ))}
    </nav>
  );
}
