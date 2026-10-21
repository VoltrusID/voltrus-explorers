import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Badge, Button, DataTable, Dialog, type Column, type Tone } from "@mactools/ui";

export interface CertInfo {
  thumbprint: string;
  subject: string;
  issuer: string;
  valid_from: string;
  valid_to: string;
  bucket: string; // own | trusted | rejected
  kind: string;   // client | server
  is_ca: boolean;
  path: string;
}

type Dir = "tx" | "rx" | "info" | "error";

const shortTp = (t: string): string => t.length > 16 ? `${t.slice(0, 16)}…` : t;
const shortSub = (s: string): string => {
  const cn = s.match(/CN=([^,]+)/);
  return cn ? cn[1]! : (s.length > 40 ? `${s.slice(0, 40)}…` : s);
};
const shortDate = (iso: string): string => {
  if (!iso) return "—";
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toISOString().slice(0, 10);
};

const bucketTone = (b: string): Tone => (b === "own" ? "accent" : b === "trusted" ? "success" : "danger");

export function CertManager({
  onClose,
  onLog,
}: {
  onClose: () => void;
  onLog?: (d: Dir, m: string) => void;
}) {
  const [certs, setCerts] = useState<CertInfo[]>([]);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");

  const load = async () => {
    setBusy(true); setErr("");
    try {
      const list = (await invoke<CertInfo[]>("cert_list")) ?? [];
      setCerts(list);
      onLog?.("rx", `Loaded ${list.length} certificates`);
    } catch (e) { setErr(String(e)); onLog?.("error", String(e)); }
    setBusy(false);
  };

  useEffect(() => { load(); /* eslint-disable-next-line */ }, []);

  const act = async (fn: () => Promise<void>, msg: string) => {
    setBusy(true); setErr("");
    try { await fn(); onLog?.("info", msg); await load(); }
    catch (e) { setErr(String(e)); onLog?.("error", String(e)); }
    setBusy(false);
  };

  const columns: Column[] = [
    { key: "bucket", label: "Bucket", width: "90px" },
    { key: "kind", label: "Kind", width: "70px" },
    { key: "subject", label: "Subject", width: "180px" },
    { key: "valid_to", label: "Valid to", width: "90px" },
    { key: "thumbprint", label: "Thumbprint", width: "140px" },
    { key: "actions", label: "Actions", width: "120px" },
  ];

  const rows = certs.map((c) => ({
    bucket: c.bucket,
    kind: c.kind,
    subject: shortSub(c.subject),
    valid_to: shortDate(c.valid_to),
    thumbprint: shortTp(c.thumbprint),
    actions: "",
    _c: c,
  }));

  return (
    <Dialog open onClose={onClose} title="Certificate manager" width={820}>
      <span className="vt-hint">
        PKI under <code>$TMPDIR/opcua-explorer-pki</code> and <code>opcua-explorer-server-*</code>. Trusted certificates are used to validate secure (Sign / SignAndEncrypt) connections. Changes apply on the next connect or server start.
      </span>

      <div style={{ display: "flex", gap: 6 }}>
        <Button size="sm" disabled={busy} onClick={load}>Refresh</Button>
        <Button size="sm" disabled={busy} onClick={() => act(() => invoke("cert_generate"), "Own certificate marked for regeneration")}>
          Regenerate own
        </Button>
      </div>

      {err && <div className="vt-hint vt-hint--error" role="alert">{err}</div>}

      <div style={{ flex: 1, minHeight: 0, overflow: "auto" }}>
        <DataTable
          columns={columns}
          data={rows}
          emptyMessage={busy ? "Reading PKI" : "No certificates yet. Connect or start the server once to generate them."}
          renderCell={(key, row) => {
            const r = row as Record<string, unknown>;
            if (key === "bucket") {
              const b = String(r.bucket);
              return <Badge tone={bucketTone(b)}>{b}</Badge>;
            }
            if (key === "thumbprint") {
              return <span className="mono">{String(r.thumbprint ?? "")}</span>;
            }
            if (key === "actions") {
              const c = r._c as CertInfo;
              return (
                <span style={{ display: "flex", gap: 4 }}>
                  {c.bucket === "rejected" && (
                    <Button size="sm" disabled={busy} onClick={() => act(() => invoke("cert_trust", { thumbprint: c.thumbprint }), `Trusted ${shortTp(c.thumbprint)}`)}>
                      Trust
                    </Button>
                  )}
                  {c.bucket === "trusted" && (
                    <Button size="sm" variant="danger" disabled={busy} onClick={() => act(() => invoke("cert_reject", { thumbprint: c.thumbprint }), `Rejected ${shortTp(c.thumbprint)}`)}>
                      Reject
                    </Button>
                  )}
                  {c.bucket === "own" && <span style={{ color: "var(--text-muted)", fontSize: "var(--fs-xs)" }}>App certificate</span>}
                </span>
              );
            }
            return String(r[key] ?? "");
          }}
        />
      </div>
    </Dialog>
  );
}
