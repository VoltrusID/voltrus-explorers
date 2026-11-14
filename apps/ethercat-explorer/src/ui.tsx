import { Badge, Button, EmptyState, ProBadge } from "@mactools/ui";
import { Lock } from "lucide-react";
import { stateTone } from "./ecat";

export type AddLog = (direction: "tx" | "rx" | "info" | "error", message: string) => void;
export type RequirePro = (feature: string) => boolean;

export const card: React.CSSProperties = {
  background: "var(--card)",
  border: "1px solid var(--border)",
  borderRadius: "var(--radius-lg)",
  padding: 12,
  minWidth: 0,
};

export const scroller: React.CSSProperties = {
  flex: 1,
  minHeight: 0,
  overflow: "auto",
  background: "var(--card)",
  border: "1px solid var(--border)",
  borderRadius: "var(--radius-lg)",
};

export const hint: React.CSSProperties = { fontSize: "var(--fs-xs)", color: "var(--text-muted)", lineHeight: 1.5 };

export const row: React.CSSProperties = { display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap" };

export const th: React.CSSProperties = { position: "sticky", top: 0, zIndex: 1, background: "var(--card)" };

export const kv: React.CSSProperties = { display: "grid", gridTemplateColumns: "max-content 1fr", columnGap: 16, rowGap: 4, fontSize: "var(--fs-sm)" };

export const dim: React.CSSProperties = { color: "var(--text-muted)" };

export function StateBadge({ state, alText }: { state: string; alText?: string | null }) {
  return (
    <span style={{ display: "inline-flex", gap: 6, alignItems: "center", minWidth: 0 }}>
      <Badge tone={stateTone(state, !!alText)}>{state}</Badge>
      {alText && <span style={{ fontSize: "var(--fs-xs)", color: "var(--danger-text, var(--danger))" }}>{alText}</span>}
    </span>
  );
}

/** Shown in place of a Pro view's content. */
export function Locked({ feature, requirePro, description }: { feature: string; requirePro: RequirePro; description?: string }) {
  return (
    <EmptyState
      icon={<Lock size={20} strokeWidth={1.75} />}
      title={`${feature} is a Pro feature`}
      description={description ?? "Scanning, diagnostics, reading the object dictionary and live inputs stay free."}
      action={
        <Button variant="primary" onClick={() => requirePro(feature)}>
          Unlock Pro <ProBadge />
        </Button>
      }
    />
  );
}
