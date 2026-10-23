import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Button, Dialog, Field, IconButton, SheetSelect, openTextFile } from "@mactools/ui";
import { Plus, Trash2 } from "lucide-react";
import type { ProtoRule, ProtoSettings } from "./protoStore";

const PROTO_FILTER = { name: "Protocol Buffers schema", extensions: ["proto", "txt"] };

const hint: React.CSSProperties = {
  margin: 0,
  fontSize: "var(--fs-xs)",
  color: "var(--text-muted)",
  lineHeight: 1.4,
};

interface Props {
  open: boolean;
  onClose: () => void;
  settings: ProtoSettings;
  /** Install the schema and rules. Rejects with the compiler message when invalid. */
  onApply: (settings: ProtoSettings) => Promise<void>;
}

export function DecoderDialog({ open, onClose, settings, onApply }: Props) {
  const [source, setSource] = useState("");
  const [rules, setRules] = useState<ProtoRule[]>([]);
  const [messages, setMessages] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const check = async (text: string): Promise<string[]> => {
    try {
      const names = await invoke<string[]>("proto_inspect", { source: text });
      setMessages(names);
      setError(null);
      return names;
    } catch (e) {
      setMessages([]);
      setError(String(e));
      return [];
    }
  };

  useEffect(() => {
    if (!open) return;
    setSource(settings.source);
    setRules(settings.rules);
    setError(null);
    setMessages([]);
    if (settings.source.trim()) void check(settings.source);
  }, [open, settings]);

  const loadFile = async () => {
    try {
      const f = await openTextFile(PROTO_FILTER);
      if (!f) return;
      setSource(f.text);
      await check(f.text);
    } catch (e) {
      setError(`Could not read the file: ${String(e)}`);
    }
  };

  const apply = async (next: ProtoSettings) => {
    setBusy(true);
    try {
      await onApply(next);
      onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const addRule = () => setRules((r) => [...r, { filter: "", message: messages[0] ?? "" }]);
  const patchRule = (i: number, patch: Partial<ProtoRule>) =>
    setRules((r) => r.map((x, j) => (j === i ? { ...x, ...patch } : x)));

  return (
    <Dialog
      open={open}
      onClose={onClose}
      title="Payload decoders"
      width={560}
      footer={
        <>
          <Button size="sm" variant="ghost" onClick={() => void apply({ source: "", rules: [] })} disabled={busy}>
            Remove schema
          </Button>
          <Button size="sm" variant="ghost" onClick={onClose}>Cancel</Button>
          <Button
            size="sm"
            variant="primary"
            disabled={busy || rules.some((r) => !r.filter.trim() || !r.message)}
            onClick={() => void apply({ source, rules })}
          >
            Apply
          </Button>
        </>
      }
    >
      <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
        <div>
          <strong style={{ fontSize: "var(--fs-sm)" }}>Sparkplug B</strong>
          <p style={hint}>
            Topics under <span className="mono">spBv1.0/</span> are decoded automatically. Aliases announced in BIRTH
            messages are used to name metrics in later DATA messages. Choose Decoded in the payload format menu to see
            the result.
          </p>
        </div>

        <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
          <strong style={{ fontSize: "var(--fs-sm)" }}>Protobuf</strong>
          <p style={hint}>
            Paste a .proto file (or open one) and say which topics carry which message. Imports of other files are
            not supported; Google well-known types such as timestamp.proto are. New messages are decoded with the
            current schema, and messages already received on connections are decoded again when you apply.
          </p>
          <Field label="Schema (.proto)">
            <textarea
              className="vt-textarea vt-textarea--mono"
              style={{ minHeight: 140, resize: "vertical" }}
              spellCheck={false}
              placeholder={'syntax = "proto3";\npackage acme;\n\nmessage Reading {\n  string sensor = 1;\n  double value = 2;\n}'}
              value={source}
              onChange={(e) => setSource(e.target.value)}
            />
          </Field>
          <div style={{ display: "flex", gap: 6 }}>
            <Button size="sm" onClick={() => void loadFile()}>Open .proto file…</Button>
            <Button size="sm" onClick={() => void check(source)} disabled={!source.trim()}>Check schema</Button>
          </div>
          {error && (
            <pre
              role="alert"
              style={{
                margin: 0,
                padding: 8,
                whiteSpace: "pre-wrap",
                fontSize: "var(--fs-xs)",
                color: "var(--danger, #ef4444)",
                background: "var(--card)",
                border: "1px solid var(--border)",
                borderRadius: "var(--radius-sm)",
                maxHeight: 120,
                overflow: "auto",
              }}
            >
              {error}
            </pre>
          )}
          {!error && messages.length > 0 && (
            <p style={hint}>
              {messages.length} message type{messages.length === 1 ? "" : "s"}: {messages.join(", ")}
            </p>
          )}
        </div>

        <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
          <strong style={{ fontSize: "var(--fs-sm)" }}>Topics</strong>
          <p style={hint}>The first matching rule is used. Filters accept + and # wildcards.</p>
          {rules.map((r, i) => (
            <div key={i} style={{ display: "flex", gap: 4 }}>
              <input
                className="vt-input vt-input--sm vt-input--mono"
                style={{ flex: 1, minWidth: 0 }}
                placeholder="sensors/#"
                value={r.filter}
                onChange={(e) => patchRule(i, { filter: e.target.value })}
                aria-label="Topic filter"
              />
              <SheetSelect
                size="sm"
                sheetTitle="Message type"
                style={{ flex: 1, minWidth: 0 }}
                value={r.message}
                options={messages.map((m) => ({ value: m, label: m }))}
                placeholder={r.message || "Choose a message"}
                onChange={(v) => patchRule(i, { message: v })}
                aria-label="Message type"
              />
              <IconButton label="Remove rule" size="sm" danger onClick={() => setRules((x) => x.filter((_, j) => j !== i))}>
                <Trash2 size={16} strokeWidth={1.75} />
              </IconButton>
            </div>
          ))}
          <Button
            size="sm"
            variant="ghost"
            icon={<Plus size={14} strokeWidth={1.75} />}
            style={{ alignSelf: "flex-start" }}
            disabled={messages.length === 0}
            onClick={addRule}
          >
            Add topic rule
          </Button>
        </div>
      </div>
    </Dialog>
  );
}
