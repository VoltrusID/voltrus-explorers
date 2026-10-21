// Protobuf schema registry UI: import .proto (file or inline), bind subjects
// to message types, test-decode a payload.

import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { FileUp, X } from "lucide-react";
import { Badge, Button, Callout, EmptyState, Field, IconButton } from "@mactools/ui";

import * as api from "../nats-client";

export function SchemaPanel({ addLog }: { addLog: (dir: "info" | "error", msg: string) => void }) {
  const [source, setSource] = useState("");
  const [rawMessages, setMessages] = useState<string[] | null>([]);
  const [rawBindings, setBindings] = useState<Record<string, string> | null>({});
  const [pattern, setPattern] = useState("");
  const [msgName, setMsgName] = useState("");
  const [testSubject, setTestSubject] = useState("");
  const [testPayload, setTestPayload] = useState("");
  const [testOut, setTestOut] = useState("");
  const [err, setErr] = useState("");

  const refresh = async () => {
    try {
      const [m, b] = await Promise.all([api.schemaMessageNames(), api.schemaBindings()]);
      setMessages(m);
      setBindings(b);
    } catch (e) {
      setErr(String(e));
    }
  };

  useEffect(() => {
    refresh();
  }, []);

  const onImportFile = async () => {
    const f = await open({ filters: [{ name: "Proto", extensions: ["proto"] }, { name: "All", extensions: ["*"] }] });
    if (typeof f !== "string") return;
    try {
      await api.schemaAddProtoFile(f);
      addLog("info", `Imported ${f}`);
      await refresh();
      setErr("");
    } catch (e) {
      setErr(String(e));
    }
  };

  const onImportSource = async () => {
    if (!source.trim()) return;
    try {
      await api.schemaAddProtoSource(source);
      addLog("info", "Imported inline .proto");
      setSource("");
      await refresh();
      setErr("");
    } catch (e) {
      setErr(String(e));
    }
  };

  const onBind = async () => {
    if (!pattern.trim() || !msgName) return;
    try {
      await api.schemaBind(pattern.trim(), msgName);
      setPattern("");
      await refresh();
    } catch (e) {
      setErr(String(e));
    }
  };

  const onUnbind = async (p: string) => {
    try {
      await api.schemaUnbind(p);
      await refresh();
    } catch (e) {
      setErr(String(e));
    }
  };

  const onTest = async () => {
    try {
      setTestOut(await api.schemaDecode(testSubject.trim(), testPayload));
      setErr("");
    } catch (e) {
      setTestOut("");
      setErr(String(e));
    }
  };

  const messages = rawMessages ?? [];
  const bindings = rawBindings ?? {};

  return (
    <div className="vt-stack" style={{ height: "100%", padding: 12, overflow: "auto" }}>
      <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
        <Button variant="primary" icon={<FileUp size={14} strokeWidth={1.75} />} onClick={onImportFile}>
          Import .proto file
        </Button>
        {err && <Callout tone="danger" style={{ flex: 1, minWidth: 200 }}>{err}</Callout>}
      </div>

      <details style={{ border: "1px solid var(--border)", borderRadius: "var(--radius)", padding: 10 }}>
        <summary style={{ cursor: "pointer", fontSize: "var(--fs-sm)", color: "var(--text-dim)" }}>Paste .proto source</summary>
        <textarea
          value={source}
          onChange={(e) => setSource(e.target.value)}
          rows={8}
          placeholder={'syntax = "proto3";\npackage acme;\nmessage Order { string id = 1; int64 amount = 2; }'}
          className="vt-textarea vt-textarea--mono"
          style={{ marginTop: 10, width: "100%", resize: "vertical" }}
        />
        <div style={{ marginTop: 8 }}>
          <Button size="sm" onClick={onImportSource} disabled={!source.trim()}>
            Import source
          </Button>
        </div>
      </details>

      <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
        <span style={{ fontSize: "var(--fs-sm)", fontWeight: 600, color: "var(--text)" }}>
          Messages <span style={{ fontWeight: 400, color: "var(--text-muted)" }}>({messages.length})</span>
        </span>
        <div
          style={{
            fontSize: "var(--fs-sm)",
            color: "var(--text-dim)",
            maxHeight: 120,
            overflow: "auto",
            border: "1px solid var(--border)",
            borderRadius: "var(--radius)",
            padding: 8,
            display: "flex",
            flexDirection: "column",
            gap: 2,
          }}
        >
          {messages.length === 0 ? (
            <span style={{ color: "var(--text-muted)" }}>No messages yet. Import a .proto file to list them.</span>
          ) : (
            messages.map((m) => (
              <div key={m} className="mono">
                {m}
              </div>
            ))
          )}
        </div>
      </div>

      <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
        <span style={{ fontSize: "var(--fs-sm)", fontWeight: 600, color: "var(--text)" }}>
          Subject bindings <span style={{ fontWeight: 400, color: "var(--text-muted)" }}>({Object.keys(bindings).length})</span>
        </span>
        {Object.keys(bindings).length === 0 ? (
          <span style={{ fontSize: "var(--fs-sm)", color: "var(--text-muted)" }}>
            No bindings yet. Bind a subject pattern to a message type below.
          </span>
        ) : (
          <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
            {Object.entries(bindings).map(([p, m]) => (
              <div key={p} style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: 8 }}>
                <span style={{ display: "flex", alignItems: "center", gap: 6, minWidth: 0 }}>
                  <Badge mono>{p}</Badge>
                  <span style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)" }}>bound to</span>
                  <span className="mono" style={{ fontSize: "var(--fs-sm)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{m}</span>
                </span>
                <IconButton size="sm" danger label={`Unbind ${p}`} onClick={() => onUnbind(p)}>
                  <X size={14} strokeWidth={1.75} />
                </IconButton>
              </div>
            ))}
          </div>
        )}
      </div>

      <div style={{ display: "flex", gap: 8, alignItems: "flex-end", flexWrap: "wrap", borderTop: "1px solid var(--border)", paddingTop: 10 }}>
        <div style={{ flex: 1, minWidth: 120 }}>
          <Field label="Subject pattern">
            {(id) => (
              <input
                id={id}
                className="vt-input vt-input--mono"
                value={pattern}
                onChange={(e) => setPattern(e.target.value)}
                placeholder="orders.*"
              />
            )}
          </Field>
        </div>
        <div style={{ flex: 1, minWidth: 140 }}>
          <Field label="Message">
            {(id) => (
              <select id={id} className="vt-select" value={msgName} onChange={(e) => setMsgName(e.target.value)}>
                <option value="">Select a message</option>
                {messages.map((m) => (
                  <option key={m} value={m}>
                    {m}
                  </option>
                ))}
              </select>
            )}
          </Field>
        </div>
        <Button variant="primary" onClick={onBind} disabled={!pattern.trim() || !msgName}>
          Bind
        </Button>
      </div>

      <details style={{ border: "1px solid var(--border)", borderRadius: "var(--radius)", padding: 10 }}>
        <summary style={{ cursor: "pointer", fontSize: "var(--fs-sm)", color: "var(--text-dim)" }}>Test decode</summary>
        <div style={{ display: "flex", flexDirection: "column", gap: 8, marginTop: 10 }}>
          <Field label="Subject">
            {(id) => (
              <input
                id={id}
                className="vt-input vt-input--mono"
                value={testSubject}
                onChange={(e) => setTestSubject(e.target.value)}
                placeholder="Must match a binding"
              />
            )}
          </Field>
          <Field label="Payload">
            {(id) => (
              <input
                id={id}
                className="vt-input vt-input--mono"
                value={testPayload}
                onChange={(e) => setTestPayload(e.target.value)}
                placeholder="Hex or text"
              />
            )}
          </Field>
          <div>
            <Button size="sm" onClick={onTest} disabled={!testSubject.trim()}>
              Decode
            </Button>
          </div>
          {testOut && (
            <pre
              className="mono"
              style={{
                margin: 0,
                padding: 8,
                background: "var(--sunken)",
                border: "1px solid var(--border)",
                borderRadius: "var(--radius)",
                fontSize: "var(--fs-sm)",
                whiteSpace: "pre-wrap",
                maxHeight: 200,
                overflow: "auto",
                color: "var(--text)",
              }}
            >
              {testOut}
            </pre>
          )}
        </div>
      </details>
    </div>
  );
}
