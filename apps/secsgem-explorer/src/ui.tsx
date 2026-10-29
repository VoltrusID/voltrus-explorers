import { Badge, Button, EmptyState, ProBadge } from "@mactools/ui";
import { Lock } from "lucide-react";
import type { AckView } from "./types";
import { ackText } from "./gem";

export type AddLog = (direction: "tx" | "rx" | "info" | "error", message: string) => void;
export type RequirePro = (feature: string) => boolean;

export const mono = (v: unknown) => <span className="mono">{String(v ?? "")}</span>;

export const card: React.CSSProperties = {
  background: "var(--card)",
  border: "1px solid var(--border)",
  borderRadius: "var(--radius-lg)",
  padding: 12,
  minWidth: 0,
};

export const hint: React.CSSProperties = { fontSize: "var(--fs-xs)", color: "var(--text-muted)", lineHeight: 1.5 };

export const row: React.CSSProperties = { display: "flex", gap: 8, alignItems: "flex-end", flexWrap: "wrap" };

export function KeyValues({ title, rows }: { title: string; rows: [string, React.ReactNode][] }) {
  return (
    <div style={{ ...card, flex: 1 }}>
      <div style={{ fontSize: "var(--fs-xs)", fontWeight: 600, color: "var(--text-dim)", marginBottom: 6 }}>{title}</div>
      {rows.map(([k, v]) => (
        <div
          key={k}
          style={{ display: "flex", justifyContent: "space-between", gap: 8, padding: "3px 0", borderBottom: "1px solid var(--border)", fontSize: "var(--fs-xs)" }}
        >
          <span style={{ color: "var(--text-dim)" }}>{k}</span>
          <span className="mono" style={{ color: "var(--text)", textAlign: "right" }}>{v}</span>
        </div>
      ))}
    </div>
  );
}

export function AckBadge({ ack }: { ack: AckView }) {
  return <Badge tone={ack.ok ? "success" : "danger"}>{ackText(ack)}</Badge>;
}

/** Shown in place of a Pro view's content. */
export function Locked({ feature, requirePro }: { feature: string; requirePro: RequirePro }) {
  return (
    <EmptyState
      icon={<Lock size={20} strokeWidth={1.75} />}
      title={`${feature} is a Pro feature`}
      description="Everything else in the app stays free."
      action={
        <Button variant="primary" onClick={() => requirePro(feature)}>
          Unlock Pro <ProBadge />
        </Button>
      }
    />
  );
}

export const clock = (ms: number) =>
  new Date(ms).toLocaleTimeString("en-US", { hour12: false, hour: "2-digit", minute: "2-digit", second: "2-digit", fractionalSecondDigits: 3 });
