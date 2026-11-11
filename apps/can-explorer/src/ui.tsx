import { Button, EmptyState, ProBadge } from "@mactools/ui";
import { Lock } from "lucide-react";

export type AddLog = (direction: "tx" | "rx" | "info" | "error", message: string) => void;
export type RequirePro = (feature: string) => boolean;

export const card: React.CSSProperties = {
  background: "var(--card)",
  border: "1px solid var(--border)",
  borderRadius: "var(--radius-lg)",
  padding: 12,
  minWidth: 0,
};

export const hint: React.CSSProperties = { fontSize: "var(--fs-xs)", color: "var(--text-muted)", lineHeight: 1.5 };

export const row: React.CSSProperties = { display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap" };

export const th: React.CSSProperties = { position: "sticky", top: 0, zIndex: 1, background: "var(--card)" };

export const mono = (v: unknown) => <span className="mono">{String(v ?? "")}</span>;

/** Shown in place of a Pro view's content. */
export function Locked({ feature, requirePro, description }: { feature: string; requirePro: RequirePro; description?: string }) {
  return (
    <EmptyState
      icon={<Lock size={20} strokeWidth={1.75} />}
      title={`${feature} is a Pro feature`}
      description={description ?? "Monitoring, the trace, DBC decoding and single frames stay free."}
      action={
        <Button variant="primary" onClick={() => requirePro(feature)}>
          Unlock Pro <ProBadge />
        </Button>
      }
    />
  );
}

/** Data bytes, the ones that changed in the last second highlighted. */
export function Bytes({ data, changedAt, now }: { data: number[]; changedAt?: number[]; now?: number }) {
  // CAN FD payloads (up to 64 bytes) wrap into lines of 16; classic frames stay on one line.
  const wrap = data.length > 16;
  return (
    <span className="mono" style={wrap ? { display: "inline-flex", flexWrap: "wrap", rowGap: 1, maxWidth: "calc(16 * (2ch + 6px))" } : { whiteSpace: "nowrap" }}>
      {data.map((b, i) => {
        const age = changedAt && now ? (now - (changedAt[i] ?? 0)) / 1e6 : Infinity;
        const hot = age < 1;
        return (
          <span
            key={i}
            style={{
              marginRight: 4,
              padding: "0 1px",
              borderRadius: 3,
              color: hot ? "var(--accent-text)" : undefined,
              background: hot ? `color-mix(in srgb, var(--accent) ${Math.round(28 * (1 - age))}%, transparent)` : undefined,
            }}
          >
            {b.toString(16).toUpperCase().padStart(2, "0")}
          </span>
        );
      })}
    </span>
  );
}
