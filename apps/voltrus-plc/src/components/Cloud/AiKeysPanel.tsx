import { useEffect, useState } from "react";
import { Badge, Button, Callout, EmptyState } from "@mactools/ui";
import { KeyRound } from "lucide-react";
import { useCloudStore } from "../../store/cloud";
import type { AiProvider } from "../../api/ai-bridge";
import "./cloud.css";

const PROVIDERS: AiProvider[] = ["claude", "ollama"];

// AI key management for cloud mode (PLC-22): the list/set/delete UI for the
// server-side provider keys the cloud worker's AI proxy uses. Keys are
// write-only from the client — the worker only ever returns masked values,
// so a saved key can be replaced or removed but never read back.
export default function AiKeysPanel() {
  const aiKeys = useCloudStore((s) => s.aiKeys);
  const token = useCloudStore((s) => s.token);
  const error = useCloudStore((s) => s.error);
  const fetchAiKeys = useCloudStore((s) => s.fetchAiKeys);
  const saveAiKey = useCloudStore((s) => s.saveAiKey);
  const deleteAiKey = useCloudStore((s) => s.deleteAiKey);

  const [provider, setProvider] = useState<AiProvider>("claude");
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (token) void fetchAiKeys();
  }, [token, fetchAiKeys]);

  if (!token) {
    return (
      <div className="cld-panel">
        <div className="cld-panel__title">AI keys</div>
        <div className="cld-hint">Sign in to manage cloud AI keys.</div>
      </div>
    );
  }

  const submit = () => {
    const trimmed = key.trim();
    if (!trimmed || busy) return;
    setBusy(true);
    void saveAiKey(provider, trimmed).finally(() => {
      setBusy(false);
      setKey("");
    });
  };

  const remove = (p: AiProvider) => {
    if (busy) return;
    setBusy(true);
    void deleteAiKey(p).finally(() => setBusy(false));
  };

  return (
    <div className="cld-panel">
      <div className="cld-panel__title">AI keys</div>
      {error && (
        <Callout tone="danger" icon={<KeyRound size={14} strokeWidth={1.75} />}>
          {error}
        </Callout>
      )}
      {aiKeys.length === 0 ? (
        <EmptyState
          compact
          title="No keys stored"
          description="Add one to use AI chat without a local API key."
        />
      ) : (
        <ul className="cld-list file-list">
          {aiKeys.map((k) => (
            <li key={k.provider} className="file-item" style={{ justifyContent: "space-between" }}>
              <span title={`Updated ${k.updatedAt}`} style={{ minWidth: 0 }}>
                <span className="cld-key-name">{k.provider}</span>
                <Badge mono>{k.masked}</Badge>
              </span>
              <Button
                size="sm"
                variant="danger"
                disabled={busy}
                onClick={() => remove(k.provider as AiProvider)}
              >
                Delete
              </Button>
            </li>
          ))}
        </ul>
      )}

      <div className="cld-form-row">
        <select
          className="vt-select vt-select--sm"
          style={{ width: "auto" }}
          value={provider}
          onChange={(e) => setProvider(e.target.value as AiProvider)}
          aria-label="AI provider"
        >
          {PROVIDERS.map((p) => (
            <option key={p} value={p}>
              {p}
            </option>
          ))}
        </select>
        <input
          className="vt-input vt-input--sm cld-grow"
          type="password"
          placeholder={provider === "claude" ? "sk-ant-api03-..." : "optional key"}
          aria-label="API key"
          value={key}
          onChange={(e) => setKey(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") submit();
          }}
        />
        <Button
          size="sm"
          variant="primary"
          disabled={busy || key.trim().length === 0}
          onClick={submit}
        >
          Save
        </Button>
      </div>
    </div>
  );
}
