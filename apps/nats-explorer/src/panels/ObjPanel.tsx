// Object Store bucket + object browser. Upload/download via file dialog,
// inline put, delete.

import { useEffect, useState } from "react";
import { open, save } from "@tauri-apps/plugin-dialog";
import { Archive, Download, Plus, RefreshCw, Trash2, Upload, X } from "lucide-react";
import { Button, Callout, DataTable, EmptyState, Field, IconButton, Panel, SheetSelect, type Column } from "@mactools/ui";

import * as api from "../nats-client";
import type { NatsObjectInfo } from "../nats-types";
import { fmtBytes } from "./shared";
import { PayloadViewer } from "./PayloadViewer";

export function ObjPanel({
  connected,
  addLog,
}: {
  connected: boolean;
  addLog: (dir: "info" | "error", msg: string) => void;
}) {
  const [buckets, setBuckets] = useState<string[]>([]);
  const [bucket, setBucket] = useState("");
  const [newBucket, setNewBucket] = useState("");
  const [objs, setObjs] = useState<NatsObjectInfo[]>([]);
  const [selObj, setSelObj] = useState<NatsObjectInfo | null>(null);
  const [selPayload, setSelPayload] = useState("");
  const [putName, setPutName] = useState("");
  const [putVal, setPutVal] = useState("");
  const [err, setErr] = useState("");

  const refreshBuckets = async () => {
    try {
      const b = await api.objList();
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

  const refreshObjs = async () => {
    if (!bucket) return;
    try {
      setObjs(await api.objListObjects(bucket));
      setErr("");
    } catch (e) {
      setErr(String(e));
      setObjs([]);
    }
  };

  useEffect(() => {
    if (bucket) refreshObjs();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [bucket]);

  const onCreate = async () => {
    if (!newBucket.trim()) return;
    try {
      await api.objCreate(newBucket.trim());
      addLog("info", `Created Object Store ${newBucket}`);
      setNewBucket("");
      await refreshBuckets();
      setBucket(newBucket.trim());
    } catch (e) {
      setErr(String(e));
    }
  };

  const onDeleteBucket = async () => {
    if (!bucket || !confirm(`Delete object store "${bucket}"?`)) return;
    try {
      await api.objDeleteBucket(bucket);
      addLog("info", `Deleted Object Store ${bucket}`);
      setBucket("");
      await refreshBuckets();
    } catch (e) {
      setErr(String(e));
    }
  };

  const onSelect = async (o: NatsObjectInfo) => {
    setSelObj(o);
    try {
      setSelPayload(await api.objGet(bucket, o.name));
    } catch (e) {
      setSelPayload("");
      setErr(String(e));
    }
  };

  const onUpload = async () => {
    if (!bucket) return;
    const f = await open({ multiple: false });
    if (typeof f !== "string") return;
    const name = putName.trim() || f.split(/[\\/]/).pop() || "object";
    try {
      await api.objUpload(bucket, name, f);
      addLog("info", `Uploaded ${name}`);
      setPutName("");
      await refreshObjs();
    } catch (e) {
      setErr(String(e));
    }
  };

  const onDownload = async (o: NatsObjectInfo) => {
    const f = await save({ defaultPath: o.name });
    if (typeof f !== "string") return;
    try {
      await api.objDownload(bucket, o.name, f);
      addLog("info", `Downloaded ${o.name} to ${f}`);
    } catch (e) {
      setErr(String(e));
    }
  };

  const onPutInline = async () => {
    if (!bucket || !putName.trim()) return;
    try {
      await api.objPutPayload(bucket, putName.trim(), putVal);
      addLog("info", `Put object ${putName}`);
      setPutName("");
      setPutVal("");
      await refreshObjs();
    } catch (e) {
      setErr(String(e));
    }
  };

  const onDelete = async (name: string) => {
    if (!bucket || !confirm(`Delete object "${name}"?`)) return;
    try {
      await api.objDelete(bucket, name);
      addLog("info", `Deleted object ${name}`);
      await refreshObjs();
      if (selObj?.name === name) setSelObj(null);
    } catch (e) {
      setErr(String(e));
    }
  };

  const cols: Column[] = [
    { key: "name", label: "Object" },
    { key: "size", label: "Size", width: "80px" },
    { key: "chunks", label: "Chunks", width: "70px" },
    { key: "digest", label: "Digest", width: "120px" },
  ];
  const rows = objs.map((o) => ({
    name: o.name,
    size: fmtBytes(o.size),
    chunks: o.chunks ?? "",
    digest: (o.digest ?? "").slice(0, 16),
    _raw: o,
  }));

  if (!connected) {
    return (
      <div style={{ height: "100%", display: "flex", flexDirection: "column" }}>
        <EmptyState
          icon={<Archive size={18} strokeWidth={1.75} />}
          title="Not connected"
          description="Connect to a server in Client mode to browse object stores."
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
        <Button size="sm" variant="ghost" icon={<RefreshCw size={14} strokeWidth={1.75} />} onClick={refreshObjs} disabled={!bucket}>
          Refresh
        </Button>
        <Button size="sm" variant="danger" icon={<Trash2 size={14} strokeWidth={1.75} />} onClick={onDeleteBucket} disabled={!bucket}>
          Delete bucket
        </Button>
      </div>

      <details style={{ border: "1px solid var(--border)", borderRadius: "var(--radius)", padding: 10 }}>
        <summary style={{ cursor: "pointer", fontSize: "var(--fs-sm)", color: "var(--text-dim)" }}>Create bucket</summary>
        <div style={{ display: "flex", gap: 8, alignItems: "flex-end", marginTop: 10 }}>
          <Field label="Name">
            {(id) => (
              <input
                id={id}
                className="vt-input vt-input--sm"
                style={{ minWidth: 140 }}
                value={newBucket}
                onChange={(e) => setNewBucket(e.target.value)}
                placeholder="my-store"
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
          columns={cols}
          data={rows}
          selectable
          emptyMessage={bucket ? "No objects in this store yet. Upload a file or put one inline below." : "Select or create a bucket to browse its objects."}
          onRowClick={(row) => onSelect(row._raw as NatsObjectInfo)}
          renderCell={(k, row) => {
            // DataTable uses renderCell for every column; keep the default text for data columns.
            if (k !== "name") return String(row[k] ?? "");
            // Rows are built in rows above; _raw carries the object info.
            const obj = row["_raw"] as NatsObjectInfo | undefined;
            if (!obj) return String(row[k] ?? "");
            return (
              <span style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: 6 }}>
                <span className="mono">{obj.name}</span>
                <span style={{ display: "flex", gap: 3 }}>
                  <IconButton size="sm" label={`Download ${obj.name}`} onClick={() => onDownload(obj)}>
                    <Download size={14} strokeWidth={1.75} />
                  </IconButton>
                  <IconButton size="sm" danger label={`Delete ${obj.name}`} onClick={() => onDelete(obj.name)}>
                    <X size={14} strokeWidth={1.75} />
                  </IconButton>
                </span>
              </span>
            );
          }}
        />
      </div>

      <div style={{ display: "flex", gap: 8, alignItems: "flex-end", flexWrap: "wrap" }}>
        <div style={{ flex: 1, minWidth: 120 }}>
          <Field label="Object name" hint="Optional. Defaults to the uploaded file name.">
            {(id) => (
              <input
                id={id}
                className="vt-input vt-input--mono"
                value={putName}
                onChange={(e) => setPutName(e.target.value)}
              />
            )}
          </Field>
        </div>
        <Button variant="primary" icon={<Upload size={14} strokeWidth={1.75} />} onClick={onUpload} disabled={!bucket}>
          Upload file
        </Button>
      </div>
      <div style={{ display: "flex", gap: 8, alignItems: "flex-end", flexWrap: "wrap" }}>
        <div style={{ flex: 1, minWidth: 160 }}>
          <Field label="Inline value">
            {(id) => (
              <input
                id={id}
                className="vt-input"
                value={putVal}
                onChange={(e) => setPutVal(e.target.value)}
                placeholder="Text or hex"
              />
            )}
          </Field>
        </div>
        <Button variant="ghost" onClick={onPutInline} disabled={!bucket || !putName.trim()}>
          Put inline
        </Button>
      </div>

      {selObj && (
        <Panel
          title={
            <span className="mono">
              {selObj.bucket}/{selObj.name}
            </span>
          }
          actions={
            <IconButton size="sm" label="Close details" onClick={() => setSelObj(null)}>
              <X size={14} strokeWidth={1.75} />
            </IconButton>
          }
        >
          <div style={{ padding: 10 }}>
            <div style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)", marginBottom: 6 }}>
              {fmtBytes(selObj.size)}
            </div>
            <PayloadViewer payload={selPayload} maxHeight={160} />
          </div>
        </Panel>
      )}
    </div>
  );
}
