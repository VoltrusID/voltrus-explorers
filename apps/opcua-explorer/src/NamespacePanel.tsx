import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Button, Dialog, EmptyState, ListGroup, PageTitle, useIsPhone } from "@mactools/ui";

export interface OpcNamespace {
  index: number;
  uri: string;
}

type Dir = "tx" | "rx" | "info" | "error";

export function NamespacePanel({
  session,
  onClose,
  onLog,
  inline,
}: {
  /** Id of the client session to read. */
  session: string;
  onClose: () => void;
  onLog?: (d: Dir, m: string) => void;
  /** On a phone the list is a tab, not a dialog. */
  inline?: boolean;
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

  const phone = useIsPhone();
  if (phone && inline) {
    return (
      <>
        <PageTitle title="Namespaces" caption="From the server" actions={<Button size="sm" loading={loading} onClick={load}>Refresh</Button>} />
        {err ? (
          <div className="vt-hint vt-hint--error" role="alert">{err}</div>
        ) : items.length === 0 ? (
          <EmptyState compact title={loading ? "Reading namespaces" : "No namespaces"} description={loading ? undefined : "The server returned an empty NamespaceArray."} />
        ) : (
          <ListGroup>
            {items.map((n) => (
              <div key={n.index} className="vm-row oc-kv">
                <span className="oc-kv__k oc-kv__k--raw">ns={n.index}</span>
                <span className="oc-kv__v">{n.uri}</span>
              </div>
            ))}
          </ListGroup>
        )}
      </>
    );
  }

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
