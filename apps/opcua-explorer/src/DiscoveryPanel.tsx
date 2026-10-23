import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Badge, Button, DataTable, Dialog, EmptyState, Field, ListGroup, ListRow, useIsPhone, type Column } from "@mactools/ui";

export interface OpcEndpoint {
  url: string;
  security_policy_uri: string;
  security_mode: string;
  transport_profile_uri: string;
  security_level: number;
  user_token_policies: string[];
}
export interface OpcServer {
  application_name: string;
  application_uri: string;
  application_type: string;
  product_uri: string;
  discovery_urls: string[];
}

type Dir = "tx" | "rx" | "info" | "error";

function policyShort(uri: string): string {
  const i = uri.lastIndexOf("#");
  return i >= 0 ? uri.slice(i + 1) : uri || "None";
}

export function DiscoveryPanel({
  onClose,
  onSelect,
  onLog,
}: {
  onClose: () => void;
  onSelect: (url: string, policy: string, mode: string) => void;
  onLog?: (d: Dir, m: string) => void;
}) {
  const [url, setUrl] = useState("opc.tcp://localhost:4840");
  const [endpoints, setEndpoints] = useState<OpcEndpoint[]>([]);
  const [servers, setServers] = useState<OpcServer[]>([]);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");

  const run = async (fn: () => Promise<void>) => {
    setBusy(true); setErr("");
    try { await fn(); } catch (e) { setErr(String(e)); onLog?.("error", String(e)); }
    setBusy(false);
  };

  const doGetEndpoints = (target: string = url) => run(async () => {
    const eps = await invoke<OpcEndpoint[]>("get_endpoints", { url: target });
    setEndpoints(eps);
    onLog?.("rx", `Discovered ${eps.length} endpoints on ${target}`);
  });
  const doFindServers = () => run(async () => {
    const ss = (await invoke<OpcServer[]>("find_servers", { url })) ?? [];
    setServers(ss);
    onLog?.("rx", `Found ${ss.length} servers on ${url}`);
  });

  useEffect(() => { doGetEndpoints(); /* mount: enumerate */ /* eslint-disable-next-line */ }, []);

  const columns: Column[] = [
    { key: "security_policy_uri", label: "Policy", width: "150px" },
    { key: "security_mode", label: "Mode", width: "130px" },
    { key: "security_level", label: "Level", width: "60px" },
    { key: "url", label: "Endpoint URL" },
  ];

  const phone = useIsPhone();
  const modeOf = (e: OpcEndpoint) => e.security_mode.replace("MessageSecurityMode::", "");
  const pick = (e: OpcEndpoint) => {
    onSelect(e.url, policyShort(e.security_policy_uri), modeOf(e));
    onLog?.("info", `Selected ${e.url} (${policyShort(e.security_policy_uri)} / ${e.security_mode})`);
    onClose();
  };

  if (phone) {
    return (
      <Dialog open onClose={onClose} title="Find servers">
        <div className="oc-stack">
        <Field stacked label="Discovery URL">
          {(id) => (
            <input
              id={id}
              className="vt-input vt-input--mono"
              type="url"
              inputMode="url"
              autoCapitalize="off"
              autoCorrect="off"
              spellCheck={false}
              value={url}
              onChange={(e) => setUrl(e.target.value)}
              placeholder="opc.tcp://host:port"
            />
          )}
        </Field>
        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 8 }}>
          <Button variant="primary" loading={busy} onClick={() => doGetEndpoints()}>Get endpoints</Button>
          <Button disabled={busy} onClick={doFindServers}>Find servers</Button>
        </div>
        {err && <div className="vt-hint vt-hint--error" role="alert">{err}</div>}
        {servers.length > 0 && (
          <>
            <div className="vm-sheet-group">Servers</div>
            <ListGroup>
              {servers.map((s, i) => (
                <ListRow
                  key={i}
                  title={s.application_name || "Unnamed server"}
                  subtitle={s.discovery_urls[0] ?? s.application_uri}
                  chevron={!!s.discovery_urls[0]}
                  onClick={s.discovery_urls[0] ? () => { setUrl(s.discovery_urls[0]!); setServers([]); void doGetEndpoints(s.discovery_urls[0]!); } : undefined}
                />
              ))}
            </ListGroup>
          </>
        )}
        {endpoints.length > 0 ? (
          <>
            <div className="vm-sheet-group">Endpoints</div>
            <ListGroup>
              {endpoints.map((e, i) => (
                <ListRow
                  key={i}
                  title={`${policyShort(e.security_policy_uri)}`}
                  subtitle={`${modeOf(e) === "SignAndEncrypt" ? "Sign and encrypt" : modeOf(e)} · ${e.url}`}
                  trailing={<Badge>Level {e.security_level}</Badge>}
                  onClick={() => pick(e)}
                />
              ))}
            </ListGroup>
            <span className="vt-hint">Tap an endpoint to use its address and security settings.</span>
          </>
        ) : (
          !busy && !err && servers.length === 0 && <EmptyState compact title="No endpoints" description="Check the discovery URL, then try again." />
        )}
        </div>
      </Dialog>
    );
  }

  const rows = endpoints.map((e) => ({
    security_policy_uri: policyShort(e.security_policy_uri),
    security_mode: e.security_mode.replace("MessageSecurityMode::", ""),
    security_level: e.security_level,
    url: e.url,
    _raw: e,
  }));

  return (
    <Dialog open onClose={onClose} title="Endpoint discovery" width={760}>
      <div style={{ display: "flex", gap: 6 }}>
        <input
          className="vt-input vt-input--mono"
          style={{ flex: 1 }}
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          placeholder="opc.tcp://host:port"
          aria-label="Discovery URL"
        />
        <Button variant="primary" disabled={busy} onClick={() => doGetEndpoints()}>Endpoints</Button>
        <Button disabled={busy} onClick={doFindServers}>Servers</Button>
      </div>

      {err && <div className="vt-hint vt-hint--error" role="alert">{err}</div>}

      {servers.length > 0 && (
        <div style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)", maxHeight: 80, overflow: "auto" }}>
          {servers.map((s, i) => (
            <div key={i} style={{ padding: "3px 0", borderBottom: "1px solid var(--border)" }}>
              <strong style={{ color: "var(--text)" }}>{s.application_name || "(unnamed)"}</strong>
              {" "}<span style={{ color: "var(--text-dim)" }}>{s.application_uri}</span>
              {s.discovery_urls[0] && (
                <Button
                  size="sm"
                  variant="ghost"
                  style={{ marginLeft: 8 }}
                  onClick={() => { setUrl(s.discovery_urls[0]!); setServers([]); }}
                >Use URL</Button>
              )}
            </div>
          ))}
        </div>
      )}

      <div style={{ flex: 1, minHeight: 0, overflow: "auto" }}>
        <DataTable
          columns={columns}
          data={rows}
          emptyMessage={busy ? "Querying" : "No endpoints. Check the discovery URL."}
          renderCell={(key, row) => (key === "url" ? <span className="mono">{String(row[key] ?? "")}</span> : String(row[key] ?? ""))}
          onRowClick={(row) => pick((row as Record<string, unknown>)._raw as OpcEndpoint)}
        />
      </div>
      <span className="vt-hint">Click a row to load its URL and security policy into the connection form.</span>
    </Dialog>
  );
}
