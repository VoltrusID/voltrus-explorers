// KV bucket + key browser. Create/delete buckets, put/get/delete keys,
// live watch via the `kv-update` event.

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { Database, Plus, RefreshCw, Trash2, X } from "lucide-react";
import { Badge, Button, Callout, DataTable, EmptyState, Field, IconButton, Panel, SheetSelect, type Column } from "@mactools/ui";

import * as api from "../nats-client";
import type { NatsKvEntry } from "../nats-types";
import { PayloadViewer } from "./PayloadViewer";

export function KvPanel({
  connected,
  addLog,
}: {
  connected: boolean;
  addLog: (dir: "info" | "error", msg: string) => void;
}) {
  const [buckets, setBuckets] = useState<string[]>([]);
  const [bucket, setBucket] = useState("");
  const [newBucket, setNewBucket] = useState("");
  const [history, setHistory] = useState(1);
  const [keys, setKeys] = useState<NatsKvEntry[]>([]);
  const [selKey, setSelKey] = useState<NatsKvEntry | null>(null);
  const [putKey, setPutKey] = useState("");
  const [putVal, setPutVal] = useState("");
  const [watchId, setWatchId] = useState<string | null>(null);
  const [err, setErr] = useState("");

  const refreshBuckets = async () => {
    try {
      const b = await api.kvList();
      setBuckets(b);
      if (!bucket && b.length) setBucket(b[0]);
    } catch (e) {
      setErr(String(e));
    }
  };

  useEffect(() => {
    if (connected) refreshBuckets();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [connected]);

  const refreshKeys = async () => {
    if (!bucket) return;
    try {
      const ks = await api.kvKeys(bucket);
      const entries = await Promise.all(ks.map((k) => api.kvGet(bucket, k)));
      setKeys(entries.filter(Boolean) as NatsKvEntry[]);
      setErr("");
    } catch (e) {
      setErr(String(e));
      setKeys([]);
    }
  };

  useEffect(() => {
    if (bucket) refreshKeys();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [bucket]);

  // Live watch on selected bucket.
  useEffect(() => {
    let un: UnlistenFn | null = null;
    let active = watchId;
    if (!connected || !bucket) return;
    api.kvWatchSubscribe(bucket, null)
      .then((id) => {
        setWatchId(id);
        active = id;
        listen<NatsKvEntry>("kv-update", (e) => {
          if (e.payload?.bucket === bucket) refreshKeys();
        }).then((u) => (un = u));
      })
      .catch(() => {});
    return () => {
      un?.();
      if (active) invoke("client_unsubscribe", { sid: active }).catch(() => {});
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [connected, bucket]);

  const onCreate = async () => {
    if (!newBucket.trim()) return;
    try {
      await api.kvCreate(newBucket.trim(), history, 0);
      addLog("info", `Created KV bucket ${newBucket}`);
      setNewBucket("");
      await refreshBuckets();
      setBucket(newBucket.trim());
    } catch (e) {
      setErr(String(e));
    }
  };

  const onDeleteBucket = async () => {
    if (!bucket) return;
    if (!confirm(`Delete bucket "${bucket}" and all its keys?`)) return;
    try {
      await api.kvDeleteBucket(bucket);
      addLog("info", `Deleted KV bucket ${bucket}`);
      setBucket("");
      await refreshBuckets();
    } catch (e) {
      setErr(String(e));
    }
  };

  const onPut = async () => {
    if (!bucket || !putKey.trim()) return;
    try {
      await api.kvPut(bucket, putKey.trim(), putVal);
      addLog("info", `KV put ${putKey}`);
      setPutKey("");
      setPutVal("");
      await refreshKeys();
    } catch (e) {
      setErr(String(e));
    }
  };

  const onDeleteKey = async (key: string) => {
    if (!bucket) return;
    try {
      await api.kvDeleteKey(bucket, key);
      addLog("info", `KV delete ${key}`);
      await refreshKeys();
      if (selKey?.key === key) setSelKey(null);
    } catch (e) {
      setErr(String(e));
    }
  };

  const keyCols: Column[] = [
    { key: "key", label: "Key" },
    { key: "revision", label: "Rev", width: "70px" },
    { key: "operation", label: "Op", width: "60px" },
  ];
  const keyRows = keys.map((k) => ({
    key: k.key,
    revision: k.revision ?? "",
    operation: k.operation ?? "",
    _raw: k,
  }));

  if (!connected) {
    return (
      <div style={{ height: "100%", display: "flex", flexDirection: "column" }}>
        <EmptyState
          icon={<Database size={18} strokeWidth={1.75} />}
          title="Not connected"
          description="Connect to a server in Client mode to browse and edit KV buckets."
        />
      </div>
    );
  }

  return (
    <div className="vt-stack" style={{ height: "100%", padding: 12, overflow: "auto" }}>
      <div style={{ display: "flex", gap: 8, alignItems: "flex-end", flexWrap: "wrap" }}>
        <Field label="Bucket">
          {(id) => (
            <SheetSelect
              id={id}
              size="sm"
              sheetTitle="Bucket"
              style={{ minWidth: 160 }}
              value={bucket}
              options={buckets.map((b) => ({ value: b, label: b }))}
              placeholder="Select a bucket"
              onChange={(v) => setBucket(v)}
            />
          )}
        </Field>
        <Button size="sm" variant="ghost" icon={<RefreshCw size={14} strokeWidth={1.75} />} onClick={refreshBuckets}>
          Refresh
        </Button>
        <Button size="sm" variant="danger" icon={<Trash2 size={14} strokeWidth={1.75} />} onClick={onDeleteBucket} disabled={!bucket}>
          Delete bucket
        </Button>
        {watchId && <Badge tone="success">Watching</Badge>}
      </div>

      <details style={{ border: "1px solid var(--border)", borderRadius: "var(--radius)", padding: 10 }}>
        <summary style={{ cursor: "pointer", fontSize: "var(--fs-sm)", color: "var(--text-dim)" }}>Create bucket</summary>
        <div style={{ display: "flex", gap: 8, alignItems: "flex-end", marginTop: 10, flexWrap: "wrap" }}>
          <Field label="Name">
            {(id) => (
              <input
                id={id}
                className="vt-input vt-input--sm"
                style={{ minWidth: 140 }}
                value={newBucket}
                onChange={(e) => setNewBucket(e.target.value)}
                placeholder="my-bucket"
              />
            )}
          </Field>
          <Field label="History">
            {(id) => (
              <input
                id={id}
                className="vt-input vt-input--sm"
                style={{ width: 70 }}
                type="number"
                value={history}
                onChange={(e) => setHistory(Number(e.target.value))}
              />
            )}
          </Field>
          <Button size="sm" variant="primary" icon={<Plus size={14} strokeWidth={1.75} />} onClick={onCreate}>
            Create
          </Button>
        </div>
      </details>

      {err && <Callout tone="danger">{err}</Callout>}

      <div style={{ flex: 1, minHeight: 160, display: "flex", flexDirection: "column" }}>
        <DataTable
          columns={keyCols}
          data={keyRows}
          selectable
          emptyMessage={bucket ? "No keys in this bucket yet. Put one below." : "Select or create a bucket to browse its keys."}
          onRowClick={(row) => setSelKey(row._raw as NatsKvEntry)}
          renderCell={(k, row) => {
            // DataTable uses renderCell for every column; keep the default text for data columns.
            if (k !== "key") return String(row[k] ?? "");
            // Rows are built in keyRows above; _raw carries the KV entry.
            const entry = row["_raw"] as NatsKvEntry | undefined;
            if (!entry) return String(row[k] ?? "");
            return (
              <span style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: 6 }}>
                <span className="mono">{entry.key}</span>
                <IconButton
                  size="sm"
                  danger
                  label={`Delete key ${entry.key}`}
                  onClick={() => onDeleteKey(entry.key)}
                >
                  <X size={14} strokeWidth={1.75} />
                </IconButton>
              </span>
            );
          }}
        />
      </div>

      <div style={{ display: "flex", gap: 8, alignItems: "flex-end", flexWrap: "wrap" }}>
        <div style={{ flex: 1, minWidth: 120 }}>
          <Field label="Key">
            {(id) => (
              <input
                id={id}
                className="vt-input vt-input--mono"
                value={putKey}
                onChange={(e) => setPutKey(e.target.value)}
                placeholder="new.key"
              />
            )}
          </Field>
        </div>
        <div style={{ flex: 2, minWidth: 160 }}>
          <Field label="Value">
            {(id) => (
              <input
                id={id}
                className="vt-input"
                value={putVal}
                onChange={(e) => setPutVal(e.target.value)}
                placeholder="Value (text or hex)"
              />
            )}
          </Field>
        </div>
        <Button variant="primary" onClick={onPut} disabled={!bucket || !putKey.trim()}>
          Put
        </Button>
      </div>

      {selKey && (
        <Panel
          title={
            <span className="mono">
              {selKey.bucket}/{selKey.key}
            </span>
          }
          actions={
            <IconButton size="sm" label="Close details" onClick={() => setSelKey(null)}>
              <X size={14} strokeWidth={1.75} />
            </IconButton>
          }
        >
          <div style={{ padding: 10 }}>
            <div style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)", marginBottom: 6 }}>
              Revision {selKey.revision}
            </div>
            <PayloadViewer payload={selKey.payload ?? ""} maxHeight={180} />
          </div>
        </Panel>
      )}
    </div>
  );
}

