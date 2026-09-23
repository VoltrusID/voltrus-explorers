import { useEffect, useState } from "react";
import { useCloudStore } from "../../store/cloud";
import type { AiProvider } from "../../api/ai-bridge";

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
      <div style={{ padding: "0 12px 10px", display: "flex", flexDirection: "column", gap: 6, fontSize: 11 }}>
        <div style={{ fontWeight: 600 }}>AI Keys</div>
        <div style={{ color: "var(--text-muted)" }}>Sign in to manage cloud AI keys.</div>
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
    <div style={{ padding: "0 12px 10px", display: "flex", flexDirection: "column", gap: 6, fontSize: 11 }}>
      <div style={{ fontWeight: 600 }}>AI Keys</div>
      {error && (
        <div role="alert" style={{ color: "var(--danger)" }}>
          {error}
        </div>
      )}
      {aiKeys.length === 0 ? (
        <div style={{ color: "var(--text-muted)" }}>
          No keys stored. Add one to use AI chat without a local API key.
        </div>
      ) : (
        <ul className="file-list">
          {aiKeys.map((k) => (
            <li key={k.provider} className="file-item" style={{ justifyContent: "space-between" }}>
              <span title={`Updated ${k.updatedAt}`}>
                <span style={{ textTransform: "capitalize" }}>{k.provider}</span>
                <span style={{ color: "var(--text-muted)", marginLeft: 6 }}>{k.masked}</span>
              </span>
              <button
                className="toolbar-btn danger"
                style={{ fontSize: 10, padding: "0 6px" }}
                disabled={busy}
                onClick={() => remove(k.provider as AiProvider)}
              >
                Delete
              </button>
            </li>
          ))}
        </ul>
      )}

      <div style={{ display: "flex", gap: 4 }}>
        <select
          value={provider}
          onChange={(e) => setProvider(e.target.value as AiProvider)}
          style={{ fontSize: 11, padding: "2px 4px", background: "var(--bg-highlight)", color: "var(--text-primary)", border: "1px solid var(--border)", borderRadius: 4 }}
        >
          {PROVIDERS.map((p) => (
            <option key={p} value={p}>
              {p}
            </option>
          ))}
        </select>
        <input
          type="password"
          placeholder={provider === "claude" ? "sk-ant-api03-..." : "optional key"}
          value={key}
          onChange={(e) => setKey(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") submit();
          }}
          style={{ flex: 1, fontSize: 11, padding: "2px 4px", minWidth: 0 }}
        />
        <button
          className="toolbar-btn"
          style={{ fontSize: 10, padding: "0 8px" }}
          disabled={busy || key.trim().length === 0}
          onClick={submit}
        >
          Save
        </button>
      </div>
    </div>
  );
}
