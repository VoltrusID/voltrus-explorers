import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Button, Dialog, EmptyState } from "@mactools/ui";

export interface OpcNamespace {
  index: number;
  uri: string;
}

type Dir = "tx" | "rx" | "info" | "error";

export function NamespacePanel({
  session,
  onClose,
  onLog,
}: {
  /** Id of the client session to read. */
  session: string;
  onClose: () => void;
  onLog?: (d: Dir, m: string) => void;
}) {
  const [items, setItems] = useState<OpcNamespace[]>([]);
  const [err, setErr] = useState("");
  const [loading, setLoading] = useState(true);

  const load = async () => {
    setLoading(true); setErr("");
    try {
      const ns = (await invoke<OpcNamespace[]>("read_namespaces", { session })) ?? [];
      setItems(ns);
      onLog?.("rx", `Loaded ${ns.length} namespaces`);
    } catch (e) { setErr(String(e)); onLog?.("error", String(e)); }
    setLoading(false);
  };

  useEffect(() => { load(); /* eslint-disable-next-line */ }, []);

  return (
    <Dialog open onClose={onClose} title="Namespace index" width={560}>
      {err ? (
        <div className="vt-hint vt-hint--error" role="alert">{err}</div>
      ) : loading ? (
        <span className="vt-hint">Reading NamespaceArray</span>
      ) : items.length === 0 ? (
        <EmptyState compact title="No namespaces" description="The server returned an empty NamespaceArray." />
      ) : (
        <div style={{ overflow: "auto" }}>
          {items.map((n) => (
            <div key={n.index} style={{ display: "flex", gap: 12, padding: "5px 0", borderBottom: "1px solid var(--border)" }}>
              <span className="mono" style={{ color: "var(--accent)", width: 44, textAlign: "right" }}>ns={n.index}</span>
              <span className="mono" style={{ color: "var(--text)" }}>{n.uri}</span>
            </div>
          ))}
        </div>
      )}
      <div style={{ display: "flex", justifyContent: "flex-end" }}>
        <Button size="sm" disabled={loading} onClick={load}>Refresh</Button>
      </div>
    </Dialog>
  );
}
